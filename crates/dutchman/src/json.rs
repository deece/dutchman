//! The machine-readable reports `--json` prints. Their field names are stable: fields are
//! added, never renamed or removed. See `docs/json.md`.

use std::collections::BTreeMap;
use std::path::Path;

use dutchman_core::{
    Check, Completeness, Error, FeatureStatus, Format, Mode, Outcome, Report, Severity, Stage,
};
use serde_json::{json, Map, Value};

/// The schema of `dutchman convert --json`.
pub const CONVERT: &str = "dutchman.convert/1";

/// The schema of `dutchman inspect --json`.
pub const INSPECT: &str = "dutchman.inspect/1";

/// The schema of `dutchman diagnose`'s report.
pub const DIAGNOSE: &str = "dutchman.diagnose/1";

/// The schema of `dutchman diagnose --narrow`'s result.
pub const NARROW: &str = "dutchman.narrow/1";

/// The schema of `dutchman diagnose --same-failure`'s result.
pub const SAME_FAILURE: &str = "dutchman.same-failure/1";

/// The schema of `dutchman diagnose --make-report`'s result.
pub const MADE_REPORT: &str = "dutchman.made-report/1";

/// The schema of an anonymous, bounded part/history trial.
pub const PART_TRIAL: &str = "dutchman.part-trial/1";

/// The schema of report consent previews and sending receipts.
pub const REPORT_SEND: &str = "dutchman.report-send/1";

/// The schema of a minimum-reproduction kit.
pub const REPRO: &str = "dutchman.repro/1";

/// The schema of `dutchman investigate`'s result.
pub const INVESTIGATE: &str = "dutchman.investigate/1";

/// Shareable reproduction facts; callers supply only allowlisted failure facts and fixed
/// anonymous narrowing categories, never a model, private key or raw conversion report.
pub(crate) fn repro_anonymous(
    input: &Path,
    failure: &Value,
    narrowing: Value,
    kinds: &Value,
) -> Value {
    let size_class = std::fs::metadata(input).ok().map(|m| match m.len() {
        0..=1_048_575 => "under_1_mib",
        1_048_576..=16_777_215 => "1_to_16_mib",
        16_777_216..=268_435_455 => "16_to_256_mib",
        _ => "256_mib_or_more",
    });
    json!({"schema": REPRO, "versions": {"dutchman": env!("CARGO_PKG_VERSION"), "cadmpeg": crate::anon::CADMPEG_VERSION},
        "input": {"format": Format::from_path(input).map(|f| f.id()), "size_class": size_class},
        "status": if failure.is_null() { "no_failure_to_reproduce" } else { "failure" },
        "failure": failure, "narrowing": narrowing, "retained_unit_kinds": kinds})
}

/// The input file as the reports describe it.
pub fn input(path: &Path) -> Value {
    let format = Format::from_path(path);
    json!({
        "path": path.display().to_string(),
        "format": format.map(Format::id),
        "format_name": format.map(Format::name),
        "program": format.map(Format::family),
        "extension": Format::source_extension(path),
        "words": format.map(Format::words),
    })
}

/// A conversion's input capability, before its result.
pub fn status_line(report: &Value) -> Option<String> {
    let input = report.get("input")?;
    Some(format!(
        "{} .{}: {}",
        input.get("program")?.as_str()?,
        input.get("extension")?.as_str()?,
        input.get("words")?.as_str()?
    ))
}

pub const FIXTURES_URL: &str = "https://dutchman-cad.github.io/fixtures/";
pub const DISCORD_URL: &str = "https://discord.gg/yTcseBEdeu";

/// One invitation for the programs whose inputs need more coverage.
pub fn fixtures_pitch<'a>(reports: impl IntoIterator<Item = &'a Value>) -> Option<String> {
    let mut programs = std::collections::BTreeSet::new();
    for report in reports {
        let input = report.get("input");
        if input.and_then(|i| i.get("format")).and_then(Value::as_str) == Some(Format::Iges.id()) {
            continue;
        }
        if report.get("ok").and_then(Value::as_bool) == Some(false)
            || matches!(
                report
                    .pointer("/verification/status")
                    .and_then(Value::as_str),
                Some("flattened" | "partial")
            )
            || report
                .pointer("/history/mapped")
                .and_then(Value::as_u64)
                .zip(report.pointer("/history/total").and_then(Value::as_u64))
                // Features that need none are mapped too, so this says the same of `rebuilt`.
                .is_some_and(|(mapped, total)| mapped < total)
        {
            programs.insert(
                input
                    .and_then(|i| i.get("program"))
                    .and_then(Value::as_str)
                    .unwrap_or("CAD programs"),
            );
        }
    }
    if programs.is_empty() {
        return None;
    }
    let programs = programs.into_iter().collect::<Vec<_>>().join(", ");
    Some(format!("Want to help Dutchman read {programs} better? If you have {programs}, you can make a few small test files from short recipes: {FIXTURES_URL} or type /volunteer in our Discord ({DISCORD_URL}). Got a file that converts wrong? Drop it, with its STEP export, in a DM to Dutchy in our Discord."))
}

/// One loss of a report as the text reports print it, `LEVEL: SUBJECT: MESSAGE`, with the
/// message cut to its first sentence unless `verbose`; and whether it was cut.
pub fn loss_line(loss: &Value, verbose: bool) -> (String, bool) {
    let field = |name: &str| loss.get(name).and_then(Value::as_str).unwrap_or("");
    let (message, cut) = if verbose {
        (field("message").to_owned(), false)
    } else {
        first_sentence(field("message"))
    };
    (
        format!("{}: {}: {message}", field("level"), field("subject")),
        cut,
    )
}

/// Compact repeated losses for stderr while retaining every entry in JSON and logs.
/// Appearance binding IDs identify individual faces, not different kinds of loss.
pub fn loss_lines(losses: &[Value], verbose: bool) -> Vec<(String, bool)> {
    if verbose {
        return losses.iter().map(|loss| loss_line(loss, true)).collect();
    }
    let mut groups: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    let mut rows: Vec<(&Value, String, usize)> = Vec::new();
    for loss in losses {
        let field = |name| loss.get(name).and_then(Value::as_str).unwrap_or("");
        let message = field("message");
        let normalized = if field("subject").rsplit(": ").next() == Some("appearance") {
            message
                .strip_prefix("Appearance binding ")
                .and_then(|binding| binding.split_once(" is left out: "))
                .map(|(_, reason)| format!("appearance bindings left out: {reason}"))
                .unwrap_or_else(|| message.to_owned())
        } else {
            message.to_owned()
        };
        let key = (
            field("level").to_owned(),
            field("subject").to_owned(),
            normalized.clone(),
        );
        if let Some(row) = groups.get(&key).and_then(|index| rows.get_mut(*index)) {
            row.2 += 1;
        } else {
            groups.insert(key, rows.len());
            rows.push((loss, normalized, 1));
        }
    }
    rows.into_iter()
        .map(|(loss, message, count)| {
            if count == 1 {
                loss_line(loss, false)
            } else {
                let mut summary = loss.clone();
                if let Some(fields) = summary.as_object_mut() {
                    let counted = if message.starts_with("appearance bindings left out: ") {
                        format!("{count} {message}")
                    } else {
                        format!("{count} notes: {message}")
                    };
                    fields.insert("message".to_owned(), json!(counted));
                }
                let (line, _) = loss_line(&summary, false);
                (line, true)
            }
        })
        .collect()
}

/// The first sentence of `text`, marked with "…" when more followed; and whether it was cut.
/// A sentence ends at ". " before a capital letter, so "e.g. so" does not end one.
pub fn first_sentence(text: &str) -> (String, bool) {
    let bytes = text.as_bytes();
    let end = text.match_indices(". ").map(|(at, _)| at).find(|at| {
        bytes
            .get(at + 2)
            .is_some_and(|next| next.is_ascii_uppercase())
    });
    match end {
        Some(at) => (format!("{} …", text.get(..=at).unwrap_or(text)), true),
        None => (text.to_owned(), false),
    }
}

