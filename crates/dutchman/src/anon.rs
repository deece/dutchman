//! The anonymous report of a batch (`dutchman.report-anon/1`): what converted and what did
//! not, by format, stage and cause, with nothing that names or shows a file. Each file is a
//! salted hash of its path; the salt stays with whoever ran the batch, so they alone can tell
//! which file a reported id is. Messages keep their wording but lose every path, file name,
//! quoted name and long id in them. See `docs/report-anon.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use dutchman_core::{Format, Stage};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// The schema of the anonymous report.
pub const SCHEMA: &str = "dutchman.report-anon/1";

/// The cadmpeg version Dutchman was built with.
pub const CADMPEG_VERSION: &str = env!("DUTCHMAN_CADMPEG_VERSION");

/// How many files' ids one cause lists.
const MAX_IDS: usize = 20;
/// The most distinct timeline positions listed for one blocked-feature cause.
const MAX_POSITIONS: usize = 50;

/// How many of the slowest files the report lists.
const SLOWEST: usize = 20;

/// The longest message the report keeps, in characters.
const MAX_MESSAGE: usize = 400;

/// File extensions whose names are replaced in messages, besides those of the formats
/// Dutchman knows.
const EXTENSIONS: [&str; 24] = [
    "f3d",
    "f3z",
    "ipt",
    "iam",
    "ipn",
    "idw",
    "sldprt",
    "sldasm",
    "slddrw",
    "step",
    "stp",
    "iges",
    "igs",
    "sat",
    "sab",
    "smb",
    "smbh",
    "x_t",
    "x_b",
    "fcstd",
    "3dm",
    "prt",
    "asm",
    "solidcraft",
];

/// Words messages join with slashes (`body/region/shell/face`, `and/or`); a slashed word of
/// anything else is taken for a path.
const ALTERNATIVES: [&str; 22] = [
    "and", "or", "body", "region", "shell", "face", "loop", "coedge", "edge", "vertex", "point",
    "curve", "surface", "solid", "sheet", "wire", "inside", "outside", "x", "y", "z", "s",
];

/// The secret that makes the report's file ids: whoever holds it can tell which file an id
/// is, by hashing their own files' paths with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Salt([u8; 16]);

impl Salt {
    /// A new random salt.
    pub fn random() -> Result<Salt, String> {
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).map_err(|err| format!("cannot make a salt: {err}"))?;
        Ok(Salt(bytes))
    }

    /// The salt written as `hex` (32 hexadecimal digits), as [`Salt::hex`] writes it.
    pub fn parse(hex: &str) -> Result<Salt, String> {
        let digits: Vec<u8> = hex
            .trim()
            .chars()
            .map(|c| c.to_digit(16).and_then(|d| u8::try_from(d).ok()))
            .collect::<Option<_>>()
            .ok_or("a salt is 32 hexadecimal digits")?;
        let bytes: Vec<u8> = digits
            .chunks(2)
            .map(|pair| match pair {
                [high, low] => Some(high * 16 + low),
                _ => None,
            })
            .collect::<Option<_>>()
            .ok_or("a salt is 32 hexadecimal digits")?;
        bytes
            .try_into()
            .map(Salt)
            .map_err(|_| "a salt is 32 hexadecimal digits".to_owned())
    }

    /// The salt as 32 hexadecimal digits.
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The id of the file at `path`: the salted hash of its full path.
    pub fn file_id(&self, path: &Path) -> String {
        let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.member_id(&full.to_string_lossy())
    }

    /// The same salted hash for a logical archive path, without resolving it on the host.
    pub(crate) fn member_id(&self, path: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(self.0);
        hash.update(path.as_bytes());
        let digest = hash.finalize();
        let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        format!("f{hex}")
    }
}

/// What the report says about the run besides its files.
#[derive(Clone, Debug, Default)]
pub struct Run {
    /// SolidCraft's version, when a SolidCraft CLI checked the designs.
    pub solidcraft: Option<String>,
    /// How the SolidCraft CLI search went when none was found: generic place names only.
    pub solidcraft_search: Option<Value>,
    /// Whether written designs were checked in SolidCraft.
    pub verify: bool,
    /// Whether only geometry was asked for.
    pub import_only: bool,
    /// How many files converted at once.
    pub jobs: usize,
    /// One file's time limit, in seconds.
    pub timeout_s: Option<u64>,
    /// One file's memory limit, in megabytes.
    pub max_memory_mb: Option<u64>,
}

