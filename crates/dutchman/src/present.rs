//! Plain presentation data for conversion reports, shared by graphical front ends.
//!
//! Builders accept the stable JSON reports, so serialized reports and freshly converted
//! reports follow the same rules. They never change the report or add text to JSON output.

use crate::report_policy::ReportPolicy;
use crate::{anon, issue, json};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Progress of a file, including files that have no conversion report yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    Waiting,
    Converting,
    Converted,
    Failed,
    NotStarted,
    Skipped,
}

/// The result of a file, independently of any UI's colours or icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Waiting,
    Converting,
    Converted,
    ConvertedWithLosses,
    Flattened,
    Incomplete,
    Failed,
    NotStarted,
    Skipped,
}

/// A loss's two separately drawn strings, plus its complete text for other renderers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LossLine {
    pub level: String,
    pub text: String,
    pub full_text: String,
}

/// Losses at one level, in their original order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LossGroup {
    pub level: String,
    pub count: usize,
    pub lines: Vec<LossLine>,
}

/// A component's losses in the CLI's shared severity order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentView {
    pub heading: String,
    pub severity_rank: u8,
    pub lines: Vec<LossLine>,
}

/// Everything a front end needs to describe a file's conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileView {
    pub name: String,
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub status: Status,
    pub status_word: &'static str,
    /// Smaller ranks need attention first. Equal ranks retain input order.
    pub severity_rank: u8,
    pub outcome: String,
    pub capability: Option<String>,
    pub error: Option<String>,
    pub incomplete: bool,
    /// None means there is no missing-parts recovery; Some(empty) means none were found.
    pub missing_parts: Option<Vec<String>>,
    pub missing_summary: Option<String>,
    pub assembly: Option<String>,
    pub assembly_causes: Vec<String>,
    pub loss_summary: String,
    pub losses: Vec<LossGroup>,
    pub components: Vec<ComponentView>,
    pub can_report: bool,
    /// Converted reports start collapsed; failed reports offer help immediately.
    pub report_case: Option<ReportCase>,
}

/// Why reporting is offered for this file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportCase {
    Failed,
    Converted,
}

pub const REPORT_CONVERTED: &str = "Something wrong? Report this file";
pub const SEND_CONVERTED_INTRO: &str =
    "If the converted result has lost features, wrong placement or missing colours, help us fix it:";

pub const REPORT_HELP: &str =
    "If the file opens in the app that made it, this is a bug in Dutchman: \
                     report it on GitHub. The form opens filled in with the error and an \
                     anonymous report that names no file; read it before you submit, and \
                     never attach the design.";
pub const MISSING_HELP: &str =
    "Choose the folder that holds them, or one of them, to convert this file again \
             searching there too.";
pub const REPORT_COPIED: &str =
    "The anonymous report is copied: paste it into the form's \"Anonymous report for this \
     file\" box.";

/// A pure issue-form action. Anonymization and random salt creation happen at the caller.
#[derive(Clone, Debug, PartialEq)]
pub struct ReportLink {
    pub issue: issue::Issue,
    /// What to tell the reporter once the anonymous report is on the clipboard.
    pub copy_notice: &'static str,
}

impl ReportLink {
    pub fn new(issue: issue::Issue) -> Self {
        Self {
            issue,
            copy_notice: REPORT_COPIED,
        }
    }
}

pub fn report_link(report: &Value, anonymous: &Value) -> ReportLink {
    let issue = issue::fill(report, anonymous);
    ReportLink::new(issue)
}

/// The secondary way to report a failed file: nothing of the file, no names.
pub const REPORT_BUTTON: &str = "Report anonymously on GitHub";
/// The folded section of a failed card that holds the other ways to send the file.
pub const SEND_FOLD: &str = "Help us fix it";
pub const SEND_INTRO: &str =
    "If this file opens in the app that made it, it's a bug in Dutchman. Help us fix it:";
pub const SEND_JOIN_TITLE: &str = "Join our Discord server";
pub const SEND_JOIN_BUTTON: &str = "Open invite";
pub const SEND_JOIN_NOTE: &str = "Already in? Skip this step.";
pub const SEND_DM_TITLE: &str = "DM Dutchy";
pub const SEND_DM_BUTTON: &str = "Copy and open DM";
pub const SEND_STEP_TITLE: &str = "Have a STEP export?";
pub const SEND_STEP_TEXT: &str = "If you can export a STEP of the same design, add it too. \
     It helps us compare, but it isn't required.";
pub const SEND_SMALL_PRINT: &str = "Only the Dutchman team sees it. It's never published or \
     used in public tests, and we delete it on request.";
pub const SEND_COPIED: &str = "The message is copied: paste it into the DM with Dutchy.";

/// Which app shows the steps, as they say where the file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Desktop,
    Web,
}

/// The steps that vary on the "send the file privately" path, from the policy and the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendSteps {
    /// What to do in the DM.
    pub dm_text: String,
    pub intro: &'static str,
    /// Whether the file goes in by hand, so the desktop app shows its folder.
    pub drag: bool,
    /// Whether to ask for a STEP export too.
    pub step_card: bool,
}

