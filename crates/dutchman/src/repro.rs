//! A local reproduction kit. Shareable data is constructed from fixed categories and
//! counts; private models, source strings, and the local key never enter that object.
use std::path::Path;
use std::sync::{Arc, Mutex};

use dutchman_core::{
    parts, Error, Format, MemberEvent, MemberObserver, Model, Progress, Report, Writer,
};
use serde_json::{json, Value};

use crate::diagnose::signature::Signature;
use crate::diagnose::{self, FailingPart};
use crate::{diagnostics, ConvertOptions};

pub const SCHEMA: &str = crate::json::REPRO;
const MAX_TRIALS: usize = 256;

struct Capture {
    diagnostic: Arc<dyn MemberObserver>,
    model: Mutex<Option<Model>>,
}
impl MemberObserver for Capture {
    fn model(&self, model: &Model) {
        if let Ok(mut captured) = self.model.lock() {
            *captured = Some(model.clone());
        }
    }
    fn phase_event(&self, phase: &'static str, event: MemberEvent, ms: u64, bytes: u64) {
        self.diagnostic.phase_event(phase, event, ms, bytes);
    }
    fn member_event(&self, path: &str, event: MemberEvent, ms: u64, code: Option<&'static str>) {
        self.diagnostic.member_event(path, event, ms, code);
    }
}

/// Run a normal default SolidCraft conversion with diagnostics, then re-run only the
/// failing stage on local subsets. An existing kit is refused; originals are never changed.
/// `include_local_key` is for a client operating locally; its result must stay local too.
pub fn run(input: &Path, out: &Path, include_local_key: bool) -> Result<Value, String> {
    run_from(input, out, include_local_key, None)
}

/// [`run`], going straight to the failing part of a `dutchman diagnose` report: the one in
/// `from` (the folder diagnose wrote to) when given, else the one in `out`, when it holds one.
pub fn run_from(
    input: &Path,
    out: &Path,
    include_local_key: bool,
    from: Option<&Path>,
) -> Result<Value, String> {
    let failing = match from {
        Some(folder) => Some(diagnose::report_from(folder)?),
        None => diagnose::report_from(out).ok(),
    }
    .as_ref()
    .and_then(diagnose::failing_part);
    std::fs::create_dir_all(out).map_err(|e| format!("cannot create reproduction folder: {e}"))?;
    let kit = out.join("kit");
    std::fs::create_dir(&kit)
        .map_err(|e| format!("cannot create kit; choose a fresh --out folder: {e}"))?;
    let result = match &failing {
        Some(part) => build_part(input, &kit, include_local_key, part),
        None => build(input, out, &kit, include_local_key),
    };
    if result.is_err() {
        // Only our newly created kit is removed, never a pre-existing folder.
        let _ = std::fs::remove_dir_all(&kit);
    }
    result
}

