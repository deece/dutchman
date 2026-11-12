//! `dutchman convert-all`: many native files converted in one run, several at a time.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use dutchman_core::{Format, Writer};
use serde_json::{json, Value};

use crate::json as report;
use crate::worker::{self, Limits};
use crate::{ConvertOptions, PartSearch};

/// The schema of `dutchman convert-all --json`.
pub const SCHEMA: &str = "dutchman.convert-all/1";

/// This Dutchman's version, which a journal's records must match.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The journal's own format. A report recorded in an older one may lack what this version
/// records (the names an anonymous report takes out, among others), so its file is converted
/// again, as one recorded by another version of Dutchman is.
const JOURNAL_FORMAT: u64 = 2;

/// The most files one run converts.
const MAX_FILES: usize = 10_000;

/// How deep folders are searched.
const MAX_DEPTH: usize = 32;

/// The most files converted at once by default.
const DEFAULT_JOBS: usize = 8;

/// One file to convert, and where its design goes.
#[derive(Clone, Debug)]
pub struct Job {
    /// Keeps privately extracted project files alive through conversion and retries.
    pub project_files: Option<std::sync::Arc<tempfile::TempDir>>,
    /// Same-name STEP export, used only if the native conversion fails.
    pub fallback: Option<PathBuf>,
    pub input: PathBuf,
    pub output: PathBuf,
    /// Whether an existing design at `output` is replaced.
    pub replace: bool,
    /// Where an assembly's part files are looked for: in the folder given that holds the
    /// file as its project, unless the options name one.
    pub search: PartSearch,
}

/// What happens to a job while a batch runs.
#[derive(Debug)]
pub enum Event {
    /// The job at this index started converting.
    Started(usize),
    /// A stage, document count, or heartbeat for this job.
    Progress(usize, dutchman_core::ProgressEvent),
    /// The job at this index finished, with its `dutchman.convert/1` report.
    Finished(usize, Value),
}

/// How a batch runs.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// How many files to convert at once (default: [`workers`]).
    pub jobs: Option<NonZeroUsize>,
    /// The program each file is converted in, as a child process of its own within
    /// `limits`; `None` converts in this process, without limits.
    pub worker: Option<PathBuf>,
    /// What one file's conversion may use, when converted in a child.
    pub limits: Limits,
    /// A file that records each file's report as it finishes; files it already records, and
    /// that have not changed since, are not converted again.
    pub journal: Option<PathBuf>,
}

impl Settings {
    /// Settings that convert each file in a child process of the running program.
    pub fn isolated() -> Result<Self, String> {
        let program = std::env::current_exe()
            .map_err(|err| format!("cannot find the running program: {err}"))?;
        Ok(Settings {
            worker: Some(program),
            ..Settings::default()
        })
    }
}

/// Sets planned `jobs` to replace designs and look for part files as `options` say: in the
/// options' project folder when they name one, else in the folder given that holds each file,
/// then in the options' other folders. Numbered saves keep existing output files.
pub fn prepare(jobs: &mut [Job], options: &ConvertOptions) {
    let extension = options.output_extension();
    for job in jobs {
        let _ = job.output.set_extension(extension);
        job.replace = !options.no_clobber && Format::part_revision(&job.input).is_none();
        if options.project.is_some() {
            job.search.project.clone_from(&options.project);
        }
        job.search.also.clone_from(&options.search);
        job.search.threads = options.threads;
        job.search.part_positions = options.effective_part_positions();
    }
}

/// Converts every file `inputs` names or holds into `out_dir` as `settings` say, writing
/// nothing outside `out_dir` (and the journal) and replacing designs already there unless
/// `options` turn that off, and returns the `dutchman.convert-all/1` report. `Err` when the
/// journal cannot be read or written.
pub fn convert_all(
    inputs: &[PathBuf],
    out_dir: &Path,
    options: &ConvertOptions,
    settings: &Settings,
) -> Result<Value, String> {
    convert_all_with_progress(inputs, out_dir, options, settings, &|_, _| {})
}

/// Converts a batch, forwarding progress for each input without changing its reports.
pub fn convert_all_with_progress(
    inputs: &[PathBuf],
    out_dir: &Path,
    options: &ConvertOptions,
    settings: &Settings,
    on_progress: &(dyn Fn(&Path, dutchman_core::ProgressEvent) + Send + Sync),
) -> Result<Value, String> {
    let owns_log = options.diagnostics_default && options.diagnostics.is_none();
    let mut prepared = options.clone();
    crate::diagnostics::prepare(&mut prepared, workers(settings.jobs))?;
    let result = convert_all_prepared(inputs, out_dir, &prepared, settings, on_progress);
    if owns_log {
        if let Some(path) = &prepared.diagnostics {
            crate::diagnostics::finish_default_log(
                path,
                if result
                    .as_ref()
                    .is_ok_and(|r| r.get("ok") == Some(&Value::Bool(true)))
                {
                    "converted"
                } else {
                    "failed"
                },
            );
        }
    }
    result
}