/// The losses of one component of a design: the design's own, or a part file's.
#[derive(Debug, PartialEq)]
pub struct Component<'a> {
    /// The part file's name; `None` for the design itself.
    pub file: Option<String>,
    /// How many losses it has at each level: Dropped, Degraded, Info.
    pub dropped: usize,
    pub degraded: usize,
    pub info: usize,
    /// Why its feature history was not written, when it was not.
    pub flattened: Option<String>,
    /// Its losses, in the order they occurred.
    pub losses: Vec<&'a Value>,
}

/// The extensions of the files a design's components are read from.
const COMPONENT_FILES: [&str; 6] = [".ipt", ".iam", ".f3d", ".f3z", ".sldprt", ".sldasm"];

/// The component a loss belongs to: the deepest part file its subject names
/// (`sub.iam: pin.ipt: …`), else the part file its message starts with
/// (`history: pin.ipt: flattened: …`), else the design itself.
fn component_of(loss: &Value) -> Option<String> {
    let field = |key: &str| loss.get(key).and_then(Value::as_str).unwrap_or("");
    let is_file = |segment: &str| {
        let lower = segment.to_lowercase();
        COMPONENT_FILES.iter().any(|ext| lower.ends_with(ext))
    };
    let subject = field("subject");
    subject
        .split(": ")
        .filter(|segment| is_file(segment))
        .last()
        .or_else(|| {
            field("message")
                .split(": ")
                .next()
                .filter(|segment| is_file(segment))
        })
        .map(str::to_owned)
}

/// A report's losses by component: the design's own first, then the components with the most
/// that matters (Dropped, then Degraded), then by name.
pub fn components(report: &Value) -> Vec<Component<'_>> {
    let mut by_file: BTreeMap<Option<String>, Component<'_>> = BTreeMap::new();
    for loss in report
        .get("losses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let file = component_of(loss);
        let component = by_file.entry(file.clone()).or_insert_with(|| Component {
            file,
            dropped: 0,
            degraded: 0,
            info: 0,
            flattened: None,
            losses: Vec::new(),
        });
        match loss.get("level").and_then(Value::as_str) {
            Some("Dropped") => component.dropped += 1,
            Some("Degraded") => component.degraded += 1,
            _ => component.info += 1,
        }
        let message = loss.get("message").and_then(Value::as_str).unwrap_or("");
        if component.flattened.is_none() {
            let flattened = match &component.file {
                Some(file) => message
                    .strip_prefix(file.as_str())
                    .and_then(|rest| rest.strip_prefix(": flattened: ")),
                None => message.strip_prefix("flattened: "),
            };
            if loss.get("subject").and_then(Value::as_str) == Some("history") {
                component.flattened = flattened.map(str::to_owned);
            }
        }
        component.losses.push(loss);
    }
    let mut out: Vec<Component<'_>> = by_file.into_values().collect();
    out.sort_by(|a, b| {
        a.file
            .is_some()
            .cmp(&b.file.is_some())
            .then_with(|| b.dropped.cmp(&a.dropped))
            .then_with(|| b.degraded.cmp(&a.degraded))
            .then_with(|| a.file.cmp(&b.file))
    });
    out
}

impl Component<'_> {
    /// Its counts in words: `history flattened (fillet is not mapped yet); 2 Dropped, 1
    /// Degraded`.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(reason) = &self.flattened {
            let (reason, _) = first_sentence(reason);
            parts.push(format!("history flattened ({reason})"));
        }
        let counts: Vec<String> = [
            (self.dropped, "Dropped"),
            (self.degraded, "Degraded"),
            (self.info, "Info"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, level)| format!("{n} {level}"))
        .collect();
        if !counts.is_empty() {
            parts.push(counts.join(", "));
        }
        parts.join("; ")
    }
}

/// Up to this many losses are listed one by one; more are summarised by component.
pub const LISTED: usize = 12;

/// The components [`grouped_lines`] lists before saying how many more there are.
const COMPONENTS_SHOWN: usize = 15;

/// A report's losses in a few lines, by component: `the design: 2 Degraded`, then
/// `pin.ipt: history flattened (…); 1 Dropped`, most that matters first.
pub fn grouped_lines(report: &Value) -> Vec<String> {
    let components = components(report);
    let total: usize = components.iter().map(|c| c.losses.len()).sum();
    let mut lines = vec![format!(
        "losses: {total} in {} component{}",
        components.len(),
        if components.len() == 1 { "" } else { "s" }
    )];
    for component in components.iter().take(COMPONENTS_SHOWN) {
        let name = component.file.as_deref().unwrap_or("the design");
        lines.push(format!("  {name}: {}", component.summary()));
    }
    if components.len() > COMPONENTS_SHOWN {
        lines.push(format!(
            "  and {} more components",
            components.len() - COMPONENTS_SHOWN
        ));
    }
    lines
}

/// The whole text report of one file, every note in full: what it became, how complete it
/// is, then every loss by component.
pub fn full_log(report: &Value, line: &str) -> String {
    let mut out = format!("{line}\n");
    for completeness in completeness_lines(report) {
        out.push_str(&completeness);
        out.push('\n');
    }
    for component in components(report) {
        let name = component.file.as_deref().unwrap_or("the design");
        out.push_str(&format!("\n{name}: {}\n", component.summary()));
        for loss in component.losses {
            let (line, _) = loss_line(loss, true);
            out.push_str(&format!("  {line}\n"));
        }
    }
    out
}

/// What the text reports say once when they cut a note short.
pub const CUT_NOTE: &str =
    "repeated notes are grouped and long notes shortened; --verbose prints every note in full";

/// A report of a run that stopped with `message`, at the input stage (the file, or how it
/// was asked for).
pub fn failure(schema: &str, input: Value, message: &str) -> Value {
    failure_at(schema, input, message, Stage::Input, None)
}

/// A report of a run that stopped with `error`.
pub fn error(schema: &str, input: Value, error: &Error) -> Value {
    let mut report = failure_at(
        schema,
        input,
        &error.to_string(),
        error.stage(),
        error.upstream(),
    );
    if let Some(fields) = report.get_mut("error").and_then(Value::as_object_mut) {
        fields.insert("kind".into(), json!(error.kind()));
        if let Error::Checks { facts, .. } = error {
            fields.insert("findings".into(), json!(facts.findings));
            fields.insert("curve_kinds".into(), json!(facts.curve_kinds));
        }
    }
    report
}

/// A report of a run that stopped with `message` at `stage`, in `upstream`'s code when it is
/// not Dutchman's own.
pub fn failure_at(
    schema: &str,
    input: Value,
    message: &str,
    stage: Stage,
    upstream: Option<&str>,
) -> Value {
    json!({
        "schema": schema,
        "ok": false,
        "input": input,
        "error": {"message": message, "stage": stage.id(), "upstream": upstream},
    })
}