/// The anonymous report of `batch` (a `dutchman.convert-all/1` report) for a run of
/// `inputs` into `out_dir`, with each file named by its id under `salt`; and the key from
/// each id to its file's path, for whoever ran the batch to keep.
pub fn report(
    batch: &Value,
    inputs: &[PathBuf],
    out_dir: &Path,
    salt: &Salt,
    run: &Run,
) -> (Value, BTreeMap<String, String>) {
    let results: &[Value] = batch
        .get("results")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let mut scrub = Scrubber::default();
    for path in inputs.iter().map(PathBuf::as_path).chain([out_dir]) {
        scrub.know_path(path);
    }
    let mut key = BTreeMap::new();
    let mut files = Vec::new();
    for result in results {
        let input = result
            .pointer("/input/path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_default();
        scrub.know_path(&input);
        if let Some(output) = result.pointer("/output/path").and_then(Value::as_str) {
            scrub.know_path(Path::new(output));
        }
        scrub.know_names_in(result);
        let id = salt.file_id(&input);
        key.insert(id.clone(), input.display().to_string());
        files.push(File::read(result, id));
    }

    let mut out = Map::new();
    out.insert("schema".to_owned(), json!(SCHEMA));
    out.insert(
        "versions".to_owned(),
        json!({
            "dutchman": env!("CARGO_PKG_VERSION"),
            "cadmpeg": CADMPEG_VERSION,
            "solidcraft": run.solidcraft.as_deref().map(version_only),
            "solidcraft_search": run.solidcraft_search,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        }),
    );
    out.insert(
        "run".to_owned(),
        json!({
            "verify": run.verify,
            "import_only": run.import_only,
            "jobs": run.jobs,
            "timeout_s": run.timeout_s,
            "max_memory_mb": run.max_memory_mb,
            "resumed": batch.get("resumed").and_then(Value::as_u64).unwrap_or(0),
        }),
    );
    out.insert("totals".to_owned(), tally(files.iter(), batch));
    let mut formats = Map::new();
    let by_format: BTreeSet<&str> = files.iter().map(|f| f.format.as_str()).collect();
    for format in by_format {
        let mut counts = tally(files.iter().filter(|f| f.format == format), &Value::Null);
        if let Some(fields) = counts.as_object_mut() {
            fields.remove("skipped");
            fields.insert(
                "timing".to_owned(),
                timing(files.iter().filter(|f| f.format == format)),
            );
        }
        formats.insert(format.to_owned(), counts);
    }
    out.insert("formats".to_owned(), Value::Object(formats));
    out.insert(
        "skipped_by_extension".to_owned(),
        batch
            .get("skipped_by_extension")
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    out.insert("stages".to_owned(), stages(&files));
    out.insert("errors".to_owned(), errors(&files, &scrub));
    out.insert("losses".to_owned(), losses(&files, &scrub));
    out.insert("blocked_features".to_owned(), blocked(&files, &scrub));
    out.insert("verification".to_owned(), verification(&files, &scrub));
    out.insert("timing".to_owned(), timing(files.iter()));
    out.insert("slowest".to_owned(), slowest(&files));
    (Value::Object(out), key)
}

/// Where to write an anonymous report, and with what salt.
#[derive(Clone, Debug)]
pub struct Request {
    /// The report's file.
    pub path: PathBuf,
    /// The salt for its file ids; a new one when `None`.
    pub salt: Option<Salt>,
    /// A file to write the key from each id to its file's path to, for whoever ran the batch
    /// to keep.
    pub key: Option<PathBuf>,
}

/// Writes the anonymous report `request` asks for of `batch`, a run of `inputs` into
/// `out_dir` with `options` and `settings`, and returns the salt its ids use.
pub fn write(
    request: &Request,
    batch: &Value,
    inputs: &[PathBuf],
    out_dir: &Path,
    options: &crate::ConvertOptions,
    settings: &crate::batch::Settings,
) -> Result<Salt, String> {
    let run = Run::of(options, Some(settings));
    write_run(request, batch, inputs, out_dir, &run)
}

/// Writes the anonymous report of one file's convert report `file`, converted from `input`
/// to `output` with `options`, as the report of a batch of that one file, and returns the
/// salt its id uses.
pub fn write_one(
    request: &Request,
    file: &Value,
    input: &Path,
    output: &Path,
    options: &crate::ConvertOptions,
) -> Result<Salt, String> {
    let batch = json!({"results": [file]});
    let run = Run::of(options, None);
    write_run(
        request,
        &batch,
        &[input.to_path_buf()],
        output.parent().unwrap_or(output),
        &run,
    )
}

impl Run {
    /// What a conversion's own report `report` says about its run, for a report saved earlier
    /// (by the desktop app, say). The SolidCraft search is made now, with `options`, only when
    /// the report found no SolidCraft.
    pub(crate) fn of_report(report: &Value, options: &crate::ConvertOptions) -> Run {
        let solidcraft = report
            .pointer("/solidcraft/found")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let solidcraft_search = (solidcraft.is_none() && options.searches_for_solidcraft())
            .then(|| options.solidcraft_search(false));
        Run {
            verify: solidcraft.is_some()
                && report
                    .pointer("/verification/status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| status != "none"),
            solidcraft_search,
            solidcraft,
            import_only: options.import_only,
            jobs: 1,
            timeout_s: None,
            max_memory_mb: None,
        }
    }

    /// What a run with `options` and `settings` (none: a single file, converted here) says
    /// about itself.
    fn of(options: &crate::ConvertOptions, settings: Option<&crate::batch::Settings>) -> Run {
        let solidcraft = match options.to {
            crate::OutputFormat::Solidcraft => options.reported_solidcraft_version(),
            crate::OutputFormat::Freecad
            | crate::OutputFormat::Fcstd
            | crate::OutputFormat::Dxf => None,
        };
        let solidcraft_search = (options.to == crate::OutputFormat::Solidcraft
            && solidcraft.is_none()
            && options.searches_for_solidcraft())
        .then(|| options.solidcraft_search(false));
        Run {
            verify: !options.no_verify && solidcraft.is_some(),
            solidcraft_search,
            solidcraft,
            import_only: options.import_only,
            jobs: settings.map_or(1, |settings| crate::batch::workers(settings.jobs)),
            timeout_s: settings
                .filter(|settings| settings.worker.is_some())
                .map(|settings| settings.limits.timeout.as_secs()),
            max_memory_mb: settings
                .filter(|settings| settings.worker.is_some())
                .and_then(|settings| settings.limits.max_memory)
                .map(|bytes| bytes >> 20),
        }
    }
}

fn write_run(
    request: &Request,
    batch: &Value,
    inputs: &[PathBuf],
    out_dir: &Path,
    run: &Run,
) -> Result<Salt, String> {
    let salt = match &request.salt {
        Some(salt) => salt.clone(),
        None => Salt::random()?,
    };
    let (anon, key) = report(batch, inputs, out_dir, &salt, run);
    let save = |path: &Path, value: &Value| {
        let text = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
        std::fs::write(path, text + "\n").map_err(|err| format!("{}: {err}", path.display()))
    };
    save(&request.path, &anon)?;
    if let Some(path) = &request.key {
        save(path, &json!({"salt": salt.hex(), "files": key}))?;
    }
    Ok(salt)
}

/// What the report needs of one file's convert report.
struct File<'a> {
    id: String,
    format: String,
    report: &'a Value,
    ok: bool,
    elapsed_ms: Option<u64>,
}

impl<'a> File<'a> {
    fn read(report: &'a Value, id: String) -> File<'a> {
        let format = report
            .pointer("/input/format")
            .and_then(Value::as_str)
            .and_then(|id| Format::ALL.iter().find(|f| f.id() == id))
            .map_or_else(|| "unknown".to_owned(), |f| f.id().to_owned());
        File {
            id,
            format,
            ok: report.get("ok") == Some(&json!(true)),
            elapsed_ms: report.get("elapsed_ms").and_then(Value::as_u64),
            report,
        }
    }

    fn losses(&self) -> impl Iterator<Item = &'a Value> {
        self.report
            .get("losses")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    }

    fn field(&self, pointer: &str) -> Option<&'a str> {
        self.report.pointer(pointer).and_then(Value::as_str)
    }

    /// What became of it: `failed`, `history_verified`, `history_unverified` or
    /// `geometry_only`.
    fn outcome(&self) -> &'static str {
        match (
            self.ok,
            self.field("/mode"),
            self.field("/verification/status"),
        ) {
            (false, _, _) => "failed",
            (true, Some("history"), Some("verified")) => "history_verified",
            (true, Some("history"), _) => "history_unverified",
            (true, _, _) => "geometry_only",
        }
    }

    /// Whether an assembly converted with placements left out or placed empty.
    fn incomplete(&self) -> bool {
        self.ok && crate::present::incomplete(self.report)
    }

    /// Whether it converted but lost something that matters (a Degraded or Dropped loss).
    fn degraded(&self) -> bool {
        self.ok
            && self
                .losses()
                .any(|loss| loss.get("level").and_then(Value::as_str) != Some("Info"))
    }
}

