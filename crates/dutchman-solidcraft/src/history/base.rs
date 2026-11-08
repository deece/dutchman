//! Base features and stored geometry: bodies a timeline holds without the steps that made
//! them (modelled directly, or imported), as SolidCraft Import features at their place in the
//! timeline, so the features after them keep their history.
//!
//! The bodies are the design's as it ends, so a base body a later feature changes would be
//! imported changed already, and the later feature would change it again. Such a base feature
//! is refused, naming the feature that changes its body.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use dutchman_core::{Model, Report, Severity};

use super::{describe, name, Context, History};
use crate::container::Container;
use crate::step;

/// Stored solids have no construction recipe to lose.
pub(crate) const STORED_SOLID: &str =
    "This part holds an imported solid; there is no feature history to bring over";

/// Where a mapped base feature keeps the source bodies it imports, until they are restated.
const BODIES: &str = "_bodies";
/// Where it keeps their STEP text, until the text is stored in the container.
const STEP: &str = "_step";

/// Whether a base feature's operation `definition` resolves to no bodies at all: the source
/// proves it holds none, so it builds nothing.
pub(super) fn holds_none(definition: &Value) -> bool {
    let bodies = definition.get("bodies").unwrap_or(&Value::Null);
    bodies.get("kind").and_then(Value::as_str) == Some("resolved")
        && bodies
            .pointer("/value/bodies")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
}

/// Maps a base feature whose operation is `definition`.
pub(super) fn map(feature: &Value, definition: &Value, ctx: &mut Context) -> Result<Value, String> {
    let bodies = definition.get("bodies").unwrap_or(&Value::Null);
    let found = bodies.get("kind").and_then(Value::as_str);
    if found != Some("resolved") {
        return Err(super::unsupported_selection("body", found));
    }
    let ids: Vec<&str> = bodies
        .pointer("/value/bodies")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if let Some(later) = later_change(feature, &ids, ctx) {
        return Err(format!(
            "its body is stored only as the design ends, and the later feature \"{later}\" \
             changes it"
        ));
    }
    import(feature, &ids, ctx)
}

/// The name of the first active feature after `feature` on the timeline that outputs one of
/// the bodies `ids`, which it then changes.
fn later_change<'a>(feature: &Value, ids: &[&str], ctx: &Context<'a>) -> Option<&'a str> {
    let ordinal = feature.get("ordinal").and_then(Value::as_u64)?;
    ctx.arena("features")
        .iter()
        .filter(|row| row.get("suppressed").and_then(Value::as_bool) != Some(true))
        .filter(|row| {
            row.get("ordinal")
                .and_then(Value::as_u64)
                .is_some_and(|at| at > ordinal)
        })
        .filter(|row| {
            row.get("outputs")
                .and_then(Value::as_array)
                .is_some_and(|outputs| {
                    outputs
                        .iter()
                        .any(|out| out.as_str().is_some_and(|out| ids.contains(&out)))
                })
        })
        .min_by_key(|row| row.get("ordinal").and_then(Value::as_u64))
        .map(|row| name(row).unwrap_or("?"))
}

/// Stored geometry names its retained bodies in the feature's outputs, not its operation.
pub(super) fn stored(
    feature: &Value,
    ctx: &mut Context,
    report: &mut Report,
) -> Result<Value, String> {
    let ids: Vec<&str> = feature
        .get("outputs")
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mapped = import(feature, &ids, ctx)?;
    report.push(Severity::Info, describe(feature), STORED_SOLID);
    Ok(mapped)
}

fn import(feature: &Value, ids: &[&str], ctx: &mut Context) -> Result<Value, String> {
    if ids.is_empty() {
        return Err("it holds no bodies".to_owned());
    }
    let id = ctx.id();
    ctx.builds = true;
    let name = name(feature)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Base Feature{id}"));
    // Validate the emitted sequence before committing body names to later selections
    // or colours. A colour correspondence gap must not discard usable history.
    let model = ctx.source.ok_or("its source geometry is not available")?;
    let part = dutchman_core::with_bodies(model, ids, name.clone())
        .map_err(|why| format!("base feature \"{name}\": {why}"))?;
    let text = step::restate(&part, &mut Report::default())
        .map_err(|err| format!("base feature \"{name}\": {err}"))?;
    let names = super::faces::import_names(ctx, ids, Some(&text));
    let mut mapped = json!({
        "id": id,
        "name": name,
        "type": "import",
        "suppressed": false,
        "file": format!("{name}.step"),
        "step": "",
        BODIES: ids,
        STEP: text,
    });
    if let (false, Some(fields)) = (names.is_empty(), mapped.as_object_mut()) {
        fields.insert("body_names".to_owned(), json!(names));
    }
    Ok(mapped)
}

