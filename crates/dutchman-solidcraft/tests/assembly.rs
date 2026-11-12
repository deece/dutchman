//! An assembly written as SolidCraft components and occurrences: the synthetic Inventor cube
//! placed twice.
//!
//! The SolidCraft check runs when `SOLIDCRAFT_CLI` names a SolidCraft CLI.

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use dutchman_core::{read, Assembly, Format, Model, Placement, Report, Writer};
use dutchman_solidcraft::{SolidCraftWriter, Verification, CLI_ENV};
use serde_json::{json, Value};
use zip::ZipArchive;

fn placed_twice() -> Model {
    let part = read(
        Format::InventorPart,
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../dutchman-core/tests/fixtures/synthetic_cube.ipt"),
        &mut Report::default(),
    )
    .unwrap();
    let at = |x: f64| {
        [
            [1.0, 0.0, 0.0, x],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ]
    };
    Model {
        name: "pair".to_owned(),
        source_file: Some("pair.iam".to_owned()),
        source_step: None,
        assembly: Some(Assembly {
            own_part: None,
            joints: Vec::new(),
            left_out: Vec::new(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
            parts: vec![part.clone()],
            placements: vec![
                Placement {
                    hidden: false,
                    name: None,
                    part: 0,
                    transform: at(0.0),
                },
                Placement {
                    hidden: false,
                    name: Some("Moved".to_owned()),
                    part: 0,
                    transform: at(30.0),
                },
            ],
        }),
        ..part
    }
}

fn written(model: &Model) -> (Vec<u8>, Report) {
    let mut report = Report::default();
    let mut bytes = Vec::new();
    SolidCraftWriter::new()
        .verification(Verification::Off)
        .write(model, &mut bytes, &mut report)
        .unwrap();
    (bytes, report)
}

fn document(bytes: &[u8]) -> Value {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    serde_json::from_reader(archive.by_name("document.json").unwrap()).unwrap()
}

#[test]
fn folders_hold_each_top_designs_features_including_nested_parts() {
    let model = nested();
    let (bytes, report) = written(&model);
    let doc = document(&bytes);
    let groups = doc["timeline_groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["name"], doc["components"][0]["name"]);
    assert_eq!(groups[1]["name"], doc["components"][2]["name"]);
    assert_eq!(groups[0]["features"], serde_json::json!([3]));
    assert_eq!(groups[1]["features"], serde_json::json!([7]));
    assert!(groups.iter().all(|group| group["parent"].is_null()));
    assert_eq!(doc["next_id"], 13);
    assert_eq!(groups[0]["id"], 11);
    assert_eq!(groups[1]["id"], 12);
    let notes: Vec<_> = report
        .losses()
        .iter()
        .filter(|note| note.subject == "timeline folders")
        .collect();
    assert_eq!(notes.len(), 2);
    assert!(notes
        .iter()
        .all(|note| note.severity == dutchman_core::Severity::Info));
    assert_eq!(notes[0].detail, "\"pair\": 1 feature(s) grouped");
    if let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) {
        assert_eq!(evaluate(&cli, &bytes).len(), 5);
    }
}

#[test]
fn a_single_part_or_one_top_design_has_no_folders() {
    let model = placed_twice();
    let mut own = with_sketches();
    own.assembly.as_mut().unwrap().own_part = Some(0);
    for model in [
        &model,
        &model.assembly.as_ref().unwrap().parts[0],
        &with_components(true),
        &own,
    ] {
        let (bytes, report) = written(model);
        assert!(document(&bytes).get("timeline_groups").is_none());
        assert!(!report
            .losses()
            .iter()
            .any(|note| note.subject == "timeline folders"));
    }
}

#[test]
fn root_features_get_a_unique_folder_and_joints_stay_out() {
    let mut model = with_sketches();
    model.name = "Block".to_owned();
    let assembly = model.assembly.as_mut().unwrap();
    assembly.joints.push(dutchman_core::Joint {
        a: vec![0],
        b: vec![1],
        frame_a: dutchman_core::placement::IDENTITY,
        frame_b: dutchman_core::placement::IDENTITY,
        suppressed: false,
        motion: dutchman_core::Motion::Rigid,
        flip: true,
    });
    let (bytes, _) = written(&model);
    let doc = document(&bytes);
    let groups = doc["timeline_groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["name"], "Block");
    assert_eq!(groups[1]["name"], "Block (2)");
    let grouped: Vec<_> = groups
        .iter()
        .flat_map(|g| g["features"].as_array().unwrap())
        .collect();
    let features: Vec<_> = doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| &f["id"])
        .collect();
    assert_eq!(grouped, features, "all features stay in timeline order");
    assert!(!grouped.contains(&&doc["assembly"]["joints"][0]["id"]));
    assert_eq!(groups[1]["features"], doc["visibility"]["hidden_sketches"]);
    if let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) {
        assert_eq!(evaluate(&cli, &bytes).len(), 2);
    }
}

#[test]
fn each_parts_history_is_reported() {
    // The cube with a mapped sphere history beside the cube with none.
    let mut model = placed_twice();
    let assembly = model.assembly.as_mut().unwrap();
    let mut part = assembly.parts[0].clone();
    part.name = "Ball".to_owned();
    let mut wire = serde_json::to_value(&part.ir).unwrap();
    wire["model"]["features"] = serde_json::json!([{
        "id": "f3d:model:feature#1", "ordinal": 0, "name": "Ball", "suppressed": false,
        "outputs": [wire["model"]["bodies"][0]["id"]],
        "definition": {"definition": "sphere", "center": {"x": 5.0, "y": 5.0, "z": 5.0},
                       "radius": 5.0, "op": "new_body"},
    }]);
    part.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    assembly.parts.push(part);
    assembly.placements[1].part = 1;
    let (_, report) = written(&model);
    let parts = report.parts();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].name, "synthetic_cube");
    assert!(matches!(
        parts[0].check,
        dutchman_core::Check::NotApplicable(_)
    ));
    assert_eq!((parts[0].mapped, parts[0].total), (0, 0));
    assert_eq!(parts[1].name, "Ball");
    assert!(matches!(
        parts[1].check,
        dutchman_core::Check::Unverified(_)
    ));
    assert_eq!((parts[1].mapped, parts[1].total), (1, 1));
}

#[test]
fn folders_keep_mapped_history_and_its_suppressed_original() {
    let mut model = placed_twice();
    let assembly = model.assembly.as_mut().unwrap();
    let mut part = assembly.parts[0].clone();
    let mut wire = serde_json::to_value(&part.ir).unwrap();
    wire["model"]["features"] = serde_json::json!([{
        "id": "f3d:model:feature#1", "ordinal": 0, "name": "Ball", "suppressed": false,
        "outputs": [wire["model"]["bodies"][0]["id"]],
        "definition": {"definition": "sphere", "center": {"x": 5.0, "y": 5.0, "z": 5.0},
                       "radius": 5.0, "op": "new_body"},
    }]);
    part.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    assembly.parts.push(part);
    assembly.placements[1].part = 1;
    let (bytes, _) = written(&model);
    let doc = document(&bytes);
    let features = doc["features"].as_array().unwrap();
    assert_eq!(features.len(), 3);
    assert_eq!(features[0]["type"], "import");
    assert_eq!(features[1]["type"], "sphere");
    assert_eq!(features[1]["name"], "Ball");
    assert_eq!(features[2]["name"], "Original geometry (imported)");
    assert_eq!(features[2]["suppressed"], true);
    assert_eq!(doc["timeline_groups"][0]["name"], "synthetic_cube");
    assert_eq!(doc["timeline_groups"][1]["name"], "synthetic_cube (2)");
    assert_eq!(
        doc["timeline_groups"][1]["features"],
        serde_json::json!([features[1]["id"], features[2]["id"]])
    );
    if let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) {
        assert_eq!(evaluate(&cli, &bytes).len(), 2);
    }
}

#[test]
fn an_assembly_parts_unverified_history_can_be_kept_hidden_beside_its_original() {
    let mut model = placed_twice();
    let assembly = model.assembly.as_mut().unwrap();
    let mut part = assembly.parts[0].clone();
    let mut wire = serde_json::to_value(&part.ir).unwrap();
    wire["model"]["features"] = serde_json::json!([{
        "id": "f3d:model:feature#1", "ordinal": 0, "name": "Ball", "suppressed": false,
        "outputs": [wire["model"]["bodies"][0]["id"]],
        "definition": {"definition": "sphere", "center": {"x": 5.0, "y": 5.0, "z": 5.0},
                       "radius": 5.0, "op": "new_body"},
    }]);
    part.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    assembly.parts.push(part);
    assembly.placements[1].part = 1;
    for target in ["0.29.0", "0.30.0", "0.40.0"] {
        let mut report = Report::default();
        let mut bytes = Vec::new();
        SolidCraftWriter::new()
            .target_version(Some(target.parse().unwrap()))
            .verification(Verification::Off)
            .hide_unverified_history(true)
            .write(&model, &mut bytes, &mut report)
            .unwrap();
        let doc = document(&bytes);
        let features = doc["features"].as_array().unwrap();
        // The original stays the part's visible model; the history is all there, hidden.
        let original = features
            .iter()
            .find(|f| f["type"] == "import" && f["component"] != features[0]["component"])
            .unwrap();
        assert_eq!(original["suppressed"], false);
        let ball = features.iter().find(|f| f["type"] == "sphere").unwrap();
        let history = doc["components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == ball["component"])
            .unwrap();
        assert_eq!(history["name"], "Editable history (unverified)");
        assert_eq!(history["parent"], original["component"]);
        assert_eq!(
            history.get("history_hidden"),
            (target != "0.29.0").then_some(&json!(true))
        );
        assert!(doc["components"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["id"] != history["id"])
            .all(|c| c.get("history_hidden").is_none()));
        let occurrence = doc["occurrences"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["component"] == history["id"])
            .unwrap();
        let hidden = doc["visibility"]["hidden_occurrences"].as_array().unwrap();
        assert!(hidden
            .iter()
            .any(|path| path.as_array().unwrap().last() == Some(&occurrence["id"])));
        assert!(report.losses().iter().any(|note| note
            .detail
            .contains("unverified: verification turned off; kept hidden")));
        if let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) {
            assert_eq!(evaluate(&cli, &bytes).len(), 3);
        }
    }
}

#[test]
fn root_bodies_and_sketches_share_a_folder_only_when_contiguous() {
    for own_first in [false, true] {
        let mut model = with_sketches();
        let assembly = model.assembly.as_mut().unwrap();
        let mut own = assembly.parts[0].clone();
        own.name = model.name.clone();
        let own_index = if own_first { 0 } else { 1 };
        assembly.parts.insert(own_index, own);
        assembly.own_part = Some(own_index);
        for placement in &mut assembly.placements {
            placement.part = 1 - own_index;
        }
        assembly.placements.push(Placement {
            hidden: false,
            name: None,
            part: own_index,
            transform: dutchman_core::placement::IDENTITY,
        });
        let (bytes, _) = written(&model);
        let doc = document(&bytes);
        let groups = doc["timeline_groups"].as_array().unwrap();
        assert_eq!(groups.len(), if own_first { 3 } else { 2 });
        if own_first {
            assert_eq!(groups[0]["name"], "blocks");
            assert_eq!(groups[2]["name"], "blocks (2)");
            assert_eq!(groups[0]["features"].as_array().unwrap().len(), 1);
            assert_eq!(groups[2]["features"], doc["visibility"]["hidden_sketches"]);
        } else {
            assert_eq!(groups[1]["name"], "blocks");
            assert_eq!(groups[1]["features"].as_array().unwrap().len(), 2);
        }
        let ids: Vec<_> = groups
            .iter()
            .flat_map(|g| g["features"].as_array().unwrap())
            .collect();
        let timeline: Vec<_> = doc["features"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| &f["id"])
            .collect();
        assert_eq!(ids, timeline);
        if let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) {
            assert_eq!(evaluate(&cli, &bytes).len(), 3);
        }
    }
}

#[test]
fn a_part_with_no_bodies_is_an_empty_component_its_joints_still_join() {
    let mut model = placed_twice();
    if let Some(assembly) = &mut model.assembly {
        let mut empty = assembly.parts[0].clone();
        let mut wire = serde_json::to_value(&empty.ir).unwrap();
        // Every arena emptied: a component with nothing in it.
        for arena in wire["model"].as_object_mut().unwrap().values_mut() {
            if arena.is_array() {
                *arena = serde_json::json!([]);
            }
        }
        empty.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        empty.name = "Spacer".to_owned();
        assembly.parts.push(empty);
        assembly.placements[1].part = 1;
        assembly.joints.push(dutchman_core::Joint {
            a: vec![0],
            b: vec![1],
            frame_a: dutchman_core::placement::IDENTITY,
            frame_b: dutchman_core::placement::IDENTITY,
            suppressed: false,
            motion: dutchman_core::Motion::Rigid,
            flip: true,
        });
    }
    let (bytes, report) = written(&model);
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let names: Vec<&str> = document["components"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(names.contains(&"Spacer"), "{names:?}");
    assert_eq!(document["occurrences"].as_array().unwrap().len(), 2);
    // Only the cube imports geometry; the joint holds both occurrences.
    let imports = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["type"] == "import")
        .count();
    assert_eq!(imports, 1, "{}", document["features"]);
    assert_eq!(document["assembly"]["joints"].as_array().unwrap().len(), 1);
    assert!(!report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("left out")));
    assert!(report.losses().iter().any(|loss| loss.subject == "part 2"
        && loss.severity == dutchman_core::Severity::Degraded
        && loss
            .detail
            .starts_with("placed as an empty component: it has no bodies")));
    let completeness = report.completeness().unwrap();
    assert_eq!(completeness.placed, 1);
    assert_eq!(completeness.placed_empty, 1);
    assert_eq!(completeness.placements(), 2);
    assert_eq!(completeness.left_out_count(), 0);
    assert!(!completeness.is_complete());
}

