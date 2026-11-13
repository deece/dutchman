//! Converting one file in a child process of its own, so a batch survives any one file: a
//! file that takes too long or uses too much memory is stopped, and one that crashes the
//! converter fails alone. The child is the running Dutchman program started again with
//! [`WORKER_ARG`]; it prints the file's convert report on stdout and, on stderr, a line as
//! each stage of the conversion begins, so a stopped file's report says where it stopped.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::Parser;
use dutchman_core::Stage;
use serde_json::Value;

use crate::batch::Job;
use crate::json::{self, CONVERT};
use crate::{ConvertOptions, PartSearch};

/// The first argument that makes a Dutchman program convert one file as a batch's child.
pub const WORKER_ARG: &str = "--dutchman-convert-worker";

/// How a stage is marked on the child's stderr.
const STAGE_MARK: &str = "dutchman-stage: ";

/// The most a child's report may hold, in bytes.
const MAX_REPORT_BYTES: u64 = 64 << 20;

/// How often a running child is looked at.
const POLL: Duration = Duration::from_millis(50);

/// How often a running child's memory is measured.
#[cfg(target_os = "linux")]
const MEMORY_POLL: Duration = Duration::from_millis(250);
#[cfg(not(target_os = "linux"))]
const MEMORY_POLL: Duration = Duration::from_secs(1);

/// What one file's conversion may use.
#[derive(Clone, Debug)]
pub struct Limits {
    /// How long it may run.
    pub timeout: Duration,
    /// How much memory it may hold, in bytes; `None` for no limit.
    pub max_memory: Option<u64>,
    /// How long reading one design may take before it counts as stuck; `None` for no limit.
    /// In a Fusion archive (.f3z) a stuck design is left out and the rest converted again.
    pub design: Option<Duration>,
}

/// Short helper programs have no window to show.
#[cfg(windows)]
trait HideConsoleWindow {
    fn hide_console_window(&mut self) -> &mut Self;
}

#[cfg(windows)]
impl HideConsoleWindow for Command {
    fn hide_console_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        self.creation_flags(CREATE_NO_WINDOW)
    }
}

/// Stops every process this one started, so a conversion can end without leaving them running.
pub(crate) fn stop_own_children() {
    let own = std::process::id();
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    for (pid, _) in process_tree(own).into_iter().flatten() {
        if pid == own {
            continue;
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        #[cfg(windows)]
        let _ = Command::new("taskkill")
            .hide_console_window()
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    let _ = own;
}

/// How often free memory is read. It costs almost nothing, and a burst of allocation has to get
/// through the whole floor between two looks to be missed.
const LOW_MEMORY_POLL: Duration = Duration::from_millis(100);

/// How long after one conversion stops the others wait for its memory to come back.
const LOW_MEMORY_SETTLE: Duration = Duration::from_secs(2);

/// What each conversion this process supervises held when last measured.
static HOLDERS: Mutex<std::collections::BTreeMap<u32, u64>> =
    Mutex::new(std::collections::BTreeMap::new());

/// When a conversion last stopped for low memory.
static LAST_STOP: Mutex<Option<Instant>> = Mutex::new(None);

/// One supervised conversion's place among those sharing this computer's memory.
struct Holder(u32);

impl Holder {
    fn new(pid: u32) -> Self {
        if let Ok(mut holders) = HOLDERS.lock() {
            holders.insert(pid, 0);
        }
        Holder(pid)
    }

    /// With free memory low: records what this conversion holds, and says whether it stops.
    /// `confirmed` is false on the first low look, when the others may not have measured yet.
    fn stop_message(&self, confirmed: bool) -> Option<String> {
        let own = resident_tree_bytes(self.0).unwrap_or(0);
        let mut holders = HOLDERS.lock().ok()?;
        holders.insert(self.0, own);
        if !confirmed {
            return None;
        }
        let mut last = LAST_STOP.lock().ok()?;
        if last.is_some_and(|at| at.elapsed() < LOW_MEMORY_SETTLE)
            || !crate::stop::stops_first(&holders, self.0)
        {
            return None;
        }
        let total = holders
            .values()
            .fold(0u64, |all, held| all.saturating_add(*held));
        let message = crate::stop::low_memory(total, own, crate::choices::available_memory())?;
        *last = Some(Instant::now());
        Some(message)
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        if let Ok(mut holders) = HOLDERS.lock() {
            holders.remove(&self.0);
        }
    }
}

/// Watches this process and what it started. When memory really runs low, stops them and
/// calls `stopped`, which ends the program.
pub fn watch_own_memory(stopped: impl FnOnce(String) + Send + 'static) {
    let check = || {
        let available = crate::choices::available_memory();
        if !crate::stop::memory_is_low(available) {
            return None;
        }
        let held = resident_tree_bytes(std::process::id())?;
        crate::stop::low_memory(held, held, available)
    };
    let first = check();
    std::thread::spawn(move || {
        let mut message = first;
        loop {
            if let Some(message) = message.take() {
                stop_own_children();
                stopped(message);
                return;
            }
            std::thread::sleep(LOW_MEMORY_POLL);
            message = check();
        }
    });
}

/// The default time one file may take.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(900);

/// How long reading one design may take. A 137 MB Fusion design once sat 6.6 h with flat
/// memory, stuck, and the 15 other designs in its archive never got a turn (2026-11-12). Large
/// designs read in minutes; twenty is far past any seen finishing.
pub const DEFAULT_DESIGN_LIMIT: Duration = Duration::from_secs(20 * 60);

/// The setting for [`DEFAULT_DESIGN_LIMIT`]: minutes, `0` for no limit.
pub const DESIGN_LIMIT_ENV: &str = "DUTCHMAN_DESIGN_TIME_LIMIT";

/// How often the memory of a design being read is looked at, and how far it must move to count
/// as progress: a design still allocating or freeing is working, not stuck.
const PROGRESS_POLL: Duration = Duration::from_secs(10);
const PROGRESS_BYTES: u64 = 64 << 20;

/// The design limit `DUTCHMAN_DESIGN_TIME_LIMIT` asks for, in minutes (`0` turns it off), else
/// `limits`'s own.
fn design_limit(limits: &Limits, setting: Option<&str>) -> Option<Duration> {
    match setting
        .map(str::trim)
        .and_then(|minutes| minutes.parse::<u64>().ok())
    {
        Some(0) => None,
        Some(minutes) => Some(Duration::from_secs(minutes.saturating_mul(60))),
        None => limits.design,
    }
}

/// `limit` in whole minutes, as a phrase ("1 minute", "20 minutes").
fn minutes(limit: Option<Duration>) -> String {
    match limit.map_or(0, |limit| limit.as_secs().div_ceil(60)) {
        1 => "1 minute".to_owned(),
        n => format!("{n} minutes"),
    }
}

/// A design may run this many times the limit in all, progress or not: a decoder caught in a
/// runaway allocation keeps moving its memory and would otherwise never count as stuck.
const CEILING_TIMES: u32 = 3;

/// What a stuck design ran past, as a phrase: "20 minutes with no progress, or 60 minutes in
/// all".
fn limit_phrase(limit: Option<Duration>) -> String {
    format!(
        "{} with no progress, or {} in all",
        minutes(limit),
        minutes(limit.map(|limit| limit.saturating_mul(CEILING_TIMES)))
    )
}

/// How many stuck designs one archive may lose before the conversion gives up.
const MAX_LEFT_OUT: usize = 3;

/// The default memory one file may hold, in megabytes; `0` for no limit. A file may use what
/// this computer has: a conversion stops when memory really runs low.
pub const DEFAULT_MAX_MEMORY_MB: u64 = 0;

impl Default for Limits {
    fn default() -> Self {
        Limits {
            timeout: DEFAULT_TIMEOUT,
            max_memory: None,
            design: Some(DEFAULT_DESIGN_LIMIT),
        }
    }
}

/// The child's arguments after [`WORKER_ARG`].
#[derive(Parser)]
#[command(name = "dutchman-convert-worker")]
struct WorkerArgs {
    input: PathBuf,
    output: PathBuf,
    #[command(flatten)]
    options: ConvertOptions,
}

/// When this process was started as a batch's child, converts its file and returns the exit
/// code; otherwise `None`, and the program carries on as itself.
pub fn run_if_worker() -> Option<ExitCode> {
    let mut args = std::env::args_os();
    let program = args.next()?;
    let first = args.next()?;
    if first == crate::diagnose::WORKER_ARG {
        return Some(crate::diagnose::run_worker(program, args));
    }
    if first != WORKER_ARG {
        return None;
    }
    let parsed = match WorkerArgs::try_parse_from(std::iter::once(program).chain(args)) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{err}");
            return Some(ExitCode::from(2));
        }
    };
    stop_when_parent_ends();
    let callback = |event| eprintln!("{}", json::progress(&parsed.input, event));
    let members: Option<Arc<dyn dutchman_core::MemberObserver>> =
        std::env::var_os("DUTCHMAN_MEMBER_PROGRESS")
            .map(|_| Arc::new(MemberProgress) as Arc<dyn dutchman_core::MemberObserver>);
    let report = crate::convert_options_with_observer(
        &parsed.input,
        &parsed.output,
        &parsed.options,
        !parsed.options.no_clobber,
        &parsed.options.part_search(),
        &callback,
        members,
    );
    println!("{report}");
    Some(ExitCode::SUCCESS)
}

