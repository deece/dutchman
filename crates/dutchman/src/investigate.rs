//! `dutchman investigate`: one command an AI runs for a user whose design did not convert the
//! way they hoped. It finds the report Dutchman saved beside the design, or converts the design
//! once into a private folder when there is none, narrows a failed conversion down to its first
//! failing feature, and writes one shareable file, `summary.md`, made only from report data the
//! anonymous report already treats as shareable. The summary is checked in code before it is
//! written: it may hold no name or path the conversion met and no number from the design.
//!
//! Then it asks one question, whether to send the summary to the Dutchman team. Yes sends it
//! through the anonymous report endpoint, with the consent rules every report uses, and prints
//! the receipt id; no leaves the files where they are. `--yes` and `--no-send` answer the
//! question in advance. Without a terminal to ask on, and without either, nothing is sent.
//!
//! Everything else stays in the private folder: the convert report, the anonymous report,
//! the narrowing and `local-key.json` (the salt and the design's path), none of which is sent.

use std::collections::BTreeSet;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::anon::{self, Salt, Scrubber};
use crate::diagnose::{self, narrow};
use crate::{json as wire, report_send, saved_report, ConvertOptions};

/// The schema of the result.
pub const SCHEMA: &str = wire::INVESTIGATE;

/// The signature a sent summary carries.
pub const SIGNATURE: &str = "investigate";

/// The shareable file.
pub const SUMMARY: &str = "summary.md";

/// What the private folder remembers of a run, so a second run (to send after asking the
/// user) reuses the summary instead of converting again.
const STATE: &str = "investigate.json";
const STATE_SCHEMA: &str = "dutchman.investigate-local/1";

/// The question asked before anything is sent.
pub const QUESTION: &str = "Send this summary to the Dutchman team? [y/N]";

/// The question when a synthetic case was made too.
pub const QUESTION_WITH_CASE: &str =
    "Send this summary and the synthetic case to the Dutchman team? [y/N]";

/// Whether the summary is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    /// Ask on the terminal; without one, don't send.
    Ask,
    /// The user already said yes.
    Yes,
    /// The user already said no.
    No,
}

/// How a run is set up.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The private folder (default: `<design>.investigate` beside the design).
    pub out: Option<PathBuf>,
    /// A saved convert report to use instead of looking beside the design.
    pub report: Option<PathBuf>,
    /// Whether to send the summary.
    pub send: Consent,
    /// Start again even when the folder holds a summary of this design.
    pub fresh: bool,
    /// The longest narrowing a failed conversion may take in all.
    pub step_time_limit: Option<Duration>,
    /// Convert with the newest nightly and the listed candidate builds too, and say what
    /// each changes.
    pub compare: bool,
    /// The longest one of those builds may take to convert the design.
    pub compare_time_limit: Duration,
    /// Try to make a synthetic case of the first SolidCraft refusal.
    pub synthetic: bool,
    /// The longest the synthetic case may take.
    pub synthetic_time_limit: Duration,
    /// The conversion's choices, for a design converted here.
    pub options: ConvertOptions,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            out: None,
            report: None,
            send: Consent::Ask,
            fresh: false,
            step_time_limit: None,
            compare: true,
            compare_time_limit: compare::DEFAULT_TIME_LIMIT,
            synthetic: true,
            synthetic_time_limit: synthetic::DEFAULT_TIME_LIMIT,
            options: ConvertOptions::default(),
        }
    }
}

/// The private folder for `input` when none is given: `<name>.investigate` beside it.
pub fn default_out(input: &Path) -> PathBuf {
    let mut name = input.file_name().unwrap_or_default().to_os_string();
    name.push(".investigate");
    input.with_file_name(name)
}

/// The output path a conversion of `input` writes by default, beside which the desktop app
/// saves its report.
fn default_output(input: &Path, options: &ConvertOptions) -> PathBuf {
    dutchman_core::Format::part_revision(input)
        .map_or_else(|| input.to_path_buf(), |(part, _)| part)
        .with_extension(options.output_extension())
}