fn convert_all_prepared(
    inputs: &[PathBuf],
    out_dir: &Path,
    options: &ConvertOptions,
    settings: &Settings,
    on_progress: &(dyn Fn(&Path, dutchman_core::ProgressEvent) + Send + Sync),
) -> Result<Value, String> {
    if options.to == crate::OutputFormat::Dxf {
        return Err(
            "DXF page output uses dutchman convert --to dxf; batch and watch are not supported yet"
                .into(),
        );
    }
    let Plan {
        jobs: mut work,
        problems,
        skipped,
        skipped_by_extension,
        skipped_revisions,
        description: project_plan,
    } = plan(inputs, out_dir);
    prepare(&mut work, options);
    let mut journal = match &settings.journal {
        Some(path) => Some(Journal::open(path)?),
        None => None,
    };
    // Files the journal already records are not converted again.
    let mut results: Vec<Option<Value>> = work
        .iter()
        .map(|job| {
            journal
                .as_ref()
                .and_then(|j| j.recorded(job))
                .filter(|report| {
                    let mut choices = options.choices();
                    let full = crate::choices::estimate_with_options(&job.input, &choices, options);
                    if let crate::choices::MemoryCheck::Ok(estimate) =
                        crate::choices::memory_check(&job.input, options)
                    {
                        if estimate.map(|e| e.peak_bytes) != full.map(|e| e.peak_bytes) {
                            choices.history = false;
                        }
                    }
                    report.get("choices") == Some(&report::placement_choices(options, choices))
                })
        })
        .collect();
    let resumed = results.iter().flatten().count();
    let todo: Vec<usize> = (0..work.len())
        .filter(|&i| results.get(i).is_some_and(Option::is_none))
        .collect();
    let pending: Vec<Job> = todo.iter().filter_map(|&i| work.get(i).cloned()).collect();
    let fresh: Mutex<Vec<Option<Value>>> = Mutex::new(vec![None; pending.len()]);
    let journal_error: Mutex<Option<String>> = Mutex::new(None);
    let journal_lock = Mutex::new(journal.take());
    let convert = |job: &Job, _: usize| {
        let callback = |event| on_progress(&job.input, event);
        match &settings.worker {
            Some(program) => convert_isolated_job_with_progress(
                program,
                job,
                options,
                &settings.limits,
                None,
                &callback,
            ),
            None => convert_options_job_with_progress(job, options, &callback),
        }
    };
    run_with(
        &pending,
        workers(settings.jobs),
        &AtomicBool::new(false),
        convert,
        |event| {
            if let Event::Finished(index, report) = event {
                if let (Ok(mut journal), Some(job)) = (journal_lock.lock(), pending.get(index)) {
                    if let Some(journal) = journal.as_mut() {
                        if let Err(err) = journal.record(job, &report) {
                            if let Ok(mut slot) = journal_error.lock() {
                                slot.get_or_insert(err);
                            }
                        }
                    }
                }
                if let Ok(mut fresh) = fresh.lock() {
                    if let Some(slot) = fresh.get_mut(index) {
                        *slot = Some(report);
                    }
                }
            }
        },
    );
    if let Some(err) = journal_error.into_inner().unwrap_or_default() {
        return Err(err);
    }
    for (index, report) in todo.into_iter().zip(fresh.into_inner().unwrap_or_default()) {
        if let Some(slot) = results.get_mut(index) {
            *slot = report;
        }
    }
    let results: Vec<Value> = results.into_iter().flatten().collect();

    let failed = results
        .iter()
        .filter(|r| r.get("ok") != Some(&json!(true)))
        .count()
        + problems.len();
    let converted = results.len() + problems.len() - failed;
    let mut all: Vec<Value> = problems;
    all.extend(results);
    Ok(json!({
        "schema": SCHEMA,
        "plan": project_plan,
        "ok": failed == 0,
        "converted": converted,
        "failed": failed,
        "skipped": skipped,
        "skipped_by_extension": skipped_by_extension,
        "skipped_revisions": skipped_revisions,
        "resumed": resumed,
        "results": all,
    }))
}

/// How many files to convert at once: `jobs`, else the machine's cores, at most 8.
pub fn workers(jobs: Option<NonZeroUsize>) -> usize {
    jobs.map_or_else(
        || {
            std::thread::available_parallelism()
                .map_or(1, NonZeroUsize::get)
                .min(DEFAULT_JOBS)
        },
        NonZeroUsize::get,
    )
}

/// The text report of a `dutchman.convert-all/1` report: one line per file, each converted
/// file's losses indented beneath it (their notes cut short unless `verbose`), then the counts.
pub fn text(report: &Value, verbose: bool) -> String {
    let mut out = String::new();
    let mut cut = false;
    if let Some(plan) = report.get("plan").and_then(Value::as_str) {
        out.push_str(plan);
        out.push('\n');
    }
    for result in report
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(status) = report::status_line(result) {
            out.push_str(&status);
            out.push('\n');
        }
        if let Some(version) = report::solidcraft_line(result) {
            out.push_str(&version);
            out.push('\n');
            if let Some(notice) = report::solidcraft_notice(result) {
                out.push_str(&notice);
                if result.pointer("/solidcraft/status").and_then(Value::as_str) == Some("too_old") {
                    out.push(' ');
                    out.push_str(report::SOLIDCRAFT_DOWNLOAD);
                }
                out.push('\n');
            }
        }
        out.push_str(&line(result));
        out.push('\n');
        for line in report::completeness_lines(result) {
            out.push_str("           ");
            out.push_str(&line);
            out.push('\n');
        }
        let losses = result
            .get("losses")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        if verbose || losses.len() <= report::LISTED {
            for loss in losses {
                let (line, shortened) = report::loss_line(loss, verbose);
                cut |= shortened;
                out.push_str("           ");
                out.push_str(&line);
                out.push('\n');
            }
        } else {
            for line in report::grouped_lines(result) {
                out.push_str("           ");
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    let count = |key: &str| report.get(key).and_then(Value::as_u64).unwrap_or(0);
    let revisions = report.get("skipped_revisions").and_then(Value::as_array);
    let older = revisions.map_or(0, Vec::len) as u64;
    out.push_str(&revision_skip_text(report));
    out.push_str(&format!(
        "{} converted, {} failed",
        count("converted"),
        count("failed")
    ));
    if count("skipped") > older {
        out.push_str(&format!(
            ", {} skipped (not in a format Dutchman knows)",
            count("skipped").saturating_sub(older)
        ));
    }
    if older > 0 {
        out.push_str(&format!(", {older} skipped (older revisions)"));
    }
    out.push('\n');
    if cut {
        out.push_str(&format!("({})\n", report::CUT_NOTE));
    }
    out
}

fn revision_skip_text(report: &Value) -> String {
    let mut out = String::new();
    for revision in report
        .get("skipped_revisions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let (Some(path), Some(newer)) = (
            revision.get("path").and_then(Value::as_str),
            revision.get("newer").and_then(Value::as_str),
        ) {
            out.push_str(&format!(
                "skipped    {path}: older revision; using {newer}\n"
            ));
        }
    }
    out
}

/// The name of the whole text report `convert-all` writes in its output folder.
pub const LOG_NAME: &str = "dutchman.log";

/// The whole text report of a `dutchman.convert-all/1` report, every note in full: each
/// file's line, how complete it is and its losses by component, then the counts.
pub fn full_log(report: &Value) -> String {
    let mut out = String::new();
    if let Some(plan) = report.get("plan").and_then(Value::as_str) {
        out.push_str(plan);
        out.push('\n');
    }
    for result in report
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        out.push_str(&report::full_log(result, &line(result)));
        out.push('\n');
    }
    out.push_str(&revision_skip_text(report));
    let count = |key: &str| report.get(key).and_then(Value::as_u64).unwrap_or(0);
    out.push_str(&format!(
        "{} converted, {} failed, {} skipped\n",
        count("converted"),
        count("failed"),
        count("skipped")
    ));
    out
}