/// Ends this child, and what it started, when the process that started it ends: nothing reads
/// its report any more. Its parent holds the other end of stdin open until then.
fn stop_when_parent_ends() {
    std::thread::spawn(|| {
        let mut sink = [0u8; 64];
        let mut stdin = std::io::stdin();
        while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
        stop_own_children();
        std::process::exit(3);
    });
}

/// Private worker protocol; names travel only to the supervising process's hashing sink.
struct MemberProgress;

impl dutchman_core::MemberObserver for MemberProgress {
    fn phase_event(
        &self,
        phase: &'static str,
        event: dutchman_core::MemberEvent,
        ms: u64,
        bytes: u64,
    ) {
        let event = if event == dutchman_core::MemberEvent::Start {
            "start"
        } else {
            "finish"
        };
        eprintln!(
            "{}",
            serde_json::json!({"dutchman_phase": phase, "event": event, "ms": ms, "bytes": bytes})
        );
    }
    fn member_classified(
        &self,
        path: &str,
        ms: u64,
        nested_scan_ms: u64,
        layers_ms: u64,
        metadata_ms: u64,
    ) {
        eprintln!(
            "{}",
            classified_record(path, ms, nested_scan_ms, layers_ms, metadata_ms)
        );
    }

    fn member_event(
        &self,
        path: &str,
        event: dutchman_core::MemberEvent,
        ms: u64,
        code: Option<&'static str>,
    ) {
        let event = match event {
            dutchman_core::MemberEvent::Start => "start",
            dutchman_core::MemberEvent::Finish => "finish",
            dutchman_core::MemberEvent::Fail => "fail",
        };
        eprintln!(
            "{}",
            serde_json::json!({"dutchman_member": path, "event": event, "ms": ms, "code": code})
        );
    }
}

fn classified_record(
    path: &str,
    ms: u64,
    nested_scan_ms: u64,
    layers_ms: u64,
    metadata_ms: u64,
) -> Value {
    serde_json::json!({"dutchman_member_classified": path, "ms": ms,
        "nested_scan_ms": nested_scan_ms, "layers_ms": layers_ms, "metadata_ms": metadata_ms})
}

impl ConvertOptions {
    /// These options as the command-line flags that give them, for a child process; where
    /// part files are looked for is the job's (see [`search_args`]).
    pub fn to_args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = Vec::new();
        args.push("--threads".into());
        args.push(self.threads.to_string().into());
        // Only a choice other than the default is passed, so the arguments of a SolidCraft run
        // stay those every `dutchman` takes.
        if self.to != crate::OutputFormat::Solidcraft {
            args.push("--to".into());
            args.push(
                if self.to == crate::OutputFormat::Dxf {
                    "dxf"
                } else if self.to == crate::OutputFormat::Fcstd {
                    "fcstd"
                } else {
                    "freecad"
                }
                .into(),
            );
        }
        if self.verify {
            args.push("--verify".into());
        }
        if self.freecad_timeout != crate::ConvertOptions::default().freecad_timeout {
            args.push("--freecad-timeout".into());
            args.push(self.freecad_timeout.to_string().into());
        }
        args.push("--part-positions".into());
        args.push(self.effective_part_positions().name().into());
        args.push("--preset".into());
        args.push(self.preset.name().into());
        for (enabled, flag) in [
            (self.no_sketches, "--no-sketches"),
            (self.no_joints, "--no-joints"),
            (self.no_memory_check, "--no-memory-check"),
            (self.write_replay, "--write-replay"),
        ] {
            if enabled {
                args.push(flag.into());
            }
        }
        if self.import_only {
            args.push("--import-only".into());
        }
        if let Some(cli) = &self.solidcraft {
            args.push("--solidcraft".into());
            args.push(cli.clone().into_os_string());
        }
        if let Some(version) = self.solidcraft_version {
            args.push("--solidcraft-version".into());
            args.push(version.to_string().into());
        }
        if self.no_verify {
            args.push("--no-verify".into());
        }
        if self.no_clobber {
            args.push("--no-clobber".into());
        }
        args.push("--verify-timeout".into());
        args.push(self.verify_timeout.to_string().into());
        args
    }
}