#[test]
fn empty_leaf_numbers_include_holders_and_are_stable_across_workers() {
    let mut model = placed_twice();
    let top = model.assembly.as_mut().unwrap();
    let mut empty = top.parts[0].clone();
    let mut wire = serde_json::to_value(&empty.ir).unwrap();
    for arena in wire["model"].as_object_mut().unwrap().values_mut() {
        if arena.is_array() {
            *arena = json!([]);
        }
    }
    empty.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    empty.name = "Synthetic empty leaf".into();
    let mut holder = empty.clone();
    holder.name = "Synthetic holder".into();
    holder.assembly = Some(Assembly {
        parts: vec![empty],
        ..top.clone()
    });
    top.parts.push(holder);
    top.placements[1].part = 1;
    assert_eq!(dutchman_core::parts::catalog(&model)[2].number, 3);
    let mut results = Vec::new();
    for threads in [1, 4] {
        let mut bytes = Vec::new();
        let mut report = Report::default();
        SolidCraftWriter::new()
            .threads(threads)
            .verification(Verification::Off)
            .write(&model, &mut bytes, &mut report)
            .unwrap();
        let counts = report.completeness().unwrap();
        assert_eq!(
            (counts.placed, counts.placed_empty, counts.placements()),
            (2, 2, 4)
        );
        let empty_losses: Vec<_> = report
            .losses()
            .iter()
            .filter(|loss| loss.detail.starts_with("placed as an empty component"))
            .collect();
        assert_eq!(empty_losses.len(), 1);
        assert_eq!(empty_losses[0].subject, "part 3");
        assert_eq!(empty_losses[0].severity, dutchman_core::Severity::Degraded);
        results.push((bytes, report.losses().to_vec()));
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn an_assembly_that_places_no_parts_is_written_as_an_empty_design() {
    let mut model = placed_twice();
    if let Some(assembly) = &mut model.assembly {
        assembly.parts.clear();
        assembly.placements.clear();
    }
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    assert_eq!(writer.convertible(&model), Ok(()));
    let (bytes, report) = written(&model);
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(document["features"], serde_json::json!([]));
    assert_eq!(
        report.outcome().map(|outcome| outcome.check.clone()),
        Some(dutchman_core::Check::NotApplicable(
            "the assembly places no parts".to_owned()
        ))
    );
}

#[test]
fn an_assembly_becomes_components_and_occurrences() {
    let (bytes, report) = written(&placed_twice());
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let blobs = (0..archive.len())
        .filter(|i| {
            archive
                .by_index(*i)
                .unwrap()
                .name()
                .unwrap()
                .starts_with("blobs/")
        })
        .count();
    assert_eq!(blobs, 1, "one part, stored once");
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();

    assert_eq!(document["name"], "pair");
    assert_eq!(
        document["components"],
        serde_json::json!([{"id": 1, "name": "synthetic_cube", "parent": 0}])
    );
    let occurrences = document["occurrences"].as_array().unwrap();
    assert_eq!(occurrences.len(), 2);
    assert_eq!(occurrences[0]["name"], "synthetic_cube:1");
    assert_eq!(occurrences[1]["name"], "Moved");
    assert_eq!(occurrences[1]["component"], 1);
    // Column-major: the translation is the fourth column.
    assert_eq!(
        occurrences[1]["transform"][3],
        serde_json::json!([30.0, 0.0, 0.0, 1.0])
    );
    let import = &document["features"][0];
    assert_eq!(import["type"], "import");
    assert_eq!(import["component"], 1);
    assert_eq!(import["file"], "synthetic_cube.ipt");
    assert_eq!(document["next_id"], 5);

    let outcome = report.outcome().unwrap();
    assert_eq!(outcome.mode, dutchman_core::Mode::Import);
}

#[test]
fn solidcraft_places_each_occurrence() {
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        eprintln!("skipped: set {CLI_ENV} to open the assembly in SolidCraft");
        return;
    };
    let (bytes, _) = written(&placed_twice());
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("pair.solidcraft");
    let output = dir.path().join("eval.json");
    std::fs::write(&input, bytes).unwrap();
    let status = std::process::Command::new(cli)
        .arg("eval")
        .arg(&input)
        .arg("--json-out")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    let result: Value = serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    let bodies = result["bodies"].as_array().unwrap();
    assert_eq!(bodies.len(), 2, "{bodies:?}");
    let min_x = |body: &Value| body["bbox"]["min"][0].as_f64().unwrap();
    assert!(
        (min_x(&bodies[1]) - min_x(&bodies[0]) - 30.0).abs() < 1e-6,
        "{bodies:?}"
    );
}

/// The pair as a sub-assembly, placed twice in a top assembly beside one loose cube.
fn nested() -> Model {
    let pair = placed_twice();
    let cube = pair.assembly.as_ref().unwrap().parts[0].clone();
    let at = |x: f64, y: f64| [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, 0.0]];
    Model {
        name: "top".to_owned(),
        source_file: Some("top.iam".to_owned()),
        source_step: None,
        assembly: Some(Assembly {
            own_part: None,
            joints: Vec::new(),
            left_out: Vec::new(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
            parts: vec![pair, cube.clone()],
            placements: vec![
                Placement {
                    hidden: false,
                    name: None,
                    part: 0,
                    transform: at(0.0, 0.0),
                },
                Placement {
                    hidden: false,
                    name: None,
                    part: 0,
                    transform: at(0.0, 50.0),
                },
                Placement {
                    hidden: false,
                    name: None,
                    part: 1,
                    transform: at(100.0, 0.0),
                },
            ],
        }),
        ..cube
    }
}

#[test]
fn a_sub_assembly_becomes_a_component_with_its_parts_beneath_it() {
    let (bytes, _) = written(&nested());
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let components: Vec<(String, u64)> = document["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap().to_owned(),
                c["parent"].as_u64().unwrap(),
            )
        })
        .collect();
    let pair = document["components"][0]["id"].as_u64().unwrap();
    assert_eq!(
        components,
        [
            ("pair".to_owned(), 0),
            ("synthetic_cube".to_owned(), pair),
            ("synthetic_cube (2)".to_owned(), 0)
        ]
    );
    let occurrences = document["occurrences"].as_array().unwrap();
    assert_eq!(occurrences.len(), 5);
    let inside_pair = occurrences.iter().filter(|o| o["parent"] == pair).count();
    assert_eq!(inside_pair, 2);
    assert_eq!(document["features"].as_array().unwrap().len(), 2);
}

#[test]
fn solidcraft_composes_nested_placements() {
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        eprintln!("skipped: set {CLI_ENV} to open the assembly in SolidCraft");
        return;
    };
    let (bytes, _) = written(&nested());
    let bodies = evaluate(&cli, &bytes);
    let mut corners: Vec<(i64, i64)> = bodies
        .iter()
        .map(|b| {
            let min = &b["bbox"]["min"];
            let max = &b["bbox"]["max"];
            // Corner-independent: the box's centre, rounded to whole millimetres.
            let centre = |axis: usize| {
                ((min[axis].as_f64().unwrap() + max[axis].as_f64().unwrap()) / 2.0).round() as i64
            };
            (centre(0), centre(1))
        })
        .collect();
    corners.sort_unstable();
    let base = corners[0];
    let offsets: Vec<(i64, i64)> = corners
        .iter()
        .map(|(x, y)| (x - base.0, y - base.1))
        .collect();
    assert_eq!(
        offsets,
        [(0, 0), (0, 50), (30, 0), (30, 50), (100, 0)],
        "{bodies:?}"
    );
}

fn evaluate(cli: &Path, design: &[u8]) -> Vec<Value> {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("design.solidcraft");
    let output = dir.path().join("eval.json");
    std::fs::write(&input, design).unwrap();
    let status = std::process::Command::new(cli)
        .arg("eval")
        .arg(&input)
        .arg("--json-out")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    let result: Value = serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    result["bodies"].as_array().unwrap().clone()
}

/// A design (CADIR built here from the synthetic cube) whose component owns the cube and is
/// placed at the origin and 30 mm along X, as a Fusion design with components would be. With
/// `nested`, both placements are inside an occurrence of the design's root component placed
/// 100 mm along X, as cadmpeg places the components a Fusion root component contains.
fn with_components(nested: bool) -> Model {
    let part = placed_twice().assembly.unwrap().parts[0].clone();
    let mut wire = serde_json::to_value(&part.ir).unwrap();
    let body = wire["model"]["bodies"][0]["id"].clone();
    wire["model"]["product_definitions"] = serde_json::json!([
        {"id": "f3d:model:component#block", "kind": "part", "label": "Block", "bodies": [body]},
        {"id": "f3d:model:component#root", "kind": "part", "label": "Top", "bodies": []},
    ]);
    let placed = |key: u32, definition: &str, x: f64, parent: Value| {
        serde_json::json!({
            "id": format!("f3d:model:occurrence#{key}"),
            "prototype": {"scope": "local", "definition": format!("f3d:model:component#{definition}")},
            "parent": parent,
            "ordinal": key,
            "transform": [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
            "scale": [1.0, 1.0, 1.0],
        })
    };
    let root = serde_json::json!({"kind": "root"});
    wire["model"]["occurrences"] = if nested {
        let inside =
            serde_json::json!({"kind": "occurrence", "occurrence": "f3d:model:occurrence#9"});
        serde_json::json!([
            placed(9, "root", 100.0, root),
            placed(0, "block", 0.0, inside.clone()),
            placed(1, "block", 30.0, inside),
        ])
    } else {
        serde_json::json!([
            placed(0, "block", 0.0, root.clone()),
            placed(1, "block", 30.0, root)
        ])
    };
    let mut model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        name: "blocks".to_owned(),
        ..part
    };
    model.assembly = dutchman_core::local_assembly(&model).unwrap();
    model
}

#[test]
fn a_design_with_components_becomes_components_and_occurrences() {
    let (bytes, _) = written(&with_components(false));
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(document["components"][0]["name"], "Block");
    assert_eq!(document["occurrences"].as_array().unwrap().len(), 2);
}

/// [`with_components`], with two sketches of the design's own: a 20 mm line on XY dimensioned
/// by parameter `span`, and one whose plane is not resolved; and parameter `spare`, which no
/// sketch uses.
fn with_sketches() -> Model {
    let model = with_components(false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let sketch = |key: u32, placement: Value| {
        serde_json::json!({"id": format!("f3d:model:sketch#{key}"), "name": format!("Layout {key}"),
                           "visible": false, "placement": placement})
    };
    wire["model"]["sketches"] = serde_json::json!([
        sketch(
            1,
            serde_json::json!({"kind": "resolved", "frame": {
                "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
                "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
                "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0},
            }})
        ),
        sketch(2, serde_json::json!({"kind": "unresolved"})),
    ]);
    wire["model"]["sketch_entities"] = serde_json::json!([{
        "id": "f3d:model:sketch-entity#1",
        "sketch": "f3d:model:sketch#1",
        "construction": false,
        "geometry": {"kind": "line", "start": {"u": 0.0, "v": 0.0}, "end": {"u": 20.0, "v": 0.0}},
    }]);
    wire["model"]["sketch_constraints"] = serde_json::json!([{
        "id": "f3d:model:sketch-constraint#1",
        "sketch": "f3d:model:sketch#1",
        "definition": {"kind": "distance", "entities": ["f3d:model:sketch-entity#1"],
                       "parameter": "f3d:model:parameter#1"},
    }]);
    let parameter = |key: u32, name: &str, value: f64| {
        serde_json::json!({"id": format!("f3d:model:parameter#{key}"), "name": name,
                           "expression": format!("{value} mm"),
                           "value": {"kind": "length", "value": value}})
    };
    wire["model"]["parameters"] =
        serde_json::json!([parameter(1, "span", 20.0), parameter(2, "spare", 5.0)]);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    }
}