/// One file's line of the text reports: what it became, or why it failed.
pub fn line(result: &Value) -> String {
    let input = result
        .pointer("/input/path")
        .and_then(Value::as_str)
        .unwrap_or("?");
    if result.get("ok") == Some(&json!(true)) {
        if let Some(pages) = result.get("pages").and_then(Value::as_array) {
            let paths = pages
                .iter()
                .filter_map(|p| p.get("path").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(", ");
            return format!("converted  {input} -> {paths} ({} DXF pages)", pages.len());
        }
        let output = result
            .pointer("/output/path")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let written = match (
            result.get("mode").and_then(Value::as_str),
            result
                .pointer("/verification/status")
                .and_then(Value::as_str),
        ) {
            (Some("partial_history"), _) => {
                "partial editable history; imported final shape visible"
            }
            (Some("history"), Some("verified")) => "editable history, verified in SolidCraft",
            (Some("history"), _) => "editable history, unverified",
            _ => "geometry without history",
        };
        let freecad = result
            .pointer("/output/format")
            .and_then(Value::as_str)
            .is_some_and(|format| format.starts_with("freecad-"));
        let written = if freecad {
            match result
                .pointer("/verification/status")
                .and_then(Value::as_str)
            {
                Some("verified") => "geometry verified in FreeCAD".to_owned(),
                Some("differs" | "unverified") => {
                    let reason = result
                        .pointer("/verification/reason")
                        .and_then(Value::as_str)
                        .unwrap_or("not checked");
                    format!("FreeCAD rebuild: {reason}")
                }
                _ => written.to_owned(),
            }
        } else {
            written.to_owned()
        };
        format!("converted  {input} -> {output} ({written})")
    } else {
        let message = result
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("failed");
        format!("failed     {input}: {message}")
    }
}

/// What a run will convert.
#[derive(Debug)]
pub struct Plan {
    /// Inventory and conversion strategy before referenced parts are grouped into roots.
    pub description: String,
    /// The files to convert, with their output paths.
    pub jobs: Vec<Job>,
    /// Failure reports for inputs that are neither a known file nor a readable folder.
    pub problems: Vec<Value>,
    /// Files skipped in directory searches: unknown formats and older numbered saves.
    pub skipped: usize,
    /// Unknown-format files, counted by their extension (see [`extension_of`]).
    pub skipped_by_extension: BTreeMap<String, usize>,
    /// Older numbered saves omitted in favour of a newer save in the same folder.
    pub skipped_revisions: Vec<Value>,
}

/// The files to convert, with their output paths; failure reports for inputs that are
/// neither a known file nor a readable folder; and how many files in the folders searched
/// were skipped as unknown formats or older numbered saves.
pub fn plan(inputs: &[PathBuf], out_dir: &Path) -> Plan {
    plan_with_limit(inputs, out_dir, |_| true, MAX_FILES)
}

/// Builds a service's conversion list, excluding paths before descending into folders.
/// Unlike one batch, a service may process more than `MAX_FILES` over its lifetime.
pub fn plan_filtered(inputs: &[PathBuf], out_dir: &Path, include: fn(&Path) -> bool) -> Plan {
    plan_with_limit(inputs, out_dir, include, usize::MAX)
}

fn plan_with_limit(
    inputs: &[PathBuf],
    out_dir: &Path,
    include: fn(&Path) -> bool,
    max_files: usize,
) -> Plan {
    let mut jobs = Vec::new();
    let mut problems = Vec::new();
    let mut skipped = BTreeMap::new();
    let mut skipped_revisions = Vec::new();
    let mut outputs = BTreeSet::new();
    for input in inputs {
        if input.is_file() && is_lock_file(input) {
            continue;
        }
        let mut found = Vec::new();
        let project_files = if extension_of(input) == "zip" {
            match unpack_project(input) {
                Ok(dir) => Some(std::sync::Arc::new(dir)),
                Err(message) => {
                    problems.push(report::failure(
                        report::CONVERT,
                        report::input(input),
                        &message,
                    ));
                    continue;
                }
            }
        } else {
            None
        };
        let extracted = project_files.as_ref().map(|dir| dir.path().join("files"));
        let root = extracted.as_deref().unwrap_or(input.as_path());
        if root.is_dir() {
            let mut left = dutchman_core::MAX_SEARCHED;
            search(root, root, 0, &mut found, &mut skipped, include, &mut left);
            if left == 0 {
                problems.push(report::failure(
                    report::CONVERT,
                    report::input(input),
                    "project search reached 200000 entries; choose a smaller folder",
                ));
            }
            newest_revisions(&mut found, &mut skipped_revisions);
        } else if input.is_file() {
            let name = input.file_name().map(PathBuf::from).unwrap_or_default();
            found.push((input.clone(), name));
        } else {
            problems.push(report::failure(
                report::CONVERT,
                report::input(input),
                "no such file or folder",
            ));
            continue;
        }
        let natives: BTreeSet<_> = found
            .iter()
            .filter(|(path, _)| !is_step(path))
            .map(|(path, _)| path.with_extension("").to_string_lossy().to_lowercase())
            .collect();
        let mut fallbacks = BTreeMap::new();
        found.retain(|(path, _)| {
            let key = path.with_extension("").to_string_lossy().to_lowercase();
            if is_step(path) && natives.contains(&key) {
                fallbacks.insert(key, path.clone());
                false
            } else {
                true
            }
        });
        for (path, relative) in found {
            if jobs.len() >= max_files {
                problems.push(report::failure(
                    report::CONVERT,
                    report::input(&path),
                    &format!("over Dutchman's limit of {max_files} files in one run"),
                ));
                continue;
            }
            let output = unique(
                out_dir
                    .join(
                        Format::part_revision(&relative)
                            .map_or_else(|| relative.clone(), |(part, _)| part),
                    )
                    .with_extension(crate::OutputFormat::Solidcraft.extension()),
                &mut outputs,
            );
            let replace = Format::part_revision(&path).is_none();
            jobs.push(Job {
                project_files: project_files.clone(),
                fallback: fallbacks
                    .get(&path.with_extension("").to_string_lossy().to_lowercase())
                    .cloned(),
                input: path,
                output,
                replace,
                search: PartSearch {
                    threads: 0,
                    part_positions: dutchman_core::PartPositions::Smart,
                    project: root.is_dir().then(|| root.to_path_buf()),
                    also: Vec::new(),
                },
            });
        }
    }
    let description = summary(&jobs);
    project_roots(&mut jobs);
    Plan {
        description,
        jobs,
        problems,
        skipped: skipped.values().sum::<usize>() + skipped_revisions.len(),
        skipped_by_extension: skipped,
        skipped_revisions,
    }
}

// Retain roots and unused parts. Keep cycles visible so conversion can report them.
fn project_roots(jobs: &mut Vec<Job>) {
    let canonical = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut edges = BTreeMap::new();
    for job in jobs
        .iter()
        .filter(|job| Format::from_path(&job.input).is_some_and(Format::is_assembly))
    {
        if let Ok(references) = dutchman_core::assembly_references(&job.input, &job.search) {
            edges.insert(
                canonical(&job.input),
                references
                    .iter()
                    .map(|path| canonical(path))
                    .collect::<Vec<_>>(),
            );
        }
    }
    retain_roots(jobs, &edges);
    jobs.sort_by_key(|job| !Format::from_path(&job.input).is_some_and(Format::is_assembly));
}

fn retain_roots(jobs: &mut Vec<Job>, edges: &BTreeMap<PathBuf, Vec<PathBuf>>) {
    let referenced: BTreeSet<_> = edges.values().flatten().cloned().collect();
    let mut covered = BTreeSet::new();
    let mut pending: Vec<_> = edges
        .keys()
        .filter(|path| !referenced.contains(*path))
        .cloned()
        .collect();
    while let Some(path) = pending.pop() {
        if let Some(children) = edges.get(&path) {
            for child in children {
                if covered.insert(child.clone()) {
                    pending.push(child.clone());
                }
            }
        }
    }
    jobs.retain(|job| {
        !covered.contains(
            &job.input
                .canonicalize()
                .unwrap_or_else(|_| job.input.clone()),
        )
    });
}

fn is_lock_file(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("~$"))
}