fn build(input: &Path, out: &Path, kit: &Path, include_local_key: bool) -> Result<Value, String> {
    let scratch = tempfile::Builder::new()
        .prefix(".repro-")
        .tempdir_in(out)
        .map_err(|e| format!("cannot create local working folder: {e}"))?;
    let options = ConvertOptions::default();
    let mut log =
        diagnostics::Log::open(&scratch.path().join("diagnostics.jsonl"), input, &options)?;
    let capture = Arc::new(Capture {
        diagnostic: log.observer(),
        model: Mutex::new(None),
    });
    let callback = |event| log.progress(event);
    let progress = Progress::new(&callback).with_members(Some(capture.clone()));
    let writer = options.preflight(input, crate::choices::available_memory());
    let conversion = dutchman_core::convert_with_progress(
        input,
        &scratch
            .path()
            .join(format!("converted.{}", writer.extension())),
        &writer,
        false,
        &options.part_search(),
        &progress,
    )
    .map(|report| crate::convert_success(input, Path::new("converted.sc"), &report))
    .unwrap_or_else(|e| crate::json::error(crate::json::CONVERT, Value::Null, &e));
    drop(progress);
    log.end(&conversion)?;
    let failure = diagnostics::anonymous_failure(&conversion);
    let model = capture
        .model
        .lock()
        .map_err(|_| "local model capture stopped")?
        .take();
    let source_document = model.as_ref().and_then(|m| m.source_file.clone());
    let mut narrowed = Narrowed::default();
    let mut model = model;
    let mut trials = 0;
    let format = Format::from_path(input);
    if !failure.is_null() && format == Some(Format::FusionMultiDocument) {
        let mut visit = |ordinal, total, name: &str, candidate: Result<Model, Error>| {
            trials += 1;
            narrowed
                .member_names
                .push(json!({"ordinal": ordinal, "total": total, "name": name}));
            let (matches, status, trial_failure) = match candidate {
                Ok(candidate) => {
                    let trial = test_model(&candidate, &failure, &writer);
                    let matches = same(&failure, &trial);
                    if matches {
                        model = Some(candidate);
                    }
                    (matches, "decoded", trial)
                }
                Err(error) => (
                    false,
                    "decode_failed",
                    diagnostics::anonymous_failure(&crate::json::error(
                        crate::json::CONVERT,
                        Value::Null,
                        &error,
                    )),
                ),
            };
            narrowed.members.push(json!({"ordinal": ordinal, "total": total, "status": status, "trial_failure": trial_failure}));
            if matches {
                narrowed.select("member", ordinal, total, Some(name), None);
            }
            matches || trials >= MAX_TRIALS
        };
        if dutchman_core::repro::members(input, &mut visit).is_err() {
            narrowed.limit(
                "Archive members could not be tested independently with the bounded member API.",
            );
        }
        if !narrowed.path.iter().any(|p| p["unit"] == "member") {
            narrowed.limit("No independent archive member reproduced the same structured check codes; composition or decoding may require several members.");
        }
    } else {
        narrowed.limit("Archive member narrowing applies only to multi-document Fusion archives.");
    }
    if !failure.is_null() {
        if let Some(retained) = model {
            narrow(retained, &failure, &writer, &mut narrowed, &mut trials)?;
        } else {
            narrowed.limit("No decoded model is available at this failure stage; component, body, feature and sketch checks are not possible.");
        }
    }
    let result = finish_kit(
        input,
        kit,
        include_local_key,
        &failure,
        &narrowed,
        source_document,
    )?;
    drop(capture);
    drop(log);
    scratch
        .close()
        .map_err(|e| format!("cannot remove local working files: {e}"))?;
    Ok(result)
}

/// The kit for the failing part a `dutchman diagnose` report named: that part is read, converted
/// alone to see the failure, and narrowed from there to a body, feature or sketch.
fn build_part(
    input: &Path,
    kit: &Path,
    include_local_key: bool,
    part: &FailingPart,
) -> Result<Value, String> {
    let options = ConvertOptions {
        no_verify: true,
        ..ConvertOptions::default()
    };
    let format = Format::from_path(input).ok_or("this file is not a format Dutchman reads")?;
    let model = dutchman_core::read_in(
        format,
        input,
        &options.part_search(),
        &mut Report::default(),
    )
    .map_err(|e| format!("cannot read the design: {e}"))?;
    let catalog = parts::catalog(&model);
    let info = catalog
        .iter()
        .find(|p| p.number == part.number)
        .ok_or_else(|| {
            format!(
                "the diagnose report names part {} but this design has {} parts; was it made from another file?",
                part.number,
                catalog.len()
            )
        })?;
    let selected = parts::select(&model, part.number, false).map_err(|e| e.to_string())?;
    drop(model);
    let writer = options.writer();
    let mut narrowed = Narrowed::default();
    narrowed.select("part", part.number, catalog.len(), Some(&info.name), None);
    let mut trials = 0;
    let failure = if part.outcome == "failed" {
        let converted = writer.write(&selected, &mut std::io::sink(), &mut Report::default());
        match converted {
            Ok(()) => {
                narrowed.limit("The failing part converted on its own here; it may need the rest of the design, or the limits that stopped it in the diagnose run.");
                Value::Null
            }
            Err(error) => diagnostics::anonymous_failure(&crate::json::error(
                crate::json::CONVERT,
                Value::Null,
                &error,
            )),
        }
    } else {
        narrowed.limit("The part did not finish within the diagnose limits, so smaller units cannot be tried automatically; rebuild the part's most complex features first.");
        json!({"stage": "map", "kind": part.outcome, "findings": {}, "curve_kinds": {}})
    };
    if part.outcome == "failed" && !failure.is_null() {
        narrow(selected, &failure, &writer, &mut narrowed, &mut trials)?;
    }
    finish_kit(input, kit, include_local_key, &failure, &narrowed, None)
}