/// The flags that look for a job's part files as `search` says.
pub(crate) fn search_args(search: &PartSearch) -> Vec<OsString> {
    let mut args = Vec::new();
    if let Some(project) = &search.project {
        args.push("--project".into());
        args.push(project.clone().into_os_string());
    }
    for folder in &search.also {
        args.push("--search".into());
        args.push(folder.clone().into_os_string());
    }
    args
}

/// Converts `job` in a child process started from `program`, within `limits`, and returns
/// its convert report, or a failure report saying why the child stopped.
pub fn convert_isolated(
    program: &Path,
    job: &Job,
    options: &ConvertOptions,
    limits: &Limits,
) -> Value {
    convert_isolated_until(program, job, options, limits, None)
}

/// Converts in a child, allowing thirty seconds to finish after shutdown is requested.
pub fn convert_isolated_until(
    program: &Path,
    job: &Job,
    options: &ConvertOptions,
    limits: &Limits,
    shutdown: Option<&std::sync::atomic::AtomicBool>,
) -> Value {
    convert_isolated_with_progress(program, job, options, limits, shutdown, &|_| {})
}

/// Converts in a child and forwards its coarse progress, including heartbeats.
pub fn convert_isolated_with_progress(
    program: &Path,
    job: &Job,
    options: &ConvertOptions,
    limits: &Limits,
    shutdown: Option<&std::sync::atomic::AtomicBool>,
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
) -> Value {
    crate::diagnostics::run(&job.input, options, on_progress, |progress, log| {
        convert_isolated_inner(program, job, options, limits, shutdown, progress, log)
    })
}

/// Converts `job` in a child process. A design in a Fusion archive (.f3z) that stays stuck
/// past [`Limits::design`] is left out and the archive converted again without it, up to
/// [`MAX_LEFT_OUT`] times, so one design no longer keeps the rest from converting.
fn convert_isolated_inner(
    program: &Path,
    job: &Job,
    options: &ConvertOptions,
    limits: &Limits,
    shutdown: Option<&std::sync::atomic::AtomicBool>,
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
    log: Option<&crate::diagnostics::Log>,
) -> Value {
    let set_limits = Limits {
        design: design_limit(limits, std::env::var(DESIGN_LIMIT_ENV).ok().as_deref()),
        ..limits.clone()
    };
    let limits = &set_limits;
    #[cfg(debug_assertions)]
    let test_limits;
    #[cfg(debug_assertions)]
    let limits = match std::env::var("DUTCHMAN_TEST_DESIGN_LIMIT_MS")
        .ok()
        .and_then(|ms| ms.parse::<u64>().ok())
    {
        Some(ms) => {
            test_limits = Limits {
                design: Some(Duration::from_millis(ms)),
                ..limits.clone()
            };
            &test_limits
        }
        None => limits,
    };
    let (mut report, mut stuck) =
        convert_once(program, job, options, limits, shutdown, on_progress, log);
    let mut left_out: Vec<String> = Vec::new();
    // Each copy without a stuck design lives until the conversion of it is done.
    let mut copies: Vec<tempfile::TempDir> = Vec::new();
    let mut current = job.clone();
    while let Some(Some(design)) = stuck.take() {
        if !is_archive(&job.input) || left_out.len() >= MAX_LEFT_OUT || left_out.contains(&design) {
            break;
        }
        let Some((dir, input)) = archive_without(&current.input, &design) else {
            break;
        };
        left_out.push(design);
        copies.push(dir);
        current = Job {
            input,
            ..job.clone()
        };
        (report, stuck) = convert_once(
            program,
            &current,
            options,
            limits,
            shutdown,
            on_progress,
            log,
        );
    }
    if !left_out.is_empty() {
        mark_left_out(&mut report, &job.input, &left_out, limits);
    }
    drop(copies);
    report
}

/// Whether `path` is a Fusion archive, whose designs can be left out one at a time.
fn is_archive(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("f3z"))
}

/// A private copy of the archive `input` without its entry `design`, under the same file name;
/// `None` when it cannot be made or has no such entry.
fn archive_without(input: &Path, design: &str) -> Option<(tempfile::TempDir, PathBuf)> {
    let dir = tempfile::Builder::new()
        .prefix("dutchman-archive-")
        .tempdir()
        .ok()?;
    let copy = dir.path().join(input.file_name()?);
    matches!(without_entry(input, design, &copy), Ok(true)).then_some((dir, copy))
}

/// Writes the zip `input` to `out` without its entry `name`, copying every other entry as it
/// is stored (nothing is decompressed). Whether `name` was there.
pub(crate) fn without_entry(input: &Path, name: &str, out: &Path) -> zip::result::ZipResult<bool> {
    let mut source = zip::ZipArchive::new(std::fs::File::open(input)?)?;
    let mut writer = zip::ZipWriter::new(std::fs::File::create(out)?);
    let mut found = false;
    for index in 0..source.len() {
        let entry = source.by_index_raw(index)?;
        if entry.name().is_ok_and(|entry_name| entry_name == name) {
            found = true;
            continue;
        }
        writer.raw_copy_file(entry)?;
    }
    writer.finish()?;
    Ok(found)
}