/// The counts of `files`: how many converted, how, and how many failed; with the batch's
/// skipped count when `batch` has one.
fn tally<'a, 'b: 'a>(files: impl Iterator<Item = &'a File<'b>>, batch: &Value) -> Value {
    let mut counts: BTreeMap<&str, u64> = BTreeMap::new();
    for key in [
        "files",
        "converted",
        "history_verified",
        "history_unverified",
        "geometry_only",
        "degraded",
        "incomplete",
        "failed",
    ] {
        counts.insert(key, 0);
    }
    for file in files {
        for key in ["files", file.outcome()] {
            *counts.entry(key).or_insert(0) += 1;
        }
        if file.ok {
            *counts.entry("converted").or_insert(0) += 1;
        }
        if file.degraded() {
            *counts.entry("degraded").or_insert(0) += 1;
        }
        if file.incomplete() {
            *counts.entry("incomplete").or_insert(0) += 1;
        }
    }
    let mut out = json!(counts);
    if let (Some(skipped), Some(fields)) = (
        batch.get("skipped").and_then(Value::as_u64),
        out.as_object_mut(),
    ) {
        fields.insert("skipped".to_owned(), json!(skipped));
    }
    out
}

/// Per stage: the files that failed there, the losses that arose there, and the files with
/// either.
fn stages(files: &[File]) -> Value {
    let mut out = Map::new();
    for stage in Stage::ALL {
        let id = stage.id();
        let mut failed = 0;
        let mut losses = 0;
        let mut touched = 0;
        for file in files {
            let fails = !file.ok && file.field("/error/stage") == Some(id);
            let lost = file
                .losses()
                .filter(|loss| loss.get("stage").and_then(Value::as_str) == Some(id))
                .count();
            failed += usize::from(fails);
            losses += lost;
            touched += usize::from(fails || lost > 0);
        }
        out.insert(
            id.to_owned(),
            json!({"failed": failed, "losses": losses, "files": touched}),
        );
    }
    Value::Object(out)
}

/// One cause, gathered across files.
#[derive(Default)]
struct Cause {
    fields: Map<String, Value>,
    /// Timeline positions: (ordinal, variant) -> files showing it.
    positions: BTreeMap<(Option<u64>, String), u64>,
    files: BTreeSet<String>,
    formats: BTreeMap<String, u64>,
    occurrences: u64,
}

impl Cause {
    fn add(&mut self, file: &File) {
        self.occurrences += 1;
        if self.files.insert(file.id.clone()) {
            *self.formats.entry(file.format.clone()).or_insert(0) += 1;
        }
    }

    fn json(self) -> Value {
        let mut out = self.fields;
        if !self.positions.is_empty() {
            let features: Vec<Value> = self
                .positions
                .iter()
                .take(MAX_POSITIONS)
                .map(|((ordinal, variant), count)| {
                    json!({"ordinal": ordinal, "variant": variant, "count": count})
                })
                .collect();
            out.insert("features".to_owned(), json!(features));
        }
        out.insert("files".to_owned(), json!(self.files.len()));
        out.insert("occurrences".to_owned(), json!(self.occurrences));
        out.insert("formats".to_owned(), json!(self.formats));
        out.insert(
            "file_ids".to_owned(),
            json!(self.files.iter().take(MAX_IDS).collect::<Vec<_>>()),
        );
        Value::Object(out)
    }
}

/// Causes gathered by key, most files first.
fn ranked(causes: BTreeMap<String, Cause>) -> Value {
    let mut causes: Vec<Cause> = causes.into_values().collect();
    causes.sort_by(|a, b| {
        b.files
            .len()
            .cmp(&a.files.len())
            .then(b.occurrences.cmp(&a.occurrences))
    });
    Value::Array(causes.into_iter().map(Cause::json).collect())
}

/// Every distinct failure, by its message with numbers set aside.
fn errors(files: &[File], scrub: &Scrubber) -> Value {
    let mut causes: BTreeMap<String, Cause> = BTreeMap::new();
    for file in files.iter().filter(|f| !f.ok) {
        let message = scrub.text(file.field("/error/message").unwrap_or("failed"));
        let stage = file.field("/error/stage").unwrap_or("input");
        let upstream = file.report.pointer("/error/upstream").cloned();
        let cause = causes
            .entry(format!("{stage}|{}", template(&message)))
            .or_default();
        if cause.fields.is_empty() {
            cause.fields.insert("message".to_owned(), json!(message));
            cause.fields.insert("stage".to_owned(), json!(stage));
            cause
                .fields
                .insert("upstream".to_owned(), upstream.unwrap_or(Value::Null));
        }
        cause.add(file);
    }
    ranked(causes)
}