/// Runs the investigation of `input`. `ask` puts [`QUESTION`] to the user and returns their
/// answer; it is called only for [`Consent::Ask`] with a terminal. `on_progress` hears what is
/// being done, in plain words.
pub fn run(
    input: &Path,
    settings: &Settings,
    ask: &dyn Fn(&str) -> bool,
    on_progress: &(dyn Fn(&str) + Sync),
) -> Result<Value, String> {
    if !input.is_file() {
        return Err(format!(
            "{} is not a file; give the design file the conversion was made from",
            input.display()
        ));
    }
    let out = settings.out.clone().unwrap_or_else(|| default_out(input));
    private_dir(&out)?;
    let fingerprint = fingerprint(input)?;
    let reused = if settings.fresh {
        None
    } else {
        reuse(&out, &fingerprint)
    };
    let (state, summary) = match reused {
        Some(found) => {
            on_progress("using the summary made earlier in this folder");
            found
        }
        None => investigate(input, &out, &fingerprint, settings, on_progress)?,
    };
    let mut result = json!({
        "schema": SCHEMA,
        "ok": true,
        "input": wire::input(input),
        "out": out.display().to_string(),
        "summary_path": out.join(SUMMARY).display().to_string(),
        "report": state.get("report").cloned().unwrap_or(Value::Null),
        "narrowed": state.get("narrowed").cloned().unwrap_or(Value::Null),
        "synthetic": state
            .pointer("/synthetic/made")
            .cloned()
            .unwrap_or(Value::Bool(false)),
        "builds": state.get("builds").cloned().unwrap_or(Value::Null),
        "private": [
            out.join("local-key.json").display().to_string(),
            out.join("convert").display().to_string(),
            out.join("anonymous.json").display().to_string(),
        ],
    });
    let report_id = state
        .get("report_id")
        .and_then(Value::as_str)
        .ok_or("the folder's investigate.json is malformed; run again with --fresh")?
        .to_owned();
    let format = state
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let made = state.pointer("/synthetic/made") == Some(&Value::Bool(true));
    let question = if made { QUESTION_WITH_CASE } else { QUESTION };
    let send = match settings.send {
        Consent::Yes => true,
        Consent::No => false,
        Consent::Ask if std::io::stdin().is_terminal() => ask(question),
        Consent::Ask => {
            set(
                &mut result,
                "summary",
                format!(
                    "Not sent. Read {} with the user, ask them whether to send it to the \
                     Dutchman team, then run this again with --yes or --no-send.",
                    out.join(SUMMARY).display()
                ),
            );
            set(&mut result, "sent", false);
            set(&mut result, "asked", false);
            return Ok(result);
        }
    };
    set(&mut result, "asked", settings.send == Consent::Ask);
    if !send {
        set(&mut result, "sent", false);
        set(
            &mut result,
            "summary",
            format!(
                "Not sent. The summary and everything else are in {}.",
                out.display()
            ),
        );
        return Ok(result);
    }
    let case = if made {
        synthetic::files(&out.join("synthetic"))?
    } else {
        Vec::new()
    };
    let sent = send_summary(
        &out,
        &report_id,
        &format,
        &summary,
        &case,
        report_send::endpoint().as_deref(),
    )?;
    let ok = sent.get("sent") == Some(&Value::Bool(true));
    set(&mut result, "sent", ok);
    set(&mut result, "send", sent.clone());
    set(
        &mut result,
        "summary",
        if ok {
            format!(
                "Sent. Receipt id {report_id}. The files stay in {}.",
                out.display()
            )
        } else {
            sent.get("summary")
                .and_then(Value::as_str)
                .unwrap_or("It did not go through. Your summary is kept locally.")
                .to_owned()
        },
    );
    if ok {
        set(&mut result, "receipt", report_id);
    }
    Ok(result)
}

fn set(result: &mut Value, key: &str, value: impl Into<Value>) {
    if let Some(map) = result.as_object_mut() {
        map.insert(key.to_owned(), value.into());
    }
}

/// The design's size and modification time, so a summary is reused only for the same file.
fn fingerprint(input: &Path) -> Result<Value, String> {
    let meta = std::fs::metadata(input).map_err(|e| format!("{}: {e}", input.display()))?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().to_string());
    let path = std::fs::canonicalize(input).unwrap_or_else(|_| input.to_path_buf());
    Ok(json!({"path": path.display().to_string(), "size": meta.len(), "modified": modified}))
}