fn is_step(path: &Path) -> bool {
    matches!(extension_of(path).as_str(), "step" | "stp")
}

fn unpack_project(input: &Path) -> Result<tempfile::TempDir, String> {
    let dir = tempfile::Builder::new()
        .prefix("dutchman-project-")
        .tempdir()
        .map_err(|err| format!("cannot create a private project folder: {err}"))?;
    let name = input
        .file_name()
        .map(|name| dutchman_serve::upload::safe_name(&name.to_string_lossy()))
        .unwrap_or_else(|| "project.zip".into());
    let archive = dir.path().join(name);
    let size = std::fs::metadata(input)
        .map_err(|err| format!("cannot read project zip: {err}"))?
        .len();
    if size > 4 * 1024 * 1024 * 1024 {
        return Err("project zip is over 4 GB; zip a smaller folder".into());
    }
    let source =
        std::fs::File::open(input).map_err(|err| format!("cannot read project zip: {err}"))?;
    let mut source = std::io::Read::take(source, 4 * 1024 * 1024 * 1024 + 1);
    let mut destination = std::fs::File::create(&archive)
        .map_err(|err| format!("cannot stage project zip: {err}"))?;
    let copied = std::io::copy(&mut source, &mut destination)
        .map_err(|err| format!("cannot read project zip: {err}"))?;
    if copied > 4 * 1024 * 1024 * 1024 {
        return Err("project zip is over 4 GB; zip a smaller folder".into());
    }
    drop(destination);
    dutchman_serve::upload::unzip(
        &archive,
        &dir.path().join("files"),
        &mut dutchman_serve::upload::ZipLimits {
            entries: 10_000,
            bytes: 4 * 1024 * 1024 * 1024,
        },
    )?;
    Ok(dir)
}

/// A new output folder beside an input, leaving existing folders untouched.
pub fn default_output(input: &Path) -> PathBuf {
    let name = if input.is_dir() {
        input.file_name()
    } else {
        input.file_stem()
    }
    .unwrap_or_default()
    .to_string_lossy();
    let base = input.with_file_name(format!("{name} (SolidCraft)"));
    if !base.exists() {
        return base;
    }
    for n in 2..10_000 {
        let path = input.with_file_name(format!("{name} (SolidCraft {n})"));
        if !path.exists() {
            return path;
        }
    }
    base
}

/// Plain project counts shown before conversion.
pub fn summary(jobs: &[Job]) -> String {
    let mut assemblies = 0;
    let mut parts = 0;
    let mut steps = 0;
    let fallbacks = jobs.iter().filter(|job| job.fallback.is_some()).count();
    for job in jobs {
        match extension_of(&job.input).as_str() {
            "sldasm" | "iam" | "f3z" => assemblies += 1,
            "step" | "stp" => steps += 1,
            _ => parts += 1,
        }
    }
    steps += fallbacks;
    let assembly_word = if assemblies == 1 {
        "assembly"
    } else {
        "assemblies"
    };
    let part_word = if parts == 1 { "part" } else { "parts" };
    let file_word = if steps == 1 { "file" } else { "files" };
    let mut text =
        format!("{assemblies} {assembly_word}, {parts} {part_word}, {steps} STEP {file_word}");
    if assemblies > 0 {
        text.push_str("; converting assemblies with their parts");
    }
    if fallbacks > 0 {
        text.push_str(&format!(
            "; {fallbacks} same-name STEP exports kept as fallbacks"
        ));
    }
    text
}

/// Keeps the highest numeric revision for each part found in a directory search.
fn newest_revisions(found: &mut Vec<(PathBuf, PathBuf)>, skipped: &mut Vec<Value>) {
    let mut newest: BTreeMap<PathBuf, (String, PathBuf)> = BTreeMap::new();
    for (path, _) in found.iter() {
        if let Some((part, revision)) = Format::part_revision(path) {
            let revision = revision.trim_start_matches('0');
            let entry = newest
                .entry(part)
                .or_insert_with(|| (revision.to_owned(), path.clone()));
            if (revision.len(), revision) > (entry.0.len(), entry.0.as_str()) {
                *entry = (revision.to_owned(), path.clone());
            }
        }
    }
    found.retain(|(path, _)| {
        let newer = Format::part_revision(path)
            .and_then(|(part, _)| newest.get(&part))
            .map(|(_, newer)| newer);
        if let Some(newer) = newer.filter(|newer| *newer != path) {
            skipped.push(json!({"path": path, "newer": newer}));
            false
        } else {
            true
        }
    });
}

/// A file's extension as the reports count it: lower case, letters and digits only, at most
/// eight of them; `other` for anything else and `none` for no extension, so no part of a
/// file's name can show through.
pub fn extension_of(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        None => "none".to_owned(),
        Some(ext)
            if !ext.is_empty()
                && ext.len() <= 8
                && ext.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            ext.to_ascii_lowercase()
        }
        Some(_) => "other".to_owned(),
    }
}