/// The fields every successful report shares: what was (or would be) written, and the losses.
pub fn success(schema: &str, input: Value, report: &Report) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("schema".to_owned(), json!(schema));
    out.insert("ok".to_owned(), json!(true));
    out.insert("input".to_owned(), input);
    if let Some((requested, used)) = report.threads() {
        out.insert(
            "threads".to_owned(),
            json!({"requested": requested, "used": used}),
        );
    }
    if let Some(outcome) = report.outcome() {
        out.extend(outcome_fields(outcome));
        if !report.parts().is_empty() {
            if let Some(history) = out.get_mut("history").and_then(Value::as_object_mut) {
                history.insert("parts".to_owned(), part_fields(report.parts()));
            }
        }
    }
    if let Some((needed, available)) = report.history_check_skipped() {
        out.insert(
            "history_check_skipped".to_owned(),
            json!({
                "needed_bytes": needed,
                "available_bytes": available,
                "reason": crate::stop::history_check_message(needed, available),
            }),
        );
    }
    if let Some(completeness) = report.completeness() {
        out.insert("completeness".to_owned(), completeness_json(completeness));
    }
    if !report.joints().is_empty() {
        out.insert("joints".to_owned(), json!(report.joints()));
    }
    if let Some((mode, c)) = report.part_positions() {
        out.insert("part_positions".to_owned(), json!({"mode":mode.name(), "total":c.total(),
            "displayed":c.displayed, "current":c.current, "first_placed":c.first_placed,
            "derived_holders":c.derived_holders, "holder_conflicts":c.holder_conflicts,
            "bound_displayed":c.displayed.saturating_sub(c.derived_holders),
            "fallbacks":{"no_current_pose":c.no_current_pose,"ambiguous":c.ambiguous,"unsupported":c.unsupported,"joints":c.joints,"bounds":c.bounds,"instances":c.instances},
            "assumed":report.placement_assumptions().iter().map(|(part, why)| json!({"part":part,"reason":why.id()})).collect::<Vec<_>>()}));
    }
    out.insert("losses".to_owned(), losses(report));
    if !report.names().is_empty() {
        out.insert("names".to_owned(), json!(report.names()));
    }
    out
}

/// How much of an assembly was placed, as the reports give it.
pub fn completeness_json(completeness: &Completeness) -> Value {
    let left_out: Vec<Value> = completeness
        .left_out
        .iter()
        .map(|group| {
            json!({
                "file": group.file,
                "reason": group.reason,
                "stage": group.stage.id(),
                "count": group.count,
            })
        })
        .collect();
    json!({
        "placed": completeness.placed,
        "placed_empty": completeness.placed_empty,
        "placements": completeness.placements(),
        "left_out": left_out,
    })
}

/// What a report's assembly brought across, in words, when it is of an assembly: `41
/// components, 6 joints (5 revolute, 1 rigid)`, or `18 of 20 components placed, …` when some
/// were left out.
pub fn assembly_line(report: &Value) -> Option<String> {
    let completeness = report.get("completeness")?;
    let count = |key: &str| completeness.get(key).and_then(Value::as_u64).unwrap_or(0);
    let (placed, placements) = (count("placed"), count("placements"));
    let empty = count("placed_empty");
    let mut components = if placed == placements {
        format!("{placed} component{}", plural(placed))
    } else {
        format!("{placed} of {placements} components placed")
    };
    if empty > 0 {
        components.push_str(&format!("; {empty} placed empty"));
    }
    let kinds: Vec<(String, u64)> = report
        .get("joints")
        .and_then(Value::as_object)
        .map(|joints| {
            joints
                .iter()
                .filter_map(|(kind, n)| Some((kind.clone(), n.as_u64()?)))
                .collect()
        })
        .unwrap_or_default();
    let total: u64 = kinds.iter().map(|(_, n)| n).sum();
    let joints = if total == 0 {
        "no joints".to_owned()
    } else {
        let each: Vec<String> = kinds
            .iter()
            .map(|(kind, n)| format!("{n} {kind}"))
            .collect();
        format!("{total} joint{} ({})", plural(total), each.join(", "))
    };
    Some(format!("{components}, {joints}"))
}