/// Says in `report` which stuck designs were left out of `input`: as losses when the rest
/// converted, in the error otherwise. Names are quoted so anonymous reports take them out.
pub(crate) fn mark_left_out(
    report: &mut Value,
    input: &Path,
    left_out: &[String],
    limits: &Limits,
) {
    let ran = limit_phrase(limits.design);
    let quoted = |names: &[String]| {
        names
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if let Some(fields) = report.as_object_mut() {
        fields.insert("input".into(), json::input(input));
        fields.insert("left_out_designs".into(), serde_json::json!(left_out));
    }
    let ok = report.get("ok").and_then(Value::as_bool) == Some(true);
    if ok {
        let losses: Vec<Value> = left_out
            .iter()
            .map(|design| {
                let users = users_of(input, design);
                let users = if users.is_empty() {
                    String::new()
                } else {
                    let (uses, is) = if users.len() == 1 {
                        ("uses", "is")
                    } else {
                        ("use", "are")
                    };
                    format!("; {} {uses} it and {is} missing its parts", quoted(&users))
                };
                serde_json::json!({
                    "level": "Dropped",
                    "code": null,
                    "subject": design,
                    "message": format!(
                        "\"{design}\" was left out: reading it ran past {ran}, Dutchman's \
                         limit for one design. The rest of the archive converted \
                         without it{users}. To give it longer, set {DESIGN_LIMIT_ENV} to more \
                         minutes (0 for no limit) and convert again"
                    ),
                    "stage": Stage::Decode.id(),
                    "upstream": "cadmpeg",
                })
            })
            .collect();
        match report.get_mut("losses").and_then(Value::as_array_mut) {
            Some(existing) => existing.extend(losses),
            None => {
                if let Some(fields) = report.as_object_mut() {
                    fields.insert("losses".into(), Value::Array(losses));
                }
            }
        }
        if report.get("loss_groups").is_some() {
            *report = json::with_loss_groups(std::mem::take(report));
        }
    } else if let Some(message) = report
        .get_mut("error")
        .and_then(|error| error.get_mut("message"))
    {
        let text = format!(
            "{} (Dutchman had already left out {}: each ran past {ran}, Dutchman's limit for \
             one design; to give it longer, set {DESIGN_LIMIT_ENV} to more minutes (0 for no \
             limit) and convert again)",
            message.as_str().unwrap_or(""),
            quoted(left_out)
        );
        *message = Value::String(text);
    }
}

/// The designs in the archive `input` that its design description says use `design`, by
/// their paths in the archive; empty when it has none or it cannot be read.
fn users_of(input: &Path, design: &str) -> Vec<String> {
    let read = || -> Option<Value> {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(input).ok()?).ok()?;
        let entry = archive.by_name("DesignDescription.json").ok()?;
        let mut text = String::new();
        entry.take(16 << 20).read_to_string(&mut text).ok()?;
        serde_json::from_str(&text).ok()
    };
    let Some(description) = read() else {
        return Vec::new();
    };
    let objects: Vec<&Value> = description
        .pointer("/designDescription/designGraphs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|graph| graph.get("designObjects")?.as_array())
        .flatten()
        .collect();
    let path = |object: &Value| {
        object
            .get("relativePath")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let ids: Vec<&Value> = objects
        .iter()
        .filter(|object| path(object).as_deref() == Some(design))
        .filter_map(|object| object.get("id"))
        .collect();
    let mut users: Vec<String> = objects
        .iter()
        .filter(|object| {
            object
                .get("references")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|reference| reference.get("ids")?.as_array())
                .flatten()
                .any(|id| ids.contains(&id))
        })
        .filter_map(|object| path(object))
        .filter(|user| user != design)
        .collect();
    users.sort();
    users.dedup();
    users.truncate(20);
    users
}

/// One conversion of `job` in a child process, and the design it was stopped on as stuck, if
/// it was (`Some(None)` when stuck outside any named design).
fn convert_once(
    program: &Path,
    job: &Job,
    options: &ConvertOptions,
    limits: &Limits,
    shutdown: Option<&std::sync::atomic::AtomicBool>,
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
    log: Option<&crate::diagnostics::Log>,
) -> (Value, Option<Option<String>>) {
    let mut options = options.clone();
    options.no_clobber = !job.replace;
    let mut command = Command::new(program);
    #[cfg(unix)]
    if shutdown.is_some() {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    if shutdown.is_some() {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    // Design events always: they time each design for the stuck-design limit. Names stay in
    // this process; only a diagnostics log, when asked for, records them (hashed).
    command.env("DUTCHMAN_MEMBER_PROGRESS", "1");
    let child = command
        .arg(WORKER_ARG)
        .arg(&job.input)
        .arg(&job.output)
        .args(options.to_args())
        .args(search_args(&job.search))
        // Kept open while this process lives: the child stops when it closes.
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(err) => {
            return (
                json::failure(
                    CONVERT,
                    json::input(&job.input),
                    &format!("cannot start the converter: {err}"),
                ),
                None,
            )
        }
    };
    // Dropped when this conversion ends, or with this process if it ends first.
    let _parent_alive = child.stdin.take();
    if let Some(log) = log {
        log.process(child.id());
    }
    std::thread::scope(|scope| {
        let stage = Arc::new(Mutex::new(Stage::Input));
        let reading = Arc::new(Mutex::new(Reading::new(is_archive(&job.input))));
        let stdout = child.stdout.take().map(|out| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = out.take(MAX_REPORT_BYTES).read_to_string(&mut text);
                text
            })
        });
        let stderr = child.stderr.take().map(|err| {
            let stage = Arc::clone(&stage);
            let reading = Arc::clone(&reading);
            scope.spawn(move || follow_stages(err, &stage, &reading, on_progress, log))
        });

        let stopped = supervise(
            &mut child,
            limits,
            &reading,
            !options.no_memory_check,
            shutdown,
        );
        let text = stdout
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        if let Some(reader) = stderr {
            let _ = reader.join();
        }
        let at = stage.lock().map_or(Stage::Input, |stage| *stage);
        let fail = |message: String| {
            if at == Stage::Write {
                // Stopped while writing: what is there is not a whole design.
                let _ = std::fs::remove_file(&job.output);
            }
            json::failure_at(
                CONVERT,
                json::input(&job.input),
                &message,
                at,
                upstream_of(at),
            )
        };
        let stuck = match &stopped {
            Stopped::Stuck(design) => Some(design.clone()),
            _ => None,
        };
        let report = match stopped {
            Stopped::Exited(status) => match serde_json::from_str::<Value>(text.trim()) {
                Ok(report) if report.is_object() => report,
                _ => fail(format!(
                    "the converter stopped unexpectedly while at the {} stage ({status})",
                    at.id()
                )),
            },
            Stopped::Stuck(design) => {
                let ran = limit_phrase(limits.design);
                let raise = format!(
                    "; to give it longer, set {DESIGN_LIMIT_ENV} to more minutes (0 for no \
                     limit) and convert again"
                );
                fail(match design {
                    Some(design) => format!(
                        "stopped reading the design \"{design}\": it ran past {ran}, Dutchman's \
                         limit for one design{raise}"
                    ),
                    None => format!(
                        "stopped: a design ran past {ran} while at the {} stage, Dutchman's \
                         limit for one design{raise}",
                        at.id()
                    ),
                })
            }
            Stopped::TimedOut => fail(format!(
                "stopped after {} s, Dutchman's limit for one file, while at the {} stage",
                limits.timeout.as_secs(),
                at.id()
            )),
            Stopped::Shutdown => fail(format!(
                "stopped after the 30 s shutdown grace period while at the {} stage",
                at.id()
            )),
            Stopped::OverMemory(bytes) => fail(format!(
            "stopped when it held {} MB, over Dutchman's limit of {} MB for one file, while at \
             the {} stage",
            bytes >> 20,
            limits.max_memory.unwrap_or(0) >> 20,
            at.id()
        )),
            Stopped::LowMemory(message) => {
                fail(format!("{message}, while at the {} stage", at.id()))
            }
            Stopped::Lost(err) => fail(format!("lost track of the converter: {err}")),
        };
        (report, stuck)
    })
}