#[test]
fn an_assemblys_own_sketches_are_kept_hidden_in_the_design() {
    let (bytes, report) = written(&with_sketches());
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let sketches: Vec<&Value> = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|feature| feature["type"] == "sketch")
        .collect();
    assert_eq!(sketches.len(), 1, "the unresolved one is left out");
    let sketch = sketches[0];
    assert_eq!(sketch["name"], "Layout 1");
    assert_eq!(sketch["component"], 0, "the design's own, not a part's");
    assert_eq!(sketch["sketch"]["curves"].as_array().unwrap().len(), 1);
    assert_eq!(
        document["visibility"]["hidden_sketches"],
        serde_json::json!([sketch["id"]])
    );
    assert!(sketch["id"].as_u64() < document["next_id"].as_u64());
    let params: Vec<&str> = document["params"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|param| param["name"].as_str())
        .collect();
    assert_eq!(params, ["span"], "only the parameter the sketch uses");
    assert!(report.losses().iter().any(|loss| loss.subject == "sketches"
        && loss.detail == "kept 1 sketch in the design itself, for reference; 1 left out"));
}

/// [`with_sketches`] with a third sketch, resolved like the first, and component `Block`
/// listing the first as its own.
fn with_owned_sketch() -> Model {
    let model = with_sketches();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["product_definitions"][0]["sketches"] = serde_json::json!(["f3d:model:sketch#1"]);
    let mut third = wire["model"]["sketches"][0].clone();
    third["id"] = "f3d:model:sketch#3".into();
    third["name"] = "Layout 3".into();
    wire["model"]["sketches"]
        .as_array_mut()
        .unwrap()
        .push(third);
    let mut model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    };
    model.assembly = dutchman_core::local_assembly(&model).unwrap();
    model
}

#[test]
fn a_sketch_a_component_owns_is_written_once_under_it_and_the_rest_stay_at_the_root() {
    let (bytes, report) = written(&with_owned_sketch());
    let document = document(&bytes);
    let block = document["components"][0]["id"].clone();
    assert_eq!(document["components"][0]["name"], "Block");
    assert_eq!(
        document["occurrences"].as_array().unwrap().len(),
        2,
        "Block is placed twice"
    );
    let sketches: Vec<(&str, &Value)> = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|feature| feature["type"] == "sketch")
        .map(|feature| (feature["name"].as_str().unwrap(), &feature["component"]))
        .collect();
    assert_eq!(sketches.len(), 2, "the unresolved one is left out");
    assert!(sketches.contains(&("Layout 1", &block)), "{sketches:?}");
    assert!(
        sketches.contains(&("Layout 3", &Value::from(0))),
        "{sketches:?}"
    );
    // Reference sketches start hidden, wherever they are.
    let hidden = document["visibility"]["hidden_sketches"]
        .as_array()
        .unwrap();
    assert_eq!(hidden.len(), 2);
    assert!(report.losses().iter().any(|loss| loss.subject == "sketches"
        && loss.detail == "kept 1 sketch in the design itself, for reference; 1 left out"));
}

/// [`with_components`] nested (component `Top`, which holds no bodies, places `Block` twice),
/// with three resolved sketches: one `Top` owns, one a component the design does not place
/// owns, and one with no owner.
fn with_sketch_of_a_component_without_bodies() -> Model {
    let model = with_components(true);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let ids = [
        "f3d:model:sketch#1",
        "f3d:model:sketch#2",
        "f3d:model:sketch#3",
    ];
    wire["model"]["sketches"] = serde_json::json!(ids
        .iter()
        .enumerate()
        .map(|(n, id)| serde_json::json!({
            "id": id, "name": format!("Layout {}", n + 1),
            "placement": {"kind": "resolved", "frame": {
                "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
                "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
                "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0},
            }},
        }))
        .collect::<Vec<_>>());
    wire["model"]["sketch_entities"] = serde_json::json!(ids
        .iter()
        .enumerate()
        .map(|(n, id)| serde_json::json!({
            "id": format!("f3d:model:sketch-entity#{n}"), "sketch": id, "construction": false,
            "geometry": {"kind": "line", "start": {"u": 0.0, "v": 0.0}, "end": {"u": 20.0, "v": 0.0}},
        }))
        .collect::<Vec<_>>());
    let definitions = wire["model"]["product_definitions"].as_array_mut().unwrap();
    definitions[1]["sketches"] = serde_json::json!([ids[0]]);
    definitions.push(serde_json::json!({
        "id": "f3d:model:component#ghost", "kind": "part", "label": "Ghost",
        "bodies": [], "sketches": [ids[1]],
    }));
    let mut model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    };
    model.assembly = dutchman_core::local_assembly(&model).unwrap();
    model
}

#[test]
fn a_sketch_of_a_component_without_bodies_sits_under_that_component() {
    let (bytes, report) = written(&with_sketch_of_a_component_without_bodies());
    let document = document(&bytes);
    let top = document["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Top")
        .expect("Top is written")["id"]
        .clone();
    let sketches: Vec<(&str, &Value)> = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|feature| feature["type"] == "sketch")
        .map(|feature| (feature["name"].as_str().unwrap(), &feature["component"]))
        .collect();
    assert_eq!(sketches.len(), 3, "{sketches:?}");
    assert!(sketches.contains(&("Layout 1", &top)), "{sketches:?}");
    assert!(
        sketches.contains(&("Layout 2", &Value::from(0))),
        "{sketches:?}"
    );
    assert!(
        sketches.contains(&("Layout 3", &Value::from(0))),
        "{sketches:?}"
    );
    assert!(report.losses().iter().any(|loss| loss.subject == "sketches"
        && loss.detail
            == "kept 2 sketches in the design itself, for reference; \
                1 owned by a component the design does not write"));
    assert!(report.losses().iter().any(|loss| loss.subject == "sketches"
        && loss.detail == "kept 1 sketch under the component that holds them, for reference"));
}

#[test]
fn solidcraft_replays_an_assembly_whose_sketch_sits_under_its_component() {
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        eprintln!("skipped: set {CLI_ENV} to check the sketches in SolidCraft");
        return;
    };
    let (bytes, _) = written(&with_owned_sketch());
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("design.solidcraft");
    let output = dir.path().join("eval.json");
    std::fs::write(&input, bytes).unwrap();
    let status = std::process::Command::new(cli)
        .arg("eval")
        .arg(&input)
        .arg("--json-out")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    let result: Value = serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    let sketches = result["sketches"].as_array().unwrap();
    assert_eq!(sketches.len(), 2);
    assert!(sketches.iter().all(|s| s["status"] == "solved"));
    assert!(sketches.iter().all(|s| s["visible"] == false));
    assert_eq!(result["bodies"].as_array().unwrap().len(), 2);
}

#[test]
fn solidcraft_solves_an_assemblys_own_sketch_and_keeps_it_hidden() {
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        eprintln!("skipped: set {CLI_ENV} to check the sketch in SolidCraft");
        return;
    };
    let (bytes, _) = written(&with_sketches());
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("design.solidcraft");
    let output = dir.path().join("eval.json");
    std::fs::write(&input, bytes).unwrap();
    let status = std::process::Command::new(cli)
        .arg("eval")
        .arg(&input)
        .arg("--json-out")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success());
    let result: Value = serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    let sketch = &result["sketches"][0];
    assert_eq!(sketch["name"], "Layout 1");
    assert_eq!(sketch["status"], "solved");
    assert_eq!(sketch["visible"], false);
    assert_eq!(result["bodies"].as_array().unwrap().len(), 2);
}

#[test]
fn solidcraft_places_each_component_occurrence() {
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        eprintln!("skipped: set {CLI_ENV} to open the design in SolidCraft");
        return;
    };
    for nested in [false, true] {
        let (bytes, _) = written(&with_components(nested));
        let bodies = evaluate(&cli, &bytes);
        assert_eq!(bodies.len(), 2, "{bodies:?}");
        let mut min_x: Vec<f64> = bodies
            .iter()
            .map(|body| body["bbox"]["min"][0].as_f64().unwrap())
            .collect();
        min_x.sort_by(f64::total_cmp);
        assert!((min_x[1] - min_x[0] - 30.0).abs() < 1e-6, "{bodies:?}");
        if nested {
            // The cube's own minimum, moved by the outer occurrence.
            let (flat, _) = written(&with_components(false));
            let flat_min = evaluate(&cli, &flat)
                .iter()
                .map(|body| body["bbox"]["min"][0].as_f64().unwrap())
                .fold(f64::INFINITY, f64::min);
            assert!((min_x[0] - flat_min - 100.0).abs() < 1e-6, "{bodies:?}");
        }
    }
}

#[test]
fn components_inside_a_component_stay_beneath_it() {
    let (bytes, _) = written(&with_components(true));
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let components = document["components"].as_array().unwrap();
    let id_of = |name: &str| {
        components
            .iter()
            .find(|c| c["name"] == name)
            .unwrap_or_else(|| panic!("{name} in {components:?}"))["id"]
            .clone()
    };
    let (top, block) = (id_of("Top"), id_of("Block"));
    assert_eq!(
        components.iter().find(|c| c["id"] == block).unwrap()["parent"],
        top
    );
    let occurrences = document["occurrences"].as_array().unwrap();
    assert_eq!(occurrences.len(), 3);
    let inside: Vec<&Value> = occurrences.iter().filter(|o| o["parent"] == top).collect();
    assert_eq!(inside.len(), 2);
    assert!(inside.iter().all(|o| o["component"] == block));
}

/// Two blocks inside the root component's occurrence (placed 100 mm along X), the second 30 mm
/// further and turned a quarter turn about Z, held by a fixed joint, and the root occurrence
/// fixed to the design itself.
fn jointed() -> Model {
    let mut model = with_components(true);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["occurrences"][2]["transform"] = serde_json::json!([
        [0.0, -1.0, 0.0, 30.0],
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0]
    ]);
    // Each connector frame is in its own part, placed so the joints hold the parts where they
    // are stored, the second facing the first (half a turn about x): A's origin seen from B
    // (30 mm on, a quarter turn round) is 30 mm along B's +Y, turned back; the root's origin
    // seen from Top (at x = 100) is 100 mm back along X.
    let frame = |key: Option<u32>| match key {
        Some(1) => serde_json::json!([
            [0.0, -1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0, 30.0],
            [0.0, 0.0, -1.0, 0.0]
        ]),
        Some(9) => serde_json::json!([
            [1.0, 0.0, 0.0, -100.0],
            [0.0, -1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0, 0.0]
        ]),
        _ => serde_json::json!([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0]
        ]),
    };
    let side = |key: Option<u32>| {
        let operand = match key {
            None => {
                serde_json::json!({"container": {"container": "root"}, "object": "root:origin"})
            }
            Some(key) => serde_json::json!({
                "container": {"container": "occurrence", "occurrence": format!("f3d:model:occurrence#{key}")},
                "object": key.to_string(),
            }),
        };
        serde_json::json!({"operand": operand, "frame": frame(key), "detached": false})
    };
    let joint = |key: u32, a: Value, b: Value| {
        serde_json::json!({
            "id": format!("f3d:model:joint#{key}"),
            "operands": {"arity": "pair", "kind": {"kind": "fixed"}, "connectors": [a, b]},
            "suppressed": false,
        })
    };
    wire["model"]["assembly_joints"] = serde_json::json!([
        joint(1, side(Some(0)), side(Some(1))),
        joint(2, side(None), side(Some(9))),
    ]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model.assembly = dutchman_core::local_assembly(&model).unwrap();
    model
}

#[test]
fn fixed_joints_become_rigid_joints_that_hold_the_parts_as_placed() {
    let (bytes, report) = written(&jointed());
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let joints = document["assembly"]["joints"].as_array().unwrap();
    assert_eq!(joints.len(), 2);
    assert!(joints
        .iter()
        .all(|j| j["kind"] == "rigid" && j["flip"] == false));
    // The blocks: A's frame is its own origin; B's is the same place seen from B, which sits
    // 30 mm further along X, turned a quarter turn: A's origin is 30 mm along B's +Y.
    let blocks = &joints[0];
    assert_eq!(
        blocks["a"]["snap"]["origin"],
        serde_json::json!([0.0, 0.0, 0.0])
    );
    let origin: Vec<f64> = blocks["b"]["snap"]["origin"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert!(
        (origin[0]).abs() < 1e-9 && (origin[1] - 30.0).abs() < 1e-9 && origin[2].abs() < 1e-9,
        "{origin:?}"
    );
    // The root occurrence, fixed to the design itself (occurrence 0).
    assert_eq!(joints[1]["a"]["occurrence"], 0);
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.subject == "joints" && loss.detail.starts_with("2 joint(s)")));
}