/// The state and summary of an earlier run on the same design, when the folder holds them.
fn reuse(out: &Path, fingerprint: &Value) -> Option<(Value, String)> {
    let state: Value =
        serde_json::from_slice(&report_send::bounded_read(&out.join(STATE), 1024 * 1024).ok()?)
            .ok()?;
    if state.get("schema").and_then(Value::as_str) != Some(STATE_SCHEMA)
        || state.get("design") != Some(fingerprint)
    {
        return None;
    }
    let summary =
        String::from_utf8(report_send::bounded_read(&out.join(SUMMARY), 1024 * 1024).ok()?).ok()?;
    Some((state, summary))
}

fn private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot make {} private: {e}", path.display()))?;
    }
    Ok(())
}

/// Writes `bytes` to `path` readable only by this user, replacing what was there.
fn private_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot replace {}: {e}", path.display())),
    }
    report_send::private_write(path, bytes)
}

/// The convert report: given, saved beside the design, or made here.
fn find_report(
    input: &Path,
    out: &Path,
    settings: &Settings,
    on_progress: &(dyn Fn(&str) + Sync),
) -> Result<(Value, &'static str), String> {
    if let Some(path) = &settings.report {
        let report = read_report(path)?;
        return Ok((report, "given"));
    }
    let output = default_output(input, &settings.options);
    for converted in [true, false] {
        let path = saved_report::path(&output, converted);
        if let Ok(report) = read_report(&path) {
            if same_input(&report, input) {
                on_progress("using the report saved beside the design");
                return Ok((report, "saved"));
            }
        }
    }
    on_progress("converting the design once (no saved report beside it)");
    let folder = out.join("convert");
    private_dir(&folder)?;
    let target = folder.join(
        output
            .file_name()
            .map_or_else(|| PathBuf::from("design.sc"), PathBuf::from),
    );
    let _ = std::fs::remove_file(&target);
    let report = crate::convert_options_with_progress(
        input,
        &target,
        &settings.options,
        true,
        &settings.options.part_search(),
        &|_| {},
    );
    private_file(
        &folder.join("report.json"),
        &serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )?;
    Ok((report, "converted"))
}

fn read_report(path: &Path) -> Result<Value, String> {
    let bytes = report_send::bounded_read(path, saved_report::MAX_BYTES as usize)
        .map_err(|e| format!("cannot read the report {}: {e}", path.display()))?;
    let report: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("the report {} is not JSON: {e}", path.display()))?;
    if report.get("schema").and_then(Value::as_str) != Some(wire::CONVERT) {
        return Err(format!(
            "{} is not a Dutchman conversion report ({})",
            path.display(),
            wire::CONVERT
        ));
    }
    Ok(report)
}

/// Whether a saved report was made from this design.
fn same_input(report: &Value, input: &Path) -> bool {
    let Some(stored) = report.pointer("/input/path").and_then(Value::as_str) else {
        return false;
    };
    let stored = Path::new(stored);
    stored == input
        || std::fs::canonicalize(stored)
            .ok()
            .zip(std::fs::canonicalize(input).ok())
            .is_some_and(|(a, b)| a == b)
}