/// What the child is reading, from its stage marks and design events: the clock behind the
/// stuck-design limit restarts at every stage change and every design start or finish.
struct Reading {
    stage: Stage,
    design: Option<String>,
    since: Instant,
    /// When this design (or this stage, before any design) began; progress does not move it.
    began: Instant,
    /// A Fusion archive: its preparation before the first design (a whole-archive read that took
    /// 54 minutes on a 396 MB archive, memory moving throughout) is not one design, so the
    /// all-in ceiling counts only while a named design is read. The no-progress limit still
    /// covers the preparation.
    archive: bool,
    /// The memory the child held when the clock last restarted.
    held: Option<u64>,
}

impl Reading {
    fn new(archive: bool) -> Self {
        Self {
            stage: Stage::Input,
            design: None,
            since: Instant::now(),
            began: Instant::now(),
            archive,
            held: None,
        }
    }

    /// Any sign of work (an archive preparation step, a design classified): the clock restarts.
    fn progress(&mut self) {
        self.since = Instant::now();
    }

    /// The child holds `bytes` now: moving [`PROGRESS_BYTES`] from the last mark is progress.
    fn memory(&mut self, bytes: u64) {
        match self.held {
            Some(held) if held.abs_diff(bytes) < PROGRESS_BYTES => {}
            Some(_) => {
                self.held = Some(bytes);
                self.progress();
            }
            None => self.held = Some(bytes),
        }
    }

    fn stage(&mut self, stage: Stage) {
        if stage != self.stage {
            self.stage = stage;
            self.since = Instant::now();
            self.began = self.since;
        }
    }

    fn design(&mut self, design: Option<String>) {
        self.design = design;
        self.since = Instant::now();
        self.began = self.since;
    }

    /// The design read past `limit` with no progress, or past [`CEILING_TIMES`] the limit in
    /// all (`Some(None)` when none is named).
    fn stuck(&self, limit: Duration) -> Option<Option<String>> {
        let reading = matches!(self.stage, Stage::Decode | Stage::Project);
        let ceiling = !self.archive || self.design.is_some();
        let over = self.since.elapsed() > limit
            || (ceiling && self.began.elapsed() > limit.saturating_mul(CEILING_TIMES));
        (reading && over).then(|| self.design.clone())
    }
}

/// The project whose code a stage runs, when it is not Dutchman's own.
fn upstream_of(stage: Stage) -> Option<&'static str> {
    match stage {
        Stage::Decode | Stage::Project => Some("cadmpeg"),
        Stage::SolidCraft => Some("solidcraft"),
        _ => None,
    }
}

/// Records each stage the child marks on `stream` in `stage`, until the stream ends.
fn follow_stages(
    stream: impl Read,
    stage: &Mutex<Stage>,
    reading: &Mutex<Reading>,
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
    log: Option<&crate::diagnostics::Log>,
) {
    use std::io::BufRead;
    for line in std::io::BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        if let Ok(record) = serde_json::from_str::<Value>(&line) {
            if let Ok(mut reading) = reading.lock() {
                if let Some(design) = record.get("dutchman_member").and_then(Value::as_str) {
                    match record.get("event").and_then(Value::as_str) {
                        Some("start") => reading.design(Some(design.to_owned())),
                        Some("finish" | "fail") => reading.design(None),
                        _ => {}
                    }
                } else if record.get("dutchman_phase").is_some()
                    || record.get("dutchman_member_classified").is_some()
                {
                    reading.progress();
                }
            }
            if log.is_none()
                && (record.get("dutchman_member").is_some()
                    || record.get("dutchman_phase").is_some()
                    || record.get("dutchman_member_classified").is_some())
            {
                continue;
            }
        }
        if let (Some(log), Ok(record)) = (log, serde_json::from_str::<Value>(&line)) {
            if let Some(phase) = record.get("dutchman_phase").and_then(Value::as_str) {
                let event = match record.get("event").and_then(Value::as_str) {
                    Some("start") => Some(dutchman_core::MemberEvent::Start),
                    Some("finish") => Some(dutchman_core::MemberEvent::Finish),
                    _ => None,
                };
                if let Some(event) = event {
                    log.phase_event(
                        phase,
                        event,
                        record.get("ms").and_then(Value::as_u64).unwrap_or(0),
                        record.get("bytes").and_then(Value::as_u64).unwrap_or(0),
                    );
                }
                continue;
            }
            if let Some(member) = record
                .get("dutchman_member_classified")
                .and_then(Value::as_str)
            {
                // Copy only fixed numeric fields; stderr cannot introduce log categories.
                let ms = |key| record.get(key).and_then(Value::as_u64).unwrap_or(0);
                log.member_classified(
                    member,
                    ms("ms"),
                    ms("nested_scan_ms"),
                    ms("layers_ms"),
                    ms("metadata_ms"),
                );
                continue;
            }
            if let Some(member) = record.get("dutchman_member").and_then(Value::as_str) {
                let event = match record.get("event").and_then(Value::as_str) {
                    Some("start") => Some(dutchman_core::MemberEvent::Start),
                    Some("finish") => Some(dutchman_core::MemberEvent::Finish),
                    Some("fail") => Some(dutchman_core::MemberEvent::Fail),
                    _ => None,
                };
                if let Some(event) = event {
                    log.member_event(
                        member,
                        event,
                        record.get("ms").and_then(Value::as_u64).unwrap_or(0),
                        record
                            .get("code")
                            .and_then(Value::as_str)
                            .map(crate::diagnostics::member_code),
                    );
                }
                continue;
            }
        }
        if let Some(event) = serde_json::from_str::<Value>(&line)
            .ok()
            .as_ref()
            .and_then(json::parse_progress)
        {
            if let Ok(mut stage) = stage.lock() {
                *stage = event.stage;
            }
            if let Ok(mut reading) = reading.lock() {
                reading.stage(event.stage);
            }
            on_progress(event);
            continue;
        }
        let Some(id) = line.strip_prefix(STAGE_MARK) else {
            continue;
        };
        if let (Some(found), Ok(mut stage)) = (
            Stage::ALL.into_iter().find(|s| s.id() == id.trim()),
            stage.lock(),
        ) {
            *stage = found;
            if let Ok(mut reading) = reading.lock() {
                reading.stage(found);
            }
        }
    }
}