/// Restates the bodies of each base feature in `history` as STEP, from `model`, the design
/// the history was mapped from; why one cannot be, else.
pub(crate) fn restate(history: &mut History, model: &Model) -> Result<(), String> {
    for feature in &mut history.features {
        let Some(fields) = feature.as_object_mut() else {
            continue;
        };
        if fields.get(STEP).and_then(Value::as_str).is_some() {
            continue;
        }
        let Some(bodies) = fields.get(BODIES).cloned() else {
            continue;
        };
        let ids: Vec<&str> = bodies
            .as_array()
            .map(|ids| ids.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let title = fields
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Base Feature")
            .to_owned();
        let part = dutchman_core::with_bodies(model, &ids, title.clone())
            .map_err(|why| format!("base feature \"{title}\": {why}"))?;
        let text = step::restate(&part, &mut Report::default())
            .map_err(|err| format!("base feature \"{title}\": {err}"))?;
        if fields
            .get("body_names")
            .and_then(Value::as_array)
            .is_some_and(|names| names.len() != super::faces::shape_count(&text))
        {
            return Err(format!("base feature \"{title}\": its written shapes cannot be matched to its appearance targets"));
        }
        fields.insert(STEP.to_owned(), Value::String(text));
    }
    Ok(())
}

/// Whether `history` holds nothing but Imports: nothing in it can be edited.
pub(crate) fn only_stored(history: &History) -> bool {
    history
        .features
        .iter()
        .all(|feature| feature.get("type").and_then(Value::as_str) == Some("import"))
}

/// Whether `history` already is `model`'s final geometry: only base features holding every
/// body of the model, and sketches. The original would repeat those Imports.
pub(crate) fn whole(history: &History, model: &Model) -> bool {
    let mut held = BTreeSet::new();
    for feature in &history.features {
        match feature.get("type").and_then(Value::as_str) {
            Some("sketch") => {}
            Some("import") => {
                let Some(bodies) = feature.get(BODIES).and_then(Value::as_array) else {
                    return false;
                };
                held.extend(bodies.iter().filter_map(Value::as_str));
            }
            _ => return false,
        }
    }
    !held.is_empty()
        && model
            .ir
            .model
            .bodies
            .iter()
            .all(|body| held.contains(body.id.as_str()))
}

/// Stores the STEP text of each base feature in `features` in `container`, pointing the
/// feature at it.
pub(crate) fn store(
    features: &mut [Value],
    container: &mut Container,
) -> Result<(), dutchman_core::Error> {
    for feature in features {
        let Some(fields) = feature.as_object_mut() else {
            continue;
        };
        fields.remove(BODIES);
        if let Some(Value::String(text)) = fields.remove(STEP) {
            let blob = container.add_step(text.as_bytes())?;
            fields.insert("step_blob".to_owned(), Value::String(blob));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_feature_whose_body_a_later_feature_changes_is_refused() {
        let base = |ordinal: u64| {
            json!({"id": "b", "name": "Base Feature 1", "ordinal": ordinal,
                "outputs": ["body#1"]})
        };
        let definition = json!({"bodies": {"kind": "resolved", "value": {"bodies": ["body#1"]}}});
        let model = json!({"features": [
            base(1),
            {"id": "m", "name": "Move 1", "ordinal": 2, "outputs": ["body#1"]},
            {"id": "s", "name": "Move 2", "ordinal": 3, "outputs": ["body#1"], "suppressed": true},
            {"id": "o", "name": "Extrude 1", "ordinal": 4, "outputs": ["body#2"]},
        ]});
        let mut ctx = Context::new(&model);
        let err = map(&base(1), &definition, &mut ctx).unwrap_err();
        assert_eq!(
            err,
            "its body is stored only as the design ends, and the later feature \"Move 1\" changes it"
        );
        // A suppressed later feature, or one making another body, changes nothing.
        let model = json!({"features": [
            base(1),
            {"id": "s", "name": "Move 2", "ordinal": 3, "outputs": ["body#1"], "suppressed": true},
            {"id": "o", "name": "Extrude 1", "ordinal": 4, "outputs": ["body#2"]},
        ]});
        let ctx = Context::new(&model);
        assert_eq!(later_change(&base(1), &["body#1"], &ctx), None);
    }
}