/// The files Dutchman knows under `folder`, with their paths relative to `root`. Symbolic
/// links are not followed, so a link cannot make the search loop.
#[allow(clippy::too_many_arguments)]
fn search(
    root: &Path,
    folder: &Path,
    depth: usize,
    out: &mut Vec<(PathBuf, PathBuf)>,
    skipped: &mut BTreeMap<String, usize>,
    include: fn(&Path) -> bool,
    left: &mut usize,
) {
    if depth > MAX_DEPTH || *left == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries
        .take(*left)
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    *left = left.saturating_sub(entries.len());
    entries.sort();
    for path in entries {
        if !path.strip_prefix(root).is_ok_and(include) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_file() && is_lock_file(&path) {
            continue;
        }
        if meta.is_dir() {
            search(root, &path, depth + 1, out, skipped, include, left);
        } else if meta.is_file() && Format::from_path(&path).is_none() {
            *skipped.entry(extension_of(&path)).or_insert(0) += 1;
        } else if meta.is_file() {
            let relative = path
                .strip_prefix(root)
                .map(Path::to_path_buf)
                .unwrap_or_default();
            out.push((path, relative));
        }
    }
}

/// `path`, numbered (`part-2.sc`) when an earlier file already writes there.
fn unique(path: PathBuf, taken: &mut BTreeSet<PathBuf>) -> PathBuf {
    let mut candidate = path.clone();
    let mut n = 2;
    while taken.contains(&candidate) {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut name = std::ffi::OsString::from(format!("{stem}-{n}"));
        if let Some(extension) = path.extension() {
            name.push(".");
            name.push(extension);
        }
        candidate = path.with_file_name(name);
        n += 1;
    }
    taken.insert(candidate.clone());
    candidate
}

/// Converts every job with `workers` threads, telling `on_event` as each starts and finishes.
/// Writers from `ConvertOptions::writer` prepare each input's estimate and memory budget.
/// Once `stop` is set, no further job starts; those already converting finish.
pub fn run_each(
    work: &[Job],
    writer: &crate::OutputWriter,
    workers: usize,
    stop: &AtomicBool,
    on_event: impl Fn(Event) + Sync,
) {
    run_with(
        work,
        workers,
        stop,
        |job, index| {
            let callback = |event| on_event(Event::Progress(index, event));
            match writer {
                crate::OutputWriter::Configured { options, .. } => {
                    convert_options_job_with_progress(job, options, &callback)
                }
                _ => convert_job_with_progress(job, writer, &callback),
            }
        },
        &on_event,
    );
}

/// [`run_each`] with each file in its own process started from `program`, so a stop for low
/// memory or a crash ends that file and not the program running the batch.
pub fn run_isolated(
    work: &[Job],
    options: &ConvertOptions,
    program: &Path,
    workers: usize,
    stop: &AtomicBool,
    on_event: impl Fn(Event) + Sync,
) {
    let limits = Limits {
        timeout: std::time::Duration::from_secs(u64::MAX / 4),
        max_memory: None,
        design: Some(crate::worker::DEFAULT_DESIGN_LIMIT),
    };
    run_with(
        work,
        workers,
        stop,
        |job, index| {
            let callback = |event| on_event(Event::Progress(index, event));
            convert_isolated_job_with_progress(program, job, options, &limits, None, &callback)
        },
        &on_event,
    );
}

/// Runs `convert` on every job with `workers` threads, as [`run_each`] does, adding each
/// file's time to its report as `elapsed_ms`.
pub(crate) fn run_with(
    work: &[Job],
    workers: usize,
    stop: &AtomicBool,
    convert: impl Fn(&Job, usize) -> Value + Sync,
    on_event: impl Fn(Event) + Sync,
) {
    let next = AtomicUsize::new(0);
    let workers = workers.min(work.len()).max(1);
    let run = |job: &Job, index: usize| {
        let started = Instant::now();
        let mut report = convert(job, index);
        if let Some(fields) = report.as_object_mut() {
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            fields.insert("elapsed_ms".to_owned(), json!(elapsed));
        }
        report
    };
    // Files stopped because they ran out of memory alongside others wait to run alone.
    let stopped = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(job) = work.get(index) else { break };
                on_event(Event::Started(index));
                let report = run(job, index);
                if workers > 1 && crate::stop::stopped_for_memory(&report) {
                    if let Ok(mut stopped) = stopped.lock() {
                        stopped.push((index, report));
                        continue;
                    }
                }
                on_event(Event::Finished(index, report));
            });
        }
    });
    let mut stopped = stopped.into_inner().unwrap_or_default();
    stopped.sort_by_key(|(index, _)| *index);
    for (index, first) in stopped {
        let Some(job) = work.get(index).filter(|_| !stop.load(Ordering::Relaxed)) else {
            on_event(Event::Finished(index, first));
            continue;
        };
        let mut report = run(job, index);
        crate::stop::note_retried_alone(&mut report);
        on_event(Event::Finished(index, report));
    }
}

/// The record of a batch's finished files, one JSON line each, kept as they finish so a run
/// stopped part way can carry on where it stopped.
struct Journal {
    file: std::fs::File,
    /// What it already records: each file's report, by the file's identity.
    known: BTreeMap<String, Value>,
}