fn investigate(
    input: &Path,
    out: &Path,
    fingerprint: &Value,
    settings: &Settings,
    on_progress: &(dyn Fn(&str) + Sync),
) -> Result<(Value, String), String> {
    let (report, source) = find_report(input, out, settings, on_progress)?;
    let salt = Salt::random()?;
    let run = anon::Run::of_report(&report, &settings.options);
    let batch = json!({"results": [report]});
    let (anonymous, key) = anon::report(&batch, &[input.to_path_buf()], out, &salt, &run);
    private_file(
        &out.join("anonymous.json"),
        &serde_json::to_vec_pretty(&anonymous).map_err(|e| e.to_string())?,
    )?;
    private_file(
        &out.join("local-key.json"),
        &serde_json::to_vec_pretty(&json!({"salt": salt.hex(), "files": key}))
            .map_err(|e| e.to_string())?,
    )?;
    let narrowed = if report.get("ok") == Some(&Value::Bool(true)) {
        None
    } else {
        on_progress("narrowing the failure down to its first failing feature");
        let rank = diagnose::Settings {
            options: settings.options.clone(),
            ..diagnose::Settings::default()
        };
        let narrowing = narrow::NarrowSettings::new(rank, settings.step_time_limit);
        Some(narrow::run(input, out, &narrowing, &|progress| {
            on_progress(&format!("narrowing: {progress}"));
        }))
    };
    let mut scrub = Scrubber::default();
    scrub.know_path(input);
    scrub.know_path(out);
    if let Some(path) = report.pointer("/output/path").and_then(Value::as_str) {
        scrub.know_path(Path::new(path));
    }
    scrub.know_names_in(&report);
    let deltas = if settings.compare {
        on_progress("looking for newer builds to compare with");
        let (builds, _) = compare::builds(compare::TARGET);
        Some(compare::compare(
            input,
            out,
            &report,
            &builds,
            settings.compare_time_limit,
            on_progress,
        ))
    } else {
        None
    };
    let made = settings.synthetic.then(|| {
        synthetic::make(
            input,
            out,
            &report,
            &settings.options,
            settings.synthetic_time_limit,
            on_progress,
        )
    });
    let md = summary(
        &report,
        &anonymous,
        narrowed.as_ref(),
        deltas.as_deref(),
        made.as_ref(),
        &scrub,
    );
    check(
        &md,
        &report,
        &anonymous,
        deltas.as_deref().unwrap_or(&[]),
        made.as_ref(),
        &scrub,
    )?;
    let summary = md.text;
    private_file(&out.join(SUMMARY), summary.as_bytes())?;
    let state = json!({
        "schema": STATE_SCHEMA,
        "design": fingerprint,
        "report_id": salt.file_id(input),
        "format": report.pointer("/input/format").and_then(Value::as_str).unwrap_or("unknown"),
        "report": source,
        "narrowed": narrowed.as_ref().map(|n| n.is_ok()),
        "synthetic": made.as_ref().map(synthetic::Synthetic::json),
        "builds": deltas.as_ref().map(|deltas| deltas.iter().map(|d| json!({
            "label": d.label,
            "title": d.title,
            "compared": d.skipped.is_none(),
            "not_compared_because": d.skipped,
            "regresses": d.regresses(),
            "converts": d.ok_after,
            "rebuilt": d.rebuilt_after,
            "better": d.better.len(),
            "worse": d.worse.len(),
        })).collect::<Vec<_>>()),
    });
    private_file(
        &out.join(STATE),
        &serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?,
    )?;
    Ok((state, summary))
}

/// The summary as Markdown. Text from the conversion goes through [`Md::design`], which takes
/// names out and writes every number `N`; only the numbers Dutchman itself counts or fixes
/// (versions, counts, timeline positions, issue links) are written as numbers, and each is
/// recorded so [`check`] can prove nothing else got in.
#[derive(Default)]
struct Md {
    /// The whole summary.
    text: String,
    /// Only what came from the conversion, for the name check.
    designed: String,
    /// Every number written, for the number check.
    allowed: BTreeSet<String>,
}

impl Md {
    /// Text Dutchman owns (headings, fixed wording, versions, links from its own tables).
    fn fixed(&mut self, text: &str) -> &mut Self {
        self.allowed.extend(numbers(text));
        self.text.push_str(text);
        self
    }

    /// A count or position Dutchman worked out.
    fn num(&mut self, n: u64) -> &mut Self {
        let text = n.to_string();
        self.allowed.insert(text.clone());
        self.text.push_str(&text);
        self
    }

    /// Text that came from the conversion: names out, numbers written `N`, and the characters
    /// Markdown uses for code, emphasis and links dropped so nothing can hide in them.
    fn design(&mut self, scrub: &Scrubber, text: &str) -> &mut Self {
        let text = without_numbers(&scrub.text(text)).replace(['`', '*', '_', '[', ']'], "");
        self.text.push_str(&text);
        self.designed.push_str(&text);
        self.designed.push('\n');
        self
    }