/// What `dutchman diagnose --narrow` found, for its kit.
pub(crate) struct NarrowedFailure<'a> {
    /// The design.
    pub(crate) input: &'a Path,
    /// The failure, as the anonymous facts of [`diagnostics::anonymous_failure`].
    pub(crate) failure: &'a Value,
    /// The part: its number, how many there are, and its private name.
    pub(crate) part: Option<(usize, usize, String)>,
    /// The first failing feature: its number, how many there are, its private name, its kind.
    pub(crate) feature: Option<(usize, usize, Option<String>, String)>,
    /// The failure's signature.
    pub(crate) signature: &'a crate::diagnose::signature::Signature,
    /// What the narrowing could not do.
    pub(crate) limits: Vec<&'static str>,
}

/// Writes the kit for what `dutchman diagnose --narrow` found, into the new folder `kit`.
pub(crate) fn narrowed_kit(found: &NarrowedFailure<'_>, kit: &Path) -> Result<Value, String> {
    std::fs::create_dir(kit).map_err(|e| format!("cannot create the kit: {e}"))?;
    let mut narrowed = Narrowed {
        signature: serde_json::to_value(found.signature).map_err(|e| e.to_string())?,
        ..Narrowed::default()
    };
    if let Some((number, total, name)) = &found.part {
        narrowed.select("part", *number, *total, Some(name), None);
    }
    if let Some((number, total, name, kind)) = &found.feature {
        narrowed.select_kind(
            "feature",
            *number,
            *total,
            name.as_deref(),
            narrowed.path.len().checked_sub(1),
            Some(kind),
        );
        narrowed.isolated = true;
    }
    for limit in &found.limits {
        narrowed.limit(limit);
    }
    finish_kit(found.input, kit, false, found.failure, &narrowed, None)
}

/// Writes the kit's files and builds the result.
fn finish_kit(
    input: &Path,
    kit: &Path,
    include_local_key: bool,
    failure: &Value,
    narrowed: &Narrowed,
    source_document: Option<String>,
) -> Result<Value, String> {
    let anonymous = anonymous(input, failure, narrowed);
    let local = json!({"schema": "dutchman.repro-local/1", "warning": "Do not share",
        "input": input, "source_document": source_document, "members": narrowed.member_names, "ordinals": narrowed.key});
    write_json(&kit.join("anonymous.json"), &anonymous, false)?;
    write_json(&kit.join("local-key.json"), &local, true)?;
    std::fs::write(kit.join("next-steps.md"), next_steps(&anonymous))
        .map_err(|e| format!("cannot write reproduction instructions: {e}"))?;
    let summary = if failure.is_null() {
        "Conversion succeeded: no failure to reproduce. Share only kit/anonymous.json. Do not share kit/local-key.json or the original file."
    } else {
        "Reproduction kit written. Share kit/anonymous.json and a small rebuilt file only if it reproduces and you are free to share it. Never share the original file or kit/local-key.json."
    };
    let mut result = json!({"schema": SCHEMA, "ok": true, "summary": summary,
        "paths": {"anonymous": kit.join("anonymous.json"), "local_key": kit.join("local-key.json"),
                  "next_steps": kit.join("next-steps.md")}, "anonymous": anonymous});
    if include_local_key {
        if let Some(fields) = result.as_object_mut() {
            fields.insert("local_key".into(), local);
        }
    }
    Ok(result)
}

fn write_json(path: &Path, value: &Value, private: bool) -> Result<(), String> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options
        .open(path)
        .map_err(|e| format!("cannot create kit file: {e}"))?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .map_err(|e| format!("cannot write kit file: {e}"))
}

fn test_model(model: &Model, failure: &Value, writer: &dyn Writer) -> Value {
    let checked = if failure["kind"] == "cadmpeg_checks" {
        dutchman_core::repro::check(model)
    } else if matches!(
        failure["kind"].as_str(),
        Some("encode" | "restate" | "no_geometry")
    ) {
        // The source STEP bypass must not conceal a subset restatement failure.
        writer.write(model, &mut std::io::sink(), &mut Report::default())
    } else {
        return Value::Null;
    };
    match checked {
        Ok(()) => Value::Null,
        Err(error) => diagnostics::anonymous_failure(&crate::json::error(
            crate::json::CONVERT,
            Value::Null,
            &error,
        )),
    }
}