impl Journal {
    /// Opens the journal at `path`, reading what it records; makes it when it is not there.
    fn open(path: &Path) -> Result<Journal, String> {
        let io = |err: std::io::Error| format!("journal {}: {err}", path.display());
        let mut known = BTreeMap::new();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                for line in text.lines() {
                    // A line cut short by a stopped run is simply not recorded.
                    let Ok(entry) = serde_json::from_str::<Value>(line) else {
                        continue;
                    };
                    // A file another version of Dutchman, or another journal format, recorded is
                    // converted again.
                    if entry.get("dutchman").and_then(Value::as_str) != Some(VERSION)
                        || entry.get("journal").and_then(Value::as_u64) != Some(JOURNAL_FORMAT)
                    {
                        continue;
                    }
                    if let (Some(key), Some(report)) = (
                        entry.get("file").and_then(Value::as_str),
                        entry.get("report"),
                    ) {
                        known.insert(key.to_owned(), report.clone());
                    }
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(io(err)),
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(io)?;
        Ok(Journal { file, known })
    }

    /// The report the journal holds for `job`, when the file has not changed since and its
    /// design, if it converted, is still where the journal says.
    fn recorded(&self, job: &Job) -> Option<Value> {
        let report = self.known.get(&identity(job)?)?;
        let output = job.output.display().to_string();
        let converted = report.get("ok") == Some(&json!(true));
        let still_there = report.pointer("/output/path").and_then(Value::as_str)
            == Some(output.as_str())
            && job.output.is_file();
        (!converted || still_there).then(|| report.clone())
    }

    /// Records `report` as `job`'s.
    fn record(&mut self, job: &Job, report: &Value) -> Result<(), String> {
        let Some(file) = identity(job) else {
            return Ok(());
        };
        let line = json!({
            "dutchman": VERSION,
            "journal": JOURNAL_FORMAT,
            "file": file,
            "report": report,
        });
        writeln!(self.file, "{line}")
            .and_then(|()| self.file.flush())
            .map_err(|err| format!("cannot record a file in the journal: {err}"))
    }
}

/// What tells one input file from another, and from itself once changed: its full path, its
/// size and when it was last changed.
fn identity(job: &Job) -> Option<String> {
    let path = std::fs::canonicalize(&job.input).ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    let changed = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_nanos());
    Some(format!("{}|{}|{changed}", path.display(), meta.len()))
}

/// One file converted, its output folder made first, as its convert report.
pub fn convert_job(job: &Job, writer: &(dyn Writer + Sync)) -> Value {
    match make_folder(job) {
        Some(failure) => failure,
        None => crate::convert(&job.input, &job.output, writer, job.replace, &job.search),
    }
}

fn convert_options_job_with_progress(
    job: &Job,
    options: &ConvertOptions,
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
) -> Value {
    let result = match make_folder(job) {
        Some(failure) => failure,
        None => crate::convert_options_with_progress(
            &job.input,
            &job.output,
            options,
            job.replace,
            &job.search,
            on_progress,
        ),
    };
    with_fallback(job, result, |fallback| {
        convert_options_job_with_progress(fallback, options, on_progress)
    })
}

fn with_fallback(job: &Job, result: Value, convert: impl FnOnce(&Job) -> Value) -> Value {
    if result.get("ok").and_then(Value::as_bool) == Some(true) {
        return result;
    }
    let Some(input) = &job.fallback else {
        return result;
    };
    let mut fallback = job.clone();
    fallback.input = input.clone();
    fallback.fallback = None;
    let mut recovered = convert(&fallback);
    if let Some(fields) = recovered.as_object_mut() {
        fields.insert("native_failure".into(), result);
        let losses = fields.entry("losses").or_insert_with(|| json!([]));
        if let Some(losses) = losses.as_array_mut() {
            losses.push(json!({
                "level": "Degraded", "stage": "input", "upstream": "dutchman",
                "code": "project/step-fallback", "subject": "STEP fallback",
                "message": "The native file could not be converted; tried its same-name STEP export."
            }));
        }
    }
    recovered
}

fn convert_job_with_progress(
    job: &Job,
    writer: &(dyn Writer + Sync),
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
) -> Value {
    match make_folder(job) {
        Some(failure) => failure,
        None => crate::convert_with_progress(
            &job.input,
            &job.output,
            writer,
            job.replace,
            &job.search,
            on_progress,
        ),
    }
}

/// Converts one job through the batch worker, making its destination folder first.
pub fn convert_isolated_job(
    program: &Path,
    job: &Job,
    options: &ConvertOptions,
    limits: &Limits,
    stop: Option<&AtomicBool>,
) -> Value {
    convert_isolated_job_with_progress(program, job, options, limits, stop, &|_| {})
}