/// Every loss, by its cadmpeg code, or for Dutchman's own by the kind of thing it is about
/// and its message with numbers set aside.
fn losses(files: &[File], scrub: &Scrubber) -> Value {
    let mut causes: BTreeMap<String, Cause> = BTreeMap::new();
    for file in files {
        for loss in file.losses() {
            let text = |key: &str| loss.get(key).and_then(Value::as_str).unwrap_or("");
            let message = scrub.text(text("message"));
            let code = match loss.get("code").and_then(Value::as_str) {
                Some(code) => generic(code, "cadmpeg/other"),
                None => format!("dutchman/{}", subject_kind(text("subject"))),
            };
            let grouping = if loss.get("code").and_then(Value::as_str).is_some() {
                String::new()
            } else {
                template(&message)
            };
            let cause = causes
                .entry(format!("{}|{code}|{grouping}", text("level")))
                .or_default();
            if cause.fields.is_empty() {
                cause.fields.insert("code".to_owned(), json!(code));
                cause
                    .fields
                    .insert("level".to_owned(), json!(text("level")));
                cause
                    .fields
                    .insert("stage".to_owned(), json!(text("stage")));
                cause.fields.insert(
                    "upstream".to_owned(),
                    loss.get("upstream").cloned().unwrap_or(Value::Null),
                );
                if !code.starts_with("dutchman/appearance.") {
                    cause.fields.insert("example".to_owned(), json!(message));
                }
            }
            cause.add(file);
        }
    }
    ranked(causes)
}

/// The features that kept a history from being written, by kind and reason.
fn blocked(files: &[File], scrub: &Scrubber) -> Value {
    let mut causes: BTreeMap<String, Cause> = BTreeMap::new();
    for file in files {
        let features = file
            .report
            .pointer("/history/features")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for feature in features {
            if feature.get("status").and_then(Value::as_str) != Some("blocked") {
                continue;
            }
            let kind = generic(
                feature.get("kind").and_then(Value::as_str).unwrap_or("?"),
                "other",
            );
            let reason = scrub.text(
                feature
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("not mapped"),
            );
            let cause = causes
                .entry(format!("{kind}|{}", template(&reason)))
                .or_default();
            if cause.fields.is_empty() {
                cause.fields.insert("kind".to_owned(), json!(kind));
                cause.fields.insert("reason".to_owned(), json!(reason));
                cause.fields.insert("stage".to_owned(), json!("map"));
                for key in ["source", "category", "upstream"] {
                    if let Some(value) = feature.get(key) {
                        cause.fields.insert(key.to_owned(), value.clone());
                    }
                }
            }
            if let Some(variant) = feature.get("variant").and_then(Value::as_str) {
                let ordinal = feature.get("ordinal").and_then(Value::as_u64);
                *cause
                    .positions
                    .entry((ordinal, variant_text(variant)))
                    .or_insert(0) += 1;
            }
            cause.add(file);
        }
    }
    ranked(causes)
}

/// A variant string re-checked against the fixed words it is made of, so a name could
/// never pass through it.
fn variant_text(variant: &str) -> String {
    let ok = variant
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ' || c == ',');
    if ok {
        variant.to_owned()
    } else {
        "other".to_owned()
    }
}

/// Why histories were written unchecked, or not written, by status and reason.
fn verification(files: &[File], scrub: &Scrubber) -> Value {
    let mut causes: BTreeMap<String, Cause> = BTreeMap::new();
    for file in files.iter().filter(|f| f.ok) {
        let status = file.field("/verification/status").unwrap_or("none");
        if matches!(status, "verified" | "none") {
            continue;
        }
        let reason = scrub.text(file.field("/verification/reason").unwrap_or(""));
        let cause = causes
            .entry(format!("{status}|{}", template(&reason)))
            .or_default();
        if cause.fields.is_empty() {
            cause.fields.insert("status".to_owned(), json!(status));
            cause.fields.insert("reason".to_owned(), json!(reason));
        }
        cause.add(file);
    }
    ranked(causes)
}

/// Percentiles of the files' times.
fn timing<'a, 'b: 'a>(files: impl Iterator<Item = &'a File<'b>>) -> Value {
    let mut times: Vec<u64> = files.filter_map(|f| f.elapsed_ms).collect();
    times.sort_unstable();
    let at = |fraction: f64| -> Option<u64> {
        let last = times.len().checked_sub(1)?;
        // The nearest rank: of 10 times, p90 is the 9th.
        let rank = ((times.len() as f64) * fraction).ceil() as usize;
        times.get(rank.saturating_sub(1).min(last)).copied()
    };
    json!({
        "count": times.len(),
        "p50_ms": at(0.5),
        "p90_ms": at(0.9),
        "p99_ms": at(0.99),
        "max_ms": times.last(),
        "total_ms": times.iter().sum::<u64>(),
    })
}

/// The slowest files, slowest first.
fn slowest(files: &[File]) -> Value {
    let mut timed: Vec<&File> = files.iter().filter(|f| f.elapsed_ms.is_some()).collect();
    timed.sort_by_key(|f| std::cmp::Reverse(f.elapsed_ms));
    Value::Array(
        timed
            .into_iter()
            .take(SLOWEST)
            .map(|f| {
                json!({
                    "file": f.id,
                    "format": f.format,
                    "ms": f.elapsed_ms,
                    "outcome": f.outcome(),
                    "stage": f.field("/error/stage"),
                })
            })
            .collect(),
    )
}

/// The kinds of thing Dutchman's own losses are about, as their codes name them. A subject
/// is a name (a file's, a part's) as often as a kind, so a code is only ever one of these.
const KINDS: [&str; 18] = [
    "appearance",
    "assembly",
    "bodies",
    "components",
    "constraints",
    "feature",
    "full",
    "geometry",
    "history",
    "joints",
    "occurrence",
    "parameter",
    "parts",
    "pattern",
    "profile",
    "sketch",
    "validation",
    "writer",
];