fn trial_matches(expected: &Value, actual: &Value) -> bool {
    same(expected, actual)
        || (!expected.is_null()
            && !actual.is_null()
            && expected["stage"] == actual["stage"]
            && expected["kind"] == actual["kind"]
            && expected["findings"]
                .as_object()
                .is_some_and(|m| m.is_empty())
            && actual["findings"].as_object().is_some_and(|m| m.is_empty()))
}

fn same(expected: &Value, actual: &Value) -> bool {
    if expected.is_null()
        || actual.is_null()
        || expected["stage"] != actual["stage"]
        || expected["kind"] != actual["kind"]
    {
        return false;
    }
    let codes = |v: &Value| {
        v["findings"]
            .as_object()
            .map(|m| m.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    // An unstructured encoder/decoder error is too broad to claim equivalence.
    !codes(expected).is_empty() && codes(expected) == codes(actual)
}

#[derive(Default)]
struct Narrowed {
    path: Vec<Value>,
    members: Vec<Value>,
    member_names: Vec<Value>,
    key: Vec<Value>,
    limits: Vec<&'static str>,
    isolated: bool,
    kinds: Value,
    /// The failure's signature, when `dutchman diagnose --narrow` recorded one.
    signature: Value,
}
impl Narrowed {
    fn limit(&mut self, message: &'static str) {
        if !self.limits.contains(&message) {
            self.limits.push(message);
        }
    }
    fn select(
        &mut self,
        unit: &'static str,
        ordinal: usize,
        total: usize,
        name: Option<&str>,
        parent: Option<usize>,
    ) {
        self.select_kind(unit, ordinal, total, name, parent, None);
    }

    /// [`Narrowed::select`], also saying what kind of feature it is (a fixed category).
    fn select_kind(
        &mut self,
        unit: &'static str,
        ordinal: usize,
        total: usize,
        name: Option<&str>,
        parent: Option<usize>,
        kind: Option<&str>,
    ) {
        let mut step = json!({"unit": unit, "ordinal": ordinal, "total": total});
        if let (Some(kind), Some(fields)) = (kind, step.as_object_mut()) {
            fields.insert("kind".into(), json!(kind));
        }
        self.path.push(step);
        self.key.push(json!({"unit": unit, "ordinal": ordinal, "total": total, "parent_step": parent,
            "name": name, "lookup": if name.is_some() { "name" } else { "unnamed in CADIR; locate manually in the CAD program, whose order may differ" }}));
    }
}

fn narrow(
    mut model: Model,
    failure: &Value,
    writer: &dyn Writer,
    result: &mut Narrowed,
    trials: &mut usize,
) -> Result<(), String> {
    if failure["findings"]
        .as_object()
        .is_some_and(|m| m.is_empty())
    {
        result.limit("No structured finding codes are available. Subset trials compare stage and category only; these are candidates, not a proven isolation of the same failure.");
        result.limit("Writer trials run mapping, restatement and any replay as one writer operation; separate retries of those steps are not supported here.");
    }
    if !trial_matches(failure, &test_model(&model, failure, writer)) {
        result.limit("The captured model does not reproduce the same structured check codes in isolation. No smaller unit is claimed.");
        return Ok(());
    }
    // Each next ordinal is relative to the previously retained unit, in CADIR arena order.
    for (arena, unit) in [
        ("product_definitions", "component"),
        ("occurrences", "occurrence"),
        ("bodies", "body"),
        ("features", "feature"),
        ("sketches", "sketch"),
    ] {
        let document = serde_json::to_value(&model.ir).map_err(|e| e.to_string())?;
        if document
            .get("model")
            .and_then(Value::as_object)
            .is_some_and(|arenas| {
                arenas
                    .values()
                    .filter_map(Value::as_array)
                    .map(Vec::len)
                    .sum::<usize>()
                    > 100_000
            })
        {
            result.limit("The 100,000-entity subset limit was reached; isolation is incomplete.");
            result.isolated = false;
            return Ok(());
        }
        let rows = document
            .get("model")
            .and_then(|m| m.get(arena))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if rows.is_empty() {
            result.limit(match unit {
                "component" | "occurrence" => "Component or occurrence structure is unavailable in the retained unit.",
                "feature" | "sketch" => "Feature or sketch history is unavailable in the retained unit; stored geometry cannot identify the operation that made it.",
                _ => "No body subset is available in the retained unit.",
            });
            continue;
        }
        let mut selected = false;
        for (index, row) in rows.iter().enumerate() {
            if *trials >= MAX_TRIALS {
                result.limit("The 256-trial budget was reached; isolation is incomplete.");
                result.isolated = false;
                return Ok(());
            }
            *trials += 1;
            let Ok(candidate) = dutchman_core::repro::subset(&model, arena, index) else {
                continue;
            };
            // A feature referencing a whole final body does not isolate that feature's
            // contribution. Only claim it if its subset actually reduces the failing model.
            if failure["kind"] == "cadmpeg_checks"
                && matches!(unit, "feature" | "sketch")
                && !candidate.ir.model.coedges.is_empty()
                && candidate.ir.model.coedges == model.ir.model.coedges
            {
                continue;
            }
            if trial_matches(failure, &test_model(&candidate, failure, writer)) {
                let parent = result.path.len().checked_sub(1);
                result.select_kind(
                    unit,
                    index + 1,
                    rows.len(),
                    row.get("name")
                        .or_else(|| row.get("label"))
                        .and_then(Value::as_str),
                    parent,
                    (unit == "feature").then(|| feature_kind(row)),
                );
                model = candidate;
                result.isolated = (unit == "body" || unit == "feature" || unit == "sketch")
                    && model.ir.model.bodies.len() <= 1
                    && failure["findings"]
                        .as_object()
                        .is_some_and(|m| !m.is_empty());
                selected = true;
                break;
            }
        }
        if !selected {
            result.limit(match unit {
            "feature" | "sketch" => "No smaller independent feature or sketch reproduces the same codes. CADIR stores final geometry; feature regeneration is not supported here.",
            "body" => "No single body reproduces the same codes; several bodies or a document-level relation may be required.",
            _ => "No independent component or occurrence reproduces the same codes.",
        });
        }
    }
    result.kinds = kinds(&model);
    Ok(())
}

/// A feature's kind, only when it is one of CADIR's fixed operation names.
fn feature_kind(feature: &Value) -> &'static str {
    const KINDS: &[&str] = &[
        "sketch",
        "extrude",
        "revolve",
        "fillet",
        "chamfer",
        "sweep",
        "loft",
        "shell",
        "boolean",
        "pattern",
        "mirror",
        "stored_geometry",
    ];
    feature
        .pointer("/definition/definition")
        .and_then(Value::as_str)
        .and_then(|k| KINDS.iter().find(|known| **known == k).copied())
        .unwrap_or("other")
}

fn kinds(model: &Model) -> Value {
    let mut surfaces = std::collections::BTreeMap::<String, u64>::new();
    let mut features = std::collections::BTreeMap::<String, u64>::new();
    // Only enum discriminants from CADIR are accepted; source-native kind/name strings
    // cannot enter the anonymous output.
    let document = serde_json::to_value(&model.ir).unwrap_or(Value::Null);
    for surface in &model.ir.model.surfaces {
        let value = serde_json::to_value(&surface.geometry).unwrap_or(Value::Null);
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .filter(|k| {
                matches!(
                    *k,
                    "plane"
                        | "cylinder"
                        | "cone"
                        | "sphere"
                        | "torus"
                        | "nurbs"
                        | "procedural"
                        | "revolution"
                        | "extrusion"
                        | "offset"
                        | "ruled"
                        | "placed"
                )
            })
            .unwrap_or("other");
        *surfaces.entry(kind.into()).or_default() += 1;
    }
    for feature in document
        .pointer("/model/features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        *features.entry(feature_kind(feature).into()).or_default() += 1;
    }
    json!({"surface_kinds": surfaces, "feature_kinds": features, "use_curve_outside_carrier": dutchman_core::repro::use_curve_outside_carrier_count(model), "finding_entity_kinds": dutchman_core::repro::entity_kinds(model).ok()})
}

fn anonymous(input: &Path, failure: &Value, narrowed: &Narrowed) -> Value {
    crate::json::repro_anonymous(
        input,
        failure,
        json!({"path": narrowed.path, "smallest_failing_unit_isolated": narrowed.isolated,
            "limits": narrowed.limits, "members": narrowed.members,
            "signature": narrowed.signature,
            "from_diagnose": narrowed.path.first().is_some_and(|p| p["unit"] == "part")}),
        &narrowed.kinds,
    )
}

/// Instructions use only fixed recipes and anonymous ordinals; they contain no private key.
pub fn next_steps(anonymous: &Value) -> String {
    let mut text = String::from("# Make a minimum reproduction\n\n");
    if anonymous["status"] == "no_failure_to_reproduce" {
        text.push_str("Conversion succeeded: no failure to reproduce.\n\n");
    } else {
        text.push_str("The conversion stopped. ");
        if anonymous.pointer("/failure/kind").and_then(Value::as_str) == Some("cadmpeg_checks") {
            text.push_str("cadmpeg's checks rejected its decoded model. The finding codes and counts are in anonymous.json.\n\n");
        } else {
            text.push_str("See the failure stage and category in anonymous.json. This category has no structured finding codes, so matching the category alone cannot prove the same failure.\n\n");
        }
        for step in anonymous
            .pointer("/narrowing/path")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let unit = step["unit"].as_str().unwrap_or("unit");
            let ordinal = step["ordinal"].as_u64().unwrap_or(0);
            let total = step["total"].as_u64().unwrap_or(0);
            if unit == "part" {
                text.push_str(&format!("The part to rebuild is part {ordinal} of {total}: the number `dutchman diagnose` gave it. Its name is in the diagnose folder's local-key.json under part {ordinal}, and in this kit's local-key.json. Keep that name local. `dutchman convert <your file> --part {ordinal}` converts it alone.\n\n"));
            } else if let Some(kind) = step["kind"].as_str() {
                text.push_str(&format!("Rebuild feature #{ordinal} of {total} in that part's history, a {kind} feature (look it up in local-key.json; keep the name local).\n\n"));
            } else {
                text.push_str(&format!("Look up {unit} #{ordinal} of {total} in local-key.json, following the preceding step's scope. Keep that name local.\n\n"));
            }
        }
        if let Some(signature) = anonymous
            .pointer("/narrowing/signature")
            .filter(|value| !value.is_null())
            .and_then(|value| serde_json::from_value::<Signature>(value.clone()).ok())
        {
            text.push_str(&rebuild_steps(&signature));
            text.push_str("Send only the report zip that last step makes. Never send the original file. Never send local-key.json.\n");
            for limit in anonymous
                .pointer("/narrowing/limits")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                text.push_str(&format!("\nLimit: {limit}\n"));
            }
            return text;
        }
        let parameter = anonymous
            .pointer("/failure/findings/parameter_domain")
            .and_then(Value::as_u64)
            .is_some();
        let nurbs = anonymous
            .pointer("/failure/curve_kinds/nurbs")
            .and_then(Value::as_u64)
            .is_some();
        // Recipes are keyed by structured check codes and anonymous carrier classes.
        const RECIPES: &[(&str, &str, &str)] = &[
            ("parameter_domain", "nurbs", "Recipe for parameter_domain with a NURBS carrier (including use-curve ranges outside their carrier domain): In a new design, make one body with a cylindrical or spline face whose edge crosses the face's seam. Extrude a full circle, then split the body along a plane through the axis. Save it and run `dutchman repro small-file --out new-results`. This is a candidate recipe, not a guarantee: this check code also covers other parameter-domain failures. Compare the stage and complete set of finding codes, not the counts.\n\n"),
        ];
        const GENERIC: &str = "Recipe: In your CAD program, copy the failing body alone into a new design using copy/paste or an export of that body. Remove unrelated features and sketches. If no body was isolated, remove unrelated components in a copy of the design one at a time. Save it and run `dutchman repro small-file --out new-results`. Keep a change only when the stage and complete set of finding codes match. Rebuild the remaining shape from scratch with simple, freely shareable dimensions.\n\n";
        let recipe = RECIPES
            .iter()
            .find(|(code, carrier, _)| {
                *code == "parameter_domain" && parameter && *carrier == "nurbs" && nurbs
            })
            .map_or(GENERIC, |(_, _, recipe)| *recipe);
        text.push_str(recipe);
        for limit in anonymous
            .pointer("/narrowing/limits")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            text.push_str(&format!("Limit: {limit}\n\n"));
        }
    }
    text.push_str("Send only anonymous.json, and the small rebuilt file if it reproduces and you are free to share it. Never send the original file. Never send local-key.json.\n");
    text
}

/// The steps from a signature to a report: rebuild, check, make the report.
fn rebuild_steps(signature: &Signature) -> String {
    crate::diagnose::recipes::render(signature)
}

pub mod synth;

#[cfg(test)]
mod tests;