/// Converts one isolated job while forwarding coarse progress snapshots.
pub fn convert_isolated_job_with_progress(
    program: &Path,
    job: &Job,
    options: &ConvertOptions,
    limits: &Limits,
    stop: Option<&AtomicBool>,
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
) -> Value {
    let started = Instant::now();
    let mut report = make_folder(job).unwrap_or_else(|| {
        worker::convert_isolated_with_progress(program, job, options, limits, stop, on_progress)
    });
    report::conversion_choices(&mut report, &job.input, options);
    if let Some(fields) = report.as_object_mut() {
        fields.insert(
            "elapsed_ms".into(),
            json!(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        );
    }
    with_fallback(job, report, |fallback| {
        convert_isolated_job_with_progress(program, fallback, options, limits, stop, on_progress)
    })
}

/// Makes the folder `job`'s design goes in; a failure report when it cannot be made.
fn make_folder(job: &Job) -> Option<Value> {
    let parent = job.output.parent()?;
    std::fs::create_dir_all(parent).err().map(|err| {
        report::failure(
            report::CONVERT,
            report::input(&job.input),
            &format!("{}: {err}", parent.display()),
        )
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn project_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn lock_files_are_absent_from_folder_zip_explicit_and_service_plans() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = dir.path().join("inputs");
        std::fs::create_dir(&inputs).unwrap();
        let mut names: Vec<_> = Format::ALL
            .into_iter()
            .flat_map(|format| {
                format
                    .extensions()
                    .iter()
                    .map(|ext| format!("~$part.{ext}"))
                    .collect::<Vec<_>>()
            })
            .collect();
        names.push("~$part.unknown".into());
        names.sort();
        names.dedup();
        for name in &names {
            std::fs::write(inputs.join(name), b"lock").unwrap();
        }
        std::fs::write(inputs.join("part.sldprt"), b"synthetic").unwrap();
        let archive = dir.path().join("project.zip");
        let mut entries: Vec<(&str, &[u8])> = names
            .iter()
            .map(|name| (name.as_str(), b"lock".as_slice()))
            .collect();
        entries.push(("part.sldprt", b"synthetic"));
        project_zip(&archive, &entries);
        let explicit: Vec<_> = names.iter().map(|name| inputs.join(name)).collect();
        for planned in [
            plan(std::slice::from_ref(&inputs), &dir.path().join("out")),
            plan(&[archive], &dir.path().join("out")),
            plan_filtered(&[inputs], &dir.path().join("out"), |_| true),
        ] {
            assert_eq!(planned.jobs.len(), 1);
            assert!(planned.problems.is_empty());
            assert_eq!(planned.skipped, 0);
            assert!(planned.skipped_by_extension.is_empty());
        }
        let planned = plan(&explicit, &dir.path().join("out"));
        assert!(planned.jobs.is_empty());
        assert!(planned.problems.is_empty());
        assert_eq!(planned.skipped, 0);
    }

    #[test]
    fn assemblies_group_referenced_parts_and_keep_unused_parts_and_cycles() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "root.iam",
            "nested.iam",
            "leaf.ipt",
            "unused.ipt",
            "cyclea.iam",
            "cycleb.iam",
        ] {
            std::fs::write(dir.path().join(name), b"synthetic placeholder").unwrap();
        }
        let mut planned = plan(&[dir.path().to_path_buf()], &dir.path().join("out"));
        let path = |name: &str| dir.path().join(name).canonicalize().unwrap();
        retain_roots(
            &mut planned.jobs,
            &BTreeMap::from([
                (path("root.iam"), vec![path("nested.iam")]),
                (path("nested.iam"), vec![path("leaf.ipt")]),
                (path("cyclea.iam"), vec![path("cycleb.iam")]),
                (path("cycleb.iam"), vec![path("cyclea.iam")]),
            ]),
        );
        let names: BTreeSet<_> = planned
            .jobs
            .iter()
            .map(|job| {
                job.input
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            names,
            BTreeSet::from([
                "root.iam".into(),
                "unused.ipt".into(),
                "cyclea.iam".into(),
                "cycleb.iam".into()
            ])
        );
    }

    #[test]
    fn solidworks_projects_convert_roots_with_nested_parts_and_then_unused_parts() {
        for root_fixture in ["nested_cubes.sldasm", "local_cubes.sldasm"] {
            let dir = tempfile::tempdir().unwrap();
            let inputs = dir.path().join("inputs");
            std::fs::create_dir(&inputs).unwrap();
            let fixtures =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../dutchman-core/tests/fixtures");
            let root = std::fs::read(fixtures.join(root_fixture)).unwrap();
            let nested = std::fs::read(fixtures.join("placed_cubes.sldasm")).unwrap();
            let cube = std::fs::read(fixtures.join("solidworks_cube.sldprt")).unwrap();
            let entries: Vec<(&str, &[u8])> = vec![
                ("assemblies/root.SLDASM", &root),
                ("assemblies/placed_cubes.SLDASM", &nested),
                ("parts/solidworks_cube.SLDPRT", &cube),
                ("a_unused.sldprt", &cube),
                ("~$ignored.SLDASM", b"lock"),
            ];
            for (name, bytes) in &entries {
                let path = inputs.join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
            let archive = dir.path().join("project.zip");
            project_zip(&archive, &entries);
            for (index, input) in [inputs, archive].into_iter().enumerate() {
                let out = dir.path().join(format!("out{index}"));
                let planned = plan(&[input], &out);
                assert!(planned.problems.is_empty(), "{:?}", planned.problems);
                assert_eq!(planned.skipped, 0);
                assert_eq!(
                    planned.description,
                    "2 assemblies, 2 parts, 0 STEP files; converting assemblies with their parts"
                );
                assert_eq!(planned.jobs.len(), 2);
                assert_eq!(planned.jobs[0].input.file_name().unwrap(), "root.SLDASM");
                assert_eq!(
                    planned.jobs[1].input.file_name().unwrap(),
                    "a_unused.sldprt"
                );
                let options = ConvertOptions {
                    preset: crate::choices::Preset::Fast,
                    no_verify: true,
                    ..Default::default()
                };
                for job in &planned.jobs {
                    let result = convert_options_job_with_progress(job, &options, &|_| {});
                    assert_eq!(result["ok"], true, "{result}");
                    assert!(job.output.is_file());
                    if job.input.extension().unwrap() == "SLDASM" {
                        assert!(
                            result["completeness"]["placed"].as_u64().unwrap() >= 4,
                            "{result}"
                        );
                        assert_eq!(result["completeness"]["left_out"], json!([]), "{result}");
                    }
                }
            }
        }
    }

    #[test]
    fn zip_projects_keep_sources_alive_until_the_last_job_is_released() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("project.zip");
        let fixture = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../dutchman-core/tests/fixtures/generated_unit_cube.f3d"),
        )
        .unwrap();
        project_zip(&archive, &[("parts/cube.f3d", &fixture)]);
        let planned = plan(std::slice::from_ref(&archive), &dir.path().join("out"));
        assert!(planned.problems.is_empty());
        assert_eq!(planned.jobs.len(), 1);
        let job = planned.jobs.first().unwrap().clone();
        let source = job.input.clone();
        drop(planned);
        assert!(source.is_file());
        let options = ConvertOptions {
            no_verify: true,
            ..Default::default()
        };
        let result = convert_options_job_with_progress(&job, &options, &|_| {});
        assert_eq!(result["ok"], true, "{result}");
        assert!(archive.is_file());
        drop(job);
        assert!(!source.exists());
    }

    #[test]
    fn zip_projects_refuse_traversal_without_removing_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("project.zip");
        project_zip(&archive, &[("../escape.ipt", b"x")]);
        let planned = plan(std::slice::from_ref(&archive), &dir.path().join("out"));
        assert!(planned.jobs.is_empty());
        assert_eq!(planned.problems.len(), 1);
        assert!(!dir.path().join("escape.ipt").exists());
        assert!(archive.is_file());
    }

    #[test]
    fn a_zip_project_recovers_a_failed_native_conversion_using_its_step_export() {
        use std::io::Read as _;
        let dir = tempfile::tempdir().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../dutchman-core/tests/fixtures/generated_unit_cube.f3d");
        let seed = dir.path().join("seed.sc");
        let options = ConvertOptions {
            preset: crate::choices::Preset::Fast,
            no_verify: true,
            ..Default::default()
        };
        let report = crate::convert_options_with_progress(
            &fixture,
            &seed,
            &options,
            true,
            &PartSearch::default(),
            &|_| {},
        );
        assert_eq!(report["ok"], true, "{report}");
        let mut design = zip::ZipArchive::new(std::fs::File::open(&seed).unwrap()).unwrap();
        let mut step = None;
        for index in 0..design.len() {
            let mut member = design.by_index(index).unwrap();
            if member.name().unwrap().ends_with(".step") {
                let mut bytes = Vec::new();
                member.read_to_end(&mut bytes).unwrap();
                step = Some(bytes);
                break;
            }
        }
        let archive = dir.path().join("project.zip");
        project_zip(
            &archive,
            &[
                ("cube.sldprt", b"synthetic unreadable native"),
                ("cube.step", &step.unwrap()),
            ],
        );
        let output = dir.path().join("out");
        let report = convert_all(&[archive], &output, &options, &Settings::default()).unwrap();
        assert_eq!(report["converted"], 1, "{report}");
        assert_eq!(report["failed"], 0, "{report}");
        assert_eq!(report["results"][0]["native_failure"]["ok"], false);
        assert_eq!(report["results"][0]["input"]["format"], "step");
        assert!(report["results"][0]["output"]["path"]
            .as_str()
            .unwrap()
            .ends_with("cube.sc"));
    }

    #[test]
    fn project_prefers_native_and_preserves_step_for_failure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cube.SLDPRT"), b"native").unwrap();
        std::fs::write(dir.path().join("cube.step"), b"export").unwrap();
        std::fs::write(dir.path().join("main.SLDASM"), b"assembly").unwrap();
        let planned = plan(&[dir.path().to_path_buf()], &dir.path().join("out"));
        assert_eq!(planned.jobs.len(), 2);
        assert!(summary(&planned.jobs).contains("converting assemblies with their parts"));
        let job = planned
            .jobs
            .iter()
            .find(|job| job.fallback.is_some())
            .unwrap();
        let failure = json!({"ok": false, "error": {"message": "native cannot be read"}});
        let result = with_fallback(job, failure.clone(), |fallback| {
            assert_eq!(fallback.input, dir.path().join("cube.step"));
            assert_eq!(fallback.output, job.output);
            assert!(fallback.fallback.is_none());
            json!({"ok": true})
        });
        assert_eq!(result["ok"], true);
        assert_eq!(result["native_failure"], failure);
        let result = with_fallback(job, json!({"ok": true}), |_| {
            panic!("STEP must not run when native succeeds")
        });
        assert_eq!(result["ok"], true);
    }

    #[test]
    fn default_folder_is_beside_the_input_and_avoids_existing_folders() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("project.zip");
        let first = default_output(&input);
        assert_eq!(first, dir.path().join("project (SolidCraft)"));
        std::fs::create_dir(&first).unwrap();
        assert_eq!(
            default_output(&input),
            dir.path().join("project (SolidCraft 2)")
        );
    }

    #[test]
    fn a_file_recorded_by_another_version_or_journal_format_is_converted_again() {
        // An older report lacks what this version records (the names an anonymous report
        // takes out, among others), so it is never reused, even under the same version number.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let report = json!({"ok": false, "error": {"message": "x"}});
        let lines = [
            json!({"dutchman": "0.0.9", "journal": JOURNAL_FORMAT, "file": "/a|1|1",
                   "report": report}),
            json!({"dutchman": VERSION, "file": "/b|1|1", "report": report}),
            json!({"dutchman": VERSION, "journal": 1, "file": "/c|1|1", "report": report}),
            json!({"dutchman": VERSION, "journal": JOURNAL_FORMAT, "file": "/d|1|1",
                   "report": report}),
        ];
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        std::fs::write(&path, text).unwrap();
        let journal = Journal::open(&path).unwrap();
        let known: Vec<&str> = journal.known.keys().map(String::as_str).collect();
        assert_eq!(known, ["/d|1|1"]);
    }

    #[test]
    fn files_in_a_folder_given_are_found_in_that_folder_as_a_project() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Robot");
        std::fs::create_dir_all(project.join("Assemblies")).unwrap();
        std::fs::write(project.join("Assemblies/top.iam"), b"").unwrap();
        let single = dir.path().join("cube.ipt");
        std::fs::write(&single, b"").unwrap();
        let plan = plan(&[project.clone(), single], &dir.path().join("out"));
        let projects: Vec<Option<PathBuf>> = plan
            .jobs
            .into_iter()
            .map(|job| job.search.project)
            .collect();
        assert_eq!(projects, [Some(project), None]);
    }

    #[test]
    fn numbered_saves_keep_the_highest_number_and_existing_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in");
        let out = dir.path().join("out");
        std::fs::create_dir_all(&input).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        for name in ["bracket.prt.3", "bracket.prt.7", "pin.prt.9", "pin.prt.10"] {
            std::fs::write(input.join(name), b"#UGC:2").unwrap();
        }
        let existing = out.join("bracket.sc");
        std::fs::write(&existing, b"keep this design").unwrap();
        let mut plan = plan(&[input], &out);
        assert_eq!(plan.jobs.len(), 2);
        assert!(plan
            .jobs
            .iter()
            .any(|job| job.input.ends_with("bracket.prt.7")));
        assert!(plan
            .jobs
            .iter()
            .any(|job| job.input.ends_with("pin.prt.10")));
        assert_eq!(plan.skipped, 2);
        assert_eq!(plan.skipped_revisions.len(), 2);
        prepare(&mut plan.jobs, &ConvertOptions::default());
        let bracket = plan
            .jobs
            .iter()
            .find(|job| job.input.ends_with("bracket.prt.7"))
            .unwrap();
        assert_eq!(bracket.output, existing);
        assert!(!bracket.replace);
        let report = convert_job(bracket, &ConvertOptions::default().writer());
        assert_eq!(report["ok"], false);
        assert!(report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already exists"));
        assert_eq!(std::fs::read(existing).unwrap(), b"keep this design");
    }

    #[test]
    fn revision_comparison_handles_large_numbers_and_leading_zeroes() {
        let names = [
            "p.prt.0009",
            "p.prt.10",
            "p.prt.999999999999999999999999999999",
        ];
        let mut found = names
            .map(|name| (PathBuf::from(name), PathBuf::from(name)))
            .to_vec();
        let mut skipped = Vec::new();
        newest_revisions(&mut found, &mut skipped);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, Path::new(names[2]));
        assert_eq!(skipped.len(), 2);
    }

    #[test]
    fn batch_names_follow_the_target_and_preserve_collision_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("part.v2.f3d");
        let second = dir.path().join("part.v2.ipt");
        std::fs::write(&first, b"").unwrap();
        std::fs::write(&second, b"").unwrap();
        for (target, extension) in [
            ("0.18.2", "solidcraft"),
            ("0.25.0", "solidcraft"),
            ("0.25.1", "sc"),
            ("0.27.0", "sc"),
        ] {
            let mut planned = plan(&[first.clone(), second.clone()], &dir.path().join("out"));
            let options = ConvertOptions {
                solidcraft_version: Some(target.parse().unwrap()),
                ..ConvertOptions::default()
            };
            prepare(&mut planned.jobs, &options);
            let names: Vec<_> = planned
                .jobs
                .iter()
                .map(|job| {
                    job.output
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            assert_eq!(
                names,
                [
                    format!("part.v2.{extension}"),
                    format!("part.v2-2.{extension}")
                ]
            );
        }
    }
}