fn plural(n: u64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// The most causes [`completeness_lines`] lists.
const CAUSES_SHOWN: usize = 10;

/// A report's completeness in words, when it has one: `assembly: 18 of 20 placements placed;
/// 2 left out:`, then a line per cause, `  pin.ipt ×2: not found …`.
pub fn completeness_lines(report: &Value) -> Vec<String> {
    let Some(completeness) = report.get("completeness") else {
        return Vec::new();
    };
    let count = |key: &str| completeness.get(key).and_then(Value::as_u64).unwrap_or(0);
    let (placed, placements) = (count("placed"), count("placements"));
    let causes: Vec<&Value> = completeness
        .get("left_out")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let empty = count("placed_empty");
    if causes.is_empty() && empty == 0 {
        return vec![format!("assembly: all {placements} placements placed")];
    }
    let mut summary = format!("assembly: {placed} of {placements} placements placed");
    if empty > 0 {
        summary.push_str(&format!("; {empty} placed empty"));
    }
    let left_out = placements.saturating_sub(placed).saturating_sub(empty);
    if !causes.is_empty() {
        summary.push_str(&format!("; {left_out} left out:"));
    }
    let mut lines = vec![summary];
    for cause in causes.iter().take(CAUSES_SHOWN) {
        let field = |key: &str| cause.get(key).and_then(Value::as_str);
        let times = cause.get("count").and_then(Value::as_u64).unwrap_or(1);
        let file = field("file").unwrap_or("(no file)");
        let reason = field("reason").unwrap_or("");
        // The reason usually starts with the file's name; say it once.
        let reason = reason
            .strip_prefix(file)
            .and_then(|rest| rest.strip_prefix(": "))
            .unwrap_or(reason);
        lines.push(format!("  {file} ×{times}: {reason}"));
    }
    if causes.len() > CAUSES_SHOWN {
        lines.push(format!(
            "  and {} more causes (--json lists them all)",
            causes.len() - CAUSES_SHOWN
        ));
    }
    lines
}

/// A check as a verification status and its reason.
fn check_fields(check: &Check) -> (&'static str, Option<String>) {
    let (status, reason) = check_raw(check);
    (status, reason.map(|r| dutchman_core::clean_refusal(r)))
}

fn check_raw(check: &Check) -> (&'static str, Option<&String>) {
    match check {
        Check::Partial(reason) => ("partial", Some(reason)),
        Check::Verified => ("verified", None),
        Check::Differs(reason) => ("differs", Some(reason)),
        Check::Unverified(reason) => ("unverified", Some(reason)),
        Check::Flattened(reason) => ("flattened", Some(reason)),
        Check::NotApplicable(reason) => ("not_applicable", Some(reason)),
    }
}

/// Each assembly part's history: its name, how it was checked and why, and its counts.
fn part_fields(parts: &[dutchman_core::PartHistory]) -> Value {
    parts
        .iter()
        .map(|part| {
            let (status, reason) = check_fields(&part.check);
            json!({
                "name": part.name,
                "status": status,
                "reason": reason,
                "mapped": part.mapped,
                "total": part.total,
                "rebuilt": part.rebuilt,
                "needing": part.needing,
                "reapplied": part.reapplied,
            })
        })
        .collect()
}

fn outcome_fields(outcome: &Outcome) -> Map<String, Value> {
    let (status, reason) = check_fields(&outcome.check);
    let features: Vec<Value> = outcome
        .features
        .iter()
        .map(|feature| {
            let refusal = feature
                .reason
                .as_deref()
                .and_then(dutchman_core::classify_refusal);
            let mut row = json!({
                "name": feature.name,
                "kind": feature.kind,
                "status": match feature.status {
                    FeatureStatus::Mapped => "mapped",
                    FeatureStatus::NotNeeded => "not_needed",
                    FeatureStatus::LeftOut => "left_out",
                    FeatureStatus::Blocked => "blocked",
                },
                "reason": refusal.as_ref().map_or(feature.reason.as_deref(), |r| Some(r.text.as_str())),
                "ordinal": feature.ordinal,
                "variant": feature.variant,
            });
            if let (true, Some(fields)) = (feature.profile_unverified, row.as_object_mut()) {
                fields.insert("profile_unverified".to_owned(), json!(true));
            }
            if let (true, Some(fields)) = (feature.reapplied, row.as_object_mut()) {
                fields.insert("reapplied".to_owned(), json!(true));
            }
            if let (true, Some(fields)) = (feature.final_body, row.as_object_mut()) {
                fields.insert("final_body".to_owned(), json!(true));
            }
            if let (Some(refusal), Some(fields)) = (&refusal, row.as_object_mut()) {
                fields.insert("source".into(), json!("solidcraft"));
                fields.insert("category".into(), json!(refusal.category));
                fields.insert("upstream".into(), json!(refusal.upstream));
                fields.insert("reason_raw".into(), json!(feature.reason));
            }
            row
        })
        .collect();
    let counted = |f: &&dutchman_core::FeatureOutcome| f.status != FeatureStatus::LeftOut;
    let mapped = outcome
        .features
        .iter()
        .filter(|f| matches!(f.status, FeatureStatus::Mapped | FeatureStatus::NotNeeded))
        .count();
    let total = outcome.features.iter().filter(counted).count();
    let rebuilt = outcome
        .features
        .iter()
        .filter(|f| f.status == FeatureStatus::Mapped && !f.reapplied)
        .count();
    let reapplied = outcome
        .features
        .iter()
        .filter(|f| f.status == FeatureStatus::Mapped && f.reapplied)
        .count();
    let needing = outcome
        .features
        .iter()
        .filter(counted)
        .filter(|f| f.status != FeatureStatus::NotNeeded)
        .count();
    let mut out = Map::new();
    out.insert(
        "mode".to_owned(),
        json!(match outcome.mode {
            Mode::PartialHistory => "partial_history",
            Mode::History => "history",
            Mode::Import => "import",
        }),
    );
    out.insert(
        "verification".to_owned(),
        json!({"status": status, "reason": reason}),
    );
    out.insert(
        "history".to_owned(),
        json!({"mapped": mapped, "total": total, "rebuilt": rebuilt, "needing": needing,
            "reapplied": reapplied, "features": features}),
    );
    out
}

/// Index a saved conversion's original losses by code and stage, preserving every field.
/// This local report keeps names and paths, unlike the anonymous diagnostic schemas.
pub(crate) fn with_loss_groups(mut report: Value) -> Value {
    let mut groups: BTreeMap<(Option<String>, Option<String>), Vec<Value>> = BTreeMap::new();
    for loss in report
        .get("losses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let field = |key| loss.get(key).and_then(Value::as_str).map(str::to_owned);
        groups
            .entry((field("code"), field("stage")))
            .or_default()
            .push(loss.clone());
    }
    let groups: Vec<Value> = groups.into_iter().map(|((code, stage), losses)| {
        json!({"code": code, "stage": stage, "count": losses.len(), "losses": losses})
    }).collect();
    if let Some(fields) = report.as_object_mut() {
        fields.insert("loss_groups".into(), json!(groups));
    }
    report
}

pub fn losses(report: &Report) -> Value {
    report
        .losses()
        .iter()
        .map(|loss| {
            json!({
                "level": match loss.severity {
                    Severity::Info => "Info",
                    Severity::Degraded => "Degraded",
                    Severity::Dropped => "Dropped",
                },
                "code": loss.code,
                "subject": loss.subject,
                "message": dutchman_core::clean_refusal(&loss.detail),
                "stage": loss.stage.id(),
                "upstream": loss.upstream(),
            })
        })
        .collect()
}

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
/// A progress event goes to stderr as one JSON line; stdout keeps its final report.
pub fn progress(input: &Path, event: dutchman_core::ProgressEvent) -> Value {
    json!({
        "schema": "dutchman.progress/1", "input": self::input(input),
        "stage": event.stage.id(), "elapsed_ms": event.elapsed_ms,
        "designs_read": event.designs_read, "designs_total": event.designs_total,
        "heartbeat": event.heartbeat,
        "running": event.running.as_ref().map(|s| json!({"kind": s.kind.id(), "name": s.name, "elapsed_ms": s.elapsed_ms, "usual_ms": s.usual_ms, "limit_ms": s.limit_ms, "consequence": s.consequence})),
    })
}

pub(crate) fn parse_progress(value: &Value) -> Option<dutchman_core::ProgressEvent> {
    if value.get("schema")?.as_str()? != "dutchman.progress/1" {
        return None;
    }
    Some(dutchman_core::ProgressEvent {
        stage: dutchman_core::Stage::ALL
            .into_iter()
            .find(|stage| Some(stage.id()) == value.get("stage").and_then(Value::as_str))?,
        elapsed_ms: value.get("elapsed_ms")?.as_u64()?,
        designs_read: value.get("designs_read")?.as_u64()?,
        designs_total: value.get("designs_total")?.as_u64(),
        heartbeat: value.get("heartbeat")?.as_bool()?,
        running: value
            .get("running")
            .filter(|v| v.is_object())
            .and_then(|v| {
                Some(dutchman_core::RunningStep {
                    kind: dutchman_core::StepKind::ALL
                        .into_iter()
                        .find(|k| Some(k.id()) == v.get("kind").and_then(Value::as_str))?,
                    name: v.get("name").and_then(Value::as_str).map(str::to_owned),
                    elapsed_ms: v.get("elapsed_ms")?.as_u64()?,
                    usual_ms: v.get("usual_ms")?.as_u64()?,
                    limit_ms: v.get("limit_ms").and_then(Value::as_u64),
                    consequence: v
                        .get("consequence")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            }),
    })
}

/// A DXF page's kept entity counts and mapping losses.
pub fn drawing_page(
    name: &str,
    path: &Path,
    entities: &BTreeMap<String, usize>,
    dimension_representation: &str,
    report: &Report,
) -> Value {
    json!({"name": name, "path": path.display().to_string(), "format": "dxf/AC1015", "units": "mm", "entities_per_layer": entities, "dimension_representation": dimension_representation, "losses": losses(report)})
}

/// The common conversion schema, with additive per-page drawing results.
pub fn drawing_success(input_path: &Path, pages: &[Value], report: &Report) -> Value {
    let mut out = success(CONVERT, input(input_path), report);
    out.insert("pages".into(), json!(pages));
    out.insert(
        "output".into(),
        json!({"path": pages.first().and_then(|p| p.get("path")), "format": "dxf/AC1015"}),
    );
    Value::Object(out)
}

/// A memory refusal, with a per-file result as well when a batch was stopped before writing.
pub fn memory_failure(
    schema: &str,
    path: &Path,
    options: &crate::ConvertOptions,
    message: &str,
) -> Value {
    let mut report = failure(CONVERT, input(path), message);
    conversion_choices(&mut report, path, options);
    if schema == crate::batch::SCHEMA {
        let single = report.clone();
        if let Some(fields) = report.as_object_mut() {
            fields.insert("schema".to_owned(), json!(schema));
            fields.insert("converted".to_owned(), json!(0));
            fields.insert("failed".to_owned(), json!(1));
            fields.insert("skipped".to_owned(), json!(0));
            fields.insert("skipped_by_extension".to_owned(), json!({}));
            fields.insert("skipped_revisions".to_owned(), json!([]));
            fields.insert("resumed".to_owned(), json!(0));
            fields.insert("results".to_owned(), json!([single]));
        }
    }
    report
}

/// SolidCraft target and discovered release, including when verification is off.
pub(crate) fn solidcraft_info(options: &crate::ConvertOptions) -> Option<Value> {
    if options.to != crate::OutputFormat::Solidcraft {
        return None;
    }
    let found = options
        .reported_solidcraft_version()
        .and_then(|line| solidcraft_release(&line));
    let searched = found.is_none() && options.searches_for_solidcraft();
    let mut values = solidcraft_values(options.solidcraft_version, found);
    if let (true, Some(fields)) = (searched, values.as_object_mut()) {
        fields.insert("search".to_owned(), options.solidcraft_search(true));
    }
    Some(values)
}

/// Normalize a discovered CLI version line for presentation.
pub fn solidcraft_release(line: &str) -> Option<String> {
    // Match the writer's accepted CLI version lines, including v prefixes and suffixes.
    let word = line.split_whitespace().last()?.trim_start_matches('v');
    let mut parts = word.split('.').map(|part| {
        part.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u64>()
            .ok()
    });
    let major = parts.next()??;
    let minor = parts.next()??;
    let patch = parts.next().flatten().unwrap_or(0);
    Some(format!("{major}.{minor}.{patch}"))
}

/// Build SolidCraft report values from a selected target and injected installed release.
pub fn solidcraft_values(
    target: Option<dutchman_solidcraft::SolidCraftVersion>,
    found: Option<String>,
) -> Value {
    let target = target
        .or_else(|| {
            found
                .as_deref()
                .and_then(|version| version.parse().ok())
                .map(|version: dutchman_solidcraft::SolidCraftVersion| {
                    version.min(Default::default())
                })
        })
        .unwrap_or_default();
    json!({"target": target.minor(), "found": found,
        "known_newest": dutchman_solidcraft::SolidCraftVersion::default().minor(),
        "status": dutchman_solidcraft::solidcraft_status(target, found.as_deref())})
}

/// The SolidCraft version line shared by single-file and batch text output. A file that
/// failed or was refused writes nothing, so it names no target.
pub fn solidcraft_line(report: &Value) -> Option<String> {
    if report.get("error").is_some_and(|error| !error.is_null()) {
        return None;
    }
    let target = report.pointer("/solidcraft/target")?.as_str()?;
    let found = report
        .pointer("/solidcraft/found")
        .and_then(Value::as_str)
        .map_or_else(
            || "none found".to_owned(),
            |version| format!("found {version}"),
        );
    Some(format!("Target SolidCraft {target} ({found})"))
}

/// SolidCraft's release download page, shared by desktop and CLI notices.
pub const SOLIDCRAFT_DOWNLOAD: &str = "https://github.com/tangency-labs/solidcraft/releases";

/// A non-blocking notice using the same report values in the CLI and desktop.
pub fn solidcraft_notice(report: &Value) -> Option<String> {
    solidcraft_line(report)?;
    let found =
        dutchman_solidcraft::solidcraft_minor(report.pointer("/solidcraft/found")?.as_str()?)?;
    let target = report.pointer("/solidcraft/target")?.as_str()?;
    match report.pointer("/solidcraft/status")?.as_str()? {
        "newer" => Some(format!(
            "SolidCraft {found} is newer than this Dutchman knows. Designs are written for {target} and open fine in {found}."
        )),
        "too_old" => Some(format!("SolidCraft {found} is too old to open Dutchman's designs. Install a newer SolidCraft to open them.")),
        "older_loses_history" => Some(format!("Designs are written for SolidCraft {target}, the version you have. A newer SolidCraft keeps more of their history.")),
        _ => None,
    }
}

/// Add the pre-read estimate and choices, including Standard's actual history decision.
pub fn conversion_choices(report: &mut Value, input: &Path, options: &crate::ConvertOptions) {
    if report.get("solidcraft").is_none() {
        if let (Some(solidcraft), Some(fields)) = (solidcraft_info(options), report.as_object_mut())
        {
            fields.insert("solidcraft".to_owned(), solidcraft);
        }
    }
    let mut choices = options.choices();
    if report
        .get("losses")
        .and_then(Value::as_array)
        .is_some_and(|losses| {
            losses.iter().any(|loss| {
                loss.get("subject").and_then(Value::as_str) == Some("history")
                    && loss
                        .get("message")
                        .and_then(Value::as_str)
                        .is_some_and(|detail| detail.starts_with("feature history left out: "))
            })
        })
    {
        choices.history = false;
    }
    if let Some(fields) = report.as_object_mut() {
        fields.insert(
            "estimate".to_owned(),
            estimate(crate::choices::estimate_with_options(
                input, &choices, options,
            )),
        );
        fields.insert("choices".to_owned(), placement_choices(options, choices));
    }
}

/// The shared estimate representation; null means it could not be pre-read.
pub fn estimate(estimate: Option<crate::choices::Estimate>) -> Value {
    estimate.map_or(Value::Null, |e| {
        json!({"peak_bytes": e.peak_bytes, "seconds": e.seconds, "seconds_exact": e.seconds_exact, "band": e.band,
            "memory_band": e.memory_band, "time_band": e.time_band})
    })
}

/// The selected placements alongside the effective retention choices.
pub fn placement_choices(
    options: &crate::ConvertOptions,
    choices: crate::choices::Choices,
) -> Value {
    let mut value = choice_values(options.preset, choices);
    if let Some(fields) = value.as_object_mut() {
        fields.insert(
            "part_positions".to_owned(),
            json!(options.effective_part_positions().name()),
        );
        fields.insert(
            "stored_placements".to_owned(),
            json!(options.effective_part_positions() == dutchman_core::PartPositions::FirstPlaced),
        );
    }
    value
}

/// The selected preset and its effective narrowed choices.
pub fn choice_values(preset: crate::choices::Preset, choices: crate::choices::Choices) -> Value {
    json!({"preset": preset.name(), "history": choices.history, "sketches": choices.sketches, "joints": choices.joints})
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    #[test]
    fn placement_counts_accumulate_and_keep_fallbacks_separate() {
        use dutchman_core::{PartPositionCounts, PartPositions};
        let mut report = Report::default();
        report.record_part_positions(
            PartPositions::Smart,
            PartPositionCounts {
                current: 2,
                first_placed: 3,
                joints: 1,
                bounds: 2,
                ..Default::default()
            },
        );
        let mut nested = Report::default();
        nested.record_part_positions(
            PartPositions::Smart,
            PartPositionCounts {
                displayed: 3,
                derived_holders: 2,
                holder_conflicts: 1,
                first_placed: 2,
                ambiguous: 2,
                ..Default::default()
            },
        );
        report.append(nested);
        let out = success("dutchman.convert/1", Value::Null, &report);
        let counts = &out["part_positions"];
        assert_eq!(counts["mode"], "smart");
        assert_eq!(counts["total"], 10);
        assert_eq!(counts["displayed"], 3);
        assert_eq!(counts["bound_displayed"], 1);
        assert_eq!(counts["derived_holders"], 2);
        assert_eq!(counts["holder_conflicts"], 1);
        assert_eq!(counts["current"], 2);
        assert_eq!(counts["first_placed"], 5);
        assert_eq!(counts["fallbacks"]["joints"], 1);
        assert_eq!(counts["fallbacks"]["bounds"], 2);
        assert_eq!(counts["fallbacks"]["ambiguous"], 2);
    }

    use dutchman_core::FeatureOutcome;

    use super::*;

    #[test]
    fn installed_version_notices_are_pure_and_non_blocking() {
        for (found, status, notice) in [
            (Some("0.42.0"), "newer", Some("SolidCraft 0.42 is newer than this Dutchman knows. Designs are written for 0.41 and open fine in 0.42.")),
            (Some("0.17.9"), "too_old", Some("SolidCraft 0.17 is too old to open Dutchman's designs. Install a newer SolidCraft to open them.")),
            (Some("0.18.1"), "current", None),
            (Some("0.41.9"), "current", None),
            (Some("0.24.0"), "older_loses_history", Some("Designs are written for SolidCraft 0.24, the version you have. A newer SolidCraft keeps more of their history.")),
            (Some("0.25.1"), "current", None),
            (Some("0.26.0"), "current", None),
            (Some("0.27.1"), "current", None),
            (Some("0.41.0"), "current", None),
            (Some("0.18.2"), "older_loses_history", Some("Designs are written for SolidCraft 0.18, the version you have. A newer SolidCraft keeps more of their history.")),
            (None, "none_found", None),
        ] {
            let mut report = json!({"solidcraft": solidcraft_values(None, found.map(str::to_owned))});
            assert_eq!(report["solidcraft"]["status"], status);
            assert_eq!(report["solidcraft"]["known_newest"], "0.41");
            assert_eq!(solidcraft_notice(&report).as_deref(), notice);
            report["error"] = json!({"message": "conversion failed"});
            assert!(solidcraft_notice(&report).is_none());
        }
        let report = json!({"solidcraft": solidcraft_values(Some(Default::default()), Some("0.24.0".into()))});
        assert_eq!(report["solidcraft"]["status"], "current");
        assert!(solidcraft_notice(&report).is_none());
    }

    #[test]
    fn cli_version_lines_are_normalized_like_the_writer() {
        for (line, version) in [
            ("solidcraft-cli 0.41.0", Some("0.41.0")),
            ("solidcraft-cli v0.26.0-beta", Some("0.26.0")),
            ("solidcraft-cli 0.25", Some("0.25.0")),
            ("not a version", None),
        ] {
            assert_eq!(solidcraft_release(line).as_deref(), version);
        }
    }

    #[test]
    fn explicit_target_is_reported_with_verification_off_and_no_cli() {
        let dir = tempfile::tempdir().unwrap();
        let options = crate::ConvertOptions {
            no_verify: true,
            solidcraft: Some(dir.path().join("missing-solidcraft-cli")),
            solidcraft_version: Some("0.25.1".parse().unwrap()),
            ..Default::default()
        };
        let mut report = json!({});
        conversion_choices(&mut report, Path::new("missing.ipt"), &options);
        assert_eq!(
            report["solidcraft"],
            json!({"target": "0.25", "found": null, "known_newest": "0.41", "status": "none_found"})
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicit_unverified_target_does_not_spawn_for_writer_or_reports() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("solidcraft-cli");
        std::fs::write(
            &cli,
            "#!/bin/sh\ntouch \"$0.spawned\"\necho 'solidcraft-cli 0.26.0'\n",
        )
        .unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        let target = "0.25.1".parse().unwrap();
        let options = crate::ConvertOptions {
            no_verify: true,
            solidcraft: Some(cli.clone()),
            solidcraft_version: Some(target),
            ..Default::default()
        };
        let writer = dutchman_solidcraft::SolidCraftWriter::new()
            .verification(options.verification())
            .target_version(options.solidcraft_version);
        assert_eq!(writer.resolved_version(), target);
        assert_eq!(
            solidcraft_info(&options).unwrap(),
            json!({"target": "0.25", "found": null, "known_newest": "0.41", "status": "none_found"})
        );
        assert!(options.reported_solidcraft_version().is_none());
        assert!(!dir.path().join("solidcraft-cli.spawned").exists());
    }

    #[test]
    fn solidcraft_versions_report_target_found_and_absence() {
        let target = "0.41.0".parse().unwrap();
        for (found, line) in [
            (
                Some("0.26.0".to_owned()),
                "Target SolidCraft 0.41 (found 0.26.0)",
            ),
            (
                Some("0.41.1".to_owned()),
                "Target SolidCraft 0.41 (found 0.41.1)",
            ),
            (None, "Target SolidCraft 0.41 (none found)"),
        ] {
            let report = json!({"solidcraft": solidcraft_values(Some(target), found.clone())});
            assert_eq!(report["solidcraft"]["target"], "0.41");
            assert_eq!(report["solidcraft"]["found"], json!(found));
            assert_eq!(solidcraft_line(&report).as_deref(), Some(line));
            let mut refused = report.clone();
            refused["error"] = json!({"message": "open it in SolidCraft directly"});
            assert!(solidcraft_line(&refused).is_none());
        }
        assert_eq!(
            solidcraft_values(None, Some("0.26.0".into()))["target"],
            "0.26"
        );
        assert_eq!(solidcraft_values(None, None)["target"], "0.41");
        for to in [
            crate::OutputFormat::Freecad,
            crate::OutputFormat::Fcstd,
            crate::OutputFormat::Dxf,
        ] {
            let mut report = json!({});
            conversion_choices(
                &mut report,
                Path::new("missing.ipt"),
                &crate::ConvertOptions {
                    to,
                    ..Default::default()
                },
            );
            assert!(report.get("solidcraft").is_none());
            assert!(solidcraft_line(&report).is_none());
        }
    }

    #[test]
    fn separate_estimate_bands_keep_legacy_coverage_and_unknown_time() {
        let value = estimate(Some(crate::choices::Estimate {
            peak_bytes: 33_000_000,
            seconds: 1,
            seconds_exact: 0.17,
            band: 0.6,
            memory_band: 0.55,
            time_band: None,
        }));
        assert_eq!(value["seconds"], json!(1));
        assert_eq!(value["seconds_exact"], json!(0.17));
        assert_eq!(value["band"], json!(0.6));
        assert_eq!(value["memory_band"], json!(0.55));
        assert!(value["time_band"].is_null());
        let timed = estimate(Some(crate::choices::Estimate {
            peak_bytes: 4_000_000_000,
            seconds: 34,
            seconds_exact: 34.0,
            band: 0.25,
            memory_band: 0.25,
            time_band: Some(0.2),
        }));
        assert_eq!(timed["time_band"], json!(0.2));
        assert!(estimate(None).is_null());
    }

    #[test]
    fn skipped_history_check_survives_nested_reports_without_changing_the_schema() {
        let mut member = Report::default();
        member.skip_history_check(9_450_000_000, 2_000_000_000);
        let mut nested = Report::default();
        nested.append_prefixed(member, "synthetic member");
        let mut report = Report::default();
        report.append(nested);
        let fields = success(CONVERT, serde_json::json!({}), &report);
        assert_eq!(fields["schema"], CONVERT);
        assert_eq!(fields["ok"], true);
        assert_eq!(
            fields["history_check_skipped"]["needed_bytes"],
            9_450_000_000u64
        );
        assert_eq!(
            fields["history_check_skipped"]["available_bytes"],
            2_000_000_000u64
        );
    }

    #[test]
    fn partial_history_is_distinct_from_full_verification() {
        let fields = outcome_fields(&Outcome {
            mode: Mode::PartialHistory,
            check: Check::Partial("2 of 3 features kept; retained features replay".to_owned()),
            features: Vec::new(),
        });
        assert_eq!(fields.get("mode"), Some(&json!("partial_history")));
        assert_eq!(
            fields.get("verification").and_then(|v| v.get("status")),
            Some(&json!("partial"))
        );
    }

    #[test]
    fn a_solidcraft_refusal_is_classified_and_cleaned_in_the_summary() {
        let raw = "feature \"Fillet3\" failed: not supported yet: the blend is larger than the \
                   neighbouring faces (site 387)";
        let fields = outcome_fields(&Outcome {
            mode: Mode::PartialHistory,
            check: Check::Partial(format!("history stopped at {raw} (2 of 3 features kept)")),
            features: vec![FeatureOutcome {
                name: Some("Fillet3".to_owned()),
                kind: "fillet".to_owned(),
                status: FeatureStatus::Blocked,
                reason: Some(raw.to_owned()),
                ordinal: Some(3),
                variant: Some("fillet, 1 radius group".to_owned()),
                profile_unverified: false,
                reapplied: false,
                final_body: false,
            }],
        });
        let fields = Value::Object(fields);
        let clean = "SolidCraft refused feature \"Fillet3\": the blend is larger than the neighbouring faces";
        assert_eq!(
            fields.pointer("/verification/reason"),
            Some(&json!(format!(
                "history stopped at {clean} (2 of 3 features kept)"
            )))
        );
        let row = fields.pointer("/history/features/0").unwrap();
        assert_eq!(row["reason"], json!(clean));
        assert_eq!(row["reason_raw"], json!(raw));
        assert_eq!(row["source"], "solidcraft");
        assert_eq!(row["category"], "fillet-overflows-neighbouring-face");
        assert_eq!(
            row["upstream"],
            "https://github.com/tangency-labs/solidcraft/issues/4782"
        );
    }

    #[test]
    fn the_invitation_names_programs_once_when_conversion_fails_or_history_does_not_map() {
        let below = json!({"ok": true, "input": {"program": "Fusion", "path": "secret.f3d"},
            "verification": {"status": "flattened"}, "history": {"mapped": 2, "total": 2}});
        let complete = json!({"ok": true, "input": {"program": "STEP"},
            "verification": {"status": "not_applicable"}, "history": {"mapped": 0, "total": 0}});
        assert!(fixtures_pitch([&complete]).is_none());
        let pitch = fixtures_pitch([&below, &below, &complete]).unwrap();
        assert_eq!(pitch.matches("Want to help").count(), 1);
        assert!(pitch.starts_with("Want to help Dutchman read Fusion better? If you have Fusion,"));
        assert!(
            !pitch.contains("secret")
                && !pitch.contains("read STEP")
                && !pitch.contains("have STEP")
        );
        assert!(pitch.ends_with("Got a file that converts wrong? Drop it, with its STEP export, in a DM to Dutchy in our Discord."));
        let refused = json!({"ok": false, "input": {"program": "STEP"}});
        assert!(fixtures_pitch([&refused])
            .unwrap()
            .contains("read STEP better?"));
        let full = json!({"ok": true, "input": {"program": "Example"},
            "verification": {"status": "unverified"}, "history": {"mapped": 2, "total": 2}});
        assert!(fixtures_pitch([&full]).is_none());
        let incomplete = json!({"ok": true, "input": {"program": "Inventor"},
            "verification": {"status": "not_applicable"}, "history": {"mapped": 0, "total": 2}});
        assert!(fixtures_pitch([&incomplete])
            .unwrap()
            .contains("read Inventor better?"));
    }

    #[test]
    fn iges_refusals_do_not_invite_test_files_in_single_or_mixed_runs() {
        let iges = error(
            CONVERT,
            input(Path::new("design.igs")),
            &Error::NotSupportedYet {
                format: Format::Iges,
            },
        );
        let creo = error(
            CONVERT,
            input(Path::new("design.prt")),
            &Error::NotSupportedYet {
                format: Format::CreoPart,
            },
        );
        assert!(fixtures_pitch([&iges]).is_none());
        assert_eq!(fixtures_pitch([&iges, &creo]), fixtures_pitch([&creo]));
    }

    #[test]
    fn losses_are_grouped_by_the_part_they_belong_to() {
        let report = json!({"losses": [
            {"level": "Info", "subject": "history", "message": "flattened: the design carries no feature history"},
            {"level": "Dropped", "subject": "occurrence \"Pin:1\"", "message": "pin.ipt: not found"},
            {"level": "Info", "subject": "history", "message": "base.ipt: flattened: feature \"Fillet1\": fillet is not mapped yet"},
            {"level": "Degraded", "subject": "base.ipt: f3d/material.not-transferred", "message": "Materials were not transferred."},
            {"level": "Degraded", "subject": "sub.iam: arm.ipt: validation", "message": "a check"},
        ]});
        let components = components(&report);
        let names: Vec<Option<&str>> = components.iter().map(|c| c.file.as_deref()).collect();
        // The design first, then the parts with the most that matters: the part not found
        // carries the placement it cost.
        assert_eq!(
            names,
            [None, Some("pin.ipt"), Some("arm.ipt"), Some("base.ipt")]
        );
        assert_eq!(components[1].dropped, 1);
        assert_eq!(
            components[3].summary(),
            "history flattened (feature \"Fillet1\": fillet is not mapped yet); 1 Degraded, 1 Info"
        );
        assert_eq!(
            grouped_lines(&report),
            [
                "losses: 5 in 4 components",
                "  the design: history flattened (the design carries no feature history); 1 Info",
                "  pin.ipt: 1 Dropped",
                "  arm.ipt: 1 Degraded",
                "  base.ipt: history flattened (feature \"Fillet1\": fillet is not mapped yet); 1 Degraded, 1 Info",
            ]
        );
        let log = full_log(&report, "converted  top.iam -> top.solidcraft");
        assert!(
            log.starts_with("converted  top.iam -> top.solidcraft\n"),
            "{log}"
        );
        assert!(log.contains("\nbase.ipt: history flattened"), "{log}");
        assert!(log.contains("  Degraded: base.ipt: f3d/material.not-transferred: Materials were not transferred.\n"), "{log}");
    }

    #[test]
    fn an_assembly_says_how_much_of_it_was_placed() {
        let completeness = Completeness {
            placed: 18,
            placed_empty: 0,
            left_out: vec![dutchman_core::LeftOutGroup {
                file: Some("pin.ipt".to_owned()),
                reason: "pin.ipt: not found at its stored path".to_owned(),
                stage: Stage::Input,
                count: 2,
            }],
        };
        let report = json!({"completeness": completeness_json(&completeness)});
        assert_eq!(report["completeness"]["placements"], 20);
        assert_eq!(report["completeness"]["left_out"][0]["stage"], "input");
        assert_eq!(
            completeness_lines(&report),
            [
                "assembly: 18 of 20 placements placed; 2 left out:",
                "  pin.ipt ×2: not found at its stored path",
            ]
        );
        let whole = Completeness {
            placed: 3,
            placed_empty: 0,
            left_out: Vec::new(),
        };
        assert_eq!(
            completeness_lines(&json!({"completeness": completeness_json(&whole)})),
            ["assembly: all 3 placements placed"]
        );
        assert!(completeness_lines(&json!({})).is_empty());
    }

    #[test]
    fn empty_placements_are_separate_from_geometry_and_left_out() {
        let counts = Completeness {
            placed: 67,
            placed_empty: 13,
            left_out: vec![dutchman_core::LeftOutGroup {
                file: Some("pin.ipt".into()),
                reason: "not found".into(),
                stage: Stage::Input,
                count: 8,
            }],
        };
        let report = json!({"ok": true, "completeness": completeness_json(&counts)});
        assert_eq!(report["completeness"]["placements"], 88);
        assert_eq!(report["completeness"]["placed_empty"], 13);
        assert_eq!(counts.left_out_count(), 8);
        assert_eq!(
            completeness_lines(&report)[0],
            "assembly: 67 of 88 placements placed; 13 placed empty; 8 left out:"
        );
        assert_eq!(
            assembly_line(&report).unwrap(),
            "67 of 88 components placed; 13 placed empty, no joints"
        );
        let empty_only = json!({"completeness": {"placed": 0, "placed_empty": 2, "placements": 2, "left_out": []}});
        assert_eq!(
            completeness_lines(&empty_only),
            ["assembly: 0 of 2 placements placed; 2 placed empty"]
        );
    }

    fn feature(kind: &str, status: FeatureStatus) -> FeatureOutcome {
        FeatureOutcome {
            name: Some(format!("{kind}1")),
            kind: kind.to_owned(),
            status,
            reason: None,
            ordinal: None,
            variant: None,
            profile_unverified: false,
            reapplied: false,
            final_body: false,
        }
    }

    #[test]
    fn a_note_is_cut_at_its_first_sentence() {
        let long = "the document carries no declaration, so nothing was verified. The document \
                    is read on the residual path. Agreement is consistency.";
        assert_eq!(
            first_sentence(long),
            (
                "the document carries no declaration, so nothing was verified. …".to_owned(),
                true
            )
        );
        let short = "1 record(s) were not written to STEP: configuration=1";
        assert_eq!(first_sentence(short), (short.to_owned(), false));
        let abbreviated = "a frame, e.g. the origin. Then more";
        assert_eq!(first_sentence(abbreviated).0, "a frame, e.g. the origin. …");
    }

    #[test]
    fn stored_geometry_reports_an_import_and_an_info_message() {
        let message =
            "This part holds an imported solid; there is no feature history to bring over";
        let mut report = Report::default();
        report.push(Severity::Info, "feature \"Stored solid\"", message);
        report.set_outcome(Outcome {
            mode: Mode::Import,
            check: Check::NotApplicable(message.to_owned()),
            features: vec![feature("stored_geometry", FeatureStatus::Mapped)],
        });
        let out = Value::Object(success(CONVERT, json!({}), &report));
        assert_eq!(out["mode"], "import");
        assert_eq!(
            out["verification"],
            json!({"status": "not_applicable", "reason": message})
        );
        assert_eq!(out["history"]["mapped"], 1);
        assert_eq!(out["history"]["features"][0]["status"], "mapped");
        assert_eq!(out["losses"][0]["level"], "Info");
        assert_eq!(out["losses"][0]["message"], message);
        assert_eq!(crate::present::outcome(&out), "Geometry without history.");
    }

    #[test]
    fn an_assembly_reports_each_parts_history() {
        let mut report = Report::default();
        report.set_outcome(Outcome {
            mode: Mode::Import,
            check: Check::NotApplicable("no part's history was written".to_owned()),
            features: vec![feature("extrude", FeatureStatus::Blocked)],
        });
        report.push_part(dutchman_core::PartHistory {
            name: "Bracket".to_owned(),
            check: Check::Flattened("feature \"Extrude1\": not mapped yet".to_owned()),
            mapped: 1,
            total: 2,
            rebuilt: 1,
            needing: 2,
            reapplied: 0,
        });
        report.push_part(dutchman_core::PartHistory {
            name: "Rail".to_owned(),
            check: Check::NotApplicable("the design carries no feature history".to_owned()),
            mapped: 0,
            total: 0,
            rebuilt: 0,
            needing: 0,
            reapplied: 0,
        });
        let out = Value::Object(success(CONVERT, json!({}), &report));
        assert_eq!(
            out["history"]["parts"],
            json!([
                {"name": "Bracket", "status": "flattened", "reason": "feature \"Extrude1\": not mapped yet", "mapped": 1, "total": 2, "rebuilt": 1, "needing": 2, "reapplied": 0},
                {"name": "Rail", "status": "not_applicable", "reason": "the design carries no feature history", "mapped": 0, "total": 0, "rebuilt": 0, "needing": 0, "reapplied": 0},
            ])
        );
    }

    #[test]
    fn a_single_part_reports_no_parts() {
        let mut report = Report::default();
        report.set_outcome(Outcome {
            mode: Mode::History,
            check: Check::Verified,
            features: vec![feature("extrude", FeatureStatus::Mapped)],
        });
        let out = Value::Object(success(CONVERT, json!({}), &report));
        assert!(out["history"].get("parts").is_none());
    }

    #[test]
    fn a_written_history_reports_its_features_and_check() {
        let mut report = Report::default();
        report.set_outcome(Outcome {
            mode: Mode::History,
            check: Check::Verified,
            features: vec![
                feature("datum_principal_plane", FeatureStatus::NotNeeded),
                feature("sketch", FeatureStatus::Mapped),
                feature("extrude", FeatureStatus::Mapped),
                feature("fillet", FeatureStatus::LeftOut),
            ],
        });
        let out = Value::Object(success(CONVERT, json!({}), &report));
        assert_eq!(out["mode"], "history");
        assert_eq!(
            out["verification"],
            json!({"status": "verified", "reason": null})
        );
        assert_eq!(out["history"]["mapped"], 3);
        assert_eq!(out["history"]["total"], 3);
        assert_eq!(out["history"]["rebuilt"], 2);
        assert_eq!(out["history"]["needing"], 2);
        assert_eq!(out["history"]["features"][3]["status"], "left_out");
        assert_eq!(out["losses"], json!([]));
    }

    #[test]
    fn a_freecad_difference_uses_the_shared_verification_report() {
        let mut report = Report::default();
        report.set_outcome(Outcome {
            mode: Mode::History,
            check: Check::Differs("part 2 (Bracket): bounding box differs".to_owned()),
            features: Vec::new(),
        });
        let out = Value::Object(success(CONVERT, json!({}), &report));
        assert_eq!(
            out["verification"],
            json!({"status": "differs", "reason": "part 2 (Bracket): bounding box differs"})
        );
    }
    #[test]
    fn an_assembly_line_counts_components_and_joints() {
        let whole = json!({"completeness": {"placed": 41, "placements": 41, "left_out": []},
            "joints": {"revolute": 5, "rigid": 1}});
        assert_eq!(
            assembly_line(&whole).unwrap(),
            "41 components, 6 joints (5 revolute, 1 rigid)"
        );
        let short = json!({"completeness": {"placed": 1, "placements": 2, "left_out": [{}]}});
        assert_eq!(
            assembly_line(&short).unwrap(),
            "1 of 2 components placed, no joints"
        );
        let one = json!({"completeness": {"placed": 1, "placements": 1},
            "joints": {"slider": 1}});
        assert_eq!(
            assembly_line(&one).unwrap(),
            "1 component, 1 joint (1 slider)"
        );
        assert_eq!(assembly_line(&json!({"losses": []})), None);
    }
}

#[cfg(test)]
mod compact_loss_tests {
    use super::*;

    #[test]
    fn face_losses_group_by_reason_and_keep_full_verbose_details() {
        let losses = ["face#1", "face#2", "face#3"].map(|id| json!({
            "level": "Dropped", "subject": "part.f3d: appearance",
            "message": format!("Appearance binding {id} is left out: its target has no written body name or resolved planar interior point")
        }));
        let compact = loss_lines(&losses, false);
        assert_eq!(compact.len(), 1);
        assert!(compact[0].0.contains("3 appearance bindings left out"));
        assert!(compact[0].1);
        let verbose = loss_lines(&losses, true);
        assert_eq!(verbose.len(), 3);
        for (line, id) in verbose.iter().zip(["face#1", "face#2", "face#3"]) {
            assert!(line.0.contains(id));
        }
        let mut different = losses.to_vec();
        different.push(json!({"level":"Dropped", "subject":"part.f3d: appearance", "message":"Appearance binding face#4 is left out: its component has no written geometry"}));
        assert_eq!(loss_lines(&different, false).len(), 2);
        assert_eq!(
            loss_lines(&losses[..1], false)[0],
            loss_line(&losses[0], false)
        );
        let repeated = json!({"level":"Info", "subject":"history", "message":"No feature history was decoded."});
        let mut notes = vec![repeated.clone(), repeated.clone()];
        assert_eq!(loss_lines(&notes, false).len(), 1);
        assert!(loss_lines(&notes, false)[0].0.contains("2 notes:"));
        notes.push(json!({"level":"Dropped", "subject":"history", "message":"No feature history was decoded."}));
        notes.push(json!({"level":"Info", "subject":"other.f3d: history", "message":"No feature history was decoded."}));
        assert_eq!(loss_lines(&notes, false).len(), 3);
        assert_eq!(notes[0], repeated);
    }
}

#[cfg(test)]
mod running_progress_tests {
    use super::*;
    #[test]
    fn running_progress_roundtrips_and_accepts_older_workers() {
        let saved = std::sync::Mutex::new(None);
        let callback = |event| *saved.lock().unwrap() = Some(event);
        let reporter = dutchman_core::Progress::new(&callback);
        let _step = reporter.step(
            dutchman_core::StepKind::Checking,
            Some("Bracket"),
            Some(std::time::Duration::from_secs(120)),
            Some(dutchman_core::progress_words::geometry_limit()),
        );
        let event = saved.lock().unwrap().clone().unwrap();
        let mut record = progress(Path::new("assembly.iam"), event.clone());
        assert_eq!(record["schema"], "dutchman.progress/1");
        assert_eq!(parse_progress(&record), Some(event));
        record.as_object_mut().unwrap().remove("running");
        assert!(parse_progress(&record).unwrap().running.is_none());
    }
}