    fn line(&mut self) -> &mut Self {
        self.text.push('\n');
        self
    }
}

/// Every run of digits (with its decimal points), as it is written.
fn numbers(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let joins =
            c == '.' && !current.is_empty() && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if c.is_ascii_digit() || joins {
            current.push(c);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// `text` with every number written `N`.
fn without_numbers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_number = false;
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let decimal = c == '.' && in_number && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if c.is_ascii_digit() || decimal {
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

fn text<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

fn count(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer).and_then(Value::as_u64)
}

/// Builds `summary.md` from the convert report, the anonymous report made of it and the
/// narrowing, if one ran.
fn summary(
    report: &Value,
    anonymous: &Value,
    narrowed: Option<&Result<Value, String>>,
    deltas: Option<&[compare::Delta]>,
    made: Option<&synthetic::Synthetic>,
    scrub: &Scrubber,
) -> Md {
    let mut md = Md::default();
    md.fixed("# Dutchman investigation\n\n");
    md.fixed("Made by `dutchman investigate` from the conversion report. It holds no file, part or feature names, no paths and no sizes from the design.\n\n");
    md.fixed(&format!(
        "- Dutchman {} (cadmpeg {}), {} {}\n",
        crate::BUILD_VERSION,
        anon::CADMPEG_VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
    if let Some(name) = text(report, "/input/format_name") {
        md.fixed(&format!("- Format: {name}\n"));
    }
    match report.get("ok") {
        Some(Value::Bool(true)) => {
            md.fixed("- Outcome: converted");
            if let Some(status) = text(report, "/verification/status") {
                md.fixed(&format!(", check {status}"));
            }
            md.line();
        }
        _ => {
            md.fixed("- Outcome: failed");
            if let Some(stage) = text(report, "/error/stage") {
                md.fixed(&format!(" in the {stage} stage", stage = word(stage)));
            }
            md.line();
        }
    }
    match text(report, "/solidcraft/found") {
        Some(version) => {
            md.fixed(&format!("- SolidCraft: {}\n", version_text(version)));
        }
        None => {
            md.fixed("- SolidCraft: not found, so nothing was checked in it");
            if let Some(places) = anonymous
                .pointer("/versions/solidcraft_search/places")
                .and_then(Value::as_array)
            {
                let places: Vec<String> = places
                    .iter()
                    .filter_map(|place| text(place, "/place"))
                    .map(label)
                    .collect();
                if !places.is_empty() {
                    md.fixed(&format!(" (looked in: {})", places.join(", ")));
                }
            }
            md.line();
        }
    }
    if let Some(total) = count(report, "/history/total") {
        md.fixed("- History: ");
        md.num(count(report, "/history/rebuilt").unwrap_or(0));
        md.fixed(" of ");
        md.num(count(report, "/history/needing").unwrap_or(total));
        md.fixed(" features that need rebuilding were rebuilt\n");
    }
    synthetic_line(&mut md, made);
    builds(&mut md, deltas);

    let features: Vec<&Value> = report
        .pointer("/history/features")
        .and_then(Value::as_array)
        .map(|f| f.iter().collect())
        .unwrap_or_default();

    // Replays that may be wrong come first: the shape is there but may not be the source's.
    let unverified: Vec<&&Value> = features
        .iter()
        .filter(|f| f.get("profile_unverified") == Some(&Value::Bool(true)))
        .collect();
    let mismatch = text(report, "/verification/reason")
        .is_some_and(|reason| reason.contains("profile region doesn't match"));
    if !unverified.is_empty() || mismatch {
        md.fixed("\n## Replays that may be wrong\n\n");
        for feature in unverified {
            md.fixed("- ");
            md.design(scrub, text(feature, "/kind").unwrap_or("feature"));
            position(&mut md, feature);
            md.fixed(": its profile is the whole of a sketch with several profiles, and the check did not confirm which one it takes\n");
        }
        if mismatch {
            md.fixed("- A kept feature's profile region does not match the source's\n");
        }
    }

    // The first blocker of each feature kind, in timeline order.
    let mut seen = BTreeSet::new();
    let mut first = Vec::new();
    for feature in features
        .iter()
        .filter(|f| text(f, "/status") == Some("blocked"))
    {
        let kind = anon::generic(text(feature, "/kind").unwrap_or("?"), "other");
        if seen.insert(kind.clone()) {
            first.push((kind, *feature));
        }
    }
    if !first.is_empty() {
        md.fixed("\n## First blocked feature of each kind\n\n");
        for (kind, feature) in &first {
            md.fixed(&format!("- {kind}"));
            position(&mut md, feature);
            if let Some(variant) = text(feature, "/variant") {
                md.fixed(" (");
                md.design(scrub, variant);
                md.fixed(")");
            }
            md.fixed(": ");
            md.design(scrub, text(feature, "/reason").unwrap_or("not mapped"));
            md.line();
        }
    }

    // SolidCraft's own refusals, with where they are tracked.
    let refusals: Vec<&&Value> = features
        .iter()
        .filter(|f| text(f, "/source") == Some("solidcraft"))
        .collect();
    if !refusals.is_empty() {
        md.fixed("\n## Refused by SolidCraft\n\n");
        let mut categories = BTreeSet::new();
        for feature in refusals {
            let category = anon::generic(text(feature, "/category").unwrap_or("other"), "other");
            let link = text(feature, "/upstream").filter(|u| u.starts_with("https://"));
            if categories.insert((category.clone(), link.map(str::to_owned))) {
                md.fixed(&format!("- {category}"));
                if let Some(link) = link {
                    md.fixed(&format!(": tracked at {link}"));
                }
                md.line();
            }
        }
    }

    if report.get("ok") != Some(&Value::Bool(true)) {
        md.fixed("\n## The failure\n\n");
        md.fixed("- ");
        md.design(
            scrub,
            text(report, "/error/message").unwrap_or("it failed without a message"),
        );
        md.line();
        match narrowed {
            Some(Ok(result)) => {
                if let Some(summary) = text(result, "/summary") {
                    md.fixed("- Narrowed: ");
                    md.design(scrub, summary);
                    md.line();
                }
            }
            Some(Err(_)) => {
                md.fixed("- Narrowing stopped before it found the first failing feature\n");
            }
            None => {}
        }
    }

    // Losses, by code, most common first.
    if let Some(losses) = anonymous.get("losses").and_then(Value::as_array) {
        let shown: Vec<&Value> = losses.iter().take(8).collect();
        if !shown.is_empty() {
            md.fixed("\n## What the conversion lost\n\n");
            for loss in shown {
                md.fixed("- ");
                md.fixed(&anon::generic(
                    text(loss, "/code").unwrap_or("other"),
                    "other",
                ));
                if let Some(example) = text(loss, "/example") {
                    md.fixed(": ");
                    md.design(scrub, example);
                }
                md.line();
            }
        }
    }
    md
}

/// A word from the report's fixed vocabulary, else `other`.
fn word(text: &str) -> String {
    if !text.is_empty()
        && text.len() <= 40
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || matches!(c, '_' | '-' | ' '))
    {
        text.to_owned()
    } else {
        "other".to_owned()
    }
}

/// Whether a synthetic case was made, how long the step took, and why there is none.
fn synthetic_line(md: &mut Md, made: Option<&synthetic::Synthetic>) {
    let Some(made) = made else {
        md.fixed("- Synthetic case: not tried\n");
        return;
    };
    if made.made() {
        md.fixed("- Synthetic case: made, ");
        md.num(made.steps.unwrap_or(0) as u64);
        md.fixed(" steps on invented dimensions, in synthetic/ (took ");
    } else {
        md.fixed("- Synthetic case: none made: ");
        md.fixed(made.reason.unwrap_or("no reason was recorded"));
        if made
            .reason
            .is_some_and(|r| r.contains("export a design with components"))
        {
            md.fixed(&format!(" ({})", synthetic::EXPORT_ISSUE));
        }
        md.fixed(" (took ");
    }
    md.num(made.seconds);
    md.fixed(" s)\n");
}

/// What newer builds change, regressions first.
fn builds(md: &mut Md, deltas: Option<&[compare::Delta]>) {
    let Some(deltas) = deltas else {
        md.fixed("- Newer builds: not compared\n");
        return;
    };
    if deltas.is_empty() {
        md.fixed("- Newer builds: none found for this platform\n");
        return;
    }
    md.fixed("\n## Newer builds\n\n");
    let mut ordered: Vec<&compare::Delta> = deltas.iter().collect();
    ordered.sort_by_key(|d| (!d.regresses(), d.skipped.is_some()));
    for delta in ordered {
        md.fixed(&format!("- {} ({})", delta.label, title_text(&delta.title)));
        if delta.skipped.is_some() {
            md.fixed(": could not be compared\n");
            continue;
        }
        md.fixed(": ");
        md.fixed(match (delta.ok_before, delta.ok_after) {
            (true, false) => "fails, where this version converts",
            (false, true) => "converts, where this version fails",
            (true, true) => "converts",
            (false, false) => "fails too",
        });
        if delta.ok_after && delta.rebuilt_after != delta.rebuilt_before {
            md.fixed(", rebuilds ");
            md.num(delta.rebuilt_after);
            md.fixed(" features (this version ");
            md.num(delta.rebuilt_before);
            md.fixed(")");
        }
        if delta.ok_before == delta.ok_after
            && delta.worse.is_empty()
            && delta.better.is_empty()
            && delta.rebuilt_after == delta.rebuilt_before
        {
            md.fixed(", with the same result");
        }
        md.line();
        for (ordinal, kind, now) in delta.worse.iter().take(10) {
            md.fixed(&format!("  - worse: {kind}, feature "));
            md.num(ordinal + 1);
            md.fixed(&format!(
                " in the timeline is now {now} (rebuilt in this version)\n"
            ));
        }
        for (ordinal, kind, before) in delta.better.iter().take(10) {
            md.fixed(&format!("  - better: {kind}, feature "));
            md.num(ordinal + 1);
            md.fixed(&format!(" in the timeline is rebuilt (here {before})\n"));
        }
    }
}

/// A build's title (`Nightly 2026-11-08`, a candidate's one-line note): letters, digits,
/// spaces and plain punctuation, at most 80 characters.
fn title_text(title: &str) -> String {
    title
        .chars()
        .filter(|c| {
            c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '.' | ',' | ':' | '(' | ')')
        })
        .take(80)
        .collect()
}

/// A place label from SolidCraft's search (`PATH`, `beside dutchman`, …): fixed labels, with
/// anything else written `other`.
fn label(text: &str) -> String {
    if !text.is_empty()
        && text.len() <= 60
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
    {
        text.to_owned()
    } else {
        "other".to_owned()
    }
}

fn version_text(version: &str) -> String {
    version
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        .take(40)
        .collect()
}

/// `, feature N of the timeline` from a feature's ordinal, counted from 1.
fn position(md: &mut Md, feature: &Value) {
    if let Some(ordinal) = count(feature, "/ordinal") {
        md.fixed(", feature ");
        md.num(ordinal + 1);
        md.fixed(" in the timeline");
    }
}

/// Proves the summary holds no name or path the conversion met and no number it did not
/// write itself. A failure is a bug in Dutchman, so nothing is written.
fn check(
    md: &Md,
    report: &Value,
    anonymous: &Value,
    deltas: &[compare::Delta],
    made: Option<&synthetic::Synthetic>,
    scrub: &Scrubber,
) -> Result<(), String> {
    if scrub.holds_known(&md.designed) {
        return Err(
            "the summary would hold a name or path from the design, so it was not \
                    written; this is a bug in Dutchman, please report it without the summary"
                .to_owned(),
        );
    }
    let allowed = allowed_numbers(report, anonymous, deltas, made);
    let stray: Vec<String> = numbers(&md.text)
        .into_iter()
        .filter(|n| !allowed.contains(n))
        .collect();
    if !stray.is_empty() {
        return Err(
            "the summary would hold a number from the design, so it was not written; \
                    this is a bug in Dutchman, please report it without the summary"
                .to_owned(),
        );
    }
    Ok(())
}

/// The numbers [`summary`] may write, worked out again from what Dutchman owns alone, so a
/// design's text passed by mistake through [`Md::fixed`] is still caught.
fn allowed_numbers(
    report: &Value,
    anonymous: &Value,
    deltas: &[compare::Delta],
    made: Option<&synthetic::Synthetic>,
) -> BTreeSet<String> {
    let mut md = Md::default();
    if let Some(made) = made {
        synthetic_line(&mut md, Some(made));
    }
    for delta in deltas {
        md.fixed(&delta.label);
        md.fixed(&title_text(&delta.title));
        for n in [delta.rebuilt_after, delta.rebuilt_before, delta.needing] {
            md.num(n);
        }
        for (ordinal, _, _) in delta.worse.iter().chain(&delta.better) {
            md.num(ordinal + 1);
        }
    }
    md.fixed(crate::BUILD_VERSION);
    md.fixed(anon::CADMPEG_VERSION);
    md.fixed(std::env::consts::OS);
    md.fixed(std::env::consts::ARCH);
    if let Some(name) = text(report, "/input/format_name") {
        md.fixed(name);
    }
    if let Some(version) = text(report, "/solidcraft/found") {
        md.fixed(&version_text(version));
    }
    for place in anonymous
        .pointer("/versions/solidcraft_search/places")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(place) = text(place, "/place") {
            md.fixed(&label(place));
        }
    }
    for pointer in ["/history/rebuilt", "/history/needing", "/history/total"] {
        if let Some(n) = count(report, pointer) {
            md.num(n);
        }
    }
    for feature in report
        .pointer("/history/features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(ordinal) = count(feature, "/ordinal") {
            md.num(ordinal + 1);
        }
        if let Some(link) = text(feature, "/upstream").filter(|u| u.starts_with("https://")) {
            md.fixed(link);
        }
        for pointer in ["/kind", "/category"] {
            if let Some(code) = text(feature, pointer) {
                md.fixed(&anon::generic(code, "other"));
            }
        }
    }
    for loss in report
        .get("losses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(code) = text(loss, "/code") {
            md.fixed(&anon::generic(code, "other"));
        }
    }
    md.allowed
}