/// How a child ended.
enum Stopped {
    /// On its own, with this status.
    Exited(std::process::ExitStatus),
    /// Stopped for running past its time.
    TimedOut,
    /// Stopped after the shutdown grace period.
    Shutdown,
    /// Stopped for holding this many bytes, over its limit.
    OverMemory(u64),
    /// Stopped because this computer's memory really ran low, with what to say.
    LowMemory(String),
    /// It could not be waited on.
    Lost(std::io::Error),
    /// Stopped for reading one design past [`Limits::design`]: that design, when named.
    Stuck(Option<String>),
}

/// Waits for `child`, stopping it when it runs past `limits`.
fn supervise(
    child: &mut Child,
    limits: &Limits,
    reading: &Mutex<Reading>,
    watch_memory: bool,
    shutdown: Option<&std::sync::atomic::AtomicBool>,
) -> Stopped {
    let mut stopping = None;
    let started = Instant::now();
    let mut measured = started;
    let mut looked = started;
    let mut first_look = true;
    let mut was_low = false;
    let holder = Holder::new(child.id());
    let mut sampled = started;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Stopped::Exited(status),
            Ok(None) => {}
            Err(err) => {
                let _ = child.kill();
                return Stopped::Lost(err);
            }
        }
        if shutdown.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
            let since = stopping.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(30) {
                stop(child);
                return Stopped::Shutdown;
            }
        }
        if started.elapsed() > limits.timeout {
            stop(child);
            return Stopped::TimedOut;
        }
        if limits.design.is_some() && sampled.elapsed() >= PROGRESS_POLL {
            sampled = Instant::now();
            if let (Some(bytes), Ok(mut reading)) =
                (resident_tree_bytes(child.id()), reading.lock())
            {
                reading.memory(bytes);
            }
        }
        if let Some(design) = limits
            .design
            .and_then(|limit| reading.lock().ok()?.stuck(limit))
        {
            stop(child);
            return Stopped::Stuck(design);
        }
        if let Some(limit) = limits.max_memory {
            if measured.elapsed() >= MEMORY_POLL {
                measured = Instant::now();
                if let Some(held) = resident_tree_bytes(child.id()).filter(|held| *held > limit) {
                    stop(child);
                    return Stopped::OverMemory(held);
                }
            }
        }
        if watch_memory && (first_look || looked.elapsed() >= LOW_MEMORY_POLL) {
            first_look = false;
            looked = Instant::now();
            if crate::stop::memory_is_low(crate::choices::available_memory()) {
                #[cfg(debug_assertions)]
                let test = std::env::var_os("DUTCHMAN_TEST_LOW_MEMORY").is_some();
                #[cfg(not(debug_assertions))]
                let test = false;
                if let Some(message) = holder.stop_message(was_low || test) {
                    stop(child);
                    return Stopped::LowMemory(message);
                }
                was_low = true;
            } else {
                was_low = false;
            }
        }
        std::thread::sleep(POLL);
    }
}