/// The kind of thing a loss of Dutchman's own is about: the first word of its subject (the
/// part after a part file's name in an assembly) when it is one of [`KINDS`], else `other`.
pub(crate) fn subject_kind(subject: &str) -> &'static str {
    let own = subject.rsplit(": ").next().unwrap_or(subject);
    let word = own.split_whitespace().next().unwrap_or("");
    KINDS
        .iter()
        .find(|kind| kind.eq_ignore_ascii_case(word))
        .copied()
        .unwrap_or("other")
}

/// Exact allow-list: design-derived strings must never become appearance categories.
const APPEARANCE_CODES: [&str; 6] = [
    "dutchman/appearance.redundant-face",
    "dutchman/appearance.wire",
    "dutchman/appearance.face-pick-missed",
    "dutchman/appearance.target-absent",
    "dutchman/appearance.unwritable-body",
    "dutchman/appearance.other",
];

/// `code` when it is a code (lower-case letters, digits and `_ . / -`, at most 80), else
/// `fallback`: cadmpeg's codes are fixed words, and nothing else is let through as one.
pub(crate) fn generic(code: &str, fallback: &str) -> String {
    if code.starts_with("dutchman/") {
        return if APPEARANCE_CODES.contains(&code) {
            code
        } else {
            "dutchman/other"
        }
        .to_owned();
    }
    let fixed = !code.is_empty()
        && code.len() <= 80
        && code.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '/' | '-')
        });
    if fixed { code } else { fallback }.to_owned()
}

/// `text` with every run of digits written `N`, so messages that differ only in their counts
/// are one cause.
fn template(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_number = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            if !in_number {
                out.push('N');
            }
            in_number = true;
        } else {
            in_number = false;
            out.push(c);
        }
    }
    out
}

/// A version as a program prints it (`solidcraft-cli 0.18.2`), cut to the version itself.
fn version_only(text: &str) -> String {
    text.split_whitespace()
        .last()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        .take(40)
        .collect()
}

/// Takes names out of messages: the paths and names it was told of, then anything that looks
/// like a path, a file name, a quoted name or a long id.
#[derive(Default)]
pub struct Scrubber {
    /// Text known to be a path, taken out wherever it appears.
    paths: BTreeSet<String>,
    /// Names known to name a file, folder, part or anything else in a design, taken out where
    /// they stand as words of their own.
    names: BTreeSet<String>,
    /// Both, matched in one pass, longest first; built on first use.
    matcher: std::sync::OnceLock<Option<Matcher>>,
}

/// Every known path and name, as one automaton, with which of the two each pattern is.
struct Matcher {
    automaton: aho_corasick::AhoCorasick,
    is_path: Vec<bool>,
}

impl Scrubber {
    /// Learns `path`: its given and full forms whole, and each of its folders' and its own
    /// names.
    pub fn know_path(&mut self, path: &Path) {
        let mut forms = vec![path.display().to_string()];
        if let Ok(full) = std::fs::canonicalize(path) {
            forms.push(full.display().to_string());
        }
        for form in forms {
            self.know_stored(&form);
        }
    }

    /// Learns a path in any machine's form, and each of its parts and their stems.
    fn know_stored(&mut self, stored: &str) {
        if stored.contains(['/', '\\']) && stored.chars().count() >= 3 {
            self.paths.insert(stored.to_owned());
        }
        for part in stored.split(['/', '\\']) {
            self.know_name(part);
            if let Some((stem, _)) = part.rsplit_once('.') {
                self.know_name(stem);
            }
        }
    }

    /// Learns `name` when it is distinctive enough to take out wherever it stands.
    fn know_name(&mut self, name: &str) {
        let name = name.trim();
        if distinctive(name) {
            self.names.insert(name.to_owned());
        }
    }

    /// Learns every name a convert report says its conversion met (`names`) and the names of
    /// its features.
    pub fn know_names_in(&mut self, report: &Value) {
        let met = report.get("names").and_then(Value::as_array);
        let features = report
            .pointer("/history/features")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|feature| feature.get("name"));
        for name in met.into_iter().flatten().chain(features) {
            if let Some(name) = name.as_str() {
                self.know_stored(name);
                self.know_name(name);
            }
        }
    }

    fn matcher(&self) -> Option<&Matcher> {
        self.matcher
            .get_or_init(|| {
                let patterns: Vec<&String> = self.paths.iter().chain(&self.names).collect();
                if patterns.is_empty() {
                    return None;
                }
                let automaton = aho_corasick::AhoCorasick::builder()
                    .match_kind(aho_corasick::MatchKind::LeftmostLongest)
                    .build(&patterns)
                    .ok()?;
                let is_path = (0..patterns.len()).map(|i| i < self.paths.len()).collect();
                Some(Matcher { automaton, is_path })
            })
            .as_ref()
    }

    /// `text` with every known path and name taken out.
    fn known(&self, text: &str) -> String {
        let Some(matcher) = self.matcher() else {
            return text.to_owned();
        };
        let mut out = String::with_capacity(text.len());
        let mut kept = 0;
        for found in matcher.automaton.find_iter(text) {
            let is_path = matcher
                .is_path
                .get(found.pattern().as_usize())
                .copied()
                .unwrap_or(false);
            let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
            let before = text
                .get(..found.start())
                .and_then(|t| t.chars().next_back());
            let after = text.get(found.end()..).and_then(|t| t.chars().next());
            if !is_path && (word(before) || word(after)) {
                continue;
            }
            out.push_str(text.get(kept..found.start()).unwrap_or(""));
            out.push_str(if is_path { "<file>" } else { "<name>" });
            kept = found.end();
        }
        out.push_str(text.get(kept..).unwrap_or(""));
        out
    }

    /// Whether `text` holds a path or name this scrubber knows, where [`Scrubber::text`] would
    /// take it out.
    pub fn holds_known(&self, text: &str) -> bool {
        self.known(text) != text
    }

    /// `text` with every name taken out.
    pub fn text(&self, text: &str) -> String {
        let out = self.known(text);
        let out = quoted(&out);
        let out = out.split(' ').map(token).collect::<Vec<_>>().join(" ");
        let out = long_ids(&out);
        if out.chars().count() > MAX_MESSAGE {
            format!("{} …", out.chars().take(MAX_MESSAGE).collect::<String>())
        } else {
            out
        }
    }
}