#[test]
fn solidcraft_reads_each_joint_as_built_and_solves_without_moving_anything() {
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        eprintln!("skipped: set {CLI_ENV} to open the design in SolidCraft");
        return;
    };
    let (bytes, _) = written(&jointed());
    let before = evaluate(&cli, &bytes);

    let dir = tempfile::tempdir().unwrap();
    let design = dir.path().join("design.solidcraft");
    let solved = dir.path().join("solved.solidcraft");
    let script = dir.path().join("solve.json");
    let results = dir.path().join("results.json");
    std::fs::write(&design, &bytes).unwrap();
    std::fs::write(
        &script,
        r#"[{"command":"joint.list","params":{}},{"command":"joint.solve","params":{}}]"#,
    )
    .unwrap();
    let status = std::process::Command::new(&cli)
        .arg("run")
        .arg(&script)
        .arg("--in")
        .arg(&design)
        .arg("--save")
        .arg(&solved)
        .arg("--results")
        .arg("--json-out")
        .arg(&results)
        .status()
        .unwrap();
    assert!(status.success());
    let results: Value = serde_json::from_str(&std::fs::read_to_string(results).unwrap()).unwrap();
    // As built: each joint's two frames, as SolidCraft places them in the world, meet face to
    // face: the same origin, opposite z axes.
    let joints = results[0]["joints"].as_array().unwrap();
    assert_eq!(joints.len(), 2, "{results}");
    for joint in joints {
        assert_eq!(joint["type"], "rigid");
        for (key, sign) in [("origin", 1.0), ("z", -1.0)] {
            let a = joint["frame_a"][key].as_array().unwrap();
            let b = joint["frame_b"][key].as_array().unwrap();
            for (a, b) in a.iter().zip(b) {
                assert!(
                    (a.as_f64().unwrap() - sign * b.as_f64().unwrap()).abs() < 1e-9,
                    "{}: {key} {a} vs {b}",
                    joint["name"]
                );
            }
        }
    }
    let solve = &results[1];
    assert_eq!(solve["conflicts"], serde_json::json!([]), "{solve}");
    assert_eq!(solve["errors"], serde_json::json!([]), "{solve}");

    let after = evaluate(&cli, &std::fs::read(&solved).unwrap());
    let boxes = |bodies: &[Value]| {
        let mut boxes: Vec<String> = bodies.iter().map(|b| b["bbox"].to_string()).collect();
        boxes.sort();
        boxes
    };
    assert_eq!(boxes(&before), boxes(&after));
    assert_eq!(before.len(), 2);
}