pub(crate) fn stop(child: &mut Child) {
    // Include kernels started by a worker, retaining the parent's normal signal group.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Some(processes) = process_tree(child.id()) {
        let _ = Command::new("kill")
            .arg("-KILL")
            .args(processes.into_iter().map(|(pid, _)| pid.to_string()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .hide_console_window()
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

/// Converter memory including kernel subprocesses where the process tree is available.
#[cfg(target_os = "linux")]
fn process_tree(pid: u32) -> Option<Vec<(u32, u64)>> {
    let mut pending = vec![pid];
    let mut seen = std::collections::BTreeSet::new();
    let mut processes = Vec::new();
    while let Some(current) = pending.pop() {
        if !seen.insert(current) {
            continue;
        }
        if seen.len() > 4096 {
            return None;
        }
        processes.push((current, resident_bytes(current).unwrap_or(0)));
        // Children are listed per thread, including kernels started by part workers.
        if let Ok(tasks) = std::fs::read_dir(format!("/proc/{current}/task")) {
            for task in tasks.take(4096).filter_map(Result::ok) {
                if let Ok(children) = std::fs::read_to_string(task.path().join("children")) {
                    pending.extend(
                        children
                            .split_whitespace()
                            .filter_map(|p| p.parse::<u32>().ok()),
                    );
                    if pending.len() > 4096 {
                        return None;
                    }
                }
            }
        }
    }
    Some(processes)
}

#[cfg(any(target_os = "macos", windows))]
fn process_tree(pid: u32) -> Option<Vec<(u32, u64)>> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    let mut children = std::collections::BTreeMap::<Pid, Vec<Pid>>::new();
    for (id, process) in system.processes() {
        if let Some(parent) = process.parent() {
            // Windows keeps a dead parent's id on its children, and reuses ids: only a process
            // started after this parent can be its child.
            if system
                .process(parent)
                .is_some_and(|parent| process.start_time() >= parent.start_time())
            {
                children.entry(parent).or_default().push(*id);
            }
        }
    }
    let mut pending = vec![Pid::from_u32(pid)];
    let mut seen = std::collections::BTreeSet::new();
    let mut processes = Vec::new();
    while let Some(current) = pending.pop() {
        if !seen.insert(current) {
            continue;
        }
        if seen.len() > 4096 {
            return None;
        }
        processes.push((
            current.as_u32(),
            system.process(current).map_or(0, |p| p.memory()),
        ));
        pending.extend(children.get(&current).into_iter().flatten().copied());
    }
    Some(processes)
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) fn resident_tree_bytes(pid: u32) -> Option<u64> {
    match process_tree(pid) {
        Some(processes) => {
            let bytes = processes
                .into_iter()
                .map(|(_, memory)| memory)
                .fold(0u64, u64::saturating_add);
            (bytes > 0).then_some(bytes)
        }
        None => Some(u64::MAX),
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn resident_tree_bytes(pid: u32) -> Option<u64> {
    resident_bytes(pid)
}

/// The memory process `pid` holds, in bytes, when it can be measured.
#[cfg(target_os = "linux")]
pub(crate) fn resident_bytes(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kb.checked_mul(1024)
}

/// The memory process `pid` holds, in bytes, when it can be measured.
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn resident_bytes(pid: u32) -> Option<u64> {
    let out = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let kb: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    kb.checked_mul(1024)
}

/// The memory process `pid` holds, in bytes, when it can be measured.
#[cfg(windows)]
pub(crate) fn resident_bytes(pid: u32) -> Option<u64> {
    // `"name","pid","session","#","12,345 K"`
    let out = Command::new("tasklist")
        .hide_console_window()
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let field = text.lines().next()?.rsplit("\",\"").next()?;
    let kb: u64 = field
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()?;
    kb.checked_mul(1024)
}

/// The memory process `pid` holds: not measured on this system.
#[cfg(not(any(unix, windows)))]
pub(crate) fn resident_bytes(_pid: u32) -> Option<u64> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tree_tests {
    use super::*;
    #[test]
    fn a_worker_limit_counts_and_stops_its_kernel_children() {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new("bash");
        command.args(["-c", "sleep 30 & wait"]).process_group(0);
        let mut child = command.spawn().unwrap();
        // The child appears, and then gains memory, some time after the spawn.
        let deadline = Instant::now() + Duration::from_secs(10);
        let counted = loop {
            let own = resident_bytes(child.id()).unwrap();
            let tree = resident_tree_bytes(child.id()).unwrap();
            if tree > own || Instant::now() > deadline {
                break tree > own;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(counted);
        let children =
            std::fs::read_to_string(format!("/proc/{}/task/{}/children", child.id(), child.id()))
                .unwrap();
        stop(&mut child);
        for pid in children.split_whitespace() {
            // A killed child may briefly remain a zombie until reaped, but holds no RSS.
            let pid = pid.parse().unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            while resident_bytes(pid).unwrap_or(0) > 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(resident_bytes(pid).unwrap_or(0), 0);
        }
    }
}

#[cfg(test)]
mod classification_tests {
    use super::*;

    #[test]
    fn member_classification_protocol_round_trip_allows_only_fixed_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let salt = crate::anon::Salt::parse("00112233445566778899aabbccddeeff").unwrap();
        let options = ConvertOptions {
            diagnostics_salt: Some(salt.clone()),
            ..Default::default()
        };
        let log = crate::diagnostics::Log::open(&path, Path::new("Private.f3z"), &options).unwrap();
        let mut record = classified_record("/private/SecretMember.f3d", 19, 11, 5, 3);
        record["type"] = "PrivateType".into();
        record["PrivateLabel"] = "PrivateValue".into();
        let mut malformed = classified_record("/private/Second.f3d", 0, 0, 0, 0);
        malformed["ms"] = "PrivateTiming".into();
        malformed["layers_ms"] = (-1).into();
        let stream = format!("{record}\n{malformed}\n");
        follow_stages(
            stream.as_bytes(),
            &Mutex::new(Stage::Input),
            &Mutex::new(Reading::new(false)),
            &|_| {},
            Some(&log),
        );
        drop(log);
        let text = std::fs::read_to_string(path).unwrap();
        let rows: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1]["type"], "member_classified");
        assert_eq!(
            rows[1]["member"],
            salt.member_id("/private/SecretMember.f3d")
        );
        assert_eq!(rows[1]["ms"], 19);
        assert_eq!(rows[1]["nested_scan_ms"], 11);
        assert_eq!(rows[1]["layers_ms"], 5);
        assert_eq!(rows[1]["metadata_ms"], 3);
        assert_eq!(rows[2]["ms"], 0);
        assert_eq!(rows[2]["layers_ms"], 0);
        for secret in ["Private", "/private", "SecretMember", "Second.f3d"] {
            assert!(!text.contains(secret));
        }
    }
}

#[cfg(test)]
mod left_out_tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use std::io::Write as _;

    fn zip_of(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn an_archive_copy_drops_only_the_stuck_design() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.f3z");
        zip_of(
            &input,
            &[
                ("Root.f3d", b"root"),
                ("Stuck.f3d", b"stuck"),
                ("Manifest.json", b"{}"),
            ],
        );
        let out = dir.path().join("out.f3z");
        assert!(without_entry(&input, "Stuck.f3d", &out).unwrap());
        let mut copy = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let names: Vec<String> = copy
            .file_names()
            .map(|name| name.unwrap().into_owned())
            .collect();
        assert_eq!(names, ["Root.f3d", "Manifest.json"]);
        let mut text = String::new();
        copy.by_name("Root.f3d")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "root");
        assert!(!without_entry(&input, "Absent.f3d", &dir.path().join("x.f3z")).unwrap());
        assert!(archive_without(&input, "Absent.f3d").is_none());
        let (_keep, copy) = archive_without(&input, "Stuck.f3d").unwrap();
        assert_eq!(copy.file_name(), input.file_name());
        assert!(is_archive(&input) && !is_archive(Path::new("part.f3d")));
    }

    #[test]
    fn a_left_out_design_is_a_quoted_loss_or_part_of_the_error() {
        let limits = Limits {
            design: Some(Duration::from_secs(20 * 60)),
            ..Limits::default()
        };
        let input = Path::new("/home/me/Assembly.f3z");
        let mut ok = serde_json::json!({"ok": true, "input": {"path": "/tmp/copy/Assembly.f3z"},
            "losses": [], "loss_groups": []});
        mark_left_out(&mut ok, input, &["Gear Box.f3d".into()], &limits);
        assert_eq!(ok["input"], json::input(input));
        assert_eq!(ok["left_out_designs"], serde_json::json!(["Gear Box.f3d"]));
        let loss = &ok["losses"][0];
        assert_eq!(loss["level"], "Dropped");
        assert_eq!(loss["stage"], "decode");
        let message = loss["message"].as_str().unwrap();
        assert!(
            message.starts_with("\"Gear Box.f3d\" was left out"),
            "{message}"
        );
        assert!(
            message.contains("ran past 20 minutes with no progress, or 60 minutes in all"),
            "{message}"
        );
        assert_eq!(minutes(Some(Duration::from_secs(30))), "1 minute");
        assert_eq!(ok["loss_groups"][0]["count"], 1);
        let mut failed = serde_json::json!({"ok": false, "error": {"message": "no root"}});
        mark_left_out(
            &mut failed,
            input,
            &["A.f3d".into(), "B.f3d".into()],
            &limits,
        );
        let message = failed["error"]["message"].as_str().unwrap();
        assert!(message.contains("\"A.f3d\", \"B.f3d\""), "{message}");
    }

    #[test]
    fn the_design_limit_is_a_setting_and_users_come_from_the_description() {
        let limits = Limits::default();
        assert_eq!(design_limit(&limits, None), Some(DEFAULT_DESIGN_LIMIT));
        assert_eq!(
            design_limit(&limits, Some(" 45 ")),
            Some(Duration::from_secs(45 * 60))
        );
        assert_eq!(design_limit(&limits, Some("0")), None);
        assert_eq!(
            design_limit(&limits, Some("lots")),
            Some(DEFAULT_DESIGN_LIMIT)
        );
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("a.f3z");
        zip_of(
            &input,
            &[(
                "DesignDescription.json",
                br#"{"designDescription":{"designGraphs":[{"rootIds":[0],"designObjects":[
                    {"id":0,"relativePath":"root.f3d","references":[{"type":"XREF","ids":[1,2]}]},
                    {"id":1,"relativePath":"gear.f3d","references":[]},
                    {"id":2,"relativePath":"shaft.f3d","references":[{"type":"XREF","ids":[1]}]}]}]}}"#,
            )],
        );
        assert_eq!(users_of(&input, "gear.f3d"), ["root.f3d", "shaft.f3d"]);
        assert!(users_of(&input, "root.f3d").is_empty());
        assert!(users_of(&dir.path().join("missing.f3z"), "gear.f3d").is_empty());
    }

    #[test]
    fn the_stuck_clock_runs_only_while_reading_and_restarts_per_design() {
        let mut reading = Reading::new(false);
        let none = Duration::ZERO;
        assert_eq!(reading.stuck(none), None);
        reading.stage(Stage::Decode);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(reading.stuck(none), Some(None));
        reading.design(Some("Part.f3d".into()));
        assert_eq!(reading.stuck(Duration::from_secs(60)), None);
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(reading.stuck(none), Some(Some("Part.f3d".into())));
        // Memory that moves is work, not a stuck design; a small wobble is not.
        reading.memory(1 << 30);
        std::thread::sleep(Duration::from_millis(5));
        reading.memory((1 << 30) + (1 << 20));
        assert!(reading.stuck(Duration::from_millis(2)).is_some());
        reading.memory(2 << 30);
        assert_eq!(reading.stuck(Duration::from_secs(60)), None);
        reading.stage(Stage::Write);
        assert_eq!(reading.stuck(none), None);
    }

    #[test]
    fn a_design_that_keeps_moving_memory_still_stops_at_three_times_the_limit() {
        // Long against the loop's 5 ms sleeps: a busy CI runner can stretch one sleep past a few
        // tens of ms, which would read as "no progress" and stop an archive that is still moving.
        let limit = Duration::from_millis(250);
        let mut reading = Reading::new(false);
        reading.stage(Stage::Decode);
        reading.design(Some("Runaway.f3d".into()));
        let began = Instant::now();
        let mut bytes = 1u64 << 30;
        // Memory grows past the progress mark every few ms, as a runaway allocation would.
        let stopped = loop {
            bytes += PROGRESS_BYTES;
            reading.memory(bytes);
            if let Some(design) = reading.stuck(limit) {
                break (design, began.elapsed());
            }
            assert!(began.elapsed() < Duration::from_secs(5), "never stopped");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(stopped.0.as_deref(), Some("Runaway.f3d"));
        assert!(stopped.1 >= limit * 3, "{:?}", stopped.1);
        // A new design starts its own allowance.
        reading.design(Some("Next.f3d".into()));
        assert_eq!(reading.stuck(limit), None);

        // An archive's preparation before its first design, memory moving all the while, runs
        // past the ceiling without being stopped (a 396 MB archive took 54 minutes); a single
        // file's decode does not get that pass.
        for archive in [true, false] {
            let mut reading = Reading::new(archive);
            reading.stage(Stage::Decode);
            let began = Instant::now();
            let mut bytes = 1u64 << 30;
            while began.elapsed() < limit * 4 {
                bytes += PROGRESS_BYTES;
                reading.memory(bytes);
                if !archive && reading.stuck(limit).is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                reading.stuck(limit).is_some(),
                !archive,
                "archive={archive}: {:?}",
                began.elapsed()
            );
            if archive {
                // Its first design gets the ceiling from the moment it starts.
                reading.design(Some("Heavy.f3d".into()));
                std::thread::sleep(limit * 3 + Duration::from_millis(10));
                reading.memory(bytes + PROGRESS_BYTES);
                assert_eq!(reading.stuck(limit), Some(Some("Heavy.f3d".into())));
            }
        }
    }
}