/// Whether a name is distinctive enough to take out wherever it stands as a word: three
/// characters or more, with anything but lower-case ASCII letters in it (a capital, a digit, a
/// space, punctuation, any other script). Plain lower-case words (`import`, `parts`) are the
/// words messages are made of; they go only within the paths and quoted names they are part
/// of.
fn distinctive(name: &str) -> bool {
    name.chars().count() >= 3 && !name.chars().all(|c| c.is_ascii_lowercase())
}

/// `text` with what is inside double quotes, curly quotes, backticks and single quotes
/// written `<name>`. A single quote opens only after a space or bracket and closes only before
/// one, so an apostrophe (`assembly's`) is left alone.
fn quoted(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let word = |i: Option<usize>| {
        i.and_then(|i| chars.get(i))
            .is_some_and(|c| c.is_alphanumeric())
    };
    let mut out = String::with_capacity(text.len());
    let mut closing: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        match closing {
            Some(close) if c == close && (close != '\'' || !word(Some(i + 1))) => {
                out.push_str("<name>");
                out.push(c);
                closing = None;
            }
            Some(_) => {}
            None => {
                out.push(c);
                closing = match c {
                    '"' => Some('"'),
                    '`' => Some('`'),
                    '\u{201c}' => Some('\u{201d}'),
                    '\u{2018}' => Some('\u{2019}'),
                    '\'' if !word(i.checked_sub(1))
                        && chars.get(i + 1..).is_some_and(|rest| rest.contains(&'\'')) =>
                    {
                        Some('\'')
                    }
                    _ => None,
                };
            }
        }
    }
    if closing.is_some() {
        // An unclosed quote: drop the rest rather than keep a name.
        out.push_str("<name>");
    }
    out
}

/// One space-separated word, written `<path>` or `<file>` when it looks like one, keeping the
/// punctuation around it.
fn token(word: &str) -> String {
    let start = word
        .find(|c: char| !matches!(c, '(' | '[' | '{' | '\''))
        .unwrap_or(word.len());
    let end = word
        .rfind(|c: char| !matches!(c, ')' | ']' | '}' | ',' | ';' | ':' | '.' | '\''))
        .map_or(start, |i| {
            i + word[i..].chars().next().map_or(1, char::len_utf8)
        });
    let (Some(before), Some(core), Some(after)) = (
        word.get(..start),
        word.get(start..end.max(start)),
        word.get(end.max(start)..),
    ) else {
        return word.to_owned();
    };
    if core.is_empty() || core.starts_with('<') && core.ends_with('>') {
        return word.to_owned();
    }
    let has_extension = core.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
                || Format::ALL
                    .iter()
                    .any(|f| f.extension().eq_ignore_ascii_case(ext)))
    });
    let drive = core.len() > 2 && core.as_bytes().get(1) == Some(&b':') && core.contains('\\');
    let rooted = core.starts_with('/') || core.starts_with('~') || core.starts_with("\\\\");
    // A relative path (`Vendor Parts/M3/screw.ipt`, `acme/jigs`), unless it is words of a
    // message joined by slashes, as messages write alternatives (`body/region/shell/face`).
    let separated = core.contains(['/', '\\'])
        && !core
            .split('/')
            .all(|word| ALTERNATIVES.contains(&word.to_ascii_lowercase().as_str()));
    if rooted || drive || separated {
        format!("{before}<path>{after}")
    } else if has_extension {
        format!("{before}<file>{after}")
    } else {
        word.to_owned()
    }
}