/// The two blocks at the root (A at x 0–10, B at x 30–40), A fixed to the design, and B joined
/// to A by `kind` about a vertical axis through (20, 0, 0), each connector frame given in its
/// own part, the second facing the first.
fn hinged(kind: Value) -> Model {
    let mut model = with_components(false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    // The hinge at (20, 0, 0): 20 mm along A (at the origin), 10 mm back from B (at x = 30).
    let at = |x: f64| {
        serde_json::json!([
            [1.0, 0.0, 0.0, x],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0]
        ])
    };
    // Half a turn about x, as the second frame faces the first.
    let facing = |x: f64| {
        serde_json::json!([
            [1.0, 0.0, 0.0, x],
            [0.0, -1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0, 0.0]
        ])
    };
    let side = |key: Option<u32>, at: Value| {
        let operand = match key {
            None => {
                serde_json::json!({"container": {"container": "root"}, "object": "root:origin"})
            }
            Some(key) => serde_json::json!({
                "container": {"container": "occurrence", "occurrence": format!("f3d:model:occurrence#{key}")},
                "object": key.to_string(),
            }),
        };
        serde_json::json!({"operand": operand, "frame": at, "detached": false})
    };
    let joint = |key: u32, kind: Value, a: Value, b: Value| {
        serde_json::json!({
            "id": format!("f3d:model:joint#{key}"),
            "operands": {"arity": "pair", "kind": kind, "connectors": [a, b]},
            "suppressed": false,
        })
    };
    wire["model"]["assembly_joints"] = serde_json::json!([
        joint(
            1,
            serde_json::json!({"kind": "fixed"}),
            side(None, at(0.0)),
            side(Some(0), facing(0.0))
        ),
        joint(
            2,
            kind,
            side(Some(0), at(20.0)),
            side(Some(1), facing(-10.0))
        ),
    ]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model.assembly = dutchman_core::local_assembly(&model).unwrap();
    model
}

#[test]
fn a_held_placement_is_written_grounded() {
    let mut model = placed_twice();
    if let Some(assembly) = &mut model.assembly {
        assembly.held = vec![vec![1]];
    }
    let (bytes, _) = written(&model);
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let grounded: Vec<&Value> = document["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|occurrence| occurrence["grounded"] == true)
        .map(|occurrence| &occurrence["name"])
        .collect();
    assert_eq!(grounded, [&serde_json::json!("Moved")]);
}

#[test]
fn joints_tied_to_nothing_ground_one_part_and_keep_their_frame_sense() {
    // The hinge alone, without the joint holding its first part to the design, and with its
    // frames aligned: its fixed side is grounded, and it is written flipped.
    let mut model = hinged(serde_json::json!({"kind": "revolute"}));
    if let Some(assembly) = &mut model.assembly {
        assembly.joints.remove(0);
        assembly.joints[0].flip = true;
    }
    let (bytes, report) = written(&model);
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let hinge = &document["assembly"]["joints"][0];
    assert_eq!(hinge["flip"], true);
    let grounded: Vec<&Value> = document["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|occurrence| occurrence["grounded"] == true)
        .map(|occurrence| &occurrence["id"])
        .collect();
    assert_eq!(grounded, [&hinge["b"]["occurrence"]]);
    assert!(
        report.losses().iter().any(|loss| loss.subject == "joints"
            && loss.detail.contains("is grounded, inferred")
            && loss.detail.contains("\"Revolute 1\"")),
        "{report:?}"
    );

    // Held to the design, nothing is grounded: the design holds the parts.
    let (bytes, _) = written(&hinged(serde_json::json!({"kind": "revolute"})));
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    assert!(document["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .all(|occurrence| occurrence["grounded"] == false));
}

#[test]
fn revolute_and_slider_joints_are_written_with_their_motion() {
    let (bytes, report) = written(&hinged(serde_json::json!({"kind": "revolute",
        "angular_limits": {"bounds": "range", "minimum": -1.0, "maximum": 2.0}})));
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let hinge = &document["assembly"]["joints"][1];
    assert_eq!(hinge["kind"], "revolute");
    assert_eq!(hinge["name"], "Revolute 1");
    assert_eq!(hinge["values"], serde_json::json!([0.0]));
    assert_eq!(hinge["limits"], serde_json::json!([[-1.0, 2.0]]));
    // A's frame is at the hinge in A's coordinates (A is at the origin); B's in B's, 30 mm on.
    assert_eq!(
        hinge["a"]["snap"]["origin"],
        serde_json::json!([20.0, 0.0, 0.0])
    );
    assert_eq!(
        hinge["b"]["snap"]["origin"],
        serde_json::json!([-10.0, 0.0, 0.0])
    );
    assert!(report.losses().iter().any(|loss| loss.detail
        == "2 joint(s) written as built (1 revolute, 1 rigid), holding their parts as placed"));
    let kinds: Vec<(&str, u64)> = report
        .joints()
        .iter()
        .map(|(kind, n)| (kind.as_str(), *n))
        .collect();
    assert_eq!(kinds, [("revolute", 1), ("rigid", 1)]);
}

#[test]
fn joint_offsets_keep_the_written_snaps_closed() {
    for (kind, expected_origin, angle) in [
        (
            json!({"kind": "fixed", "translation_offset": [5.0, 7.0, 9.0], "angle": 0.5}),
            [-5.0, -3.0, -9.0],
            0.5_f64,
        ),
        (
            json!({"kind": "revolute", "angle": 0.5}),
            [-10.0, 4.0, 0.0],
            0.5_f64,
        ),
        (
            json!({"kind": "slider", "translation_offset": [5.0, 7.0, 9.0], "distance": 7.0}),
            [-5.0, -3.0, -9.0],
            0.0_f64,
        ),
    ] {
        for sense in ["opposed", "aligned"] {
            let mut model = hinged(kind.clone());
            let mut wire = serde_json::to_value(&model.ir).unwrap();
            wire["model"]["assembly_joints"][1]["operands"]["frame_sense"] = json!(sense);
            // Start off the joint axis so each kind must place the unposed second part.
            wire["model"]["assembly_joints"][1]["operands"]["connectors"][1]["frame"][1][3] =
                json!(4.0);
            model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
            model.assembly = dutchman_core::local_assembly(&model).unwrap();
            let (bytes, _) = written(&model);
            let doc = document(&bytes);
            let joint = &doc["assembly"]["joints"][1];
            assert_eq!(joint["flip"], sense == "aligned");
            assert_ne!(joint["suppressed"], true);
            assert_eq!(joint["a"]["snap"]["origin"], json!([20.0, 0.0, 0.0]));
            assert_eq!(joint["a"]["snap"]["x"], json!([1.0, 0.0, 0.0]));
            assert_eq!(joint["b"]["snap"]["origin"], json!(expected_origin));
            assert_eq!(joint["b"]["snap"]["z"], json!([0.0, 0.0, -1.0]));
            // The second connector faces down Z, so its Rz(angle) turns x toward -Y.
            let x = joint["b"]["snap"]["x"].as_array().unwrap();
            for (actual, expected) in x.iter().zip([angle.cos(), -angle.sin(), 0.0]) {
                assert!(
                    (actual.as_f64().unwrap() - expected).abs() < 1e-9,
                    "{joint}"
                );
            }
            if kind["kind"] == "slider" {
                assert_eq!(
                    joint["values"],
                    json!([if sense == "aligned" { -7.0 } else { 7.0 }])
                );
            }
            let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
                continue;
            };
            let dir = tempfile::tempdir().unwrap();
            let design = dir.path().join("design.sc");
            let solved = dir.path().join("solved.sc");
            let script = dir.path().join("solve.json");
            let results = dir.path().join("results.json");
            std::fs::write(&design, &bytes).unwrap();
            std::fs::write(&script, r#"[{"command":"joint.solve","params":{}}]"#).unwrap();
            let status = std::process::Command::new(&cli)
                .arg("run")
                .arg(&script)
                .arg("--in")
                .arg(&design)
                .arg("--save")
                .arg(&solved)
                .arg("--results")
                .arg("--json-out")
                .arg(&results)
                .status()
                .unwrap();
            assert!(status.success(), "{kind}, {sense}");
            let results: Value =
                serde_json::from_reader(std::fs::File::open(results).unwrap()).unwrap();
            assert_eq!(
                results[0]["conflicts"],
                json!([]),
                "{kind}, {sense}: {results}"
            );
            assert_eq!(
                results[0]["errors"],
                json!([]),
                "{kind}, {sense}: {results}"
            );
            let after = document(&std::fs::read(solved).unwrap());
            for before in doc["occurrences"].as_array().unwrap() {
                let now = after["occurrences"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["id"] == before["id"])
                    .unwrap();
                for (old, new) in before["transform"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(now["transform"].as_array().unwrap())
                {
                    for (old, new) in old.as_array().unwrap().iter().zip(new.as_array().unwrap()) {
                        assert!(
                            (old.as_f64().unwrap() - new.as_f64().unwrap()).abs() < 1e-6,
                            "{kind}, {sense}: {before} -> {now}"
                        );
                    }
                }
            }
        }
    }
}

/// A grounded base and a hinge, both turned a quarter turn about Z and translated.
fn grounded_hinge() -> Model {
    let mut model = hinged(serde_json::json!({"kind": "revolute"}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let connector = wire["model"]["assembly_joints"][0]["operands"]["connectors"][1].clone();
    wire["model"]["assembly_joints"][0]["operands"] = serde_json::json!({
        "arity": "grounded", "connector": connector,
    });
    for (index, y) in [5.0, 35.0].into_iter().enumerate() {
        wire["model"]["occurrences"][index]["transform"] = serde_json::json!([
            [0.0, -1.0, 0.0, 15.0],
            [1.0, 0.0, 0.0, y],
            [0.0, 0.0, 1.0, 7.0],
        ]);
    }
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model.assembly = dutchman_core::local_assembly(&model).unwrap();
    model
}

#[test]
fn a_grounded_joint_fixes_its_occurrence_and_keeps_the_revolute_joint() {
    let model = grounded_hinge();
    let assembly = model.assembly.as_ref().unwrap();
    assert!(assembly.left_out.is_empty(), "{:?}", assembly.left_out);
    assert_eq!(assembly.joints.len(), 2);
    let ground = &assembly.joints[0];
    assert!(ground.a.is_empty());
    assert_eq!(ground.b, [0]);
    assert_eq!(ground.motion, dutchman_core::Motion::Rigid);
    assert_eq!(ground.frame_a, assembly.placements[0].transform);

    let (bytes, report) = written(&model);
    assert!(
        report
            .losses()
            .iter()
            .all(|loss| loss.severity != dutchman_core::Severity::Dropped),
        "{report:?}"
    );
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    let document: Value = serde_json::from_str(&text).unwrap();
    let occurrences = document["occurrences"].as_array().unwrap();
    assert_eq!(occurrences.len(), 2);
    assert_eq!(
        occurrences[0]["transform"],
        serde_json::json!([
            [0.0, 1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [15.0, 5.0, 7.0, 1.0],
        ])
    );
    let joints = document["assembly"]["joints"].as_array().unwrap();
    assert_eq!(joints.len(), 2);
    assert_eq!(joints[0]["kind"], "rigid");
    assert_eq!(joints[0]["a"]["occurrence"], 0);
    assert_eq!(joints[0]["b"]["occurrence"], occurrences[0]["id"]);
    assert_eq!(joints[1]["kind"], "revolute");
    assert_eq!(joints[1]["a"]["occurrence"], occurrences[0]["id"]);
    assert_eq!(joints[1]["b"]["occurrence"], occurrences[1]["id"]);
    assert_eq!(report.joints().get("rigid"), Some(&1));
    assert_eq!(report.joints().get("revolute"), Some(&1));
}

#[test]
fn solidcraft_keeps_the_grounded_base_fixed_when_driving_its_hinge() {
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        eprintln!("skipped: set {CLI_ENV} to drive the joints in SolidCraft");
        return;
    };
    let (bytes, _) = written(&grounded_hinge());
    assert_eq!(
        driven(&cli, &bytes, "Revolute 1", 90.0),
        [[-5.0, 15.0, 7.0, 5.0, 25.0], [5.0, 5.0, 7.0, 15.0, 15.0]]
    );
}

/// Drives the joint named `joint` to `value` (degrees, or mm for a slider) in SolidCraft and
/// returns each body's bounding box after: min x, y, z, max x, y.
fn driven(cli: &Path, design: &[u8], joint: &str, value: f64) -> Vec<[f64; 5]> {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("design.solidcraft");
    let moved = dir.path().join("moved.solidcraft");
    let script = dir.path().join("drive.json");
    std::fs::write(&input, design).unwrap();
    std::fs::write(
        &script,
        serde_json::json!([{"command": "joint.drive", "params": {"joint": joint, "value": value}}])
            .to_string(),
    )
    .unwrap();
    let status = std::process::Command::new(cli)
        .arg("run")
        .arg(&script)
        .arg("--in")
        .arg(&input)
        .arg("--save")
        .arg(&moved)
        .status()
        .unwrap();
    assert!(status.success());
    let mut boxes: Vec<[f64; 5]> = evaluate(cli, &std::fs::read(&moved).unwrap())
        .iter()
        .map(|b| {
            let c = |k: &str, i: usize| (b["bbox"][k][i].as_f64().unwrap() * 1e6).round() / 1e6;
            [
                c("min", 0),
                c("min", 1),
                c("min", 2),
                c("max", 0),
                c("max", 1),
            ]
        })
        .collect();
    boxes.sort_by(|a, b| a.partial_cmp(b).unwrap());
    boxes
}

#[test]
fn solidcraft_moves_the_part_when_the_joint_is_driven() {
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        eprintln!("skipped: set {CLI_ENV} to drive the joints in SolidCraft");
        return;
    };
    // A quarter turn about the hinge at (20, 0): B swings from x 30–40, y 0–10 to x 10–20,
    // y 10–20; A, fixed to the design, stays at the origin.
    let revolute = written(&hinged(serde_json::json!({"kind": "revolute"}))).0;
    assert_eq!(
        driven(&cli, &revolute, "Revolute 1", 90.0),
        [[0.0, 0.0, 0.0, 10.0, 10.0], [10.0, 10.0, 0.0, 20.0, 20.0]]
    );
    // A slider along the hinge's z: B rises 5 mm and stays over x 30–40.
    let slider = written(&hinged(serde_json::json!({"kind": "slider"}))).0;
    assert_eq!(
        driven(&cli, &slider, "Slider 1", 5.0),
        [[0.0, 0.0, 0.0, 10.0, 10.0], [30.0, 0.0, 5.0, 40.0, 10.0]]
    );
}

#[test]
fn a_joined_part_is_placed_where_its_joint_puts_it_not_where_it_was_stored() {
    // Store B 40 mm further on than its joint puts it, as an unsolved placement would be.
    let mut model = hinged(serde_json::json!({"kind": "revolute"}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["occurrences"][1]["transform"][0][3] = serde_json::json!(70.0);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model.assembly = dutchman_core::local_assembly(&model).unwrap();
    let placed = &model.assembly.as_ref().unwrap().placements;
    assert_eq!(placed[1].transform[0][3], 30.0, "{placed:?}");

    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        eprintln!("skipped: set {CLI_ENV} to open the design in SolidCraft");
        return;
    };
    let (bytes, _) = written(&model);
    let mut starts: Vec<f64> = evaluate(&cli, &bytes)
        .iter()
        .map(|b| b["bbox"]["min"][0].as_f64().unwrap())
        .collect();
    starts.sort_by(f64::total_cmp);
    assert_eq!(starts, [0.0, 30.0]);
}

fn document_of(model: &Model) -> Value {
    let (bytes, _) = written(model);
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn a_placement_the_source_hides_is_written_hidden() {
    // The second cube is hidden in the source: it is still placed, and hidden.
    let mut model = placed_twice();
    if let Some(assembly) = &mut model.assembly {
        assembly.placements[1].hidden = true;
    }
    let document = document_of(&model);
    let moved = document["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["name"] == "Moved")
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    assert_eq!(document["occurrences"].as_array().unwrap().len(), 2);
    assert_eq!(
        document["visibility"]["hidden_occurrences"],
        serde_json::json!([[moved]])
    );

    // Inside a sub-assembly placed twice, it is hidden in each placement of it.
    let mut outer = placed_twice();
    if let Some(assembly) = &mut outer.assembly {
        let mut inner = model.clone();
        inner.name = "pair".to_owned();
        assembly.parts = vec![inner];
    }
    let document = document_of(&outer);
    let hidden = document["visibility"]["hidden_occurrences"]
        .as_array()
        .unwrap();
    assert_eq!(hidden.len(), 2, "{hidden:?}");
    assert!(hidden
        .iter()
        .all(|path| path.as_array().unwrap().len() == 2));
    assert!(placed_twice_shows_nothing_hidden());
}

fn placed_twice_shows_nothing_hidden() -> bool {
    document_of(&placed_twice())
        .get("visibility")
        .and_then(|v| v.get("hidden_occurrences"))
        .is_none()
}

#[test]
fn flattened_parts_keep_helper_info_once_per_part() {
    let mut model = placed_twice();
    let part = &mut model.assembly.as_mut().unwrap().parts[0];
    let mut wire = serde_json::to_value(&part.ir).unwrap();
    wire["model"]["features"] = serde_json::json!([
        {"id": "test:model:feature#helper", "ordinal": 0, "name": "Unused point",
            "suppressed": false, "definition": {"definition": "datum_point",
                "position": {"x": 0.0, "y": 0.0, "z": 0.0}}},
        {"id": "test:model:feature#later", "ordinal": 1, "name": "Later",
            "suppressed": false, "definition": {"definition": "native", "kind": "Mystery"}}
    ]);
    part.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = written(&model);
    let notes: Vec<_> = report
        .losses()
        .iter()
        .filter(|loss| loss.detail == "helper not converted: nothing uses it")
        .collect();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].severity, dutchman_core::Severity::Info);
    assert!(notes[0].subject.contains("Unused point"));
}

#[test]
fn freecad_external_body_links_replay_at_the_recipe_volumes_and_bounds() {
    if !Format::FreeCadDocument.can_convert() {
        return;
    }
    let input = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures/freecad-1.1.4-A01/A01a.FCStd");
    let model = read(Format::FreeCadDocument, &input, &mut Report::default()).unwrap();
    let (bytes, _) = written(&model);
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        return;
    };
    let mut bodies = evaluate(&cli, &bytes);
    assert_eq!(bodies.len(), 2);
    bodies.sort_by(|a, b| {
        a["bbox"]["min"][0]
            .as_f64()
            .unwrap()
            .total_cmp(&b["bbox"]["min"][0].as_f64().unwrap())
    });
    // CC0 recipe A01a, FreeCAD 1.1.4 manifest.json (0.01 mm, 1e-4 relative volume).
    for (body, volume, low, high) in [
        (&bodies[0], 6000.0, [-10.0, -15.0, 0.0], [10.0, 15.0, 10.0]),
        (
            &bodies[1],
            62831.8530718,
            [40.0, -30.0, 0.0],
            [100.0, 30.0, 40.0],
        ),
    ] {
        let actual = body["volume_mm3"].as_f64().unwrap();
        assert!((actual - volume).abs() <= volume * 1e-4, "{body}");
        for (axis, (low, high)) in low.into_iter().zip(high).enumerate() {
            assert!(
                (body["bbox"]["min"][axis].as_f64().unwrap() - low).abs() <= 0.01,
                "{body}"
            );
            assert!(
                (body["bbox"]["max"][axis].as_f64().unwrap() - high).abs() <= 0.01,
                "{body}"
            );
        }
    }
}

#[test]
fn sketch_and_joint_choices_leave_only_geometry_at_stored_placements() {
    let mut model = with_sketches();
    let assembly = model.assembly.as_mut().unwrap();
    assembly.stored_placements = assembly.placements.iter().map(|p| p.transform).collect();
    let stored = assembly.placements[1].transform;
    assembly.placements[1].transform[0][3] += 123.0;
    assembly.joints.push(dutchman_core::Joint {
        a: vec![0],
        b: vec![1],
        frame_a: dutchman_core::placement::IDENTITY,
        frame_b: dutchman_core::placement::IDENTITY,
        suppressed: false,
        motion: dutchman_core::Motion::Rigid,
        flip: true,
    });
    for history in [false, true] {
        let mut bytes = Vec::new();
        let mut report = Report::default();
        SolidCraftWriter::new()
            .import_only(!history)
            .sketches(false)
            .joints(false)
            .verification(Verification::Off)
            .write(&model, &mut bytes, &mut report)
            .unwrap();
        let doc = document(&bytes);
        assert!(doc.get("assembly").is_none());
        assert!(doc["features"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["type"] != "sketch"));
        assert_eq!(doc["occurrences"][1]["transform"][3][0], stored[0][3]);
        assert_eq!(
            report
                .losses()
                .iter()
                .filter(
                    |loss| loss.subject == "joints" && loss.detail.contains("left out by choice")
                )
                .count(),
            1
        );
        assert!(!report
            .losses()
            .iter()
            .any(|loss| loss.detail.contains("for reference")));
    }
    let (bytes, _) = written(&model);
    let doc = document(&bytes);
    assert!(!doc["assembly"]["joints"].as_array().unwrap().is_empty());
    assert!(doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["type"] == "sketch"));
}

#[test]
fn reference_sketch_choice_applies_to_single_parts_and_component_parts() {
    let mut part = with_sketches();
    part.assembly = None;
    for assembly in [false, true] {
        let model = if assembly {
            let mut model = placed_twice();
            model.assembly.as_mut().unwrap().parts = vec![part.clone()];
            model
        } else {
            part.clone()
        };
        for history in [false, true] {
            for sketches in [false, true] {
                let mut bytes = Vec::new();
                let mut report = Report::default();
                SolidCraftWriter::new()
                    .import_only(!history)
                    .sketches(sketches)
                    .verification(Verification::Off)
                    .write(&model, &mut bytes, &mut report)
                    .unwrap();
                let doc = document(&bytes);
                let kept = doc["features"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f["type"] == "sketch");
                assert_eq!(kept, sketches, "assembly={assembly}, history={history}");
            }
        }
    }
}

#[test]
fn parallel_parts_keep_the_complete_container_and_report_in_serial_order() {
    let mut models = vec![
        nested(),
        with_components(true),
        with_sketches(),
        jointed(),
        mixed_histories(),
    ];
    if Format::FreeCadDocument.can_convert() {
        for file in ["product_assembly.FCStd", "freecad-1.1.4-A01/A01a.FCStd"] {
            models.push(
                read(
                    Format::FreeCadDocument,
                    &Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../dutchman-core/tests/fixtures")
                        .join(file),
                    &mut Report::default(),
                )
                .unwrap(),
            );
        }
    }
    for model in models {
        for import_only in [false, true] {
            for sketches in [false, true] {
                let writer = SolidCraftWriter::new()
                    .verification(Verification::Off)
                    .import_only(import_only)
                    .sketches(sketches);
                let mut serial = Vec::new();
                let mut serial_report = Report::default();
                writer
                    .clone()
                    .threads(1)
                    .write(&model, &mut serial, &mut serial_report)
                    .unwrap();
                let mut parallel = Vec::new();
                let mut parallel_report = Report::default();
                writer
                    .threads(4)
                    .write(&model, &mut parallel, &mut parallel_report)
                    .unwrap();
                assert_eq!(serial, parallel, "{}", model.name);
                parallel_report.set_threads(1, 1);
                assert_eq!(
                    format!("{serial_report:?}"),
                    format!("{parallel_report:?}"),
                    "{}",
                    model.name
                );
            }
        }
    }
}

fn mixed_histories() -> Model {
    let mut model = placed_twice();
    let assembly = model.assembly.as_mut().unwrap();
    let mut part = assembly.parts[0].clone();
    part.name = "Repeated name".to_owned();
    let mut wire = serde_json::to_value(&part.ir).unwrap();
    let sphere = serde_json::json!({
        "id": "test:model:feature#sphere", "ordinal": 0, "name": "Ball", "suppressed": false,
        "outputs": [wire["model"]["bodies"][0]["id"]],
        "definition": {"definition": "sphere", "center": {"x": 5.0, "y": 5.0, "z": 5.0},
                       "radius": 5.0, "op": "new_body"},
    });
    wire["model"]["features"] = serde_json::json!([sphere]);
    part.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    assembly.parts.push(part.clone());
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "id": "test:model:feature#later", "ordinal": 1, "name": "Later", "suppressed": false,
            "definition": {"definition": "native", "kind": "Mystery"},
        }));
    part.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    assembly.parts.push(part.clone());
    // A restatement failure must retry serially and keep its Dropped note at this position.
    part.ir.model.faces.clear();
    assembly.parts.push(part.clone());
    part.ir.model.bodies.clear();
    assembly.parts.push(part);
    assembly.placements = assembly
        .parts
        .iter()
        .enumerate()
        .map(|(index, _)| Placement {
            hidden: false,
            name: None,
            part: index,
            transform: dutchman_core::placement::IDENTITY,
        })
        .collect();
    model
}

#[test]
fn parallel_replay_checks_keep_partial_histories_and_reports_identical() {
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        return;
    };
    let model = mixed_histories();
    let writer = SolidCraftWriter::new().verification(Verification::With(cli));
    let mut serial = Vec::new();
    let mut serial_report = Report::default();
    writer
        .clone()
        .threads(1)
        .write(&model, &mut serial, &mut serial_report)
        .unwrap();
    let mut parallel = Vec::new();
    let mut parallel_report = Report::default();
    writer
        .threads(4)
        .write(&model, &mut parallel, &mut parallel_report)
        .unwrap();
    assert_eq!(serial, parallel);
    parallel_report.set_threads(1, 1);
    assert_eq!(format!("{serial_report:?}"), format!("{parallel_report:?}"));
}

/// CADIR face bindings use a BREP namespace independent of the component/body ids.
fn bound_components(mode: &str, stored: bool) -> Model {
    let mut model = with_components(false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let face_ids: Vec<_> = wire["model"]["faces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|face| face["id"].as_str().unwrap().to_owned())
        .collect();
    let mut text = wire.to_string();
    for (n, id) in face_ids.iter().enumerate() {
        text = text.replace(id, &format!("f3d:brep/component.smb/brep:entity#{n}"));
    }
    wire = serde_json::from_str(&text).unwrap();
    let body = wire["model"]["bodies"][0]["id"].clone();
    wire["model"]["bodies"][0]
        .as_object_mut()
        .unwrap()
        .remove("color");
    wire["model"]["appearances"] = json!([
        {"id": "synthetic:test:appearance#red", "name": "Red paint", "base_color": {"r": 1.0, "g": 0.0, "b": 0.0, "a": 0.4}},
        {"id": "synthetic:test:appearance#blue", "name": "Blue paint", "base_color": {"r": 0.0, "g": 0.0, "b": 1.0, "a": 1.0}},
    ]);
    let mut bindings = Vec::new();
    if mode.starts_with("body") {
        bindings.push(json!({"id": "synthetic:test:appearance-binding#body", "target": {"kind": "body", "id": body}, "appearance": "synthetic:test:appearance#red"}));
    }
    if mode != "body" {
        for (n, face) in wire["model"]["faces"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            if mode == "body_face" && n != 0 {
                continue;
            }
            bindings.push(json!({"id": format!("synthetic:test:appearance-binding#{n}"), "target": {"kind": "face", "id": face["id"]},
                "appearance": if mode.starts_with("body_") || (mode == "mixed" && n == 0) { "synthetic:test:appearance#blue" } else { "synthetic:test:appearance#red" }}));
        }
    }
    wire["model"]["appearance_bindings"] = json!(bindings);
    wire["model"]["features"] = if stored {
        json!([{"id": "synthetic:test:feature#stored", "ordinal": 0, "name": "Stored solid", "suppressed": false, "dependencies": [], "outputs": [body], "definition": {"definition": "stored_geometry"}}])
    } else {
        json!([])
    };
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model.assembly = dutchman_core::local_assembly(&model).unwrap();
    model
}

#[test]
fn component_appearance_bindings_colour_imports_and_stored_solids() {
    for stored in [false, true] {
        for mode in [
            "uniform",
            "mixed",
            "body",
            "body_face",
            "body_uniform_faces",
        ] {
            let model = bound_components(mode, stored);
            let part = &model.assembly.as_ref().unwrap().parts[0];
            assert!(!part.ir.model.appearance_bindings.is_empty());
            assert_eq!(
                part.ir.model.appearances.len(),
                if mode == "mixed" || mode.starts_with("body_") {
                    2
                } else {
                    1
                }
            );
            let (bytes, report) = written(&model);
            let doc = document(&bytes);
            let import = doc["features"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["type"] == "import" && f["suppressed"] == false)
                .unwrap();
            let name = import["body_names"][0].as_str().unwrap();
            if mode == "mixed" {
                assert!(doc["appearances"]["bodies"]
                    .as_object()
                    .is_none_or(|b| b.is_empty()));
                let faces = doc["appearances"]["faces"].as_array().unwrap();
                assert_eq!(faces.len(), 6);
                assert!(faces.iter().all(|face| face["body"] == name));
                assert_eq!(
                    faces
                        .iter()
                        .filter(|f| f["look"]["color"] == json!([0, 0, 255]))
                        .count(),
                    1
                );
            } else {
                assert_eq!(
                    doc["appearances"]["bodies"][name]["color"],
                    json!([255, 0, 0])
                );
                assert!(
                    (doc["appearances"]["bodies"][name]["opacity"]
                        .as_f64()
                        .unwrap()
                        - 0.4)
                        .abs()
                        < 1e-6
                );
                assert_eq!(doc["appearances"]["bodies"][name]["name"], "Red paint");
                let faces = doc["appearances"]["faces"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(
                    faces.len(),
                    match mode {
                        "body_face" => 1,
                        "body_uniform_faces" => 6,
                        _ => 0,
                    }
                );
                assert!(faces
                    .iter()
                    .all(|f| f["body"] == name && f["look"]["color"] == json!([0, 0, 255])));
            }
            let component = import["component"].clone();
            assert_eq!(
                doc["occurrences"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|o| o["component"] == component)
                    .count(),
                2
            );
            assert!(
                !report
                    .losses()
                    .iter()
                    .any(|loss| loss.subject == "appearance"),
                "{:?}",
                report.losses()
            );
        }
    }
}

#[test]
fn missing_face_appearance_bindings_are_reported() {
    let mut model = bound_components("uniform", false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["appearance_bindings"].as_array_mut().unwrap().push(json!({
        "id": "synthetic:test:appearance-binding#missing", "target": {"kind": "face", "id": "f3d:brep/component.smb/brep:entity#missing"}, "appearance": "synthetic:test:appearance#red"
    }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model.assembly = dutchman_core::local_assembly(&model).unwrap();
    let (_, report) = written(&model);
    assert!(
        report
            .losses()
            .iter()
            .any(|loss| loss.subject == "appearance"
                && loss.severity == dutchman_core::Severity::Dropped
                && loss.code.as_deref() == Some("dutchman/appearance.target-absent")
                && loss
                    .detail
                    .contains("synthetic:test:appearance-binding#missing")),
        "{:?}",
        report.losses()
    );
}

#[test]
fn solidcraft_inherits_component_appearance_bindings_in_both_placements() {
    let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) else {
        return;
    };
    for stored in [false, true] {
        // From SolidCraft 0.26.0, OBJ export writes body colours to a material file and the
        // appearance list names only faces that override their body. Older releases open the
        // same coloured design and list every face with its effective colour.
        let current_listing = dutchman_solidcraft::solidcraft_version(
            &dutchman_solidcraft::Verification::With(cli.clone()),
        )
        .and_then(|line| {
            line.split_whitespace()
                .last()?
                .trim_start_matches('v')
                .parse::<dutchman_solidcraft::SolidCraftVersion>()
                .ok()
        })
        .is_some_and(|version| version >= "0.26.0".parse().unwrap());
        for mode in [
            "uniform",
            "mixed",
            "body",
            "body_face",
            "body_uniform_faces",
        ] {
            let (bytes, _) = written(&bound_components(mode, stored));
            let dir = tempfile::tempdir().unwrap();
            let design = dir.path().join("pair.solidcraft");
            std::fs::write(&design, &bytes).unwrap();
            let measured = evaluate(&cli, &bytes);
            assert_eq!(measured.len(), 2);
            let script = dir.path().join("looks.json");
            let doc = document(&bytes);
            let name = doc["features"]
                .as_array()
                .unwrap()
                .iter()
                .find(|feature| feature["type"] == "import" && feature["suppressed"] == false)
                .unwrap()["body_names"][0]
                .clone();
            // Appearance queries address definition bodies. OBJ material export resolves
            // their colours for each placed body, including the second world-name alias.
            let mesh = dir.path().join("pair.obj");
            let commands = vec![
                json!({"command": "appearance.list", "params": {"body": name}}),
                json!({"command": "file.export", "params": {"path": mesh}}),
            ];
            std::fs::write(&script, json!(commands).to_string()).unwrap();
            let output = dir.path().join("looks.out.json");
            let status = std::process::Command::new(&cli)
                .arg("run")
                .arg(&script)
                .arg("--in")
                .arg(&design)
                .arg("--results")
                .arg("--json-out")
                .arg(&output)
                .status()
                .unwrap();
            assert!(status.success());
            let results: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
            if mode != "mixed" && current_listing {
                let materials = std::fs::read_to_string(dir.path().join("pair.mtl")).unwrap();
                assert_eq!(
                    materials
                        .lines()
                        .filter(|line| *line == "Kd 1.000 0.000 0.000")
                        .count(),
                    2,
                    "{materials}"
                );
            }
            for result in results.as_array().unwrap().iter().take(1) {
                if mode == "mixed" {
                    assert_eq!(
                        result["body"]["faces"].as_array().unwrap().len(),
                        6,
                        "{results}"
                    );
                    assert_eq!(
                        result["body"]["faces"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|face| face["appearance"]["color"] == "#0000ff")
                            .count(),
                        1,
                        "{results}"
                    );
                } else {
                    assert_eq!(
                        result["body"]["appearance"]["color"], "#ff0000",
                        "{results}"
                    );
                    assert!(
                        (result["body"]["appearance"]["opacity"].as_f64().unwrap() - 0.4).abs()
                            < 1e-6,
                        "{results}"
                    );
                    if current_listing {
                        assert_eq!(
                            result["body"]["faces"].as_array().unwrap().len(),
                            match mode {
                                "body_face" => 1,
                                "body_uniform_faces" => 6,
                                _ => 0,
                            },
                            "{results}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn component_appearance_bindings_do_not_colour_an_unpainted_namesake() {
    let mut model = bound_components("body", false);
    let assembly = model.assembly.as_mut().unwrap();
    let coloured = assembly.parts[0].clone();
    assembly.parts[0].ir.model.appearance_bindings.clear();
    assembly.parts[0].ir.model.appearances.clear();
    assembly.parts.push(coloured);
    assembly.placements[1].part = 1;
    let (bytes, report) = written(&model);
    let doc = document(&bytes);
    let imports: Vec<_> = doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["type"] == "import" && f["suppressed"] == false)
        .collect();
    let first = imports[0]["body_names"][0].as_str().unwrap();
    let second = imports[1]["body_names"][0].as_str().unwrap();
    assert_ne!(first, second);
    assert!(doc["appearances"]["bodies"].get(first).is_none());
    assert_eq!(
        doc["appearances"]["bodies"][second]["color"],
        json!([255, 0, 0])
    );
    assert!(
        !report
            .losses()
            .iter()
            .any(|loss| loss.subject == "appearance"),
        "{:?}",
        report.losses()
    );
}

fn occurrence_rows(o: &Value) -> dutchman_core::placement::Placement3x4 {
    std::array::from_fn(|r| std::array::from_fn(|c| o["transform"][c][r].as_f64().unwrap()))
}

fn assert_proper_occurrences(doc: &Value) {
    for o in doc["occurrences"].as_array().unwrap() {
        let t = occurrence_rows(o);
        let (r, reflected) = dutchman_core::placement::rigid(&t).unwrap();
        assert!(!reflected, "{o}");
        for (a, b) in t.iter().flatten().zip(r.iter().flatten()) {
            assert!((a - b).abs() < 1e-12, "{o}");
        }
    }
}

fn definition_points(bytes: &[u8], component: &Value) -> Vec<[f64; 3]> {
    let doc = document(bytes);
    let feature = doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["component"] == *component && f["type"] == "import" && f["suppressed"] == false)
        .unwrap();
    let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut step = Vec::new();
    zip.by_name(feature["step_blob"].as_str().unwrap())
        .unwrap()
        .read_to_end(&mut step)
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("definition.step");
    std::fs::write(&path, step).unwrap();
    let model = read(Format::Step, &path, &mut Report::default()).unwrap();
    model
        .ir
        .model
        .points
        .iter()
        .map(|p| {
            let p = p.position().get();
            [p.x, p.y, p.z]
        })
        .collect()
}

fn placed_points(points: &[[f64; 3]], t: &dutchman_core::placement::Placement3x4) -> Vec<[f64; 3]> {
    let mut points: Vec<_> = points
        .iter()
        .map(|p| std::array::from_fn(|r| (0..3).map(|c| t[r][c] * p[c]).sum::<f64>() + t[r][3]))
        .collect();
    points.sort_by(|a, b| {
        a.iter()
            .zip(b)
            .map(|(a, b)| a.total_cmp(b))
            .find(|c| !c.is_eq())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    points
}

fn assert_same_points(a: &[[f64; 3]], b: &[[f64; 3]]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().flatten().zip(b.iter().flatten()) {
        assert!((a - b).abs() < 1e-7, "{a} != {b}");
    }
}

#[test]
fn reflection_definitions_share_geometry_and_write_only_proper_occurrences() {
    use dutchman_core::placement::{compose, IDENTITY, MIRROR_X};
    let mut model = placed_twice();
    let a = model.assembly.as_mut().unwrap();
    let turn = [
        [0.0, -1.0, 0.0, 30.0],
        [1.0, 0.0, 0.0, 4.0],
        [0.0, 0.0, 1.0, 2.0],
    ];
    a.placements[1].transform = compose(&turn, &MIRROR_X);
    let mut third = a.placements[1].clone();
    third.transform = MIRROR_X;
    a.placements.push(third);
    let source = a.placements[1].transform;
    let (bytes, report) = written(&model);
    let doc = document(&bytes);
    assert_proper_occurrences(&doc);
    assert_eq!(doc["components"].as_array().unwrap().len(), 2);
    let occurrences = doc["occurrences"].as_array().unwrap();
    assert_ne!(occurrences[0]["component"], occurrences[1]["component"]);
    assert_eq!(occurrences[1]["component"], occurrences[2]["component"]);
    assert_eq!(occurrence_rows(&occurrences[0]), IDENTITY);
    let original = definition_points(&bytes, &occurrences[0]["component"]);
    let mirrored = definition_points(&bytes, &occurrences[1]["component"]);
    assert_same_points(
        &placed_points(&original, &MIRROR_X),
        &placed_points(&mirrored, &IDENTITY),
    );
    assert_same_points(
        &placed_points(&original, &source),
        &placed_points(&mirrored, &occurrence_rows(&occurrences[1])),
    );
    assert!(report.losses().iter().any(|n| n.severity == dutchman_core::Severity::Info && n.detail == "2 occurrence(s) placed by a reflection were written with a mirrored copy of their component"));
    if let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) {
        let bodies = evaluate(&cli, &bytes);
        assert_eq!(bodies.len(), 3);
        for body in &bodies {
            let volume = body["volume_mm3"].as_f64().unwrap();
            assert!((volume - 1000.0).abs() < 1e-6, "{body}");
        }
        // The turned mirror image of the cube [0,10]^3 spans x 20..30, y -6..4, z 2..12.
        assert!(bodies.iter().any(|b| {
            let corners = [&b["bbox"]["min"], &b["bbox"]["max"]];
            [[20.0, -6.0, 2.0], [30.0, 4.0, 12.0]]
                .iter()
                .zip(corners)
                .all(|(want, got)| {
                    (0..3).all(|i| (got[i].as_f64().unwrap() - want[i]).abs() < 1e-6)
                })
        }));
    }
}

#[test]
fn reflection_nested_placements_compose_and_cancel() {
    use dutchman_core::placement::{compose, IDENTITY, MIRROR_X};
    let mut model = nested();
    let assembly = model.assembly.as_mut().unwrap();
    assembly.placements[1].transform = MIRROR_X;
    let inner = assembly.parts[0].assembly.as_mut().unwrap();
    inner.placements[1].transform = compose(&inner.placements[1].transform, &MIRROR_X);
    let (bytes, _) = written(&model);
    let doc = document(&bytes);
    assert_proper_occurrences(&doc);
    let top = doc["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| {
            o["parent"] == 0
                && o["component"]
                    == doc["components"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|c| c["name"] == "pair (mirrored)")
                        .unwrap()["id"]
        })
        .unwrap();
    let children: Vec<_> = doc["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["parent"] == top["component"])
        .collect();
    assert_eq!(children.len(), 2);
    let original = definition_points(&bytes, &doc["occurrences"][0]["component"]);
    for (index, child) in children.iter().enumerate() {
        let geometry = definition_points(&bytes, &child["component"]);
        let source = model.assembly.as_ref().unwrap().parts[0]
            .assembly
            .as_ref()
            .unwrap()
            .placements[index]
            .transform;
        let expected = placed_points(&original, &compose(&MIRROR_X, &source));
        let actual = placed_points(
            &geometry,
            &compose(&occurrence_rows(top), &occurrence_rows(child)),
        );
        assert_same_points(&expected, &actual);
    }
    assert_eq!(occurrence_rows(top), IDENTITY);
    if let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) {
        let bodies = evaluate(&cli, &bytes);
        assert_eq!(bodies.len(), 5);
        assert!(bodies
            .iter()
            .all(|b| (b["volume_mm3"].as_f64().unwrap() - 1000.0).abs() < 1e-6));
    }
}

#[test]
fn reflection_bad_placements_are_left_out_and_drift_is_repaired() {
    let mut model = placed_twice();
    let a = model.assembly.as_mut().unwrap();
    a.placements[0].transform[0][0] += 1e-8;
    a.placements[0].transform[1][0] += 1e-8;
    a.placements[1].transform[0][3] = f64::NAN;
    for t in [
        [
            [2.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
        [
            [1.0, 0.0001, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    ] {
        let mut p = a.placements[0].clone();
        p.transform = t;
        a.placements.push(p);
    }
    let (bytes, report) = written(&model);
    let doc = document(&bytes);
    assert_proper_occurrences(&doc);
    assert_eq!(doc["occurrences"].as_array().unwrap().len(), 1);
    for (index, kind) in [(1, "non-finite"), (2, "scaled"), (3, "sheared")] {
        assert!(
            report
                .losses()
                .iter()
                .any(|n| n.severity == dutchman_core::Severity::Dropped
                    && n.detail.contains(&format!("Occurrence index {index}"))
                    && n.detail.contains(kind)),
            "{:?}",
            report.losses()
        );
    }
    assert_eq!(report.completeness().unwrap().placed, 1);
    assert_eq!(report.completeness().unwrap().placements(), 4);
}

#[test]
fn reflection_flattens_only_the_mirrored_definitions_history() {
    let mut model = bound_components("body", true);
    model.assembly.as_mut().unwrap().placements[1].transform = dutchman_core::placement::MIRROR_X;
    let (bytes, report) = written(&model);
    let doc = document(&bytes);
    assert_proper_occurrences(&doc);
    let original = &doc["occurrences"][0]["component"];
    let mirrored = &doc["occurrences"][1]["component"];
    let features = doc["features"].as_array().unwrap();
    let copy_features: Vec<_> = features
        .iter()
        .filter(|f| f["component"] == *mirrored)
        .collect();
    assert_eq!(copy_features.len(), 1);
    assert_eq!(copy_features[0]["type"], "import");
    assert_eq!(copy_features[0]["suppressed"], false);
    // The original keeps its stored geometry as history, which already is the part.
    assert!(features.iter().any(|f| f["component"] == *original
        && f["suppressed"] == false
        && f["name"] != "Original geometry (imported)"));
    assert!(report.losses().iter().any(|n| n.subject == "history"
        && n.detail
            .contains("mirrored definition written as imported geometry only")));
}

#[test]
fn reflection_validation_also_applies_to_restored_stored_placements() {
    let mut model = placed_twice();
    let a = model.assembly.as_mut().unwrap();
    a.stored_placements = vec![
        dutchman_core::placement::MIRROR_X,
        dutchman_core::placement::IDENTITY,
    ];
    a.placements[0].transform[0][0] = f64::NAN;
    let mut report = Report::default();
    let mut bytes = Vec::new();
    SolidCraftWriter::new()
        .joints(false)
        .verification(Verification::Off)
        .write(&model, &mut bytes, &mut report)
        .unwrap();
    let doc = document(&bytes);
    assert_proper_occurrences(&doc);
    assert_eq!(doc["occurrences"].as_array().unwrap().len(), 2);
    assert!(report
        .losses()
        .iter()
        .any(|n| n.subject == "placements" && n.severity == dutchman_core::Severity::Info));
}

/// Every placement in the world the assembly gives, one for each way of reaching a part.
fn solver_worlds(
    assembly: &Assembly,
    outer: &dutchman_core::placement::Placement3x4,
    out: &mut Vec<dutchman_core::placement::Placement3x4>,
) {
    use dutchman_core::placement::compose;
    for placement in &assembly.placements {
        let world = compose(outer, &placement.transform);
        out.push(world);
        if let Some(inner) = assembly.parts[placement.part].assembly.as_ref() {
            solver_worlds(inner, &world, out);
        }
    }
}

/// Every occurrence's placement in the world as the written document composes it: its own
/// transform after those of each occurrence of the component that holds it.
fn written_worlds(
    doc: &Value,
    occurrence: &Value,
    depth: usize,
) -> Vec<dutchman_core::placement::Placement3x4> {
    use dutchman_core::placement::compose;
    let own = occurrence_rows(occurrence);
    let parent = occurrence["parent"].as_u64().unwrap();
    if parent == 0 {
        return vec![own];
    }
    assert!(depth < 16, "occurrences nest inside themselves");
    doc["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|holder| holder["component"].as_u64() == Some(parent))
        .flat_map(|holder| written_worlds(doc, holder, depth + 1))
        .map(|holder| compose(&holder, &own))
        .collect()
}

#[test]
fn composed_written_placements_equal_the_assemblys_world_placements() {
    let inner = nested();
    let cube = inner.assembly.as_ref().unwrap().parts[1].clone();
    let turned = [
        [0.0, -1.0, 0.0, 7.0],
        [1.0, 0.0, 0.0, 3.0],
        [0.0, 0.0, 1.0, 1.0],
    ];
    let moved = [
        [1.0, 0.0, 0.0, -40.0],
        [0.0, 1.0, 0.0, 12.0],
        [0.0, 0.0, 1.0, 5.0],
    ];
    let top = Model {
        name: "top2".to_owned(),
        assembly: Some(Assembly {
            own_part: None,
            joints: Vec::new(),
            left_out: Vec::new(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
            parts: vec![inner, cube.clone()],
            placements: [(0, turned), (0, moved), (1, turned)]
                .into_iter()
                .map(|(part, transform)| Placement {
                    hidden: false,
                    name: None,
                    part,
                    transform,
                })
                .collect(),
        }),
        ..cube
    };
    let mut expected = Vec::new();
    solver_worlds(
        top.assembly.as_ref().unwrap(),
        &dutchman_core::placement::IDENTITY,
        &mut expected,
    );
    let (bytes, _) = written(&top);
    let doc = document(&bytes);
    let mut found: Vec<_> = doc["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|o| written_worlds(&doc, o, 0))
        .collect();
    // Occurrences inside a component placed more than once are reached once per copy, as
    // the solver reaches them; the multisets must agree.
    let key = |t: &dutchman_core::placement::Placement3x4| {
        t.iter()
            .flatten()
            .map(|v| (v * 1e6).round() as i64)
            .collect::<Vec<_>>()
    };
    expected.sort_by_key(key);
    found.sort_by_key(key);
    assert_eq!(expected.len(), found.len());
    for (a, b) in expected.iter().zip(&found) {
        assert_eq!(key(a), key(b), "{a:?} != {b:?}");
    }
}

/// A frame of two cubes, the second at `y`, with its own cube at the origin, as a part.
fn frame_with_cube_at(name: &str, y: f64) -> Model {
    let inner = nested();
    let cube = inner.assembly.as_ref().unwrap().parts[1].clone();
    let at = |x: f64, y: f64| [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, 0.0]];
    Model {
        name: name.to_owned(),
        assembly: Some(Assembly {
            own_part: None,
            joints: Vec::new(),
            left_out: Vec::new(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
            parts: vec![cube.clone()],
            placements: vec![Placement {
                hidden: false,
                name: None,
                part: 0,
                transform: at(0.0, y),
            }],
        }),
        ..cube
    }
}

#[test]
fn layout_variants_and_a_mirrored_instance_write_at_their_world_placements() {
    let turned = [
        [0.0, -1.0, 0.0, 7.0],
        [1.0, 0.0, 0.0, 3.0],
        [0.0, 0.0, 1.0, 1.0],
    ];
    let moved = [
        [1.0, 0.0, 0.0, -40.0],
        [0.0, 1.0, 0.0, 12.0],
        [0.0, 0.0, 1.0, 5.0],
    ];
    let reflected = dutchman_core::placement::compose(&dutchman_core::placement::MIRROR_X, &moved);
    // Two definitions of one frame, whose cube sits in a different place in each, placed
    // three times: the second layout twice, once of them mirrored.
    let (first, second) = (
        frame_with_cube_at("Frame", 5.0),
        frame_with_cube_at("Frame (2)", 60.0),
    );
    let cube = first.assembly.as_ref().unwrap().parts[0].clone();
    let top = Model {
        name: "variants".to_owned(),
        assembly: Some(Assembly {
            own_part: None,
            joints: Vec::new(),
            left_out: Vec::new(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
            parts: vec![first, second],
            placements: [(0, turned), (1, moved), (1, reflected)]
                .into_iter()
                .map(|(part, transform)| Placement {
                    hidden: false,
                    name: None,
                    part,
                    transform,
                })
                .collect(),
        }),
        ..cube
    };
    let mut expected = Vec::new();
    solver_worlds(
        top.assembly.as_ref().unwrap(),
        &dutchman_core::placement::IDENTITY,
        &mut expected,
    );
    let (bytes, _) = written(&top);
    let doc = document(&bytes);
    let names: Vec<&str> = doc["components"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(
        names.contains(&"Frame") && names.contains(&"Frame (2)"),
        "{names:?}"
    );
    assert!(names.iter().any(|n| n.contains("mirrored")), "{names:?}");
    let found: Vec<_> = doc["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|o| written_worlds(&doc, o, 0))
        .collect();
    assert_eq!(expected.len(), found.len());
    // The reflected instance is written as a mirrored definition at the rotation part of
    // its placement, so its cube is compared by position rather than by matrix.
    let origin = |t: &dutchman_core::placement::Placement3x4| {
        [t[0][3], t[1][3], t[2][3]].map(|v| (v * 1e6).round() as i64)
    };
    let mut want: Vec<_> = expected.iter().map(origin).collect();
    let mut have: Vec<_> = found.iter().map(origin).collect();
    want.sort();
    have.sort();
    assert_eq!(want, have);
}

fn placement(part: usize, name: &str, x: f64) -> Placement {
    Placement {
        hidden: false,
        name: Some(name.to_owned()),
        part,
        transform: [
            [1.0, 0.0, 0.0, x],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    }
}

/// `parts`, placed by `placements`, as a sub-assembly (or the design) named `name`.
fn holder(
    name: &str,
    parts: Vec<(Model, &str)>,
    placements: Vec<Placement>,
    cube: &Model,
) -> Model {
    let (parts, identities): (Vec<Model>, Vec<&str>) = parts.into_iter().unzip();
    Model {
        name: name.to_owned(),
        source_file: Some("repeats.f3d".to_owned()),
        source_step: None,
        assembly: Some(Assembly {
            own_part: None,
            joints: Vec::new(),
            left_out: Vec::new(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: identities
                .into_iter()
                .map(|identity| Some(identity.to_owned()))
                .collect(),
            definitions: Vec::new(),
            parts,
            placements,
        }),
        ..cube.clone()
    }
}

/// Two towers with different layouts, both holding the same carriage, itself holding a cube.
fn towers() -> Model {
    let pair = placed_twice();
    let cube = pair.assembly.as_ref().unwrap().parts[0].clone();
    let carriage = holder(
        "Carriage",
        vec![(cube.clone(), "cube")],
        vec![placement(0, "Block", 5.0)],
        &cube,
    );
    let tower = |name: &str, x: f64| {
        holder(
            name,
            vec![(carriage.clone(), "carriage"), (cube.clone(), "cube")],
            vec![placement(0, "Carriage", x), placement(1, "Cube", 100.0)],
            &cube,
        )
    };
    holder(
        "top",
        vec![(tower("Tower", 0.0), "a"), (tower("Tower", 50.0), "b")],
        vec![placement(0, "One", 0.0), placement(1, "Two", 200.0)],
        &cube,
    )
}

fn component_names(document: &Value) -> Vec<&str> {
    let components = document["components"].as_array().unwrap();
    components
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect()
}

#[test]
fn a_part_repeated_across_layouts_is_one_definition_placed_in_each() {
    let (bytes, report) = written(&towers());
    let doc = document(&bytes);
    assert_eq!(
        component_names(&doc),
        ["Tower", "Carriage", "synthetic_cube", "Tower (2)"]
    );
    let features = doc["features"].as_array().unwrap();
    assert_eq!(features.len(), 1, "one Import for every placement");
    let carriage = doc["components"][1]["id"].clone();
    let cube = doc["components"][2]["id"].clone();
    let tower_two = doc["components"][3]["id"].clone();
    let occurrences = doc["occurrences"].as_array().unwrap();
    let placed_in = |parent: &Value| -> Vec<(Value, Value)> {
        occurrences
            .iter()
            .filter(|o| o["parent"] == *parent)
            .map(|o| (o["component"].clone(), o["name"].clone()))
            .collect()
    };
    assert_eq!(
        placed_in(&tower_two),
        [
            (carriage.clone(), json!("Carriage")),
            (cube.clone(), json!("Cube"))
        ]
    );
    assert_eq!(placed_in(&carriage), [(cube, json!("Block"))]);
    assert!(report.losses().iter().any(|loss| loss
        .detail
        .starts_with("3 repeated component(s) written once")));
    if let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) {
        assert_eq!(evaluate(&cli, &bytes).len(), 4);
    }
}

#[test]
fn a_sub_assembly_a_joint_reaches_inside_keeps_its_own_occurrences() {
    let mut model = towers();
    let frame = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    model
        .assembly
        .as_mut()
        .unwrap()
        .joints
        .push(dutchman_core::Joint {
            a: vec![0, 0, 0],
            b: vec![1, 0, 0],
            frame_a: frame,
            frame_b: frame,
            suppressed: false,
            motion: dutchman_core::Motion::Rigid,
            flip: false,
        });
    let (bytes, _) = written(&model);
    let doc = document(&bytes);
    assert_eq!(
        component_names(&doc),
        [
            "Tower",
            "Carriage",
            "synthetic_cube",
            "Tower (2)",
            "Carriage (2)"
        ]
    );
    assert_eq!(doc["features"].as_array().unwrap().len(), 1);
    let joint = &doc["assembly"]["joints"][0];
    assert_ne!(joint["a"]["occurrence"], joint["b"]["occurrence"]);
}

#[test]
fn a_part_placed_at_a_changed_layout_is_not_repeated() {
    let mut model = towers();
    let assembly = model.assembly.as_mut().unwrap();
    let second = assembly.parts[1].assembly.as_mut().unwrap();
    let carriage = second.parts[0].assembly.as_mut().unwrap();
    carriage.placements[0].transform[0][3] = 6.0;
    let (bytes, _) = written(&model);
    assert_eq!(
        component_names(&document(&bytes)),
        [
            "Tower",
            "Carriage",
            "synthetic_cube",
            "Tower (2)",
            "Carriage (2)"
        ]
    );
}

/// A saved free edge beside a painted solid, in the same component definition.
fn bound_components_with_wire(stored: bool) -> Model {
    let mut model = bound_components("body", stored);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let edge = wire["model"]["edges"][0]["id"].clone();
    let body = "synthetic:test:body#saved-edge";
    let region = "synthetic:test:region#saved-edge";
    let shell = "synthetic:test:shell#saved-edge";
    wire["model"]["bodies"].as_array_mut().unwrap().push(json!({
        "id": body, "kind": "wire", "regions": [region], "name": "Body"
    }));
    wire["model"]["regions"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": region, "body": body, "shells": [shell]
        }));
    wire["model"]["shells"].as_array_mut().unwrap().push(json!({
        "id": shell, "region": region, "members": [{"kind": "wire_edge", "id": edge}]
    }));
    wire["model"]["product_definitions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|definition| definition["id"] == "f3d:model:component#block")
        .unwrap()["bodies"]
        .as_array_mut()
        .unwrap()
        .push(json!(body));
    if stored {
        wire["model"]["features"][0]["outputs"]
            .as_array_mut()
            .unwrap()
            .push(json!(body));
    }
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model.assembly = dutchman_core::local_assembly(&model).unwrap();
    model
}

#[test]
fn saved_wire_edges_keep_the_solid_appearance_in_each_placement() {
    for stored in [false, true] {
        let model = bound_components_with_wire(stored);
        assert!(model.assembly.as_ref().unwrap().parts[0]
            .ir
            .model
            .bodies
            .iter()
            .any(|body| body.id.as_str() == "synthetic:test:body#saved-edge"));
        let (bytes, report) = written(&model);
        let doc = document(&bytes);
        assert!(
            report
                .losses()
                .iter()
                .all(|loss| loss.subject != "appearance" && !loss.subject.ends_with(": appearance")),
            "{:?}",
            report.losses()
        );
        assert_eq!(doc["appearances"]["bodies"].as_object().unwrap().len(), 1);
        let imports: Vec<_> = doc["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|feature| feature["type"] == "import" && feature["suppressed"] == false)
            .collect();
        assert_eq!(imports.len(), 1);
        assert!(imports[0].get("body_names").is_none());
        let name = doc["appearances"]["bodies"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap();
        assert_eq!(
            doc["appearances"]["bodies"][name]["color"],
            json!([255, 0, 0])
        );
        if let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) {
            assert_eq!(
                evaluate(&cli, &bytes).len(),
                2,
                "free edges must not shift the solid's body name"
            );
        }
    }
}

#[test]
fn saved_wire_edges_keep_the_solid_bodys_original_name() {
    for stored in [false, true] {
        let model = bound_components_with_wire(stored);
        let (bytes, _) = written(&model);
        let doc = document(&bytes);
        let import = doc["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|feature| feature["type"] == "import" && feature["suppressed"] == false)
            .unwrap();
        // These are the names of the STEP products used before wire-edge
        // correspondence was available, rather than the solid's CADIR "Body".
        assert!(import.get("body_names").is_none());
        let expected = if stored { "Stored solid" } else { "Block" };
        assert!(doc["appearances"]["bodies"].get(expected).is_some());
        let mut archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut step = String::new();
        archive
            .by_name(import["step_blob"].as_str().unwrap())
            .unwrap()
            .read_to_string(&mut step)
            .unwrap();
        assert!(step.contains(&format!("PRODUCT('{expected}','{expected}'")));
    }
}

#[test]
fn an_unwritable_sibling_keeps_the_written_bodys_appearance() {
    for (stored, with_wire) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut model = if with_wire {
            bound_components_with_wire(stored)
        } else {
            bound_components("body", stored)
        };
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let body = "synthetic:test:body#unresolved-solid";
        let region = "synthetic:test:region#unresolved-solid";
        let shell = "synthetic:test:shell#unresolved-solid";
        let face = "synthetic:test:face#unresolved-solid";
        let surface = "synthetic:test:surface#unresolved-solid";
        wire["model"]["bodies"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":body,"kind":"solid","name":"Body","regions":[region]}));
        wire["model"]["regions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":region,"body":body,"shells":[shell]}));
        wire["model"]["shells"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":shell,"region":region,"members":[{"kind":"face","id":face}]}));
        wire["model"]["faces"].as_array_mut().unwrap().push(json!({"id":face,"shell":shell,"surface":surface,"sense":"forward","loops":{"classification":"unspecified","loops":[]}}));
        wire["model"]["surfaces"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":surface,"geometry":{
            "kind":"unknown"}}));
        wire["model"]["product_definitions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|definition| definition["id"] == "f3d:model:component#block")
            .unwrap()["bodies"]
            .as_array_mut()
            .unwrap()
            .push(json!(body));
        if stored {
            wire["model"]["features"][0]["outputs"]
                .as_array_mut()
                .unwrap()
                .push(json!(body));
        }
        wire["model"]["appearance_bindings"].as_array_mut().unwrap().push(json!({
            "id":"synthetic:test:binding#unresolved-solid", "target":{"kind":"body","id":body},"appearance":"synthetic:test:appearance#blue"}));
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        model.assembly = dutchman_core::local_assembly(&model).unwrap();
        let part = &model.assembly.as_ref().unwrap().parts[0];
        assert!(
            part.ir.model.bodies.iter().any(|b| b.id.as_str() == body),
            "{:?}",
            part.ir.model.bodies
        );
        assert!(
            part.ir
                .model
                .appearance_bindings
                .iter()
                .any(|b| b.id.as_str() == "synthetic:test:binding#unresolved-solid"),
            "{:?}",
            part.ir.model.appearance_bindings
        );
        let (bytes, report) = written(&model);
        let doc = document(&bytes);
        let notes: Vec<_> = report
            .losses()
            .iter()
            .filter(|l| l.subject == "appearance" || l.subject.ends_with(": appearance"))
            .collect();
        assert_eq!(notes.len(), 1, "{:?}", report.losses());
        assert_eq!(
            notes[0].code.as_deref(),
            Some("dutchman/appearance.unwritable-body")
        );
        assert!(notes[0].detail.contains(body));
        assert!(notes[0]
            .detail
            .ends_with("its body has no geometry the STEP writer can write"));
        let looks = doc["appearances"]["bodies"].as_object().unwrap();
        assert_eq!(looks.len(), 1);
        assert_eq!(looks.values().next().unwrap()["color"], json!([255, 0, 0]));
        assert!(looks.contains_key(if stored { "Stored solid" } else { "Block" }));
        assert!(doc["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|feature| feature["type"] == "import" && feature["suppressed"] == false)
            .all(|feature| feature.get("body_names").is_none()));
        if let Some(cli) = std::env::var_os(CLI_ENV).map(PathBuf::from) {
            assert_eq!(evaluate(&cli, &bytes).len(), 2);
        }
    }
}