/// What a sent summary carries: the summary, and the synthetic case's files when one was made.
struct SummarySource<'a>(&'a str, &'a [(String, Vec<u8>)]);

impl report_send::BundleSource for SummarySource<'_> {
    fn parts(&self) -> Result<Vec<report_send::BundlePart>, String> {
        let mut parts = vec![report_send::BundlePart {
            role: "summary".to_owned(),
            bytes: self.0.as_bytes().to_vec(),
        }];
        parts.extend(self.1.iter().map(|(role, bytes)| report_send::BundlePart {
            role: role.clone(),
            bytes: bytes.clone(),
        }));
        Ok(parts)
    }
    fn manifest(&self) -> Value {
        let mut manifest = vec![json!({"role": "summary", "kind": "markdown"})];
        manifest.extend(
            self.1
                .iter()
                .map(|(role, _)| json!({"role": role, "kind": "synthetic"})),
        );
        Value::Array(manifest)
    }
}

/// Sends the summary through the anonymous report endpoint, once per report id.
fn send_summary(
    out: &Path,
    report_id: &str,
    format: &str,
    summary: &str,
    case: &[(String, Vec<u8>)],
    endpoint: Option<&str>,
) -> Result<Value, String> {
    let folder = report_send::local_folder(out, report_id);
    if folder.join("receipt.json").exists() {
        return Ok(
            json!({"schema": report_send::SCHEMA, "ok": true, "sent": true, "report_id": report_id, "already": true}),
        );
    }
    let prepared = if folder.join("prepared.json").exists() {
        report_send::Prepared::load(&folder)?
    } else {
        report_send::Prepared::new(
            report_id,
            format,
            SIGNATURE,
            Some(&SummarySource(summary, case)),
            &folder,
        )?
    };
    report_send::submit(&prepared, &folder, "bundle", false, true, endpoint)
}

/// Puts [`QUESTION`] on the terminal and reads the answer; only `y` or `yes` is yes.
pub fn ask_on_terminal(question: &str) -> bool {
    eprint!("{question} ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

pub mod compare;
pub mod synthetic;

#[cfg(test)]
mod tests;