/// `text` with every run of 16 or more hexadecimal digits and dashes (a GUID, a hash) written
/// `<id>`.
fn long_ids(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        let digits = run.chars().filter(char::is_ascii_hexdigit).count();
        if digits >= 16 {
            out.push_str("<id>");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in text.chars() {
        if c.is_ascii_hexdigit() || c == '-' {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn appearance_codes_use_an_exact_allow_list() {
        for code in APPEARANCE_CODES {
            assert_eq!(generic(code, "cadmpeg/other"), code);
        }
        for code in [
            "dutchman/appearance.private-name",
            "dutchman/appearance.face-pick-missed/private-id",
        ] {
            assert_eq!(generic(code, "cadmpeg/other"), "dutchman/other");
        }
    }

    #[test]
    fn a_salt_reads_back_as_it_was_written() {
        let salt = Salt::random().unwrap();
        assert_eq!(Salt::parse(&salt.hex()).unwrap(), salt);
        assert!(Salt::parse("xyz").is_err());
        assert!(Salt::parse("00").is_err());
        let path = Path::new("/nowhere/part.f3d");
        assert_eq!(salt.file_id(path), salt.file_id(path));
        assert_ne!(salt.file_id(path), Salt::random().unwrap().file_id(path));
        assert_eq!(salt.file_id(path).len(), 17);
    }

    #[test]
    fn names_paths_and_ids_are_taken_out_of_messages() {
        let mut scrub = Scrubber::default();
        scrub.know_path(Path::new("/home/someone/Client Work/Gearbox_Housing.f3d"));
        let cases = [
            (
                "/home/someone/Client Work/Gearbox_Housing.f3d: not a file",
                "<file>: not a file",
            ),
            (
                "feature \"Housing Boss 3\": fillet is not mapped yet",
                "feature \"<name>\": fillet is not mapped yet",
            ),
            (
                "the selected stream `FusionAssetName[Active]/Breps.BlobParts/BREP.61330db9-cd9a-4963-b43a-df00caf080cb.smb` is not a decodable SAB",
                "the selected stream `<name>` is not a decodable SAB",
            ),
            (
                "Gearbox_Housing_v2.ipt: not found next to the assembly",
                "<file>: not found next to the assembly",
            ),
            (
                "cannot read C:\\Users\\someone\\bracket.sldprt, nor ./parts/pin.ipt.",
                "cannot read <path>, nor <path>.",
            ),
            (
                "part ef3b6157-6149-40e6-9f67-ad3ee2f50422 is external",
                "part <id> is external",
            ),
            (
                "B-rep topology (body/region/shell/face) was not built",
                "B-rep topology (body/region/shell/face) was not built",
            ),
            ("3 of 5 constraints not carried", "3 of 5 constraints not carried"),
            (
                "the assembly's part 'Gear Shaft:1' is unplaced, as is \u{2018}Cam\u{2019}",
                "the assembly's part '<name>' is unplaced, as is \u{2018}<name>\u{2019}",
            ),
        ];
        for (text, want) in cases {
            assert_eq!(scrub.text(text), want, "{text}");
        }
    }

    /// Every string anywhere in `value`.
    fn all_text(value: &Value, out: &mut String) {
        match value {
            Value::String(text) => {
                out.push_str(text);
                out.push('\n');
            }
            Value::Array(values) => values.iter().for_each(|v| all_text(v, out)),
            Value::Object(fields) => fields.iter().for_each(|(k, v)| {
                out.push_str(k);
                out.push('\n');
                all_text(v, out);
            }),
            _ => {}
        }
    }

    /// Names a user's library might hold, each a thing the report must never show.
    const NASTY: [&str; 12] = [
        "Kunde Müller GmbH",
        "Projekt Ω-7",
        "Kühlkörper_v2",
        "Secret Rover",
        "Gripper Jaw (left)",
        "ACME-0042",
        "Pack 'Go' 3_2_1",
        "Version 1 - Clamp",
        "設計部",
        "Lid Assy",
        "Cam Follower",
        "FlangeMount",
    ];

    /// A batch of convert reports that name the [`NASTY`] names everywhere a conversion can:
    /// loss subjects and messages, feature names, reasons, errors, codes, paths relative to
    /// an Inventor project and its folders, and the names cadmpeg and Dutchman met.
    fn nasty_batch(root: &Path) -> (Value, Vec<PathBuf>) {
        let project = root.join("Kunde Müller GmbH").join("Projekt Ω-7");
        let library = root.join("設計部").join("Vendor Parts");
        let assembly = project.join("Secret Rover").join("Lid Assy.iam");
        let part = project.join("Version 1 - Clamp").join("Kühlkörper_v2.ipt");
        let used = "Pack 'Go' 3_2_1/Version 1 - Clamp/Kühlkörper_v2.ipt";
        let results = json!([
            {"ok": true, "input": {"path": assembly, "format": "inventor_assembly"},
             "output": {"path": root.join("out").join("Lid Assy.solidcraft")},
             "mode": "import",
             "verification": {"status": "flattened",
                              "reason": "feature \"Gripper Jaw (left)\": not mapped"},
             "names": ["Lid Assy", "Gripper Jaw (left)", "Cam Follower", "ACME-0042",
                       used, library.join("FlangeMount.ipt"), project],
             "losses": [
                {"level": "Info", "stage": "input", "code": null,
                 "subject": "Kühlkörper_v2.ipt",
                 "message": format!("2 files in the project have this name; used \"{used}\"")},
                {"level": "Info", "stage": "input", "code": null,
                 "subject": "parts search",
                 "message": format!("parts not at their stored paths were searched for in the \
                    project folder {}, then the folders the Inventor project file \
                    Projekt Ω-7.ipj names: library {}", project.display(), library.display())},
                {"level": "Dropped", "stage": "input", "code": null,
                 "subject": "occurrence \"Cam Follower:1\"",
                 "message": "FlangeMount.ipt: not found; Cam Follower stays out"},
                {"level": "Dropped", "stage": "map", "code": null,
                 "subject": "Kühlkörper_v2.ipt: feature \"Gripper Jaw (left)\"",
                 "message": "Kühlkörper_v2.ipt: Gripper Jaw (left) is not mapped yet"},
                {"level": "Degraded", "stage": "decode",
                 "code": "inventor/Secret Rover.missing", "subject": "x",
                 "message": "component Lid Assy has no bodies; part number ACME-0042"},
                {"level": "Info", "stage": "map", "code": null, "subject": "FlangeMount",
                 "message": "Version 1 - Clamp/FlangeMount carried as a body"},
             ],
             "history": {"features": [
                {"name": "Gripper Jaw (left)", "kind": "Secret Rover", "status": "blocked",
                 "reason": "Gripper Jaw (left) uses 設計部 geometry"}]}},
            {"ok": false, "input": {"path": part, "format": "inventor_part"},
             "error": {"message": format!("cannot read {}: Kühlkörper_v2 is locked by \
                Kunde Müller GmbH", part.display()), "stage": "decode"}},
        ]);
        let batch = json!({"results": results, "skipped": 0});
        (batch, vec![root.to_path_buf()])
    }

    #[test]
    fn no_name_a_conversion_met_reaches_the_report() {
        let root = Path::new("/home/someone/CAD");
        let (batch, inputs) = nasty_batch(root);
        let salt = Salt::random().unwrap();
        let (anon, _) = report(&batch, &inputs, &root.join("out"), &salt, &Run::default());
        let mut text = String::new();
        all_text(&anon, &mut text);
        for name in NASTY.iter().chain(&["someone", "Vendor Parts", "Projekt"]) {
            assert!(!text.contains(name), "{name} is in the report:\n{text}");
        }
        // Codes are fixed words, never a name.
        let codes: Vec<&str> = anon["losses"]
            .as_array()
            .unwrap()
            .iter()
            .map(|loss| loss["code"].as_str().unwrap())
            .collect();
        for code in &codes {
            assert!(
                code.starts_with("cadmpeg/") && *code == "cadmpeg/other"
                    || KINDS
                        .iter()
                        .chain(&["other"])
                        .any(|k| *code == format!("dutchman/{k}")),
                "{code}"
            );
        }
        assert!(codes.contains(&"dutchman/parts"), "{codes:?}");
        assert_eq!(anon["blocked_features"][0]["kind"], "other");
        // What is left still says what happened.
        assert!(text.contains("files in the project have this name; used"));
        assert!(text.contains("is not mapped yet"));
    }

    #[test]
    fn a_left_out_stuck_design_is_named_nowhere_in_the_report() {
        let root = Path::new("/home/someone/CAD");
        let entry = "Secret Gearbox 7.f3d";
        let limits = crate::worker::Limits::default();
        let mut converted = json!({"ok": true,
            "input": {"path": root.join("Plant.f3z"), "format": "fusion_archive"},
            "mode": "import", "verification": {"status": "flattened", "reason": "x"},
            "losses": []});
        crate::worker::mark_left_out(
            &mut converted,
            &root.join("Plant.f3z"),
            &[entry.into()],
            &limits,
        );
        let mut failed = json!({"ok": false,
            "input": {"path": root.join("Mill.f3z"), "format": "fusion_archive"},
            "error": {"message": "cannot read this design", "stage": "decode"}});
        crate::worker::mark_left_out(
            &mut failed,
            &root.join("Mill.f3z"),
            &[entry.into()],
            &limits,
        );
        assert!(converted.to_string().contains(entry) && failed.to_string().contains(entry));
        let batch = json!({"results": [converted, failed], "skipped": 0});
        let salt = Salt::random().unwrap();
        let (anon, _) = report(
            &batch,
            &[root.to_path_buf()],
            &root.join("out"),
            &salt,
            &Run::default(),
        );
        let mut text = String::new();
        all_text(&anon, &mut text);
        for name in [entry, "Secret Gearbox", "Gearbox"] {
            assert!(!text.contains(name), "{name} is in the report:\n{text}");
        }
        // What is left still says what happened.
        assert!(text.contains("was left out"), "{text}");
    }

    #[test]
    fn blocked_features_list_their_timeline_position_and_variant() {
        let root = Path::new("/home/someone/CAD");
        let feature = |ordinal: u64, variant: &str| {
            json!({"name": "Secret Boss", "kind": "extrude", "status": "blocked",
                   "reason": "extrude is not mapped yet", "ordinal": ordinal,
                   "variant": variant})
        };
        let result = |name: &str, features: Vec<Value>| {
            json!({"ok": true, "input": {"path": root.join(name), "format": "fusion_archive"},
                   "mode": "import", "verification": {"status": "flattened", "reason": "x"},
                   "losses": [], "history": {"features": features}})
        };
        let batch = json!({"results": [
            result("a.f3d", vec![feature(7, "extrude, to a plane"),
                                 feature(2, "extrude, blind, start from a face")]),
            result("b.f3d", vec![feature(2, "extrude, blind, start from a face")]),
        ], "skipped": 0});
        let salt = Salt::random().unwrap();
        let (anon, _) = report(
            &batch,
            &[root.to_path_buf()],
            &root.join("out"),
            &salt,
            &Run::default(),
        );
        let blocked = &anon["blocked_features"];
        assert_eq!(blocked.as_array().unwrap().len(), 1);
        assert_eq!(blocked[0]["files"], 2);
        assert_eq!(
            blocked[0]["features"],
            json!([
                {"ordinal": 2, "variant": "extrude, blind, start from a face", "count": 2},
                {"ordinal": 7, "variant": "extrude, to a plane", "count": 1},
            ])
        );
        let mut text = String::new();
        all_text(&anon, &mut text);
        assert!(!text.contains("Secret Boss"));
    }

    #[test]
    fn plain_words_stay_and_names_go_only_where_they_stand_alone() {
        let mut scrub = Scrubber::default();
        scrub.know_path(Path::new("/data/import/Block/Blocker.ipt"));
        assert_eq!(
            scrub.text("SolidCraft can import it; Block and Blocks"),
            "SolidCraft can import it; <name> and Blocks"
        );
        // A path no conversion met loses every word with a separator in it.
        assert_eq!(
            scrub.text("used Vendor Parts/M3/screw.ipt and acme/jigs, not body/region/shell"),
            "used Vendor <path> and <path>, not body/region/shell"
        );
    }

    #[test]
    fn a_large_batch_is_scrubbed_in_one_pass() {
        let mut results = Vec::new();
        for i in 0..8000 {
            let path = format!(
                "/library/Customer {i}/Assembly {}/Part Number {i}.ipt",
                i % 97
            );
            let names: Vec<String> = (0..40).map(|n| format!("Feature {i}-{n}")).collect();
            let losses: Vec<Value> = (0..6)
                .map(|n| {
                    json!({"level": "Degraded", "stage": "map", "code": null,
                           "subject": format!("feature \"Feature {i}-{n}\""),
                           "message": format!("Feature {i}-{n} in Part Number {i}.ipt is not \
                                               mapped yet")})
                })
                .collect();
            results.push(
                json!({"ok": true, "input": {"path": path, "format": "inventor_part"},
                                "names": names, "losses": losses}),
            );
        }
        let batch = json!({"results": results});
        let started = std::time::Instant::now();
        let (anon, _) = report(
            &batch,
            &[PathBuf::from("/library")],
            Path::new("/out"),
            &Salt::random().unwrap(),
            &Run::default(),
        );
        let took = started.elapsed();
        assert!(took.as_secs() < 60, "{took:?}");
        let mut text = String::new();
        all_text(&anon, &mut text);
        assert!(!text.contains("Customer"), "{text}");
        assert!(!text.contains("Feature 7"), "{text}");
    }

    #[test]
    fn messages_that_differ_only_in_counts_are_one_cause() {
        assert_eq!(
            template("3 of 12 constraints not carried"),
            template("1 of 4 constraints not carried")
        );
        assert_eq!(subject_kind("part.ipt: feature \"Boss\""), "feature");
        assert_eq!(subject_kind("joints"), "joints");
        assert_eq!(subject_kind("Gearbox_Housing.ipt"), "other");
        assert_eq!(subject_kind("Housing"), "other");
        assert_eq!(version_only("solidcraft-cli 0.18.2\n"), "0.18.2");
    }
}