impl SendSteps {
    pub fn new(
        policy: &ReportPolicy,
        size_bytes: Option<u64>,
        report: &Value,
        surface: Surface,
    ) -> Self {
        let drag = policy.fits_discord(size_bytes);
        let dm_text = match (drag, surface) {
            (true, Surface::Desktop) => "Drag the file in (its folder is open).".to_owned(),
            (true, Surface::Web) => "Drag the file in (the file you picked).".to_owned(),
            (false, _) => format!(
                "Paste the message. Dutchy replies with a private upload link (up to {}).",
                policy.link_limits()
            ),
        };
        Self {
            dm_text,
            intro: if report.get("ok").and_then(Value::as_bool) == Some(true) {
                SEND_CONVERTED_INTRO
            } else {
                SEND_INTRO
            },
            drag,
            step_card: !source_is_step(report),
        }
    }
}

/// Whether the file of `report` is itself a STEP file.
fn source_is_step(report: &Value) -> bool {
    report.pointer("/input/format").and_then(Value::as_str) == Some("step")
}

/// Whether a STEP of the file of `report` is known to be at hand.
pub fn step_known(report: &Value) -> Option<bool> {
    source_is_step(report).then_some(true)
}

/// The fixed word for the failure in `report`, if it has one.
pub fn error_code(report: &Value) -> Option<&str> {
    ["/error/code", "/error/kind"]
        .into_iter()
        .find_map(|key| report.pointer(key).and_then(Value::as_str))
}

/// The anonymous report of the file `report` is of, and the file's id in it, under the run's
/// `salt`: the same id the run's diagnostic log gives the file.
pub fn anonymous_report(
    report: &Value,
    out_dir: &Path,
    salt: &anon::Salt,
    run: &anon::Run,
) -> (Value, String) {
    let input = report
        .pointer("/input/path")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_default();
    let id = salt.file_id(&input);
    let batch = json!({"results": [report]});
    let (anonymous, _) = anon::report(&batch, &[input], out_dir, salt, run);
    (anonymous, id)
}

pub fn loss_line(loss: &Value) -> LossLine {
    let field = |key: &str| loss.get(key).and_then(Value::as_str).unwrap_or("");
    LossLine {
        level: field("level").to_owned(),
        text: format!("{}: {}", field("subject"), field("message")),
        full_text: json::loss_line(loss, true).0,
    }
}

impl FileView {
    /// Present a report using its paths and final result.
    pub fn from_report(report: &Value) -> Self {
        let path = |key| {
            report
                .pointer(key)
                .and_then(Value::as_str)
                .map(PathBuf::from)
        };
        let input = path("/input/path").unwrap_or_default();
        let progress = if report.get("ok").and_then(Value::as_bool) == Some(true) {
            Progress::Converted
        } else {
            Progress::Failed
        };
        Self::new(
            &input,
            path("/output/path").as_deref(),
            progress,
            Some(report),
        )
    }

