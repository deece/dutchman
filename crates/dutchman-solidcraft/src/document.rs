//! The design document, in one of two shapes:
//!
//! - an **import design**: what SolidCraft makes of a file it imports as STEP;
//! - a **history design**: the replayed history, then the original geometry as an import,
//!   suppressed, at the end of the timeline, unless the history already is that geometry.
//!
//! SolidCraft reads an import's assembly tree from the STEP itself. Imports receiving
//! explicit looks also name their output bodies so each look has a stable target.

use dutchman_core::{Model, Report, Severity};
use serde_json::{json, Value};

use crate::history::History;
use crate::{SolidCraftVersion, FORMAT};

/// The timeline name of the original geometry in a history design.
pub(crate) const ORIGINAL: &str = "Original geometry (imported)";

/// SolidCraft's longest component name.
const MAX_COMPONENT_NAME: usize = 128;

/// The ids SolidCraft gives the parts of a fresh import: its component, the component's
/// occurrence, and the Import feature.
const COMPONENT: u64 = 1;
const OCCURRENCE: u64 = 2;
const FEATURE: u64 = 3;

/// The identity placement, column-major.
const IDENTITY: [[f64; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// A design holding the model as one Import feature in its own component, the STEP text kept
/// in the container at `step_blob`.
pub(crate) fn import_design(model: &Model, step_blob: &str) -> Value {
    let component = component_name(&model.name);
    json!({
        "format": FORMAT,
        "name": model.name,
        "units": "mm",
        "params": [],
        "next_id": FEATURE + 1,
        "components": [{ "id": COMPONENT, "name": component, "parent": 0 }],
        "occurrences": [{
            "id": OCCURRENCE,
            "name": format!("{component}:1"),
            "component": COMPONENT,
            "parent": 0,
            "grounded": false,
            "transform": IDENTITY,
        }],
        "features": [import_feature(model, FEATURE, &feature_name(model), Some(COMPONENT), false, step_blob)],
    })
}

/// Applies explicit import body names and their definition-level looks.
pub(crate) fn import_looks(doc: &mut Value, looks: &History) {
    if let Some(names) = looks.import_body_names() {
        if let Some(fields) = doc
            .get_mut("features")
            .and_then(Value::as_array_mut)
            .and_then(|features| features.first_mut())
            .and_then(Value::as_object_mut)
        {
            fields.insert("body_names".to_owned(), names.clone());
        }
    }
    if let (false, Some(fields)) = (
        looks.looks.is_empty() && looks.face_looks.is_empty(),
        doc.as_object_mut(),
    ) {
        fields.insert(
            "appearances".to_owned(),
            json!({"bodies": looks.looks, "faces": looks.face_looks}),
        );
    }
}

/// Names and colours the visible final shape beside an editable history copy.
pub(crate) fn original_looks(doc: &mut Value, looks: &History) {
    if let Some(names) = looks.import_body_names() {
        if let Some(fields) = doc
            .get_mut("features")
            .and_then(Value::as_array_mut)
            .and_then(|features| {
                features
                    .iter_mut()
                    .find(|f| f.get("name").and_then(Value::as_str) == Some(ORIGINAL))
            })
            .and_then(Value::as_object_mut)
        {
            fields.insert("body_names".to_owned(), names.clone());
        }
    }
    if looks.looks.is_empty() && looks.face_looks.is_empty() {
        return;
    }
    let Some(fields) = doc.as_object_mut() else {
        return;
    };
    let appearances = fields.entry("appearances").or_insert_with(|| json!({}));
    let Some(appearances) = appearances.as_object_mut() else {
        return;
    };
    if let Some(bodies) = appearances
        .entry("bodies")
        .or_insert_with(|| json!({}))
        .as_object_mut()
    {
        bodies.extend(
            looks
                .looks
                .iter()
                .map(|(name, look)| (name.clone(), look.clone())),
        );
    }
    if let Some(faces) = appearances
        .entry("faces")
        .or_insert_with(|| json!([]))
        .as_array_mut()
    {
        faces.extend(looks.face_looks.iter().cloned());
    }
}

/// Appends mapped sketches to the import's component, hidden but editable. Parameters stay
/// available to their dimensions; none of the body-building history is retained.
pub(crate) fn keep_sketches(doc: &mut Value, references: &History) {
    if references.features.is_empty() {
        return;
    }
    let mut references = references.clone();
    references.relocate(FEATURE, COMPONENT, "", &mut Default::default());
    let hidden: Vec<Value> = references
        .features
        .iter()
        .filter_map(|feature| feature.get("id").cloned())
        .collect();
    if let Some(features) = doc.get_mut("features").and_then(Value::as_array_mut) {
        features.extend(references.features);
    }
    if let Some(fields) = doc.as_object_mut() {
        fields.insert("params".to_owned(), json!(references.params));
        fields.insert("next_id".to_owned(), json!(references.next_id));
        fields.insert("visibility".to_owned(), json!({"hidden_sketches": hidden}));
    }
}

/// A row-major 3 × 4 rigid transform as SolidCraft's column-major 4 × 4.
pub(crate) fn column_major(rows: &[[f64; 4]; 3]) -> [[f64; 4]; 4] {
    let mut out = IDENTITY;
    for (column, out_column) in out.iter_mut().enumerate() {
        for (row, values) in rows.iter().enumerate() {
            if let (Some(cell), Some(value)) = (out_column.get_mut(row), values.get(column)) {
                *cell = *value;
            }
        }
    }
    out
}

/// Which side of a history design is live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Live {
    /// The replayed history, as the design is written.
    History,
    /// As `History`, for a history that already is the original geometry, which is left out.
    Whole,
    /// Editable history in a hidden component; the final original remains visible.
    Partial,
    /// As `Partial`, for a whole history that could not be checked against the original.
    Unverified,
    /// The original geometry, with the history suppressed: what the replay is checked against.
    Original,
}

/// The document of a design holding `params` and no geometry or features.
pub(crate) fn parameters_design(model: &Model, params: &[Value]) -> Value {
    json!({
        "format": FORMAT,
        "name": model.name,
        "units": "mm",
        "params": params,
        "next_id": 1,
        "features": [],
    })
}

/// A design holding the replayed `history`, then the original geometry (STEP at
/// `step_blob`) as an Import at the end of the timeline, with only the `live` side
/// unsuppressed.
pub(crate) fn history_design(
    model: &Model,
    history: &History,
    step_blob: &str,
    live: Live,
    target: SolidCraftVersion,
) -> Value {
    let mut history = history.clone();
    let component = history.next_id;
    let occurrence = component.saturating_add(1);
    let hidden = matches!(live, Live::Partial | Live::Unverified);
    if hidden {
        history.relocate(0, component, "", &mut Default::default());
        history.next_id = occurrence.saturating_add(1);
    }
    let mut features = history.features.clone();
    if live == Live::Original {
        for feature in &mut features {
            if let Some(fields) = feature.as_object_mut() {
                fields.insert("suppressed".to_owned(), Value::Bool(true));
            }
        }
    }
    let original = history.next_id;
    if live != Live::Whole {
        features.push(import_feature(
            model,
            original,
            ORIGINAL,
            None,
            live == Live::History,
            step_blob,
        ));
    }
    let mut doc = json!({
        "format": FORMAT,
        "name": model.name,
        "units": "mm",
        "params": history.params,
        "next_id": original.saturating_add(1),
        "features": features,
    });
    if let (Some(first), Some(fields)) = (history.sheet_rules.first(), doc.as_object_mut()) {
        fields.insert(
            "sheet".to_owned(),
            json!({"active": first.get("name"), "rules": history.sheet_rules}),
        );
    }
    if let (true, Some(fields)) = (hidden, doc.as_object_mut()) {
        let part = if live == Live::Unverified {
            "Editable history (unverified)"
        } else {
            "Editable history (partial)"
        };
        fields.insert(
            "components".to_owned(),
            json!([editable_history_component(component, part, 0, target)]),
        );
        fields.insert("occurrences".to_owned(), json!([{"id": occurrence, "name": format!("{part}:1"), "component": component, "parent": 0, "grounded": true, "transform": IDENTITY}]));
        let sketches: Vec<Value> = history
            .features
            .iter()
            .filter(|f| f.get("type").and_then(Value::as_str) == Some("sketch"))
            .filter_map(|f| f.get("id").cloned())
            .collect();
        fields.insert(
            "visibility".to_owned(),
            json!({"hidden_occurrences": [[occurrence]], "hidden_sketches": sketches}),
        );
    }
    let has_looks = !history.looks.is_empty() || !history.face_looks.is_empty();
    if let (true, Some(fields)) = (has_looks, doc.as_object_mut()) {
        let mut appearances = json!({"bodies": history.looks});
        if let (false, Some(look_fields)) =
            (history.face_looks.is_empty(), appearances.as_object_mut())
        {
            look_fields.insert("faces".to_owned(), json!(history.face_looks));
        }
        fields.insert("appearances".to_owned(), appearances);
    }
    doc
}

/// A hidden editable-history copy. Hiding its History bar entries leaves rebuilding and
/// Browser access intact; older targets keep their existing component fields.
pub(crate) fn editable_history_component(
    id: u64,
    name: &str,
    parent: u64,
    target: SolidCraftVersion,
) -> Value {
    let mut component = json!({"id": id, "name": name, "parent": parent});
    if target >= SolidCraftVersion::V0_30_0 {
        if let Some(fields) = component.as_object_mut() {
            fields.insert("history_hidden".to_owned(), Value::Bool(true));
        }
    }
    component
}

/// An Import feature holding the model's STEP restatement, in `component` (the root when
/// `None`).
pub(crate) fn import_feature(
    model: &Model,
    id: u64,
    name: &str,
    component: Option<u64>,
    suppressed: bool,
    step_blob: &str,
) -> Value {
    let mut feature = json!({
        "id": id,
        "name": name,
        "type": "import",
        "suppressed": suppressed,
        "file": source_file(model),
        "step": "",
        "step_blob": step_blob,
    });
    if let (Some(component), Some(fields)) = (component, feature.as_object_mut()) {
        fields.insert("component".to_owned(), json!(component));
    }
    feature
}

/// Records what an import design does not carry over from the model.
pub(crate) fn report_unmapped(model: &Model, references: Option<&History>, report: &mut Report) {
    let m = &model.ir.model;
    let sketches = if references.is_none() {
        m.sketches.len()
    } else {
        0
    };
    let parameters = if references.is_some_and(|history| !history.features.is_empty()) {
        0
    } else {
        m.parameters.len()
    };
    let flattened = [
        (m.features.len(), "feature history"),
        (sketches, "sketches"),
        (parameters, "parameters"),
    ];
    for (count, what) in flattened {
        if count > 0 {
            report.push(
                Severity::Degraded,
                what,
                format!("{count} carried as a non-parametric body, not as editable history"),
            );
        }
    }
}

/// The component's name: the design's, as SolidCraft names a component after the file it
/// imports.
pub(crate) fn component_name(design: &str) -> String {
    let name: String = design.trim().chars().take(MAX_COMPONENT_NAME).collect();
    if name.is_empty() {
        "Import".to_owned()
    } else {
        name
    }
}

/// The timeline name SolidCraft gives the import: the STEP product's name, which cadmpeg takes
/// from the first body, unless that is generic; else the design's name.
pub(crate) fn feature_name(model: &Model) -> String {
    model
        .ir
        .model
        .bodies
        .first()
        .and_then(|body| body.name.as_deref())
        .filter(|name| !generic_name(name))
        .map_or_else(|| model.name.clone(), str::to_owned)
}

/// A name SolidCraft treats as no name at all.
fn generic_name(name: &str) -> bool {
    name.trim().is_empty()
        || name.starts_with('(')
        || name.eq_ignore_ascii_case("import")
        || name.eq_ignore_ascii_case("unnamed")
}

/// The file the import came from, as the Import feature records it.
fn source_file(model: &Model) -> String {
    model
        .source_file
        .clone()
        .unwrap_or_else(|| format!("{}.step", model.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_names_are_trimmed_and_capped() {
        assert_eq!(component_name("  part  "), "part");
        assert_eq!(component_name(""), "Import");
        assert_eq!(component_name(&"x".repeat(300)).len(), MAX_COMPONENT_NAME);
    }

    #[test]
    fn generic_names_match_solidcraft() {
        for name in ["", "  ", "(unnamed)", "Import", "UNNAMED"] {
            assert!(generic_name(name), "{name:?}");
        }
        assert!(!generic_name("Bracket"));
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    fn history() -> History {
        History {
            default_bodies: Default::default(),
            stored_bodies: Default::default(),
            params: vec![json!({"name": "d1", "expr": "10 mm", "unit": "mm", "model": true})],
            features: vec![
                json!({"id": 1, "name": "Sketch1", "type": "sketch", "suppressed": false}),
                json!({"id": 2, "name": "Extrude1", "type": "extrude", "suppressed": false}),
            ],
            looks: Default::default(),
            face_looks: Vec::new(),
            sheet_rules: Vec::new(),
            next_id: 3,
            pattern_replay_required: false,
            mapped: 2,
            not_needed: 0,
            reapplied: 0,
            total: 2,
            blends: Vec::new(),
            hidden: Vec::new(),
            unconfirmed: Default::default(),
            stop: None,
            sources: Default::default(),
        }
    }

    #[test]
    fn only_hidden_copies_hide_history_on_new_targets() {
        let model = crate::testing::cube();
        for target in ["0.29.0", "0.30.0", "0.31.0", "0.40.0"] {
            let version = target.parse().unwrap();
            for live in [
                Live::Partial,
                Live::Unverified,
                Live::History,
                Live::Whole,
                Live::Original,
            ] {
                let doc = history_design(&model, &history(), "blobs/x.step", live, version);
                assert_eq!(doc["format"], FORMAT);
                // The 0.40 target keeps the 0.30 representation, including hidden history.
                if target == "0.40.0" {
                    assert_eq!(
                        doc,
                        history_design(
                            &model,
                            &history(),
                            "blobs/x.step",
                            live,
                            SolidCraftVersion::V0_30_0,
                        )
                    );
                }
                if matches!(live, Live::Partial | Live::Unverified) {
                    let component = &doc["components"][0];
                    assert_eq!(
                        component.get("history_hidden"),
                        (version >= SolidCraftVersion::V0_30_0).then_some(&Value::Bool(true))
                    );
                    assert_eq!(doc["visibility"]["hidden_occurrences"], json!([[4]]));
                    assert!(doc["features"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|f| f["suppressed"] == false));
                } else {
                    assert!(doc.get("components").is_none());
                }
            }
        }
    }

    #[test]
    fn the_original_follows_the_history_suppressed() {
        let model = crate::testing::cube();
        let doc = history_design(
            &model,
            &history(),
            "blobs/x.step",
            Live::History,
            SolidCraftVersion::default(),
        );
        let features = doc["features"].as_array().unwrap();
        assert_eq!(features.len(), 3);
        let original = features.last().unwrap();
        assert_eq!(original["id"], 3);
        assert_eq!(original["name"], ORIGINAL);
        assert_eq!(original["type"], "import");
        assert_eq!(original["suppressed"], true);
        assert_eq!(original["step_blob"], "blobs/x.step");
        assert!(original.get("component").is_none(), "at the root");
        assert!(features.iter().take(2).all(|f| f["suppressed"] == false));
        assert_eq!(doc["next_id"], 4);
        assert_eq!(doc["params"], json!(history().params));
        assert!(doc.get("components").is_none());
    }

    #[test]
    fn a_whole_history_leaves_the_original_out() {
        let model = crate::testing::cube();
        let doc = history_design(
            &model,
            &history(),
            "",
            Live::Whole,
            SolidCraftVersion::default(),
        );
        let features = doc["features"].as_array().unwrap();
        assert_eq!(features.len(), 2);
        assert!(features.iter().all(|f| f["name"] != ORIGINAL));
    }

    #[test]
    fn the_check_design_has_only_the_original_live() {
        let model = crate::testing::cube();
        let doc = history_design(
            &model,
            &history(),
            "blobs/x.step",
            Live::Original,
            SolidCraftVersion::default(),
        );
        let features = doc["features"].as_array().unwrap();
        assert!(features.iter().take(2).all(|f| f["suppressed"] == true));
        assert_eq!(features.last().unwrap()["suppressed"], false);
    }
}