    /// Present a file during a run. Paths are supplied separately for queued files.
    pub fn new(
        input: &Path,
        output: Option<&Path>,
        progress: Progress,
        report: Option<&Value>,
    ) -> Self {
        let incomplete = report.is_some_and(incomplete);
        let missing_parts = report.and_then(missing_parts);
        let missing_summary = missing_parts.as_deref().map(missing_summary);
        let raw: Vec<&Value> = report
            .and_then(|r| r.get("losses"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();
        let flattened = report
            .and_then(|r| r.pointer("/verification/status"))
            .and_then(Value::as_str)
            == Some("flattened");
        let status = match progress {
            Progress::Waiting => Status::Waiting,
            Progress::Converting => Status::Converting,
            Progress::Failed => Status::Failed,
            Progress::NotStarted => Status::NotStarted,
            Progress::Skipped => Status::Skipped,
            Progress::Converted if incomplete => Status::Incomplete,
            Progress::Converted if flattened => Status::Flattened,
            Progress::Converted if !raw.is_empty() => Status::ConvertedWithLosses,
            Progress::Converted => Status::Converted,
        };
        let (status_word, severity_rank) = match status {
            Status::Failed => ("Failed", 0),
            Status::Incomplete => ("Incomplete", 0),
            Status::Flattened | Status::ConvertedWithLosses => ("Converted", 1),
            Status::Converted => ("Converted", 2),
            Status::Waiting => ("Waiting", 3),
            Status::Converting => ("Converting", 3),
            Status::NotStarted => ("Not started", 3),
            Status::Skipped => ("Skipped", 3),
        };
        let losses = ["Info", "Degraded", "Dropped"]
            .into_iter()
            .filter_map(|level| {
                let lines: Vec<_> = raw
                    .iter()
                    .filter(|l| l.get("level").and_then(Value::as_str) == Some(level))
                    .map(|l| loss_line(l))
                    .collect();
                (!lines.is_empty()).then(|| LossGroup {
                    level: level.to_owned(),
                    count: lines.len(),
                    lines,
                })
            })
            .collect();
        let components = report
            .map(json::components)
            .unwrap_or_default()
            .into_iter()
            .map(|c| ComponentView {
                heading: format!(
                    "{}: {}",
                    c.file.as_deref().unwrap_or("The design"),
                    c.summary()
                ),
                severity_rank: if c.dropped > 0 {
                    0
                } else if c.degraded > 0 || c.flattened.is_some() {
                    1
                } else {
                    2
                },
                lines: c.losses.into_iter().map(loss_line).collect(),
            })
            .collect();
        let error = report
            .and_then(|r| r.pointer("/error/message"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let assembly_lines = report.map(json::completeness_lines).unwrap_or_default();
        let report_case = if missing_parts.is_some() {
            None
        } else if progress == Progress::Converted && report.is_some() {
            Some(ReportCase::Converted)
        } else if error.is_some() {
            Some(ReportCase::Failed)
        } else {
            None
        };
        Self {
            name: input
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| input.display().to_string()),
            input: input.to_path_buf(),
            output: output.map(Path::to_path_buf),
            status,
            status_word,
            severity_rank,
            outcome: error.clone().unwrap_or_else(|| {
                if progress == Progress::Converted {
                    report.map(outcome).unwrap_or_default()
                } else {
                    String::new()
                }
            }),
            capability: report.and_then(json::status_line),
            can_report: report_case.is_some(),
            report_case,
            error,
            incomplete,
            missing_parts,
            missing_summary,
            assembly: report
                .and_then(json::assembly_line)
                .filter(|_| !assembly_lines.is_empty()),
            assembly_causes: assembly_lines
                .into_iter()
                .skip(1)
                .map(|s| s.trim().to_owned())
                .collect(),
            loss_summary: loss_counts(&raw),
            losses,
            components,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub converted: usize,
    pub incomplete: usize,
    pub failed: usize,
    pub not_started: usize,
    pub skipped: usize,
}

/// Files sorted by severity; source indices allow front ends to preserve input order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunView {
    pub files: Vec<(usize, FileView)>,
    pub counts: Counts,
    pub invitation: Option<String>,
    pub summary: Option<String>,
    pub first_to_show: usize,
}

impl RunView {
    /// Present files in a live run, with one invitation derived from all available reports.
    pub fn from_files<'a>(
        files: Vec<FileView>,
        reports: impl IntoIterator<Item = &'a Value>,
        running: bool,
    ) -> Self {
        Self::new(files, json::fixtures_pitch(reports), running)
    }

    pub fn new(files: Vec<FileView>, invitation: Option<String>, running: bool) -> Self {
        let mut counts = Counts::default();
        for file in &files {
            match file.status {
                Status::Converted
                | Status::ConvertedWithLosses
                | Status::Flattened
                | Status::Incomplete => counts.converted += 1,
                Status::Failed => counts.failed += 1,
                Status::NotStarted => counts.not_started += 1,
                Status::Skipped => counts.skipped += 1,
                _ => {}
            }
            counts.incomplete += usize::from(file.status == Status::Incomplete);
        }
        let first_to_show = files
            .iter()
            .position(|f| matches!(f.status, Status::Failed | Status::Incomplete))
            .unwrap_or(0);
        let summary = (!files.is_empty()).then(|| {
            if running {
                return format!(
                    "{} of {} done",
                    counts.converted + counts.failed,
                    files.len()
                );
            }
            let mut text = if counts.incomplete > 0 {
                format!(
                    "{} converted ({} incomplete), {} failed",
                    counts.converted, counts.incomplete, counts.failed
                )
            } else {
                format!("{} converted, {} failed", counts.converted, counts.failed)
            };
            if counts.not_started > 0 {
                text.push_str(&format!(", {} not started", counts.not_started));
            }
            text
        });
        let mut files: Vec<_> = files.into_iter().enumerate().collect();
        files.sort_by_key(|(_, file)| file.severity_rank);
        Self {
            files,
            counts,
            invitation: if running { None } else { invitation },
            summary,
            first_to_show,
        }
    }

    /// Present either a convert report or a convert-all report. JSON remains untouched.
    pub fn from_report(report: &Value) -> Self {
        let reports: Vec<&Value> = match report.get("results").and_then(Value::as_array) {
            Some(results) => results.iter().collect(),
            None => vec![report],
        };
        let mut view = Self::new(
            reports.iter().map(|r| FileView::from_report(r)).collect(),
            json::fixtures_pitch(reports.iter().copied()),
            false,
        );
        view.counts.skipped = report
            .get("skipped")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0);
        view
    }
}

/// Whether an assembly converted with placements left out or placed empty.
pub fn incomplete(report: &Value) -> bool {
    report
        .pointer("/completeness/placed_empty")
        .and_then(Value::as_u64)
        .is_some_and(|n| n > 0)
        || report
            .pointer("/completeness/left_out")
            .and_then(Value::as_array)
            .is_some_and(|causes| !causes.is_empty())
}

/// What marks a part file an assembly places as not found, in its loss.
const NOT_FOUND: &str = ": not found at its stored path";

/// The part files an assembly's conversion could not find, by name, when it missed any; an
/// empty list when it failed for want of all of them, without naming them.
pub fn missing_parts(report: &Value) -> Option<Vec<String>> {
    let mut names: Vec<String> = report
        .get("losses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|loss| loss.get("message").and_then(Value::as_str))
        .filter_map(|message| message.find(NOT_FOUND).map(|at| message.get(..at)))
        .flatten()
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    let none_found = report
        .pointer("/error/message")
        .and_then(Value::as_str)
        .is_some_and(|message| message.contains("parts could be found or read"));
    (!names.is_empty() || none_found).then_some(names)
}

/// A missing-parts recovery summary, with at most five names.
pub fn missing_summary(missing: &[String]) -> String {
    const SHOWN: usize = 5;
    let text = match missing.len() {
        0 => "Its part files were not found.".to_owned(),
        n => {
            let mut text = format!(
                "{n} part file{} not found: {}",
                if n == 1 { " was" } else { "s were" },
                missing
                    .iter()
                    .take(SHOWN)
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if n > SHOWN {
                text.push_str(&format!(" and {} more", n - SHOWN));
            }
            text
        }
    };
    text
}

/// What was written, in a sentence.
pub fn outcome(report: &Value) -> String {
    let mut text = outcome_without_memory_note(report);
    if report.get("retried_after_low_memory") == Some(&Value::Bool(true))
        && report.get("ok") == Some(&Value::Bool(true))
    {
        text.push_str(" Converted on its own after running out of memory alongside other files.");
    }
    if let Some(reason) = report
        .pointer("/history_check_skipped/reason")
        .and_then(Value::as_str)
    {
        if !text.contains(reason) {
            text.push(' ');
            text.push_str(reason);
            text.push('.');
        }
        if report
            .pointer("/history/rebuilt")
            .or_else(|| report.pointer("/history/mapped"))
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0)
            && !text.contains("unverified")
        {
            text.push_str(" Retained history unverified.");
        }
    }
    if let Some(note) = region_note(report) {
        text.push(' ');
        text.push_str(&note);
    }
    if let Some(note) = body_note(report) {
        text.push(' ');
        text.push_str(&note);
    }
    if let Some(note) = identity_note(report) {
        text.push(' ');
        text.push_str(&note);
    }
    text
}

/// The rebuilt extrudes whose sketch region nothing checked against the source. They replay,
/// but one could sweep the wrong region, so the card says how many rather than leave it to
/// the JSON report.
fn region_note(report: &Value) -> Option<String> {
    let count = report
        .pointer("/history/features")?
        .as_array()?
        .iter()
        .filter(|feature| feature.get("profile_unverified") == Some(&Value::Bool(true)))
        .count();
    match count {
        0 => None,
        1 => Some(
            "1 rebuilt extrude may use the wrong sketch region; it couldn't be checked against \
             the source."
                .to_owned(),
        ),
        n => Some(format!(
            "{n} rebuilt extrudes may use the wrong sketch region; it couldn't be checked against \
             the source."
        )),
    }
}

/// The partial histories whose replay doesn't hold the bodies the source stores where the
/// history stops (`history/body-mismatch`). The imported final shape is still right, but the
/// rebuilt features are not the source's, so the card says so, and what to do, rather than
/// leave it to the JSON report.
fn body_note(report: &Value) -> Option<String> {
    const ADVICE: &str =
        "The final shape is correct; check the rebuilt features before editing them.";
    let found: Vec<&str> = report
        .get("losses")?
        .as_array()?
        .iter()
        .filter(|loss| loss.get("code").and_then(Value::as_str) == Some("history/body-mismatch"))
        .map(|loss| loss.get("message").and_then(Value::as_str).unwrap_or(""))
        .collect();
    match found.as_slice() {
        [] => None,
        [why] => Some(match body_difference(why) {
            Some(short) => format!(
                "The rebuilt features don't match the original where the history stops ({short}). {ADVICE}"
            ),
            None => format!(
                "The rebuilt features don't match the original where the history stops. {ADVICE}"
            ),
        }),
        all => Some(format!(
            "In {} parts, the rebuilt features don't match the original where the history stops. {ADVICE}",
            all.len()
        )),
    }
}

/// A body-mismatch message in a few plain words: "0 bodies where it has 1", or "a rebuilt
/// body is 12.6 mm from where it has it". `None` for a message of another form.
fn body_difference(message: &str) -> Option<String> {
    if let Some(rest) = message.strip_prefix("the retained features give ") {
        let (rebuilt, source) = rest.split_once(" where the source has ")?;
        let count = source.split_whitespace().next()?;
        return Some(format!("{rebuilt} where it has {count}"));
    }
    let rest = message.strip_prefix("a retained body sits ")?;
    let distance = rest.strip_suffix(" from where the source has it at the same point")?;
    Some(format!("a rebuilt body is {distance} from where it has it"))
}

/// The parts at their design's origin because their placement could not be read. Among
/// hundreds of losses a warning goes unseen, so this one stays in the sentence always shown.
fn identity_note(report: &Value) -> Option<String> {
    const SHOWN: usize = 3;
    let parts: Vec<&str> = report
        .pointer("/part_positions/assumed")?
        .as_array()?
        .iter()
        .filter(|part| part.get("reason").and_then(Value::as_str) == Some("identity"))
        .filter_map(|part| part.get("part").and_then(Value::as_str))
        .collect();
    let names = parts
        .iter()
        .take(SHOWN)
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    match parts.len() {
        0 => None,
        1 => Some(format!(
            "1 part may not be where the source has it, because its placement could not be read: {names}."
        )),
        n if n <= SHOWN => Some(format!(
            "{n} parts may not be where the source has them, because their placement could not be read: {names}."
        )),
        n => Some(format!(
            "{n} parts may not be where the source has them, because their placement could not be read: {names} and {} more.",
            n - SHOWN
        )),
    }
}

fn outcome_without_memory_note(report: &Value) -> String {
    let field = |pointer: &str| report.pointer(pointer).and_then(Value::as_str);
    let history = |rebuilt: u64, needing: u64| format!("{rebuilt} of {needing} features rebuilt");
    // A report saved by an older version has only the counts with features that need none.
    let count = |new: &str, old: &str| {
        report
            .pointer(new)
            .or_else(|| report.pointer(old))
            .and_then(Value::as_u64)
    };
    let counts = (
        count("/history/rebuilt", "/history/mapped"),
        count("/history/needing", "/history/total"),
    );
    match field("/mode") {
        Some("partial_history") => match counts {
            (Some(m), Some(t)) => format!(
                "Partial editable history, {m} of {t} features rebuilt; imported final shape visible."
            ),
            _ => "Partial editable history; imported final shape visible.".to_owned(),
        },
        Some("history") => {
            let check = match (field("/verification/status"), field("/verification/reason")) {
                (Some("verified"), _) => {
                    if field("/output/format").is_some_and(|format| format.starts_with("freecad-"))
                    {
                        "verified in FreeCAD".to_owned()
                    } else {
                        "verified in SolidCraft".to_owned()
                    }
                }
                (Some("differs"), Some(reason)) => format!("rebuild differs: {reason}"),
                (_, Some(reason)) => format!("unverified: {reason}"),
                _ => "unverified".to_owned(),
            };
            match counts {
                (Some(m), Some(t)) => format!("Editable history, {}; {check}.", history(m, t)),
                _ => format!("Editable history; {check}."),
            }
        }
        _ => match (field("/verification/status"), field("/verification/reason")) {
            (Some("flattened"), Some(reason)) => {
                format!("Geometry without history (flattened: {reason}).")
            }
            _ => "Geometry without history.".to_owned(),
        },
    }
}

pub fn loss_counts(losses: &[&Value]) -> String {
    let count = |level: &str| {
        losses
            .iter()
            .filter(|l| l.get("level").and_then(Value::as_str) == Some(level))
            .count()
    };
    let parts: Vec<String> = ["Info", "Degraded", "Dropped"]
        .into_iter()
        .map(|level| (level, count(level)))
        .filter(|(_, n)| *n > 0)
        .map(|(level, n)| format!("{n} {level}"))
        .collect();
    format!("Losses: {}", parts.join(", "))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn the_card_says_when_rebuilt_features_do_not_match_the_source() {
        let report = |messages: &[&str]| {
            let losses: Vec<Value> = messages
                .iter()
                .map(|message| {
                    serde_json::json!({"code": "history/body-mismatch", "level": "Degraded",
                        "message": message})
                })
                .chain([serde_json::json!({"code": "f3d/other", "message": "unrelated"})])
                .collect();
            serde_json::json!({"ok": true, "mode": "partial_history",
                "history": {"rebuilt": 3, "needing": 4}, "losses": losses})
        };
        let advice = "The final shape is correct; check the rebuilt features before editing them.";
        assert!(!outcome(&report(&[])).contains("don't match"));
        assert!(outcome(&report(&["the retained features give 0 bodies where the source has 1 body at the same point"]))
            .ends_with(&format!("The rebuilt features don't match the original where the history stops (0 bodies where it has 1). {advice}")));
        assert!(outcome(&report(&["a retained body sits 12.6 mm from where the source has it at the same point"]))
            .ends_with(&format!("The rebuilt features don't match the original where the history stops (a rebuilt body is 12.6 mm from where it has it). {advice}")));
        assert!(outcome(&report(&["something else"])).ends_with(&format!(
            "The rebuilt features don't match the original where the history stops. {advice}"
        )));
        assert!(outcome(&report(&["a", "b"]))
            .ends_with(&format!("In 2 parts, the rebuilt features don't match the original where the history stops. {advice}")));
    }

    #[test]
    fn the_card_counts_extrudes_whose_region_was_not_checked() {
        let report = |flags: &[bool]| {
            let features: Vec<Value> = flags
                .iter()
                .map(|flag| {
                    let mut row = serde_json::json!({"kind": "extrude", "status": "mapped"});
                    if *flag {
                        row["profile_unverified"] = Value::Bool(true);
                    }
                    row
                })
                .collect();
            serde_json::json!({"ok": true, "mode": "partial_history",
                "history": {"rebuilt": 3, "needing": 4, "features": features}})
        };
        assert!(!outcome(&report(&[false, false])).contains("sketch region"));
        assert!(outcome(&report(&[true, false]))
            .ends_with("1 rebuilt extrude may use the wrong sketch region; it couldn't be checked against the source."));
        assert!(outcome(&report(&[true, true]))
            .ends_with("2 rebuilt extrudes may use the wrong sketch region; it couldn't be checked against the source."));
    }

    #[test]
    fn the_card_counts_features_rebuilt_and_an_older_report_still_reads() {
        let now = serde_json::json!({"ok": true, "mode": "partial_history",
            "history": {"mapped": 8, "total": 12, "rebuilt": 2, "needing": 6}});
        assert!(
            outcome(&now).contains("2 of 6 features rebuilt"),
            "{}",
            outcome(&now)
        );
        let older = serde_json::json!({"ok": true, "mode": "partial_history",
            "history": {"mapped": 8, "total": 12}});
        assert!(outcome(&older).contains("8 of 12 features rebuilt"));
    }

    #[test]
    fn parts_at_identity_stay_in_the_sentence_always_shown() {
        let part = |name: &str, reason: &str| serde_json::json!({"part": name, "reason": reason});
        let report = |assumed: Value| serde_json::json!({"ok": true, "mode": "import", "part_positions": {"assumed": assumed}});
        let one = outcome(&report(serde_json::json!([part("Lid", "identity")])));
        assert!(
            one.contains("1 part may not be where the source has it"),
            "{one}"
        );
        assert!(one.contains(": Lid."), "{one}");
        let many = outcome(&report(serde_json::json!([
            part("A", "identity"),
            part("B", "identity"),
            part("C", "identity"),
            part("D", "identity"),
            part("E", "only_stored_pose")
        ])));
        assert!(many.contains("4 parts may not be"), "{many}");
        assert!(many.contains(": A, B, C and 1 more."), "{many}");
        let stored = outcome(&report(serde_json::json!([part("E", "only_stored_pose")])));
        assert!(!stored.contains("may not be"), "{stored}");
    }

    #[test]
    fn a_retried_file_says_so_where_the_card_always_shows() {
        let report =
            serde_json::json!({"ok": true, "mode": "import", "retried_after_low_memory": true});
        assert!(outcome(&report).contains("Converted on its own"));
        let first_try = serde_json::json!({"ok": true, "mode": "import"});
        assert!(!outcome(&first_try).contains("Converted on its own"));
    }

    #[test]
    fn a_partial_history_card_shows_a_memory_skip() {
        let reason = "History check skipped: needs about 9.4 GB, 2.0 GB free";
        let report = serde_json::json!({"ok": true, "mode": "partial_history",
            "history": {"mapped": 2, "total": 3},
            "verification": {"status": "partial", "reason": "prefix unverified"},
            "history_check_skipped": {"reason": reason}});
        assert!(outcome(&report).contains(reason));
        assert!(outcome(&report).contains("unverified"));
    }

    use serde_json::json;

    #[test]
    fn statuses_and_outcomes_preserve_desktop_words() {
        let cases = [
            (
                json!({"ok": true}),
                Status::Converted,
                "Geometry without history.",
            ),
            (
                json!({"ok": true, "losses": [{"level": "Info"}]}),
                Status::ConvertedWithLosses,
                "Geometry without history.",
            ),
            (
                json!({"ok": true, "verification": {"status": "flattened", "reason": "unsupported feature"}}),
                Status::Flattened,
                "Geometry without history (flattened: unsupported feature).",
            ),
            (
                json!({"ok": true, "mode": "history", "history": {"mapped": 2, "total": 3}, "verification": {"status": "verified"}}),
                Status::Converted,
                "Editable history, 2 of 3 features rebuilt; verified in SolidCraft.",
            ),
            (
                json!({"ok": true, "mode": "history", "output": {"format": "freecad-document/1"}, "verification": {"status": "verified"}}),
                Status::Converted,
                "Editable history; verified in FreeCAD.",
            ),
            (
                json!({"ok": true, "mode": "history", "verification": {"status": "differs", "reason": "volume"}}),
                Status::Converted,
                "Editable history; rebuild differs: volume.",
            ),
            (
                json!({"ok": true, "mode": "history", "verification": {"reason": "no CLI"}}),
                Status::Converted,
                "Editable history; unverified: no CLI.",
            ),
            (
                json!({"ok": true, "mode": "history"}),
                Status::Converted,
                "Editable history; unverified.",
            ),
            (
                json!({"ok": true, "completeness": {"left_out": [{"count": 1}]}}),
                Status::Incomplete,
                "Geometry without history.",
            ),
            (
                json!({"ok": false, "error": {"message": "no geometry"}}),
                Status::Failed,
                "no geometry",
            ),
        ];
        for (report, status, outcome) in cases {
            let view = FileView::from_report(&report);
            assert_eq!(view.status, status);
            assert_eq!(view.outcome, outcome);
        }
        for (progress, status, word) in [
            (Progress::Waiting, Status::Waiting, "Waiting"),
            (Progress::Converting, Status::Converting, "Converting"),
            (Progress::NotStarted, Status::NotStarted, "Not started"),
            (Progress::Skipped, Status::Skipped, "Skipped"),
        ] {
            let file = FileView::new(Path::new("part.f3d"), None, progress, None);
            assert_eq!((file.status, file.status_word), (status, word));
        }
    }

    #[test]
    fn reporting_distinguishes_converted_and_failed_files_and_excludes_missing_parts() {
        for report in [
            json!({"ok": true}),
            json!({"ok": true, "losses": [{"level": "Dropped", "message": "Colour omitted"}]}),
            json!({"ok": true, "verification": {"status": "flattened"}}),
            json!({"ok": true, "completeness": {"placed_empty": 1}}),
        ] {
            let view = FileView::from_report(&report);
            assert!(view.can_report);
            assert_eq!(view.report_case, Some(ReportCase::Converted));
        }
        let failed = FileView::from_report(&json!({"ok": false, "error": {"message": "Stopped"}}));
        assert!(failed.can_report);
        assert_eq!(failed.report_case, Some(ReportCase::Failed));
        for ok in [true, false] {
            let view = FileView::from_report(&json!({"ok": ok, "error": {"message": "Stopped"},
                "losses": [{"message": "pin.ipt: not found at its stored path"}]}));
            assert!(!view.can_report);
            assert_eq!(view.report_case, None);
        }
        for progress in [
            Progress::Waiting,
            Progress::Converting,
            Progress::NotStarted,
            Progress::Skipped,
            Progress::Converted,
        ] {
            let view = FileView::new(Path::new("part.f3d"), None, progress, None);
            assert!(!view.can_report);
            assert_eq!(view.report_case, None);
        }
    }

    #[test]
    fn run_order_and_reveal_keep_equal_severity_in_input_order() {
        let report = json!({"results": [
            {"ok": true},
            {"ok": true, "completeness": {"left_out": [{"count": 1}]}},
            {"ok": false}, {"ok": false},
            {"ok": true, "losses": [{"level": "Info"}]}
        ], "skipped": 4});
        let run = RunView::from_report(&report);
        assert_eq!(
            run.files.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            [1, 2, 3, 4, 0]
        );
        assert_eq!(run.first_to_show, 1);
        assert_eq!(
            run.counts,
            Counts {
                converted: 3,
                incomplete: 1,
                failed: 2,
                skipped: 4,
                not_started: 0
            }
        );
        assert_eq!(
            run.summary.as_deref(),
            Some("3 converted (1 incomplete), 2 failed")
        );
        let files = vec![FileView::new(
            Path::new("a"),
            None,
            Progress::NotStarted,
            None,
        )];
        assert_eq!(
            RunView::new(files.clone(), None, false).summary.as_deref(),
            Some("0 converted, 0 failed, 1 not started")
        );
        assert_eq!(
            RunView::new(files, Some("invite".into()), true)
                .summary
                .as_deref(),
            Some("0 of 1 done")
        );
        assert!(RunView::new(vec![], None, false).summary.is_none());
        assert_eq!(RunView::new(vec![], None, false).first_to_show, 0);
    }

    #[test]
    fn losses_use_shared_component_order_and_full_messages() {
        let report = json!({"ok": true, "losses": [
            {"level": "Info", "subject": "a.ipt: sketch", "message": "First. Second."},
            {"level": "Dropped", "subject": "z.ipt: face", "message": "gone"},
            {"level": "Degraded", "subject": "history", "message": "flattened: feature"},
            {"level": "Info", "subject": "a.ipt: parameter", "message": "kept"}
        ]});
        let view = FileView::from_report(&report);
        assert_eq!(view.loss_summary, "Losses: 2 Info, 1 Degraded, 1 Dropped");
        assert_eq!(
            view.losses
                .iter()
                .map(|g| (g.level.as_str(), g.count))
                .collect::<Vec<_>>(),
            [("Info", 2), ("Degraded", 1), ("Dropped", 1)]
        );
        assert_eq!(
            view.losses[0].lines[0].text,
            "a.ipt: sketch: First. Second."
        );
        assert_eq!(
            view.losses[0].lines[0].full_text,
            "Info: a.ipt: sketch: First. Second."
        );
        assert_eq!(
            view.components
                .iter()
                .map(|c| c.heading.as_str())
                .collect::<Vec<_>>(),
            [
                "The design: history flattened (feature); 1 Degraded",
                "z.ipt: 1 Dropped",
                "a.ipt: 2 Info"
            ]
        );
    }

    #[test]
    fn missing_parts_are_unique_sorted_and_bounded_in_the_summary() {
        let report = json!({"ok": true, "losses": [
            {"message": "pin.ipt: not found at its stored path"},
            {"message": "base.ipt: not found at its stored path"},
            {"message": "pin.ipt: not found at its stored path"}
        ]});
        let view = FileView::from_report(&report);
        assert_eq!(view.missing_parts.unwrap(), ["base.ipt", "pin.ipt"]);
        assert_eq!(
            view.missing_summary.as_deref(),
            Some("2 part files were not found: base.ipt, pin.ipt")
        );
        let failed = FileView::from_report(
            &json!({"ok": false, "error": {"message": "none of the assembly's parts could be found or read"}}),
        );
        assert_eq!(failed.missing_parts, Some(vec![]));
        assert!(!failed.can_report);
        assert_eq!(
            failed.missing_summary.as_deref(),
            Some("Its part files were not found.")
        );
        assert_eq!(
            missing_summary(&(0..7).map(|n| n.to_string()).collect::<Vec<_>>()),
            "7 part files were not found: 0, 1, 2, 3, 4 and 2 more"
        );
        assert!(!incomplete(&json!({"completeness": {"left_out": []}})));
    }

    #[test]
    fn empty_placements_make_the_shared_card_incomplete() {
        let report = json!({"ok": true, "completeness": {"placed": 2, "placed_empty": 1,
            "placements": 3, "left_out": []}});
        let view = FileView::from_report(&report);
        assert!(view.incomplete);
        assert_eq!(view.status, Status::Incomplete);
        assert_eq!(
            view.assembly.as_deref(),
            Some("2 of 3 components placed; 1 placed empty, no joints")
        );
    }

    fn failed_on(format: &str) -> Value {
        json!({"ok": false, "input": {"path": "/tmp/part.x", "format": format},
               "error": {"message": "failed", "kind": "no_geometry"}})
    }

    #[test]
    fn the_steps_follow_the_size_and_the_surface() {
        let policy = ReportPolicy::bundled();
        let report = failed_on("fusion_archive");
        let small = SendSteps::new(policy, Some(1 << 20), &report, Surface::Desktop);
        assert_eq!(small.dm_text, "Drag the file in (its folder is open).");
        assert!(small.drag);
        let web = SendSteps::new(policy, Some(1 << 20), &report, Surface::Web);
        assert_eq!(web.dm_text, "Drag the file in (the file you picked).");
        let big = SendSteps::new(policy, Some(11 << 20), &report, Surface::Desktop);
        assert!(!big.drag);
        assert_eq!(
            big.dm_text,
            "Paste the message. Dutchy replies with a private upload link (up to 300 MB for \
             the design, 300 MB for its STEP, or a 1.5 GB zip, for 72 hours)."
        );
        assert!(!big.dm_text.contains("Drag"));
    }

    #[test]
    fn only_a_step_source_skips_the_step_card_and_counts_as_known() {
        let policy = ReportPolicy::bundled();
        let step = failed_on("step");
        assert!(!SendSteps::new(policy, None, &step, Surface::Web).step_card);
        assert_eq!(step_known(&step), Some(true));
        let other = failed_on("inventor_part");
        assert!(SendSteps::new(policy, None, &other, Surface::Web).step_card);
        assert_eq!(step_known(&other), None);
        assert_eq!(error_code(&other), Some("no_geometry"));
    }

    #[test]
    fn the_card_and_the_log_name_a_file_by_one_id() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("Secret Bracket.f3d");
        std::fs::write(&input, b"not a design").unwrap();
        let salt = anon::Salt::random().unwrap();
        let log_path = dir.path().join("log.jsonl");
        let options = crate::ConvertOptions {
            diagnostics_salt: Some(salt.clone()),
            ..crate::ConvertOptions::default()
        };
        drop(crate::diagnostics::Log::open(&log_path, &input, &options).unwrap());
        let log = std::fs::read_to_string(&log_path).unwrap();
        let first: Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
        let report = json!({"ok": false, "input": {"path": input},
                            "error": {"message": "failed"}});
        let run = anon::Run::default();
        let (anonymous, id) = anonymous_report(&report, dir.path(), &salt, &run);
        assert_eq!(first["file"], id.as_str());
        assert!(!anonymous.to_string().contains("Secret"));
        // A second link for the same file under the same salt says the same id.
        assert_eq!(anonymous_report(&report, dir.path(), &salt, &run).1, id);
    }

    #[test]
    fn issue_links_reuse_the_form_builder_and_explain_the_paste_fallback() {
        let report = json!({"ok": false, "input": {"format_name": "Fusion"}, "error": {"message": "private name"}});
        let anonymous = json!({"errors": [{"message": "no geometry", "stage": "decode", "upstream": "cadmpeg"}]});
        let link = report_link(&report, &anonymous);
        assert_eq!(link.issue, issue::fill(&report, &anonymous));
        for field in [
            "dutchman-version=",
            "os=",
            "what-happened=",
            "stage=stage%3A%20decode%2C%20upstream%3A%20cadmpeg",
            "report-anon=Paste",
        ] {
            assert!(link.issue.url.contains(field));
        }
        assert!(!link.issue.url.contains("private"));
        assert_eq!(link.copy_notice, REPORT_COPIED);
        let converted = report_link(
            &json!({"ok": true, "input": {"format_name": "Fusion"}}),
            &json!({"errors": []}),
        );
        assert!(converted.issue.url.contains("converted%20incorrectly"));
        assert!(converted.issue.url.contains("result%20is%20wrong"));
        assert!(!converted.issue.url.contains("failed"));
        let long = json!({"errors": [], "losses": [{"message": "x".repeat(issue::MAX_URL)}]});
        let link = report_link(&report, &long);
        assert_eq!(link.copy_notice, REPORT_COPIED);
        assert!(link.issue.url.len() <= issue::MAX_URL);
        assert!(!link.issue.url.contains("xxxx"));
    }

    #[test]
    fn invitations_are_once_per_finished_run_and_never_added_to_reports() {
        let report = json!({"schema": "dutchman.convert-all/1", "results": [
            {"ok": false, "input": {"program": "Inventor"}},
            {"ok": false, "input": {"program": "Inventor"}},
            {"ok": true, "input": {"program": "Fusion"}, "verification": {"status": "flattened"}}
        ]});
        let before = report.to_string();
        let view = RunView::from_report(&report);
        let invitation = view.invitation.unwrap();
        assert_eq!(invitation.matches("Want to help").count(), 1);
        assert!(invitation.contains("Fusion, Inventor"));
        assert_eq!(report.to_string(), before);
        assert!(!before.contains("Want to help"));
        assert!(RunView::new(vec![], Some(invitation), true)
            .invitation
            .is_none());
        assert!(RunView::from_report(&json!({"ok": true}))
            .invitation
            .is_none());
        assert!(
            RunView::from_report(&json!({"ok": false, "input": {"format": "iges"}}))
                .invitation
                .is_none()
        );
    }
}
