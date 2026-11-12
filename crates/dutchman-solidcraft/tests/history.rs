//! Feature history written end to end, on cadmpeg's example 10 mm cube with a synthetic
//! sketch-and-extrude history added as CADIR rows.
//!
//! The tests that replay the history in SolidCraft run when `SOLIDCRAFT_CLI` names a
//! SolidCraft CLI (CI's round-trip job sets it) and are skipped otherwise. Set
//! `UPDATE_GOLDEN=1` to rewrite the golden file after an intended change.

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use dutchman_core::{Check, Mode, Model, Report, Severity, Writer};
use dutchman_solidcraft::{SolidCraftWriter, Verification, CLI_ENV};
use serde_json::{json, Value};
use zip::ZipArchive;

const SKETCH: &str = "f3d:model:sketch#1";

fn entity(key: u32, geometry: Value) -> Value {
    json!({
        "id": format!("f3d:model:sketch-entity#{key}"),
        "sketch": SKETCH,
        "construction": false,
        "geometry": geometry,
    })
}

fn line(key: u32, a: [f64; 2], b: [f64; 2]) -> Value {
    entity(
        key,
        json!({"kind": "line", "start": {"u": a[0], "v": a[1]}, "end": {"u": b[0], "v": b[1]}}),
    )
}

/// The example cube with a history: a 10 mm square on XY, extruded by `length` with
/// `extent` (`None`: one-sided blind) from `profile`.
fn cube_with_history(length: f64, profile: Value, extent: Option<Value>) -> Model {
    let ir = cadmpeg_ir::examples::unit_cube().unwrap();
    let mut wire = serde_json::to_value(&ir).unwrap();
    let model = &mut wire["model"];
    model["bodies"][0]["color"] = json!({"r": 0.8, "g": 0.2, "b": 0.1, "a": 1.0});
    model["sketches"] = json!([{
        "id": SKETCH,
        "name": "Base",
        "placement": {"kind": "resolved", "frame": {
            "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
            "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
            "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0},
        }},
        "profiles": [[
            {"entity": "f3d:model:sketch-entity#1"},
            {"entity": "f3d:model:sketch-entity#2"},
            {"entity": "f3d:model:sketch-entity#3"},
            {"entity": "f3d:model:sketch-entity#4"},
        ]],
    }]);
    model["sketch_entities"] = json!([
        line(1, [0.0, 0.0], [10.0, 0.0]),
        line(2, [10.0, 0.0], [10.0, 10.0]),
        line(3, [10.0, 10.0], [0.0, 10.0]),
        line(4, [0.0, 10.0], [0.0, 0.0]),
    ]);
    let constraint = |key: u32, definition: Value| {
        json!({"id": format!("f3d:model:sketch-constraint#{key}"), "sketch": SKETCH,
               "definition": definition})
    };
    model["sketch_constraints"] = json!([
        constraint(
            1,
            json!({"kind": "horizontal", "entity": "f3d:model:sketch-entity#1"})
        ),
        constraint(
            2,
            json!({"kind": "vertical", "entity": "f3d:model:sketch-entity#2"})
        ),
        constraint(
            3,
            json!({"kind": "perpendicular", "first": "f3d:model:sketch-entity#1",
                              "second": "f3d:model:sketch-entity#4"})
        ),
        constraint(
            4,
            json!({"kind": "equal", "first": "f3d:model:sketch-entity#1",
                              "second": "f3d:model:sketch-entity#2"})
        ),
        constraint(
            5,
            json!({"kind": "distance", "entities": ["f3d:model:sketch-entity#1"],
                              "parameter": "f3d:model:parameter#2"})
        ),
        constraint(
            7,
            json!({"kind": "horizontal", "entity": "f3d:model:sketch-entity#3"})
        ),
        constraint(
            8,
            json!({"kind": "vertical", "entity": "f3d:model:sketch-entity#4"})
        ),
        constraint(
            6,
            json!({"kind": "native", "native_kind": "Fusion only", "entities": ["f3d:model:sketch-entity#3"]})
        ),
    ]);
    let extent = extent.unwrap_or_else(|| {
        json!({"kind": "one_sided", "side": {"termination": {"kind": "blind", "length": length}}})
    });
    model["features"] = json!([
        {
            "id": "f3d:model:feature#1",
            "ordinal": 0,
            "name": "Base sketch",
            "suppressed": false,
            "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": SKETCH}},
        },
        {
            "id": "f3d:model:feature#2",
            "ordinal": 1,
            "name": "Block",
            "suppressed": false,
            "dependencies": ["f3d:model:feature#1"],
            "outputs": ["synthetic:cube:body#0"],
            "definition": {
                "definition": "extrude",
                "profile": profile,
                "direction": {"kind": "profile_normal"},
                "start": {"kind": "profile_plane"},
                "extent": extent,
                "op": "new_body",
            },
        },
    ]);
    model["parameters"] = json!([
        {
            "id": "f3d:model:parameter#1",
            "owner": "f3d:model:feature#2",
            "name": "d1",
            "expression": format!("{length} mm"),
            "value": {"kind": "length", "value": length},
        },
        {
            "id": "f3d:model:parameter#2",
            "owner": "f3d:model:feature#1",
            "ordinal": 1,
            "name": "side",
            "expression": "10 mm",
            "value": {"kind": "length", "value": 10.0},
        },
    ]);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        fidelity: None,
        name: "cube".to_owned(),
        source_file: Some("cube.f3d".to_owned()),
        source_step: None,
        assembly: None,
    }
}

const TOP: &str = "f3d:model:sketch#2";

/// The example cube built instead by a loft from its 10 mm base square to a square of side
/// `top` centred above it, 10 mm up; `change` edits the loft's definition.
fn loft_history(top: f64, change: impl FnOnce(&mut Value)) -> Model {
    let base = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    let model = &mut wire["model"];
    let mut sketches = model["sketches"].as_array().unwrap().clone();
    let (a, b) = (5.0 - top / 2.0, 5.0 + top / 2.0);
    let top_loop: Vec<Value> = [11, 12, 13, 14]
        .map(|k| json!({"entity": format!("f3d:model:sketch-entity#{k}")}))
        .to_vec();
    sketches.push(json!({
        "id": TOP,
        "name": "Top",
        "placement": {"kind": "resolved", "frame": {
            "origin": {"x": 0.0, "y": 0.0, "z": 10.0},
            "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
            "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0},
        }},
        "profiles": [top_loop],
    }));
    model["sketches"] = Value::Array(sketches);
    let top_line = |key: u32, from: [f64; 2], to: [f64; 2]| {
        let mut row = line(key, from, to);
        row["sketch"] = json!(TOP);
        row
    };
    let entities = model["sketch_entities"].as_array_mut().unwrap();
    entities.extend([
        top_line(11, [a, a], [b, a]),
        top_line(12, [b, a], [b, b]),
        top_line(13, [b, b], [a, b]),
        top_line(14, [a, b], [a, a]),
    ]);
    let mut definition = json!({
        "definition": "loft",
        "sections": [
            first_profile(),
            {"kind": "sketch_profiles", "value": {"sketch": TOP, "profiles": [0]}},
        ],
        "guidance": {"kind": "guides", "path": []},
        "op": "new_body",
    });
    change(&mut definition);
    model["features"] = json!([
        model["features"][0].clone(),
        {
            "id": "f3d:model:feature#3",
            "ordinal": 1,
            "name": "Top sketch",
            "suppressed": false,
            "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": TOP}},
        },
        {
            "id": "f3d:model:feature#2",
            "ordinal": 2,
            "name": "Block",
            "suppressed": false,
            "dependencies": ["f3d:model:feature#1", "f3d:model:feature#3"],
            "outputs": ["synthetic:cube:body#0"],
            "definition": definition,
        },
    ]);
    // The extrude's length parameter has no feature to own it now.
    model["parameters"] = json!([model["parameters"][1].clone()]);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    }
}

const PATH: &str = "f3d:model:sketch#3";

/// The example cube built instead by sweeping its 10 mm base square along a 10 mm line up Z,
/// drawn in a sketch on XZ; `change` edits the sweep's definition.
fn sweep_history(change: impl FnOnce(&mut Value)) -> Model {
    let base = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    let model = &mut wire["model"];
    let mut sketches = model["sketches"].as_array().unwrap().clone();
    sketches.push(json!({
        "id": PATH,
        "name": "Path",
        "placement": {"kind": "resolved", "frame": {
            "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
            "normal": {"x": 0.0, "y": -1.0, "z": 0.0},
            "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0},
        }},
        "profiles": [],
    }));
    model["sketches"] = Value::Array(sketches);
    let mut path = line(21, [0.0, 0.0], [0.0, 10.0]);
    path["sketch"] = json!(PATH);
    model["sketch_entities"].as_array_mut().unwrap().push(path);
    let mut definition = json!({
        "definition": "sweep",
        "shape": {"mode": "solid", "op": "new_body",
                  "section": {"kind": "profile", "value": first_profile()}},
        "path": {"kind": "sketch_curves",
                 "value": {"sketch": PATH, "curves": ["f3d:model:sketch-entity#21"]}},
        "path_tangent": false,
        "linearize": false,
        "path_extent": {"along_fraction": 1.0, "against_fraction": 0.0},
    });
    change(&mut definition);
    model["features"] = json!([
        model["features"][0].clone(),
        {
            "id": "f3d:model:feature#3",
            "ordinal": 1,
            "name": "Path sketch",
            "suppressed": false,
            "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": PATH}},
        },
        {
            "id": "f3d:model:feature#2",
            "ordinal": 2,
            "name": "Block",
            "suppressed": false,
            "dependencies": ["f3d:model:feature#1", "f3d:model:feature#3"],
            "outputs": ["synthetic:cube:body#0"],
            "definition": definition,
        },
    ]);
    model["parameters"] = json!([model["parameters"][1].clone()]);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    }
}

#[test]
fn a_sweep_along_sketch_curves_becomes_a_solidcraft_sweep() {
    let (document, report) = convert(
        &sweep_history(|_| {}),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let features = document["features"].as_array().unwrap();
    let id_of = |name: &str| features.iter().find(|f| f["name"] == name).unwrap()["id"].clone();
    let sweep = features.iter().find(|f| f["type"] == "sweep").unwrap();
    assert_eq!(sweep["name"], "Block");
    assert_eq!(sweep["sketch"], id_of("Base sketch"));
    assert_eq!(sweep["path_sketch"], id_of("Path sketch"));
    assert_eq!(sweep["path"].as_array().unwrap().len(), 1);
    assert_eq!(sweep["operation"], "new_body");
    assert_eq!(sweep["body_names"], json!(["unit cube"]));
    assert!(sweep.get("twist").is_none());
    assert!(sweep.get("scale").is_none());
    assert!(sweep.get("orientation").is_none());
    assert!(
        history_line(&report).contains("3 of 3 features rebuilt"),
        "{}",
        history_line(&report)
    );

    let (document, _) = convert(
        &sweep_history(|d| d["twist"] = json!(std::f64::consts::FRAC_PI_2)),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let sweep = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "sweep")
        .unwrap()
        .clone();
    assert_eq!(sweep["twist"], "90 deg");
}

const SPATIAL: &str = "f3d:model:spatial-sketch#1";

fn xyz(p: [f64; 3]) -> Value {
    json!({"x": p[0], "y": p[1], "z": p[2]})
}

fn spatial_entity(key: u32, geometry: Value) -> Value {
    json!({
        "id": format!("f3d:model:spatial-sketch-entity#{key}"),
        "sketch": SPATIAL,
        "construction": false,
        "geometry": geometry,
    })
}

fn spatial_id(key: u32) -> String {
    format!("f3d:model:spatial-sketch-entity#{key}")
}

/// The unit square swept along a 3D sketch: a 30 mm line up Z, a quarter arc of radius 20 mm
/// turning toward +X, then a cubic Bezier spline; `change` edits the spatial sketch's entities
/// and the sweep's definition.
fn spatial_sweep_history(change: impl FnOnce(&mut Vec<Value>, &mut Value)) -> Model {
    let base = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    let model = &mut wire["model"];
    model["spatial_sketches"] = json!([{"id": SPATIAL, "name": "Rail"}]);
    let mut entities = vec![
        spatial_entity(
            1,
            json!({"kind": "line", "start": xyz([0.0, 0.0, 0.0]), "end": xyz([0.0, 0.0, 30.0])}),
        ),
        spatial_entity(
            2,
            json!({"kind": "arc", "center": xyz([20.0, 0.0, 30.0]), "normal": xyz([0.0, 1.0, 0.0]),
                   "reference_direction": xyz([-1.0, 0.0, 0.0]), "radius": 20.0,
                   "start_angle": 0.0, "end_angle": std::f64::consts::FRAC_PI_2}),
        ),
        spatial_entity(
            3,
            json!({"kind": "nurbs", "curve": {
                "degree": 3,
                "knots": [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
                "poles": {"form": "polynomial", "points": [
                    xyz([20.0, 0.0, 50.0]), xyz([30.0, 0.0, 50.0]),
                    xyz([40.0, 5.0, 52.0]), xyz([50.0, 10.0, 60.0])]},
                "periodic": false}}),
        ),
    ];
    let mut definition = json!({
        "definition": "sweep",
        "shape": {"mode": "solid", "op": "new_body",
                  "section": {"kind": "profile", "value": first_profile()}},
        "path": {"kind": "spatial_sketch_curves",
                 "value": {"sketch": SPATIAL, "curves": [spatial_id(1), spatial_id(2), spatial_id(3)]}},
        "path_tangent": false,
        "linearize": false,
        "path_extent": {"along_fraction": 1.0, "against_fraction": 0.0},
    });
    change(&mut entities, &mut definition);
    model["spatial_sketch_entities"] = Value::Array(entities);
    model["features"] = json!([
        model["features"][0].clone(),
        {
            "id": "f3d:model:feature#3",
            "ordinal": 1,
            "name": "Rail sketch",
            "suppressed": false,
            "definition": {"definition": "spatial_sketch", "sketch": SPATIAL},
        },
        {
            "id": "f3d:model:feature#2",
            "ordinal": 2,
            "name": "Block",
            "suppressed": false,
            "dependencies": ["f3d:model:feature#1", "f3d:model:feature#3"],
            "outputs": ["synthetic:cube:body#0"],
            "definition": definition,
        },
    ]);
    model["parameters"] = json!([model["parameters"][1].clone()]);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    }
}

#[test]
fn a_spatial_sketch_becomes_a_3d_sketch_and_a_sweep_follows_its_curves() {
    let (document, report) = convert(
        &spatial_sweep_history(|_, _| {}),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let features = document["features"].as_array().unwrap();
    let rail = features
        .iter()
        .find(|f| f["name"] == "Rail sketch")
        .unwrap();
    assert_eq!(rail["type"], "sketch");
    assert_eq!(rail["sketch"]["view"]["three_d"], true);
    let wires = rail["sketch"]["wires"].as_array().unwrap();
    assert_eq!(wires.len(), 3);
    assert_eq!(wires[0]["fit"], json!([[0.0, 0.0, 0.0], [0.0, 0.0, 30.0]]));
    assert_eq!(wires[1]["arc"]["radius"], 20.0);
    assert_eq!(wires[1]["arc"]["end"][2], 50.0);
    assert!(wires[2]["pts"].as_array().unwrap().len() > 20);
    assert_eq!(wires[2]["pts"][0], json!([20.0, 0.0, 50.0]));
    assert_eq!(
        wires[2]["pts"].as_array().unwrap().last().unwrap(),
        &json!([50.0, 10.0, 60.0])
    );
    assert_eq!(rail["sketch"]["counters"], json!({"q": 0, "w": 3}));
    let sweep = features.iter().find(|f| f["type"] == "sweep").unwrap();
    assert_eq!(sweep["path_sketch"], rail["id"]);
    assert_eq!(sweep["path"], json!(["w1", "w2", "w3"]));
    assert!(
        history_line(&report).contains("3 of 3 features rebuilt"),
        "{}",
        history_line(&report)
    );
    assert!(
        report
            .losses()
            .iter()
            .any(|loss| loss.subject == "3D sketch \"Rail sketch\""
                && loss.detail.contains("1 spline")),
        "{report:?}"
    );
}

#[test]
fn circles_points_and_unmappable_geometry_in_a_spatial_sketch_are_reported() {
    let model = spatial_sweep_history(|entities, definition| {
        entities.push(spatial_entity(
            4,
            json!({"kind": "circle", "center": xyz([0.0, 0.0, 0.0]), "normal": xyz([0.0, 0.0, 1.0]),
                   "reference_direction": xyz([1.0, 0.0, 0.0]), "radius": 3.0}),
        ));
        entities.push(spatial_entity(
            5,
            json!({"kind": "point", "position": xyz([1.0, 2.0, 3.0])}),
        ));
        entities.push(spatial_entity(
            6,
            json!({"kind": "native", "native_kind": "Helix"}),
        ));
        definition["path"]["value"]["curves"] =
            json!([spatial_id(1), spatial_id(2), spatial_id(3), spatial_id(4)]);
    });
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let rail = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Rail sketch")
        .unwrap();
    let wires = rail["sketch"]["wires"].as_array().unwrap();
    // line, arc, spline, two half arcs of the circle, and the point
    assert_eq!(wires.len(), 6, "{wires:?}");
    assert_eq!(wires[3]["arc"]["radius"], 3.0);
    assert_eq!(wires[5]["id"], "q1");
    assert_eq!(rail["sketch"]["counters"], json!({"q": 1, "w": 5}));
    let sweep = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "sweep")
        .unwrap();
    assert_eq!(sweep["path"], json!(["w1", "w2", "w3", "w4", "w5"]));
    let losses = report
        .losses()
        .iter()
        .map(|loss| format!("{}: {}", loss.subject, loss.detail))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        losses.contains("1 curve left out: source geometry \"Helix\" has no neutral form"),
        "{losses}"
    );
}

#[test]
fn a_sweep_along_a_curve_a_spatial_sketch_could_not_carry_stops_the_history_there() {
    let model = spatial_sweep_history(|entities, _| {
        entities[2] = spatial_entity(3, json!({"kind": "native", "native_kind": "Helix"}));
    });
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let line = history_line(&report);
    assert!(
        line.contains("its path uses a curve the 3D sketch does not carry"),
        "{line}"
    );
}

#[test]
fn a_sweep_selecting_spatial_curves_by_native_records_is_a_precise_reason() {
    let model = spatial_sweep_history(|_, definition| {
        definition["path"] = json!({"kind": "spatial_sketch_selection",
            "value": {"sketch": SPATIAL, "selections": ["native"]}});
    });
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains("native selection records"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn a_sweep_along_a_spatial_sketch_replays_in_solidcraft() {
    let Some(cli) = solidcraft() else { return };
    let version =
        dutchman_solidcraft::SolidCraftVersion::detected(&Verification::With(cli.clone()));
    if version < "0.26.1".parse().unwrap() {
        eprintln!("skipped: 3D sketches were checked from SolidCraft 0.26.1");
        return;
    }
    // The line and arc entities are stored against the path direction in the second run.
    for reversed in [false, true] {
        let model = spatial_sweep_history(|entities, _| {
            if reversed {
                entities[0] = spatial_entity(
                    1,
                    json!({"kind": "line", "start": xyz([0.0, 0.0, 30.0]), "end": xyz([0.0, 0.0, 0.0])}),
                );
            }
        });
        let mut report = Report::default();
        let mut bytes = Vec::new();
        SolidCraftWriter::new()
            .verification(Verification::Off)
            .write(&model, &mut bytes, &mut report)
            .unwrap();
        let body = bounds(&cli, &bytes);
        assert_eq!(body.len(), 1);
        let (min, max) = body[0];
        assert!(min[2].abs() < 1e-6, "{min:?}");
        assert!(max[2] > 59.0 && max[0] > 49.0, "{max:?}");
        let volume = volumes(&cli, &bytes)[0];
        // A 10 mm square swept some 80 mm along its path.
        assert!(volume > 5000.0 && volume < 12000.0, "{volume}");
    }
}

#[test]
fn sweep_options_keep_their_units_and_result_mode() {
    for surface in [false, true] {
        for (change, key, expected) in [
            (json!({"scale": 2.0}), "scale", json!("2")),
            (json!({"scale": 0.01}), "scale", json!("0.01")),
            (json!({"scale": 100.0}), "scale", json!("100")),
            (
                json!({"orientation": {"kind": "binormal",
                    "direction": {"x": 0.0, "y": 1.0, "z": 0.0}}}),
                "orientation",
                json!({"direction": [0.0, 1.0, 0.0]}),
            ),
            (
                json!({"orientation": {"kind": "fixed"}}),
                "orientation",
                json!("parallel"),
            ),
            (
                json!({"twist": -std::f64::consts::FRAC_PI_2}),
                "twist",
                json!("-90 deg"),
            ),
        ] {
            let model = sweep_history(|d| {
                if surface {
                    d["shape"]["mode"] = json!("surface");
                    d["shape"].as_object_mut().unwrap().remove("op");
                }
                for (key, value) in change.as_object().unwrap() {
                    d[key] = value.clone();
                }
            });
            let (document, report) = convert(
                &model,
                &SolidCraftWriter::new().verification(Verification::Off),
            );
            let kind = if surface { "surface_sweep" } else { "sweep" };
            let sweep = document["features"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["type"] == kind)
                .unwrap();
            assert_eq!(sweep[key], expected);
            assert!(sweep["profiles"].is_object());
            assert_eq!(sweep["body_names"], json!(["unit cube"]));
            assert_eq!(sweep.get("operation").is_none(), surface);
            assert!(history_line(&report).contains("3 of 3 features rebuilt"));
        }
    }
}

#[test]
fn a_single_curve_whole_sketch_maps_as_a_sweep_path() {
    let model = sweep_history(|d| d["path"] = json!({"kind": "sketch", "value": PATH}));
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let features = document["features"].as_array().unwrap();
    let sweep = features.iter().find(|f| f["type"] == "sweep").unwrap();
    let path = features
        .iter()
        .find(|f| f["name"] == "Path sketch")
        .unwrap();
    assert_eq!(sweep["path_sketch"], path["id"]);
    assert_eq!(sweep["path"].as_array().unwrap().len(), 1);
    assert_eq!(sweep["path"][0], path["sketch"]["curves"][0]["id"]);
    assert!(history_line(&report).contains("3 of 3 features rebuilt"));
}

#[test]
fn whole_sketch_paths_require_one_non_construction_curve() {
    for (empty, construction, reason) in [
        (true, false, Some("its path sketch has no path curves")),
        (
            false,
            false,
            Some("a whole-sketch path with several curves has no resolved traversal order"),
        ),
        (false, true, None),
    ] {
        let base = sweep_history(|d| d["path"] = json!({"kind": "sketch", "value": PATH}));
        let mut wire = serde_json::to_value(&base.ir).unwrap();
        let entities = wire["model"]["sketch_entities"].as_array_mut().unwrap();
        if empty {
            entities.retain(|entity| entity["sketch"] != PATH);
        } else {
            let mut extra = line(22, [0.0, 10.0], [0.0, 20.0]);
            extra["sketch"] = json!(PATH);
            extra["construction"] = json!(construction);
            entities.push(extra);
        }
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..base
        };
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        if let Some(reason) = reason {
            assert!(
                history_line(&report).contains(reason),
                "{}",
                history_line(&report)
            );
        } else {
            let sweep = document["features"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["type"] == "sweep")
                .unwrap();
            assert_eq!(sweep["path"].as_array().unwrap().len(), 1);
        }
    }
}

fn railed_sweep_history(whole_sketch: bool) -> Model {
    const RAIL: &str = "f3d:model:sketch#4";
    let base = sweep_history(|d| {
        d["guide_rail"] = json!({
            "path": {"kind": "sketch_curves", "value": {
                "sketch": RAIL, "curves": ["f3d:model:sketch-entity#22"]}},
            "extent": {"along_fraction": 1.0, "against_fraction": 0.0},
        });
        if whole_sketch {
            d["guide_rail"]["path"] = json!({"kind": "sketch", "value": RAIL});
        }
    });
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    let model = &mut wire["model"];
    let mut rail_sketch = model["sketches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sketch| sketch["id"] == PATH)
        .unwrap()
        .clone();
    rail_sketch["id"] = json!(RAIL);
    rail_sketch["name"] = json!("Rail");
    model["sketches"].as_array_mut().unwrap().push(rail_sketch);
    let mut rail = line(22, [10.0, 0.0], [10.0, 10.0]);
    rail["sketch"] = json!(RAIL);
    model["sketch_entities"].as_array_mut().unwrap().push(rail);
    let features = model["features"].as_array_mut().unwrap();
    let sweep = features.last_mut().unwrap();
    sweep["ordinal"] = json!(3);
    sweep["dependencies"]
        .as_array_mut()
        .unwrap()
        .push(json!("f3d:model:feature#4"));
    features.insert(
        2,
        json!({
            "id": "f3d:model:feature#4", "ordinal": 2, "name": "Rail sketch",
            "suppressed": false,
            "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": RAIL}},
        }),
    );
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    }
}

#[test]
fn a_sweep_guide_rail_keeps_its_sketch_and_curve_selection() {
    for whole_sketch in [false, true] {
        let (document, report) = convert(
            &railed_sweep_history(whole_sketch),
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let features = document["features"].as_array().unwrap();
        let sweep = features.iter().find(|f| f["type"] == "sweep").unwrap();
        let rail = features
            .iter()
            .find(|f| f["name"] == "Rail sketch")
            .unwrap();
        assert_eq!(sweep["rails"][0]["sketch"], rail["id"]);
        assert_eq!(sweep["rails"][0]["curves"].as_array().unwrap().len(), 1);
        assert_ne!(sweep["rails"][0]["sketch"], sweep["path_sketch"]);
        assert_eq!(
            sweep["rails"][0]["curves"][0],
            rail["sketch"]["curves"][0]["id"]
        );
        for key in ["orientation", "twist", "scale"] {
            assert!(sweep.get(key).is_none());
        }
        assert!(history_line(&report).contains("4 of 4 features rebuilt"));
    }
}

#[test]
fn solidcraft_replays_whole_sketch_paths_and_guide_rails() {
    let Some(cli) = solidcraft() else { return };
    for model in [
        sweep_history(|d| d["path"] = json!({"kind": "sketch", "value": PATH})),
        railed_sweep_history(false),
        railed_sweep_history(true),
    ] {
        let volume = volumes(&cli, &written(&model).0);
        assert_eq!(volume.len(), 1);
        assert!((volume[0] - 1000.0).abs() / 1000.0 < dutchman_solidcraft::VOLUME_TOLERANCE);
    }
}

#[test]
fn guide_rails_cannot_be_combined_with_sweep_options() {
    for change in [
        json!({"scale": 2.0}),
        json!({"twist": 0.5}),
        json!({"orientation": {"kind": "fixed"}}),
    ] {
        let model = sweep_history(|d| {
            d["guide_rail"] = json!({"path": d["path"].clone(),
                "extent": {"along_fraction": 1.0, "against_fraction": 0.0}});
            for (key, value) in change.as_object().unwrap() {
                d[key] = value.clone();
            }
        });
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(history_line(&report).contains(
            "sweeps combining a guide rail with orientation, twist or scale are not mapped yet"
        ));
    }
}

#[test]
fn solidcraft_replays_scaled_and_binormal_sweeps_and_surface_sweeps() {
    let Some(cli) = solidcraft() else { return };
    // Linear scaling from 1 to 2 gives area 100 * (1 + t)^2 over 10 mm.
    let scaled = sweep_history(|d| d["scale"] = json!(2.0));
    let volume = volumes(&cli, &written(&scaled).0);
    let expected = 1000.0 * 7.0 / 3.0;
    assert_eq!(volume.len(), 1);
    assert!((volume[0] - expected).abs() / expected < dutchman_solidcraft::VOLUME_TOLERANCE);

    let directed = sweep_history(|d| {
        d["orientation"] = json!({"kind": "binormal",
            "direction": {"x": 0.0, "y": 1.0, "z": 0.0}});
    });
    let volume = volumes(&cli, &written(&directed).0);
    assert_eq!(volume.len(), 1);
    assert!((volume[0] - 1000.0).abs() / 1000.0 < dutchman_solidcraft::VOLUME_TOLERANCE);

    let surface = sweep_history(|d| {
        d["shape"]["mode"] = json!("surface");
        d["shape"].as_object_mut().unwrap().remove("op");
    });
    let boxes = bounds(&cli, &written(&surface).0);
    assert_eq!(boxes.len(), 1);
    for (actual, expected) in [
        (boxes[0].0, [0.0, 0.0, 0.0]),
        (boxes[0].1, [10.0, 10.0, 10.0]),
    ] {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-6, "{boxes:?}");
        }
    }
}

#[test]
fn sweeps_solidcraft_would_build_differently_flatten_the_history() {
    for (change, reason) in [
        (
            json!({"shape": {"mode": "surface", "section": {"kind": "unresolved", "value": null}}}),
            "its sweep section is not resolved",
        ),
        (
            json!({"path_extent": {"along_fraction": 0.5, "against_fraction": 0.0}}),
            "sweeps along part of their path are not mapped yet",
        ),
        (json!({"taper": 0.1}), "tapered sweeps are not mapped yet"),
        (
            json!({"scale": 0.001}),
            "sweep end scales outside 0.01 to 100 are not mapped yet",
        ),
        (
            json!({"scale": 101.0}),
            "sweep end scales outside 0.01 to 100 are not mapped yet",
        ),
        (
            json!({"twist": 5.0 * std::f64::consts::TAU}),
            "sweep twists beyond four turns are not mapped yet",
        ),
        (
            json!({"orientation": {"kind": "frenet"}}),
            "sweeps oriented by frenet are not mapped yet",
        ),
        (
            json!({"orientation": {"kind": "auxiliary", "tangent": false,
                "curvilinear": true, "path": {"kind": "sketch", "value": PATH}}}),
            "sweeps oriented by an auxiliary path are not mapped yet",
        ),
        (
            json!({"orientation": {"kind": "guide_surface", "faces": {"kind": "unresolved"}}}),
            "sweeps oriented by guide faces are not mapped yet",
        ),
        (
            json!({"transition": "right_corner"}),
            "sweeps with right corner transitions are not mapped yet",
        ),
        (
            json!({"transformation": "linear"}),
            "sweeps with linear section interpolation are not mapped yet",
        ),
        (
            json!({"shape": {"mode": "solid", "op": "new_body",
                "section": {"kind": "profile", "value": first_profile()},
                "sections": [{"kind": "profile", "value": first_profile()}]}}),
            "sweeps through several sections are not mapped yet",
        ),
        (
            json!({"transition": "round_corner"}),
            "sweeps with round corner transitions are not mapped yet",
        ),
        (
            json!({"path": {"kind": "native", "value": "n"}}),
            "a path given by a native reference is not mapped yet",
        ),
        (
            json!({"path": {"kind": "unresolved", "value": "n"}}),
            "its path is not resolved",
        ),
        (
            json!({"guide_rail": {"path": {"kind": "sketch", "value": PATH},
                "extent": {"along_fraction": 0.5, "against_fraction": 0.0}}}),
            "sweeps along part of their guide rail are not mapped yet",
        ),
        (
            json!({"path_tangent": true}),
            "sweeps that extend the path through tangent-connected edges are not mapped yet",
        ),
        (
            json!({"linearize": true}),
            "sweeps that simplify linear edges and planar faces are not mapped yet",
        ),
        (
            json!({"transformation": "multi_section"}),
            "sweeps with multiple section interpolation are not mapped yet",
        ),
        (
            json!({"transformation": "s_shape"}),
            "sweeps with S-shaped section interpolation are not mapped yet",
        ),
        (
            json!({"transformation": "interpolation"}),
            "sweeps with native smooth section interpolation are not mapped yet",
        ),
        (
            json!({"shape": {"mode": "solid", "op": "new_body",
                "section": {"kind": "generated", "value": {
                    "shape": "circular_region", "region": {"outer_radius": 2.0}}}}}),
            "sweeps with generated circular sections are not mapped yet",
        ),
    ] {
        let model = sweep_history(|definition| {
            for (key, value) in change.as_object().unwrap() {
                definition[key] = value.clone();
            }
        });
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            document["features"]
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f["type"] != "sweep" && f["type"] != "surface_sweep"),
            "{reason}"
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn a_sweep_inside_an_assembly_keeps_its_own_sketches() {
    let mut model = assembly_of_histories();
    for part in &mut model.assembly.as_mut().unwrap().parts {
        part.ir = sweep_history(|_| {}).ir;
    }
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let features = document["features"].as_array().unwrap();
    let sweeps: Vec<&Value> = features.iter().filter(|f| f["type"] == "sweep").collect();
    assert_eq!(sweeps.len(), 2);
    for sweep in sweeps {
        for key in ["sketch", "path_sketch"] {
            let sketch = features.iter().find(|f| f["id"] == sweep[key]).unwrap();
            assert_eq!(sketch["type"], "sketch", "{key}");
            assert_eq!(sketch["component"], sweep["component"], "{key}");
        }
    }
}

#[test]
fn solidcraft_sweeps_the_profile_along_the_path() {
    let Some(cli) = solidcraft() else { return };
    // The cube, and twisted a quarter turn about the path through the square's corner: the
    // same square at every height, so the same volume.
    for twist in [None, Some(std::f64::consts::FRAC_PI_2)] {
        let model = sweep_history(|d| {
            if let Some(twist) = twist {
                d["twist"] = json!(twist);
            }
        });
        let volume = volumes(&cli, &written(&model).0);
        assert_eq!(volume.len(), 1, "{twist:?}");
        // A twisted sweep's faces are fitted splines: within the history check's tolerance.
        assert!(
            (volume[0] - 1000.0).abs() / 1000.0 < dutchman_solidcraft::VOLUME_TOLERANCE,
            "{twist:?}: {volume:?}"
        );
    }
    // Checked against the original cube by the writer itself.
    let (_, report) = convert(
        &sweep_history(|_| {}),
        &SolidCraftWriter::new().verification(Verification::With(cli)),
    );
    assert!(
        history_line(&report).contains("verified against the original in SolidCraft"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn a_loft_between_sketch_profiles_becomes_a_solidcraft_loft() {
    let (document, report) = convert(
        &loft_history(10.0, |_| {}),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let features = document["features"].as_array().unwrap();
    let id_of = |name: &str| features.iter().find(|f| f["name"] == name).unwrap()["id"].clone();
    let loft = features.iter().find(|f| f["type"] == "loft").unwrap();
    assert_eq!(loft["name"], "Block");
    assert_eq!(loft["operation"], "new_body");
    assert_eq!(loft["body_names"], json!(["unit cube"]));
    let sections = loft["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0]["sketch"], id_of("Base sketch"));
    assert_eq!(sections[1]["sketch"], id_of("Top sketch"));
    assert_eq!(sections[1]["profiles"]["type"], "curves");
    assert!(
        history_line(&report).contains("3 of 3 features rebuilt"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn lofts_solidcraft_would_build_differently_flatten_the_history() {
    for (change, reason) in [
        (
            json!({"ruled": true, "sections": [first_profile(),
                {"kind": "sketch_profiles", "value": {"sketch": TOP, "profiles": [0]}},
                {"kind": "point", "value": {"x": 5.0, "y": 5.0, "z": 20.0}}]}),
            "ruled lofts through more than two sections are not mapped yet",
        ),
        (json!({"closed": true}), "closed lofts are not mapped yet"),
        (
            json!({"max_degree": 3}),
            "lofts with a degree limit are not mapped yet",
        ),
        (
            json!({"guidance": {"kind": "centerline", "path": {"kind": "native", "value": "n"}}}),
            "lofts along a centreline are not mapped yet",
        ),
        (
            json!({"sections": [first_profile(), {"kind": "native_point", "value": "native:apex"}]}),
            "lofts that close to an unresolved point are not mapped yet",
        ),
        (
            json!({"sections": [first_profile(),
                {"kind": "point", "value": {"x": 5.0, "y": 5.0, "z": 10.0}},
                {"kind": "sketch", "value": TOP}]}),
            "lofts with an interior point section are not mapped yet",
        ),
        (
            json!({"sections": vec![first_profile(); 51]}),
            "a loft needs between two and fifty sections",
        ),
        (
            json!({"solid": false, "sections": [first_profile(),
                {"kind": "point", "value": {"x": 5.0, "y": 5.0, "z": 10.0}}]}),
            "surface lofts to a point are not mapped yet",
        ),
    ] {
        let model = loft_history(10.0, |definition| {
            for (key, value) in change.as_object().unwrap() {
                definition[key] = value.clone();
            }
        });
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            document["features"]
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f["type"] != "loft"),
            "{reason}"
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

/// A square pyramid, with a solved position or a B-rep vertex as its apex.
fn pointed_loft(vertex: bool, first: bool) -> Model {
    let model = loft_history(10.0, |definition| {
        let apex = json!({"kind": "point", "value": {"x": 5.0, "y": 5.0, "z": 10.0}});
        definition["sections"] = if first {
            json!([apex, first_profile()])
        } else {
            json!([first_profile(), apex])
        };
    });
    if !vertex {
        return model;
    }
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["points"].as_array_mut().unwrap().push(json!({
        "id": "synthetic:apex:point#0",
        "position": {"x": 5.0, "y": 5.0, "z": 10.0},
    }));
    wire["model"]["vertices"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "synthetic:apex:vertex#0", "point": "synthetic:apex:point#0",
        }));
    let index = if first { 0 } else { 1 };
    wire["model"]["features"][2]["definition"]["sections"][index] =
        json!({"kind": "vertex", "value": "synthetic:apex:vertex#0"});
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    }
}

#[test]
fn resolved_loft_point_ends_become_sketch_points() {
    for vertex in [false, true] {
        for first in [false, true] {
            let (document, report) = convert(
                &pointed_loft(vertex, first),
                &SolidCraftWriter::new().verification(Verification::Off),
            );
            let features = document["features"].as_array().unwrap();
            let loft = features.iter().find(|f| f["type"] == "loft").unwrap();
            let index = if first { 0 } else { 1 };
            let section = &loft["sections"][index];
            assert_eq!(section["point"], "origin");
            assert_eq!(section["profiles"], json!({"type": "all"}));
            let sketch = features
                .iter()
                .find(|f| f["id"] == section["sketch"])
                .unwrap();
            assert_eq!(sketch["plane"]["plane"]["origin"], json!([5.0, 5.0, 10.0]));
            assert_eq!(sketch["sketch"]["points"][0]["id"], "origin");
            assert!(sketch["id"].as_u64().unwrap() < loft["id"].as_u64().unwrap());
            assert!(history_line(&report).contains("3 of 3 features rebuilt"));
        }
    }
}

#[test]
fn generated_loft_apex_names_do_not_shadow_source_features() {
    let model = pointed_loft(false, false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][1]["name"] = json!("Loft apex 3");
    wire["model"]["features"][2]["name"] = json!("Loft apex 3 (2)");
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    };
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let features = document["features"].as_array().unwrap();
    let names: Vec<&str> = features.iter().filter_map(|f| f["name"].as_str()).collect();
    let unique: std::collections::BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(names.len(), unique.len());
    assert!(names.contains(&"Loft apex 3 (2)"));
}

/// Two separate square regions in one sketch form the feet of an arch; a third
/// square, halfway across and 10 mm above them, forms its middle section.
fn shared_sketch_loft() -> Model {
    let model = loft_history(10.0, |definition| {
        definition["sections"].as_array_mut().unwrap().push(json!({
            "kind": "sketch_regions", "value": {"sketch": SKETCH,
            "regions": [{"region": "loops", "outer": 1, "holes": []}]},
        }));
    });
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    m["sketches"][0]["profiles"]
        .as_array_mut()
        .unwrap()
        .push(json!([
            {"entity": "f3d:model:sketch-entity#41"},
            {"entity": "f3d:model:sketch-entity#42"},
            {"entity": "f3d:model:sketch-entity#43"},
            {"entity": "f3d:model:sketch-entity#44"},
        ]));
    m["sketch_entities"].as_array_mut().unwrap().extend([
        line(41, [20.0, 0.0], [30.0, 0.0]),
        line(42, [30.0, 0.0], [30.0, 10.0]),
        line(43, [30.0, 10.0], [20.0, 10.0]),
        line(44, [20.0, 10.0], [20.0, 0.0]),
    ]);
    m["sketches"][1]["placement"]["frame"] = json!({
        "origin": {"x": 15.0, "y": 0.0, "z": 5.0},
        "normal": {"x": 1.0, "y": 0.0, "z": 0.0},
        "u_axis": {"x": 0.0, "y": 1.0, "z": 0.0},
    });
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    }
}

#[test]
fn a_loft_section_taking_several_loops_of_its_sketch_is_refused() {
    for section in [
        json!({"kind": "sketch", "value": SKETCH}),
        json!({"kind": "sketch_profiles", "value": {"sketch": SKETCH, "profiles": [0, 1]}}),
    ] {
        let model = shared_sketch_loft();
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let loft = wire["model"]["features"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|f| f["definition"]["definition"] == "loft")
            .unwrap();
        loft["definition"]["sections"][0] = section;
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(!document["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["type"] == "loft"));
        assert!(
            history_line(&report).contains("takes several loops of its sketch"),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn loft_sections_in_one_sketch_keep_their_selected_regions() {
    let (document, report) = convert(
        &shared_sketch_loft(),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let loft = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "loft")
        .unwrap();
    assert_eq!(loft["sections"][0]["sketch"], loft["sections"][2]["sketch"]);
    assert_eq!(loft["sections"][0]["profiles"]["type"], "curves");
    assert_eq!(loft["sections"][2]["profiles"]["type"], "whole_boundaries");
    let first = &loft["sections"][0]["profiles"]["loops"][0];
    let last = &loft["sections"][2]["profiles"]["profiles"][0]["outer"];
    assert_eq!(first.as_array().unwrap().len(), 4);
    assert_eq!(last.as_array().unwrap().len(), 4);
    assert!(first
        .as_array()
        .unwrap()
        .iter()
        .all(|id| !last.as_array().unwrap().contains(id)));
    assert!(history_line(&report).contains("3 of 3 features rebuilt"));
}

#[test]
fn solidcraft_lofts_to_resolved_point_ends() {
    let Some(cli) = solidcraft() else { return };
    for vertex in [false, true] {
        for first in [false, true] {
            let volume = volumes(&cli, &written(&pointed_loft(vertex, first)).0);
            assert_eq!(volume.len(), 1);
            let expected = 1000.0 / 3.0;
            assert!(
                (volume[0] - expected).abs() / expected < dutchman_solidcraft::VOLUME_TOLERANCE,
                "{volume:?}"
            );
        }
    }
}

#[test]
fn solidcraft_lofts_through_regions_of_a_shared_sketch() {
    let Some(cli) = solidcraft() else { return };
    let volume = volumes(&cli, &written(&shared_sketch_loft()).0);
    assert_eq!(volume.len(), 1);
    assert!(volume[0].is_finite() && volume[0] > 0.0, "{volume:?}");
}

#[test]
fn a_loft_inside_an_assembly_keeps_its_own_sketches() {
    for pointed in [false, true] {
        let mut model = assembly_of_histories();
        let assembly = model.assembly.as_mut().unwrap();
        for part in &mut assembly.parts {
            part.ir = if pointed {
                pointed_loft(false, false).ir
            } else {
                loft_history(10.0, |_| {}).ir
            };
        }
        let (document, _) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let features = document["features"].as_array().unwrap();
        let lofts: Vec<&Value> = features.iter().filter(|f| f["type"] == "loft").collect();
        assert_eq!(lofts.len(), 2);
        for loft in lofts {
            for section in loft["sections"].as_array().unwrap() {
                let sketch = features
                    .iter()
                    .find(|f| f["id"] == section["sketch"])
                    .unwrap();
                assert_eq!(sketch["type"], "sketch");
                assert_eq!(sketch["component"], loft["component"]);
            }
        }
    }
}

const RAIL: &str = "f3d:model:sketch#4";

/// [`loft_history`] guided by a rail: a line up Z along the cube's edge at the origin, drawn in
/// its own sketch on XZ before the loft.
fn railed_loft(top: f64) -> Model {
    let model = loft_history(top, |definition| {
        definition["guidance"] = json!({"kind": "guides", "path": [
            {"kind": "sketch_curves", "value": {"sketch": RAIL, "curves": ["f3d:model:sketch-entity#31"]}}
        ]});
    });
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    m["sketches"].as_array_mut().unwrap().push(json!({
        "id": RAIL,
        "name": "Rail",
        "placement": {"kind": "resolved", "frame": {
            "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
            "normal": {"x": 0.0, "y": -1.0, "z": 0.0},
            "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0},
        }},
        "profiles": [],
    }));
    let mut rail = line(31, [0.0, 0.0], [0.0, 10.0]);
    rail["sketch"] = json!(RAIL);
    m["sketch_entities"].as_array_mut().unwrap().push(rail);
    let features = m["features"].as_array_mut().unwrap();
    features.insert(
        2,
        json!({
            "id": "f3d:model:feature#4",
            "ordinal": 2,
            "name": "Rail sketch",
            "suppressed": false,
            "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": RAIL}},
        }),
    );
    features[3]["ordinal"] = json!(3);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    }
}

/// A whole-sketch rail: the rail line, or with `split` that line in two spans, beside a
/// construction line and a standalone point.
fn whole_sketch_railed_loft(split: bool) -> Model {
    let model = railed_loft(10.0);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    m["features"][3]["definition"]["guidance"]["path"] = json!([{"kind": "sketch", "value": RAIL}]);
    let entities = m["sketch_entities"].as_array_mut().unwrap();
    if split {
        let first = entities
            .iter_mut()
            .find(|e| e["id"] == "f3d:model:sketch-entity#31")
            .unwrap();
        first["geometry"]["end"]["v"] = json!(5.0);
        let mut second = line(32, [0.0, 5.0], [0.0, 10.0]);
        second["sketch"] = json!(RAIL);
        entities.push(second);
    }
    let mut construction = line(33, [1.0, 0.0], [1.0, 10.0]);
    construction["sketch"] = json!(RAIL);
    construction["construction"] = json!(true);
    let mut point = entity(
        34,
        json!({"kind": "point", "position": {"u": 2.0, "v": 2.0}}),
    );
    point["sketch"] = json!(RAIL);
    entities.extend([construction, point]);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    }
}

#[test]
fn a_whole_sketch_loft_rail_is_its_one_curve() {
    let (document, report) = convert(
        &whole_sketch_railed_loft(false),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let features = document["features"].as_array().unwrap();
    let sketch = features
        .iter()
        .find(|f| f["name"] == "Rail sketch")
        .unwrap();
    let loft = features.iter().find(|f| f["type"] == "loft").unwrap();
    let curve = sketch["sketch"]["curves"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["construction"] != json!(true))
        .unwrap();
    assert_eq!(loft["rails"][0]["sketch"], sketch["id"]);
    assert_eq!(loft["rails"][0]["curves"], json!([curve["id"]]));
    assert!(history_line(&report).contains("4 of 4 features rebuilt"));

    let (_, report) = convert(
        &whole_sketch_railed_loft(true),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains(
            "a whole-sketch guide rail with several curves has no resolved traversal order"
        ),
        "{}",
        history_line(&report)
    );
}

#[test]
fn a_loft_rail_of_edges_is_refused() {
    let model = loft_history(10.0, |definition| {
        definition["guidance"] = json!({"kind": "guides", "path": [
            {"kind": "edges", "value": []}
        ]});
    });
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(document["features"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["type"] != "loft"));
    assert!(
        history_line(&report).contains("a guide rail of topological edges is not mapped yet"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn solidcraft_lofts_along_a_whole_sketch_rail() {
    let Some(cli) = solidcraft() else { return };
    let volume = volumes(&cli, &written(&whole_sketch_railed_loft(false)).0);
    assert_eq!(volume.len(), 1);
    assert!(
        (volume[0] - 1000.0).abs() / 1000.0 < dutchman_solidcraft::VOLUME_TOLERANCE,
        "{volume:?}"
    );
}

#[test]
fn a_loft_along_a_guide_rail_keeps_its_rail() {
    let (document, report) = convert(
        &railed_loft(10.0),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let features = document["features"].as_array().unwrap();
    let rail_sketch = features
        .iter()
        .find(|f| f["name"] == "Rail sketch")
        .unwrap()["id"]
        .clone();
    let loft = features.iter().find(|f| f["type"] == "loft").unwrap();
    let rails = loft["rails"].as_array().unwrap();
    assert_eq!(rails.len(), 1);
    assert_eq!(rails[0]["sketch"], rail_sketch);
    assert_eq!(rails[0]["curves"].as_array().unwrap().len(), 1);
    assert!(
        history_line(&report).contains("4 of 4 features rebuilt"),
        "{}",
        history_line(&report)
    );

    // Three rails are more than SolidCraft takes.
    let three = loft_history(10.0, |definition| {
        let rail = json!({"kind": "sketch_curves", "value": {"sketch": TOP, "curves": ["f3d:model:sketch-entity#11"]}});
        definition["guidance"] =
            json!({"kind": "guides", "path": [rail.clone(), rail.clone(), rail]});
    });
    let (_, report) = convert(
        &three,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains("lofts along more than two guide rails are not mapped yet"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn solidcraft_lofts_along_the_rail() {
    let Some(cli) = solidcraft() else { return };
    let volume = volumes(&cli, &written(&railed_loft(10.0)).0);
    assert_eq!(volume.len(), 1);
    assert!(
        (volume[0] - 1000.0).abs() / 1000.0 < dutchman_solidcraft::VOLUME_TOLERANCE,
        "{volume:?}"
    );
    // Checked against the original cube by the writer itself.
    let (_, report) = convert(
        &railed_loft(10.0),
        &SolidCraftWriter::new().verification(Verification::With(cli)),
    );
    assert!(
        history_line(&report).contains("verified against the original in SolidCraft"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn ruled_and_surface_lofts_map_to_what_solidcraft_builds() {
    for (change, kind) in [
        (json!({"ruled": true}), "loft"),
        (json!({"solid": false}), "surface_loft"),
        (json!({"linearize": true}), "loft"),
    ] {
        let model = loft_history(4.0, |definition| {
            for (key, value) in change.as_object().unwrap() {
                definition[key] = value.clone();
            }
        });
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let features = document["features"].as_array().unwrap();
        let loft = features.iter().find(|f| f["name"] == "Block").unwrap();
        assert_eq!(loft["type"], kind, "{change}");
        assert_eq!(loft["sections"].as_array().unwrap().len(), 2);
        assert_eq!(loft.get("operation").is_none(), kind == "surface_loft");
        assert!(
            history_line(&report).contains("3 of 3 features rebuilt"),
            "{}",
            history_line(&report)
        );
    }
    let railed = railed_loft(10.0);
    let mut wire = serde_json::to_value(&railed.ir).unwrap();
    for feature in wire["model"]["features"].as_array_mut().unwrap() {
        if feature["definition"]["definition"] == "loft" {
            feature["definition"]["ruled"] = json!(true);
        }
    }
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..railed
    };
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains("ruled lofts along guide rails are not mapped yet"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn solidcraft_replays_ruled_and_surface_lofts() {
    let Some(cli) = solidcraft() else { return };
    // A ruled frustum from the 10 mm base square to a 4 mm top: 520 mm³.
    let ruled = loft_history(4.0, |d| d["ruled"] = json!(true));
    let volume = volumes(&cli, &written(&ruled).0);
    assert_eq!(volume.len(), 1);
    assert!((volume[0] - 520.0).abs() < 1e-6, "{volume:?}");

    // The surface loft is the cube's four walls: one sheet with the cube's bounds.
    let surface = loft_history(10.0, |d| {
        d["solid"] = json!(false);
    });
    let boxes = bounds(&cli, &written(&surface).0);
    assert_eq!(boxes.len(), 1);
    for (actual, expected) in [
        (boxes[0].0, [0.0, 0.0, 0.0]),
        (boxes[0].1, [10.0, 10.0, 10.0]),
    ] {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-6, "{boxes:?}");
        }
    }
}

#[test]
fn solidcraft_lofts_between_the_sections() {
    let Some(cli) = solidcraft() else { return };
    // The cube again, then a frustum: 10/3 × (100 + 16 + 40) mm³.
    for (top, expected) in [(10.0, 1000.0), (4.0, 520.0)] {
        let volume = volumes(&cli, &written(&loft_history(top, |_| {})).0);
        assert_eq!(volume.len(), 1);
        assert!((volume[0] - expected).abs() < 1e-6, "{volume:?}");
    }
}

fn whole_sketch() -> Value {
    json!({"kind": "sketch", "value": SKETCH})
}

fn first_profile() -> Value {
    json!({"kind": "sketch_profiles", "value": {"sketch": SKETCH, "profiles": [0]}})
}

fn convert(model: &Model, writer: &SolidCraftWriter) -> (Value, Report) {
    let mut report = Report::default();
    let mut bytes = Vec::new();
    writer.write(model, &mut bytes, &mut report).unwrap();
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name("document.json")
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    (serde_json::from_str(&text).unwrap(), report)
}

fn history_line(report: &Report) -> String {
    report
        .losses()
        .iter()
        .filter(|loss| loss.subject == "history")
        .map(|loss| loss.detail.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

fn types(document: &Value) -> Vec<(String, bool)> {
    document["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["type"].as_str().unwrap().to_owned(),
                f["suppressed"].as_bool().unwrap(),
            )
        })
        .collect()
}

#[test]
fn a_sketch_and_extrude_history_is_written() {
    let model = cube_with_history(10.0, first_profile(), None);
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, report) = convert(&model, &writer);

    assert_eq!(
        history_line(&report),
        "2 of 2 features rebuilt; unverified: verification turned off"
    );
    assert_eq!(
        types(&document),
        [
            ("sketch".to_owned(), false),
            ("extrude".to_owned(), false),
            ("import".to_owned(), true)
        ]
    );

    let mut pretty = serde_json::to_string_pretty(&document).unwrap() + "\n";
    // The STEP blob's name depends on cadmpeg's STEP writer, not on the history.
    let blob = document["features"][2]["step_blob"].as_str().unwrap();
    pretty = pretty.replace(blob, "blobs/<sha256>.step");
    let golden =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/cube_history.document.json");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &pretty).unwrap();
    }
    assert_eq!(pretty, std::fs::read_to_string(&golden).unwrap());
}

#[test]
fn an_unverified_history_can_be_kept_hidden_beside_the_visible_original() {
    let model = cube_with_history(10.0, first_profile(), None);
    let writer = SolidCraftWriter::new()
        .verification(Verification::Off)
        .hide_unverified_history(true);
    let (document, report) = convert(&model, &writer);

    assert!(history_line(&report).contains("unverified: verification turned off; kept hidden"));
    // The original is the visible model; the history is all there, in a hidden part.
    assert_eq!(
        types(&document),
        [
            ("sketch".to_owned(), false),
            ("extrude".to_owned(), false),
            ("import".to_owned(), false)
        ]
    );
    assert_eq!(
        document["components"][0]["name"],
        "Editable history (unverified)"
    );
    let occurrence = document["occurrences"][0]["id"].clone();
    assert_eq!(
        document["visibility"]["hidden_occurrences"],
        json!([[occurrence]])
    );
}

#[test]
fn a_symmetric_through_all_extent_is_mapped() {
    let extent = json!({"kind": "symmetric", "side": {"termination": {"kind": "through_all"}}});
    let model = cube_with_history(10.0, whole_sketch(), Some(extent));
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, _) = convert(&model, &writer);
    let extrude = &document["features"][1];
    assert_eq!(extrude["profiles"], json!({"type": "all"}));
    assert_eq!(
        extrude["extent"],
        json!({"direction": "symmetric", "convention": "total_distance",
               "distance": "0 mm", "through_all": true})
    );
}

/// The final cube with a blocked feature between two otherwise mapped sketch features.
fn reference_sketch_history() -> Model {
    let mut model = loft_history(10.0, |definition| {
        *definition = json!({"definition": "native", "kind": "Reference blocker"});
    });
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][1]["ordinal"] = json!(2);
    wire["model"]["features"][2]["ordinal"] = json!(1);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

fn assert_reference_sketches(document: &Value, count: usize) {
    let mut expected = vec![("import".to_owned(), false)];
    expected.extend((0..count).map(|_| ("sketch".to_owned(), false)));
    assert_eq!(types(document), expected);
    let features = document["features"].as_array().unwrap();
    let ids: Vec<_> = features.iter().skip(1).map(|f| f["id"].clone()).collect();
    assert_eq!(document["visibility"]["hidden_sketches"], json!(ids));
    for feature in features.iter().skip(1) {
        assert_eq!(feature["component"], document["features"][0]["component"]);
        assert!(feature["id"].as_u64().unwrap() > 3);
        assert!(feature["id"].as_u64().unwrap() < document["next_id"].as_u64().unwrap());
    }
    let unique: std::collections::BTreeSet<_> =
        features.iter().map(|f| f["id"].as_u64().unwrap()).collect();
    assert_eq!(unique.len(), features.len());
}

#[test]
fn flattened_history_keeps_mapped_sketches_hidden_in_source_order() {
    let model = reference_sketch_history();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_reference_sketches(&document, 2);
    assert_eq!(document["features"][1]["name"], "Base sketch");
    assert_eq!(document["features"][2]["name"], "Top sketch");
    assert_eq!(document["features"][2]["plane"]["type"], "custom");
    // The first sketch's dimension still has its parameter.
    assert!(document["params"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "side"));
    let losses: Vec<_> = report
        .losses()
        .iter()
        .filter(|loss| loss.subject == "sketches")
        .collect();
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0].severity, Severity::Info);
    assert_eq!(losses[0].detail, "kept 2 sketches for reference");
    assert!(matches!(
        report.outcome().unwrap().check,
        Check::Flattened(_)
    ));
}

#[test]
fn flattened_history_leaves_out_a_sketch_with_an_unresolved_plane() {
    let mut model = reference_sketch_history();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketches"][0]["placement"] = json!({"kind": "unresolved"});
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_reference_sketches(&document, 1);
    assert_eq!(document["features"][1]["name"], "Top sketch");
    let losses: Vec<_> = report
        .losses()
        .iter()
        .filter(|loss| loss.subject == "sketches")
        .collect();
    assert_eq!(losses.len(), 1);
    assert_eq!(losses[0].severity, Severity::Info);
    assert_eq!(losses[0].detail, "kept 1 sketch for reference; 1 left out");
}

#[test]
fn flattened_history_leaves_out_a_sketch_that_does_not_map() {
    let mut model = reference_sketch_history();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketch_entities"][0]["geometry"] =
        json!({"kind": "native", "native_kind": "Reference curve"});
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_reference_sketches(&document, 1);
    assert_eq!(document["features"][1]["name"], "Top sketch");
    assert!(report.losses().iter().any(|loss| {
        loss.subject == "sketches"
            && loss.severity == Severity::Info
            && loss.detail == "kept 1 sketch for reference; 1 left out"
    }));
}

#[test]
fn reference_sketches_preserve_duplicate_source_names() {
    let mut model = reference_sketch_history();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][1]["name"] = json!("Base sketch");
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_reference_sketches(&document, 2);
    assert_eq!(document["features"][1]["name"], "Base sketch");
    assert_eq!(document["features"][2]["name"], "Base sketch");
}

#[test]
fn hidden_reference_sketches_leave_the_imported_body_unchanged_in_solidcraft() {
    let Some(cli) = solidcraft() else { return };
    let model = reference_sketch_history();
    let (bytes, _) = written(&model);
    let mut original = Vec::new();
    SolidCraftWriter::new()
        .import_only(true)
        .verification(Verification::Off)
        .write(&model, &mut original, &mut Report::default())
        .unwrap();
    let actual = volumes(&cli, &bytes);
    assert_eq!(actual, volumes(&cli, &original));
    assert_eq!(actual.len(), 1);
    assert!((actual[0] - 1000.0).abs() < 1e-6);
}

#[test]
fn import_only_never_writes_history() {
    let model = cube_with_history(10.0, first_profile(), None);
    let (document, report) = convert(&model, &SolidCraftWriter::new().import_only(true));
    assert_eq!(history_line(&report), "not written (--import-only)");
    assert_eq!(
        types(&document),
        [("import".to_owned(), false), ("sketch".to_owned(), false)]
    );
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().import_only(true).sketches(false),
    );
    assert_eq!(types(&document), [("import".to_owned(), false)]);
}

/// The SolidCraft CLI the replay tests use, if one is configured.
fn solidcraft() -> Option<PathBuf> {
    let cli = std::env::var_os(CLI_ENV).map(PathBuf::from);
    if cli.is_none() {
        eprintln!("skipped: set {CLI_ENV} to replay histories in SolidCraft");
    }
    cli
}

#[test]
fn a_faithful_history_is_verified_in_solidcraft() {
    let Some(cli) = solidcraft() else { return };
    let model = cube_with_history(10.0, first_profile(), None);
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::With(cli)),
    );
    assert_eq!(
        history_line(&report),
        "2 of 2 features rebuilt; verified against the original in SolidCraft"
    );
    assert_eq!(types(&document).len(), 3);
}

#[test]
fn a_history_that_replays_differently_stays_editable_beside_the_original() {
    let Some(cli) = solidcraft() else { return };
    // The history says 4 mm; the geometry is the 10 mm cube.
    let model = cube_with_history(4.0, first_profile(), None);
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::With(cli)),
    );
    let line = history_line(&report);
    assert!(line.contains("replay differs"), "{line}");
    assert_partial(&document, &report, 1);
    assert_eq!(document["features"][1]["type"], "extrude");
    assert!(
        line.contains("retained features replay in SolidCraft"),
        "{line}"
    );
    assert_eq!(
        report
            .outcome()
            .unwrap()
            .features
            .iter()
            .filter(|f| f.status == dutchman_core::FeatureStatus::Mapped)
            .count(),
        2
    );
}

/// The design's container with parameter `name`'s expression changed, every other entry kept.
fn with_param(bytes: &[u8], name: &str, expr: &str) -> Vec<u8> {
    let mut archive = ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let path = entry.name().unwrap().into_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        if path == "document.json" {
            let mut document: Value = serde_json::from_slice(&data).unwrap();
            for param in document["params"].as_array_mut().unwrap() {
                if param["name"] == name {
                    param["expr"] = json!(expr);
                }
            }
            data = serde_json::to_vec(&document).unwrap();
        }
        out.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut out, &data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

/// SolidCraft's evaluation of a design, requiring every timeline feature to succeed.
fn evaluated(cli: &Path, design: &[u8]) -> Value {
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
    for entry in result["timeline"].as_array().unwrap() {
        assert!(entry["error"].is_null(), "{entry}");
    }
    result
}

/// SolidCraft's body volumes for a design.
fn volumes(cli: &Path, design: &[u8]) -> Vec<f64> {
    evaluated(cli, design)["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|body| body["volume_mm3"].as_f64().unwrap())
        .collect()
}

#[test]
fn a_converted_dimension_drives_the_geometry_in_solidcraft() {
    let Some(cli) = solidcraft() else { return };
    let model = cube_with_history(10.0, first_profile(), None);
    let mut report = Report::default();
    let mut bytes = Vec::new();
    SolidCraftWriter::new()
        .verification(Verification::Off)
        .write(&model, &mut bytes, &mut report)
        .unwrap();

    assert_eq!(volumes(&cli, &bytes), [1000.0]);
    assert_eq!(
        volumes(&cli, &with_param(&bytes, "side", "20 mm")),
        [4000.0]
    );
}

/// The example cube's geometry with a revolve history: a 5 mm × 10 mm rectangle on XZ, 5 mm
/// to 10 mm out from Z, turned about the Z axis by `extent`.
fn ring_history(extent: Value) -> Model {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    m["sketches"][0]["placement"]["frame"]["normal"] = json!({"x": 0.0, "y": 1.0, "z": 0.0});
    // On XZ, sketch v runs along -Z.
    m["sketch_entities"] = json!([
        line(1, [5.0, 0.0], [10.0, 0.0]),
        line(2, [10.0, 0.0], [10.0, -10.0]),
        line(3, [10.0, -10.0], [5.0, -10.0]),
        line(4, [5.0, -10.0], [5.0, 0.0]),
    ]);
    m["sketch_constraints"] = json!([]);
    m["features"][1] = json!({
        "id": "f3d:model:feature#2",
        "ordinal": 1,
        "name": "Ring",
        "suppressed": false,
        "definition": {
            "definition": "revolve",
            "construction": {
                "state": "resolved",
                "profile": whole_sketch(),
                "axis": {
                    "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
                    "direction": {"x": 0.0, "y": 0.0, "z": 1.0},
                },
                "extent": extent,
            },
            "op": "new_body",
        },
    });
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

fn changed_ring(change: impl FnOnce(&mut Value)) -> Model {
    let mut model = ring_history(json!({"kind": "one_sided",
        "termination": {"kind": "angle", "angle": std::f64::consts::TAU}}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    change(&mut wire["model"]["features"][1]["definition"]["construction"]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

fn written(model: &Model) -> (Vec<u8>, Report) {
    written_with(
        model,
        &SolidCraftWriter::new()
            .verification(Verification::Off)
            .target_version(Some(dutchman_solidcraft::SolidCraftVersion::detected(
                &Verification::Auto,
            ))),
    )
}

fn helical_history(change: impl FnOnce(&mut Value)) -> Model {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    m["sketch_constraints"] = json!([]);
    m["sketch_entities"] = json!([entity(
        1,
        json!({
            "kind": "circle", "center": {"u": 10.0, "v": 0.0}, "radius": 1.0
        })
    )]);
    m["sketches"][0]["profiles"] = json!([[{"entity": "f3d:model:sketch-entity#1"}]]);
    m["sketches"][0]["placement"]["frame"] = json!({
        "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
        "normal": {"x": 0.0, "y": -0.9968486721503291, "z": -0.0793266968436579},
        "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0}
    });
    m["parameters"] = json!([
        {"id": "f3d:model:parameter#1", "owner": "f3d:model:feature#2", "name": "Pitch",
         "expression": "5 mm", "value": {"kind": "length", "value": 5.0}},
        {"id": "f3d:model:parameter#2", "owner": "f3d:model:feature#2", "name": "Height",
         "expression": "20 mm", "value": {"kind": "length", "value": 20.0}},
        {"id": "f3d:model:parameter#3", "owner": "f3d:model:feature#2", "name": "Turns",
         "expression": "4", "value": {"kind": "real", "value": 4.0}}
    ]);
    m["features"][1]["definition"] = json!({
        "definition": "post_process", "refine": true, "fuzzy_tolerance": {"kind": "kernel_default"},
        "operation": {"definition": "helical_sweep", "op": "join", "construction": {
            "profile": {"kind": "sketch", "value": SKETCH},
            "axis_origin": {"x": 0.0, "y": 0.0, "z": 0.0},
            "axis_direction": {"x": 1e-16, "y": -1e-16, "z": 1.0},
            "law": "pitch_height_angle", "pitch": 5.0,
            "travel": {"kind": "axial", "height": 20.0}, "turns": 4.0,
            "cone_angle": 0.0, "left_handed": false, "reversed": false,
            "tolerance": 0.0001, "allow_multi_profile_faces": false
        }}
    });
    change(&mut m["features"][1]["definition"]["operation"]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

fn helix_cli() -> Option<PathBuf> {
    let cli = solidcraft()?;
    let version =
        dutchman_solidcraft::SolidCraftVersion::detected(&Verification::With(cli.clone()));
    if version < dutchman_solidcraft::SolidCraftVersion::V0_25_1 {
        eprintln!("skipped: sketch-profile coils need SolidCraft 0.25 patch 1 or newer");
        return None;
    }
    Some(cli)
}

#[test]
fn helical_sweeps_block_history_for_the_compatibility_floor() {
    let (document, report) = convert(
        &helical_history(|_| {}),
        &SolidCraftWriter::new()
            .verification(Verification::Off)
            .target_version(Some("0.18.2".parse().unwrap())),
    );
    assert_eq!(
        types(&document).first(),
        Some(&("import".to_owned(), false))
    );
    let reason = report
        .outcome()
        .unwrap()
        .features
        .iter()
        .find(|feature| feature.kind == "helical_sweep")
        .unwrap()
        .reason
        .as_deref()
        .unwrap();
    assert_eq!(
        reason,
        "needs SolidCraft 0.25 patch 1 or newer (target 0.18)"
    );
    assert!(history_line(&report).contains(reason));
}

#[test]
fn helical_sweeps_keep_the_profile_axis_law_hand_and_operation() {
    for (law, pitch, turns) in [
        ("pitch_height_angle", "Pitch", "(Height) / (Pitch)"),
        ("pitch_turns_angle", "Pitch", "Turns"),
        ("height_turns_angle", "(Height) / (Turns)", "Turns"),
        ("height_turns_growth", "(Height) / (Turns)", "Turns"),
    ] {
        for op in ["join", "cut", "intersect"] {
            for reversed in [false, true] {
                for left in [false, true] {
                    let model = helical_history(|d| {
                        d["op"] = json!(op);
                        d["construction"]["law"] = json!(law);
                        d["construction"]["reversed"] = json!(reversed);
                        d["construction"]["left_handed"] = json!(left);
                    });
                    let (document, report) = convert(
                        &model,
                        &SolidCraftWriter::new()
                            .verification(Verification::Off)
                            .target_version(Some(dutchman_solidcraft::SolidCraftVersion::V0_25_1)),
                    );
                    let coil = &document["features"][1];
                    assert!(history_line(&report).contains("target SolidCraft 0.25"));
                    assert_eq!(coil["type"], "coil");
                    assert_eq!(coil["sketch"], document["features"][0]["id"]);
                    assert_eq!(coil["profiles"], json!({"type": "all"}));
                    assert_eq!(
                        coil["axis"],
                        json!([0.0, 0.0, if reversed { -1.0 } else { 1.0 }])
                    );
                    assert_eq!(coil["pitch"], pitch);
                    assert_eq!(coil["turns"], turns);
                    assert_eq!(coil["clockwise"], left);
                    assert_eq!(coil["operation"], op);
                }
            }
        }
    }
}

#[test]
fn conical_helical_sweeps_name_the_missing_solidcraft_option() {
    let model = helical_history(|d| {
        d["construction"]["cone_angle"] = json!(0.1);
        d["construction"]["travel"] =
            json!({"kind": "conical", "height": 20.0, "radial_growth": 0.5});
    });
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new()
            .verification(Verification::Off)
            .target_version(Some(dutchman_solidcraft::SolidCraftVersion::V0_25_1)),
    );
    assert!(history_line(&report).contains("no radial growth or cone angle"));
}

#[test]
fn helical_turn_parameters_drive_the_replayed_coil() {
    let Some(cli) = helix_cli() else { return };
    let model = helical_history(|d| d["construction"]["law"] = json!("pitch_turns_angle"));
    let bytes = written(&model).0;
    let full = volumes(&cli, &bytes);
    let half = volumes(&cli, &with_param(&bytes, "Turns", "2"));
    assert_eq!(full.len(), 1);
    assert_eq!(half.len(), 1);
    assert!(
        (full[0] / half[0] - 2.0).abs() < 1e-3,
        "{full:?} / {half:?}"
    );
}

#[test]
fn helical_handedness_and_reversal_replay_on_the_correct_side_of_the_axis() {
    let Some(cli) = helix_cli() else { return };
    for reversed in [false, true] {
        for left in [false, true] {
            let model = helical_history(|d| {
                d["construction"]["law"] = json!("pitch_turns_angle");
                d["construction"]["left_handed"] = json!(left);
                d["construction"]["reversed"] = json!(reversed);
            });
            let bytes = with_param(&written(&model).0, "Turns", "0.25");
            let measurement = evaluated(&cli, &bytes);
            let center = &measurement["bodies"][0]["center_of_mass"];
            let y = center[1].as_f64().unwrap();
            let z = center[2].as_f64().unwrap();
            assert_eq!(y.is_sign_negative(), left != reversed, "{measurement}");
            assert_eq!(z.is_sign_negative(), reversed, "{measurement}");
        }
    }
}

#[test]
fn freecad_f13a_keeps_its_helix_and_replays_at_the_recipe_volume() {
    let input =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/F13a-freecad-1.1.4.FCStd");
    let model = dutchman_core::read(
        dutchman_core::Format::FreeCadDocument,
        &input,
        &mut Report::default(),
    )
    .unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new()
            .verification(Verification::Off)
            .target_version(Some(dutchman_solidcraft::SolidCraftVersion::V0_25_1)),
    );
    assert!(
        history_line(&report).starts_with("2 of 2 features rebuilt"),
        "{}",
        history_line(&report)
    );
    let coil = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "coil")
        .unwrap();
    assert_eq!(coil["axis"], json!([0.0, 0.0, 1.0]));
    let Some(cli) = helix_cli() else { return };
    let (bytes, report) = written_with(
        &model,
        &SolidCraftWriter::new().verification(Verification::With(cli.clone())),
    );
    assert_eq!(report.outcome().unwrap().mode, Mode::History);
    assert_eq!(report.outcome().unwrap().check, Check::Verified);
    let volume = volumes(&cli, &bytes);
    assert_eq!(volume.len(), 1);
    // The existing replay bound is area times mesh chord (5e-5 of the body's size).
    let allowed = 1590.4 * 5e-5 * 22.0;
    assert!((volume[0] - 792.06459406).abs() <= allowed, "{volume:?}");
}

fn written_with(model: &Model, writer: &SolidCraftWriter) -> (Vec<u8>, Report) {
    let mut report = Report::default();
    let mut bytes = Vec::new();
    writer.write(model, &mut bytes, &mut report).unwrap();
    (bytes, report)
}

#[test]
fn full_and_symmetric_revolves_are_written() {
    let full = ring_history(json!({"kind": "one_sided",
        "termination": {"kind": "angle", "angle": std::f64::consts::TAU}}));
    let (document, report) = convert(
        &full,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    let revolve = &document["features"][1];
    assert_eq!(revolve["type"], "revolve");
    assert_eq!(revolve["angle"], "360 deg");
    assert_eq!(
        revolve["axis"],
        json!({"type": "line", "origin": [0.0, 0.0, 0.0], "dir": [0.0, 0.0, 1.0]})
    );
    assert_eq!(revolve["convention"], "sketch_normal");
}

#[test]
fn one_sided_revolves_follow_the_oriented_axis() {
    let cli = solidcraft();
    for axis in [1.0, -1.0] {
        for normal in [1.0, -1.0] {
            for radial in [1.0, -1.0] {
                for placed in [false, true] {
                    let mut model = ring_history(json!({"kind": "one_sided",
                    "termination": {"kind": "angle", "angle": std::f64::consts::PI / 3.0}}));
                    let mut wire = serde_json::to_value(&model.ir).unwrap();
                    wire["model"]["sketches"][0]["placement"]["frame"]["normal"]["y"] =
                        json!(normal);
                    wire["model"]["features"][1]["definition"]["construction"]["axis"]
                        ["direction"]["z"] = json!(axis);
                    for entity in wire["model"]["sketch_entities"].as_array_mut().unwrap() {
                        for end in ["start", "end"] {
                            let point = &mut entity["geometry"][end];
                            point["u"] = json!(point["u"].as_f64().unwrap() * radial);
                            point["v"] = json!(point["v"].as_f64().unwrap() * normal);
                        }
                    }
                    if placed {
                        // Cycle world axes and translate: sketch u is +Y, normal is +/-Z,
                        // and the revolution axis is +/-X through the moved sketch origin.
                        wire["model"]["sketches"][0]["placement"]["frame"] = json!({
                            "origin": {"x": 11.0, "y": 22.0, "z": 33.0},
                            "normal": {"x": 0.0, "y": 0.0, "z": normal},
                            "u_axis": {"x": 0.0, "y": 1.0, "z": 0.0}
                        });
                        wire["model"]["features"][1]["definition"]["construction"]["axis"] = json!({
                            "origin": {"x": 11.0, "y": 22.0, "z": 33.0},
                            "direction": {"x": axis, "y": 0.0, "z": 0.0}
                        });
                    }
                    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
                    let (design, report) = written(&model);
                    assert!(
                        history_line(&report).starts_with("2 of 2 features rebuilt"),
                        "{report:?}"
                    );
                    let (document, _) = convert(
                        &model,
                        &SolidCraftWriter::new().verification(Verification::Off),
                    );
                    let revolve = &document["features"][1];
                    let degrees = revolve["angle"]
                        .as_str()
                        .unwrap()
                        .trim_end_matches(" deg")
                        .parse::<f64>()
                        .unwrap();
                    assert!((degrees - 60.0 * axis * radial * normal).abs() < 1e-10);
                    assert_eq!(revolve["convention"], "sketch_normal");
                    if let Some(cli) = &cli {
                        let side = axis * radial;
                        let expected_min = [
                            if radial > 0.0 { 2.5 } else { -10.0 },
                            if side > 0.0 { 0.0 } else { -75.0_f64.sqrt() },
                            0.0,
                        ];
                        let expected_max = [
                            if radial > 0.0 { 10.0 } else { -2.5 },
                            if side > 0.0 { 75.0_f64.sqrt() } else { 0.0 },
                            10.0,
                        ];
                        let place = |corner: [f64; 3]| {
                            if placed {
                                [corner[2] + 11.0, corner[0] + 22.0, corner[1] + 33.0]
                            } else {
                                corner
                            }
                        };
                        let expected_min = place(expected_min);
                        let expected_max = place(expected_max);
                        let boxes = bounds(cli, &design);
                        assert_eq!(boxes.len(), 1);
                        for coordinate in 0..3 {
                            assert!(
                                (boxes[0].0[coordinate] - expected_min[coordinate]).abs() < 1e-6,
                                "{boxes:?}"
                            );
                            assert!(
                                (boxes[0].1[coordinate] - expected_max[coordinate]).abs() < 1e-6,
                                "{boxes:?}"
                            );
                        }
                        let actual = volumes(cli, &design);
                        let expected = 125.0 * std::f64::consts::PI;
                        assert_eq!(actual.len(), 1);
                        assert!((actual[0] - expected).abs() / expected < 1e-3, "{actual:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn one_sided_revolves_keep_selected_curves_and_a_sketch_axis() {
    for axis in [1.0, -1.0] {
        let mut model = ring_history(json!({"kind": "one_sided",
            "termination": {"kind": "angle", "angle": std::f64::consts::FRAC_PI_2}}));
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        // Move the axis to the inner profile edge. Its sketch line runs opposite +Z.
        let construction = &mut wire["model"]["features"][1]["definition"]["construction"];
        construction["axis"]["origin"]["x"] = json!(5.0);
        construction["axis"]["direction"]["z"] = json!(axis);
        construction["axis"]["reference"] = json!({"kind": "sketch_curves", "value": {
            "sketch": SKETCH, "curves": ["f3d:model:sketch-entity#4"]}});
        construction["profile"] = json!({"kind": "sketch_entities", "value": {
            "sketch": SKETCH, "entities": (1..=4).map(|id| format!("f3d:model:sketch-entity#{id}")).collect::<Vec<_>>()}});
        // An unselected curve on the other side must not change the proof.
        wire["model"]["sketch_entities"]
            .as_array_mut()
            .unwrap()
            .push(line(5, [-10.0, 0.0], [-10.0, -10.0]));
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).starts_with("2 of 2 features rebuilt"),
            "{report:?}"
        );
        assert_eq!(document["features"][1]["axis"]["type"], "sketch_line");
        if let Some(cli) = solidcraft() {
            let design = written(&model).0;
            let boxes = bounds(&cli, &design);
            assert_eq!(boxes.len(), 1);
            let expected = if axis > 0.0 { (0.0, 5.0) } else { (-5.0, 0.0) };
            assert!((boxes[0].0[0] - 5.0).abs() < 1e-6, "{boxes:?}");
            assert!((boxes[0].1[0] - 10.0).abs() < 1e-6, "{boxes:?}");
            assert!((boxes[0].0[1] - expected.0).abs() < 1e-6, "{boxes:?}");
            assert!((boxes[0].1[1] - expected.1).abs() < 1e-6, "{boxes:?}");
            let actual = volumes(&cli, &design);
            assert_eq!(actual.len(), 1);
            let expected = 62.5 * std::f64::consts::PI;
            assert!((actual[0] - expected).abs() / expected < 1e-3, "{actual:?}");
        }
    }
}

#[test]
fn one_sided_revolves_bound_circular_profiles() {
    for radial in [7.0, -7.0, 0.0] {
        let mut model = ring_history(json!({"kind": "one_sided",
            "termination": {"kind": "angle", "angle": std::f64::consts::FRAC_PI_2}}));
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["sketch_entities"] = json!([entity(
            1,
            json!({"kind": "circle",
            "center": {"u": radial, "v": -5.0}, "radius": 2.0})
        )]);
        wire["model"]["features"][1]["definition"]["construction"]["profile"] = whole_sketch();
        wire["model"]["sketches"][0]["profiles"] =
            json!([[{"entity": "f3d:model:sketch-entity#1"}]]);
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (design, report) = written(&model);
        if radial == 0.0 {
            assert!(history_line(&report).contains("sweep side is not proven"));
        } else {
            assert!(
                history_line(&report).starts_with("2 of 2 features rebuilt"),
                "{report:?}"
            );
            if let Some(cli) = solidcraft() {
                let boxes = bounds(&cli, &design);
                assert_eq!(boxes.len(), 1);
                let expected = if radial > 0.0 {
                    (0.0, 9.0)
                } else {
                    (-9.0, 0.0)
                };
                // SolidCraft measures a circular profile's extrema from its mesh.
                assert!((boxes[0].0[1] - expected.0).abs() < 5e-4, "{boxes:?}");
                assert!((boxes[0].1[1] - expected.1).abs() < 5e-4, "{boxes:?}");
                let actual = volumes(&cli, &design);
                assert_eq!(actual.len(), 1);
                let expected = 14.0 * std::f64::consts::PI.powi(2);
                assert!((actual[0] - expected).abs() / expected < 1e-3, "{actual:?}");
            }
        }
    }
}

#[test]
fn a_one_sided_revolve_with_profiles_on_both_sides_names_the_missing_proof() {
    let mut model = ring_history(json!({"kind": "one_sided",
        "termination": {"kind": "angle", "angle": std::f64::consts::FRAC_PI_2}}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    for entity in wire["model"]["sketch_entities"].as_array_mut().unwrap() {
        for end in ["start", "end"] {
            let point = &mut entity["geometry"][end];
            point["u"] = json!(point["u"].as_f64().unwrap() - 7.5);
        }
    }
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = written(&model);
    assert!(history_line(&report)
        .contains("selected profile curves must lie on one side of the oriented axis"));
}

#[test]
fn revolves_rebuild_their_volume_in_solidcraft() {
    let Some(cli) = solidcraft() else { return };
    let ring = std::f64::consts::PI * (10.0_f64.powi(2) - 5.0_f64.powi(2)) * 10.0;
    let close = |got: &[f64], want: f64| {
        assert_eq!(got.len(), 1, "{got:?}");
        assert!((got[0] - want).abs() / want < 1e-3, "{got:?} vs {want}");
    };
    let full = ring_history(json!({"kind": "one_sided",
        "termination": {"kind": "angle", "angle": std::f64::consts::TAU}}));
    close(&volumes(&cli, &written(&full).0), ring);
    let quarter = ring_history(json!({"kind": "symmetric",
        "termination": {"kind": "angle", "angle": std::f64::consts::FRAC_PI_2}}));
    close(&volumes(&cli, &written(&quarter).0), ring / 4.0);
}

#[test]
fn unequal_two_sided_full_turns_map_without_an_angular_start() {
    for (first, second) in [(90.0_f64, 270.0_f64), (270.0, 90.0)] {
        let model = ring_history(json!({"kind": "two_sided",
            "first": {"kind": "angle", "angle": first.to_radians()},
            "second": {"kind": "angle", "angle": second.to_radians()}}));
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
        let revolve = &document["features"][1];
        assert_eq!(revolve["angle"], "360 deg");
        assert_eq!(revolve["convention"], "sketch_normal");
        assert!(revolve.get("symmetric").is_none());
        assert!(revolve.get("angle2").is_none());
        if let Some(cli) = solidcraft() {
            let actual = volumes(&cli, &written(&model).0);
            assert_eq!(actual.len(), 1);
            let expected = 750.0 * std::f64::consts::PI;
            assert!((actual[0] - expected).abs() / expected < 1e-3, "{actual:?}");
        }
    }
}

#[test]
fn unequal_partial_and_overlapping_revolve_sides_are_refused() {
    for (first, second, reason) in [
        (30.0_f64, 60.0_f64, "which way each side turns"),
        (180.0, 270.0, "beyond a full turn"),
    ] {
        let model = ring_history(json!({"kind": "two_sided",
            "first": {"kind": "angle", "angle": first.to_radians()},
            "second": {"kind": "angle", "angle": second.to_radians()}}));
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn every_non_angle_revolve_end_has_a_specific_refusal() {
    let face = json!({"kind": "faces", "value": ["synthetic:cube:face#top"]});
    for (termination, reason) in [
        (json!({"kind": "unresolved"}), "termination is not resolved"),
        (json!({"kind": "through_all"}), "through_all revolve ends"),
        (json!({"kind": "through_next"}), "through_next revolve ends"),
        (json!({"kind": "to_first"}), "to_first revolve ends"),
        (json!({"kind": "to_last"}), "to_last revolve ends"),
        (json!({"kind": "to_face", "face": face}), "to-face revolves"),
        (
            json!({"kind": "to_face", "face": face, "offset": 1.0}),
            "to-face revolves with an offset",
        ),
        (
            json!({"kind": "to_vertex", "vertex": {"kind": "unresolved"}}),
            "to-vertex revolves",
        ),
        (
            json!({"kind": "offset_from_face", "face": face, "offset": 1.0}),
            "offset-from-face revolves",
        ),
        (
            json!({"kind": "to_shape", "target": face}),
            "to-shape revolves",
        ),
    ] {
        for extent in [
            json!({"kind": "one_sided", "termination": termination}),
            json!({"kind": "symmetric", "termination": termination}),
            json!({"kind": "two_sided", "first": termination,
                "second": {"kind": "angle", "angle": 1.0}}),
            json!({"kind": "two_sided", "first": {"kind": "angle", "angle": 1.0},
                "second": termination}),
        ] {
            let (_, report) = convert(
                &ring_history(extent),
                &SolidCraftWriter::new().verification(Verification::Off),
            );
            assert!(
                history_line(&report).contains(reason),
                "{}",
                history_line(&report)
            );
        }
    }
}

#[test]
fn revolve_axis_sources_use_the_resolved_line_placement() {
    for reference in [
        Value::Null,
        json!({"kind": "native", "value": "synthetic datum axis"}),
        json!({"kind": "sketch_curves", "value": {
            "sketch": "f3d:model:sketch#9", "curves": ["f3d:model:sketch-entity#1"]}}),
        json!({"kind": "edges", "value": ["synthetic:cube:edge#0"]}),
        json!({"kind": "curves", "value": ["synthetic:cube:curve#0"]}),
    ] {
        let model = changed_ring(|construction| {
            if !reference.is_null() {
                construction["axis"]["reference"] = reference;
            }
            construction["axis"]["origin"]["z"] = json!(3.0);
            construction["axis"]["direction"]["z"] = json!(-2.0);
        });
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
        assert_eq!(
            document["features"][1]["axis"],
            json!({
                "type": "line", "origin": [0.0, 0.0, 3.0], "dir": [0.0, 0.0, -1.0]
            })
        );
    }
}

#[test]
fn a_revolve_about_a_line_of_its_sketch_keeps_that_line() {
    // The rectangle's inner side, 5 mm out from Z, turns the ring into a cylinder.
    let model = changed_ring(|construction| {
        construction["axis"]["origin"]["x"] = json!(5.0);
        construction["axis"]["reference"] = json!({"kind": "sketch_curves", "value": {
            "sketch": SKETCH, "curves": ["f3d:model:sketch-entity#4"]}});
    });
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    let axis = &document["features"][1]["axis"];
    assert_eq!(axis["type"], "sketch_line");
    let curve = axis["curve"].as_str().unwrap();
    let sketch = &document["features"][0];
    assert!(
        sketch["sketch"]["curves"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == curve && c["type"] == "line"),
        "{sketch}"
    );
    if let Some(cli) = solidcraft() {
        let actual = volumes(&cli, &written(&model).0);
        assert_eq!(actual.len(), 1);
        let expected = 250.0 * std::f64::consts::PI;
        assert!((actual[0] - expected).abs() / expected < 1e-3, "{actual:?}");
    }
}

#[test]
fn surface_revolves_short_of_a_full_turn_map() {
    let quarter = json!({"kind": "symmetric",
        "termination": {"kind": "angle", "angle": std::f64::consts::FRAC_PI_2}});
    let model = changed_ring(|construction| {
        construction["solid"] = json!(false);
        construction["face_maker_class"] = json!("Part::FaceMakerBullseye");
        construction["extent"] = quarter;
    });
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    let revolve = &document["features"][1];
    assert_eq!(revolve["type"], "surface_revolve");
    assert_eq!(revolve["angle"], "90 deg");
    assert_eq!(revolve["symmetric"], true);
    assert!(revolve.get("operation").is_none());
    assert!(revolve.get("convention").is_none());
    assert!(revolve["profiles"].is_object());
    if let Some(cli) = solidcraft() {
        let boxes = bounds(&cli, &written(&model).0);
        assert_eq!(boxes.len(), 1, "{boxes:?}");
        let (min, max) = boxes[0];
        assert!(
            min[2].abs() < 1e-6 && (max[2] - 10.0).abs() < 1e-6,
            "{boxes:?}"
        );
        assert!((min[1] + max[1]).abs() < 1e-6 && max[1] > 0.0, "{boxes:?}");
    }

    let model = changed_ring(|construction| construction["solid"] = json!(false));
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains("full-turn surface revolves are not supported yet"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn revolve_construction_options_map_or_name_the_face_maker() {
    for maker in [
        "Part::FaceMakerSimple",
        "Part::FaceMakerCheese",
        "Part::FaceMakerExtrusion",
        "Part::FaceMakerUnified",
        "SyntheticFaceMaker",
    ] {
        let model = changed_ring(|construction| construction["face_maker_class"] = json!(maker));
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(&format!("revolves whose faces are built by {maker}")),
            "{}",
            history_line(&report)
        );
    }
    for change in [
        json!({"face_maker_class": "Part::FaceMakerBullseye", "solid": true}),
        json!({"fuse_order": "base_first", "allow_multi_profile_faces": false}),
        json!({"fuse_order": "feature_first", "allow_multi_profile_faces": true}),
    ] {
        let model = changed_ring(|construction| {
            for (key, value) in change.as_object().unwrap() {
                construction[key] = value.clone();
            }
        });
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).starts_with("2 of 2 features rebuilt"),
            "{change}: {}",
            history_line(&report)
        );
        assert_eq!(document["features"][1]["type"], "revolve");
    }
}

#[test]
fn unresolved_revolve_inputs_name_the_missing_operand() {
    for missing in ["profile", "axis", "extent"] {
        let model = changed_ring(|construction| {
            construction["state"] = json!("unresolved");
            construction["missing"] = json!(missing);
            construction.as_object_mut().unwrap().remove(missing);
        });
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(&format!("revolve {missing} is not resolved")),
            "{}",
            history_line(&report)
        );
    }
}

const HOLES: &str = "f3d:model:sketch#3";

/// The example cube's history, with a sketch on its top face holding a circle of radius 2 about
/// (5, 5) and a simple hole of `diameter` drilled down from it; `change` edits the hole.
fn holed_cube(diameter: f64, change: impl FnOnce(&mut Value)) -> Model {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    m["sketches"].as_array_mut().unwrap().push(json!({
        "id": HOLES,
        "name": "Holes",
        "placement": {"kind": "resolved", "frame": {
            "origin": {"x": 0.0, "y": 0.0, "z": 10.0},
            "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
            "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0},
        }},
    }));
    m["sketch_entities"].as_array_mut().unwrap().push(json!({
        "id": "f3d:model:sketch-entity#9",
        "sketch": HOLES,
        "construction": false,
        "geometry": {"kind": "circle", "center": {"u": 5.0, "v": 5.0}, "radius": 2.0},
    }));
    let mut hole = json!({
        "definition": "hole",
        "profile": {"kind": "sketch", "value": HOLES},
        "profile_filter": "circles",
        "direction": {"x": 0.0, "y": 0.0, "z": -1.0},
        "shape": {"construction": {"construction": "form", "kind": {"kind": "simple"}},
                  "diameter": diameter},
        "extent": {"kind": "through_all"},
    });
    change(&mut hole);
    let features = m["features"].as_array_mut().unwrap();
    features.push(json!({
        "id": "f3d:model:feature#3",
        "ordinal": 2,
        "name": "Hole sketch",
        "suppressed": false,
        "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": HOLES}},
    }));
    features.push(json!({
        "id": "f3d:model:feature#4",
        "ordinal": 3,
        "name": "Hole",
        "suppressed": false,
        "dependencies": ["f3d:model:feature#3", "f3d:model:feature#2"],
        "definition": hole,
    }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn a_simple_hole_at_a_sketch_circle_becomes_a_solidcraft_hole() {
    let (document, report) = convert(
        &holed_cube(4.0, |_| {}),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("4 of 4 features rebuilt"),
        "{}",
        history_line(&report)
    );
    let hole = &document["features"][3];
    assert_eq!(hole["type"], "hole");
    assert_eq!(hole["diameter"], "4 mm");
    assert_eq!(hole["direction"], json!([0.0, 0.0, -1.0]));
    assert_eq!(hole["position"], json!([5.0, 5.0, 10.0]));
    assert_eq!(hole["points"]["sketch"], document["features"][2]["id"]);
}

#[test]
fn counterbores_and_countersinks_keep_their_entry_dimensions() {
    for version in ["0.18.2", "0.25.1"] {
        for (form, expected) in [
            (
                json!({"kind": "counterbore", "diameter": 8.0, "depth": 2.0}),
                json!({"type": "counterbore", "cb_diameter": "8 mm", "cb_depth": "2 mm"}),
            ),
            (
                json!({"kind": "countersink", "diameter": 8.0,
                "angle": std::f64::consts::FRAC_PI_2}),
                json!({"type": "countersink", "cs_diameter": "8 mm", "cs_angle": "90"}),
            ),
            (
                json!({"kind": "chamfer", "diameter": 8.0,
                "angle": std::f64::consts::FRAC_PI_2}),
                json!({"type": "countersink", "cs_diameter": "8 mm", "cs_angle": "90"}),
            ),
        ] {
            for blind in [false, true] {
                let mut expected = expected.clone();
                if version
                    .parse::<dutchman_solidcraft::SolidCraftVersion>()
                    .unwrap()
                    >= dutchman_solidcraft::SolidCraftVersion::V0_22_1
                {
                    expected["tip_angle"] = json!("180");
                }
                let model = holed_cube(4.0, |hole| {
                    hole["shape"]["construction"]["kind"] = form.clone();
                    if blind {
                        hole["extent"] = json!({"kind": "blind", "length": 6.0});
                        hole["bottom"] = json!({"kind": "flat"});
                    }
                });
                let (document, report) = convert(
                    &model,
                    &SolidCraftWriter::new()
                        .verification(Verification::Off)
                        .target_version(Some(version.parse().unwrap())),
                );
                assert!(
                    history_line(&report).starts_with("4 of 4 features rebuilt"),
                    "{}",
                    history_line(&report)
                );
                let hole = &document["features"][3];
                assert_eq!(hole["hole"], expected);
                assert_eq!(hole["diameter"], "4 mm");
                assert_eq!(hole["position"], json!([5.0, 5.0, 10.0]));
                assert_eq!(hole["direction"], json!([0.0, 0.0, -1.0]));
                assert_eq!(hole["points"]["sketch"], document["features"][2]["id"]);
                if blind {
                    assert_eq!(hole["depth"], "6 mm");
                } else {
                    assert!(hole.get("depth").is_none());
                }
            }
        }
    }
}

#[test]
fn solidcraft_drills_counterbores_and_countersinks() {
    let Some(cli) = solidcraft() else { return };
    let bore_area = std::f64::consts::PI * 2.0_f64.powi(2);
    let entry_area = std::f64::consts::PI * 4.0_f64.powi(2);
    let angle = 82_f64.to_radians();
    let sink_depth = (4.0 - 2.0) / (angle / 2.0).tan();
    // The countersink is a conical frustum; subtract the bore already counted in it.
    let sink_extra = std::f64::consts::PI * sink_depth / 3.0
        * (4.0_f64.powi(2) + 4.0 * 2.0 + 2.0_f64.powi(2))
        - bore_area * sink_depth;
    for (form, extra) in [
        (
            json!({"kind": "counterbore", "diameter": 8.0, "depth": 2.0}),
            (entry_area - bore_area) * 2.0,
        ),
        (
            json!({"kind": "countersink", "diameter": 8.0, "angle": angle}),
            sink_extra,
        ),
        (
            json!({"kind": "chamfer", "diameter": 8.0, "angle": angle}),
            sink_extra,
        ),
    ] {
        for depth in [None, Some(6.0)] {
            let model = holed_cube(4.0, |hole| {
                hole["shape"]["construction"]["kind"] = form.clone();
                if let Some(depth) = depth {
                    hole["extent"] = json!({"kind": "blind", "length": depth});
                    hole["bottom"] = json!({"kind": "flat"});
                }
            });
            let got = volumes(&cli, &written(&model).0);
            let want = 1000.0 - bore_area * depth.unwrap_or(10.0) - extra;
            assert_eq!(got.len(), 1, "{got:?}");
            assert!(
                (got[0] - want).abs() / want < 1e-3,
                "{form}: {got:?} vs {want}"
            );
        }
    }
}

#[test]
fn a_plain_drilled_form_keeps_its_angle_and_shoulder_depth() {
    let model = holed_cube(4.0, |hole| {
        hole["shape"]["construction"]["kind"] = json!({"kind": "simple_drilled",
            "drill_point_angle": std::f64::consts::FRAC_PI_2});
        hole["extent"] = json!({"kind": "blind", "length": 6.0});
    });
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("4 of 4 features rebuilt"));
    let hole = &document["features"][3];
    assert_eq!(hole["hole"], json!({"type": "drilled", "tip_angle": "90"}));
    assert_eq!(hole["diameter"], "4 mm");
    assert_eq!(hole["depth"], "6 mm");
    assert_eq!(hole["position"], json!([5.0, 5.0, 10.0]));
    assert_eq!(hole["direction"], json!([0.0, 0.0, -1.0]));
    assert_eq!(hole["points"]["sketch"], document["features"][2]["id"]);
}

#[test]
fn entry_recesses_write_flat_tips_only_for_new_targets() {
    for version in ["0.18.2", "0.22.0", "0.22.1", "0.25.1"] {
        for form in [
            json!({"kind": "counterbore", "diameter": 8.0, "depth": 2.0}),
            json!({"kind": "countersink", "diameter": 8.0, "angle": std::f64::consts::FRAC_PI_2}),
            json!({"kind": "chamfer", "diameter": 8.0, "angle": std::f64::consts::FRAC_PI_2}),
        ] {
            let model = holed_cube(4.0, |hole| {
                hole["shape"]["construction"]["kind"] = form;
                hole["extent"] = json!({"kind": "blind", "length": 6.0});
                hole["bottom"] = json!({"kind": "flat"});
            });
            let (document, _) = convert(
                &model,
                &SolidCraftWriter::new()
                    .verification(Verification::Off)
                    .target_version(Some(version.parse().unwrap())),
            );
            let tip = &document["features"][3]["hole"]["tip_angle"];
            if version
                .parse::<dutchman_solidcraft::SolidCraftVersion>()
                .unwrap()
                >= dutchman_solidcraft::SolidCraftVersion::V0_22_1
            {
                assert_eq!(tip, "180");
            } else {
                assert!(tip.is_null());
            }
        }
    }
}

#[test]
fn solidcraft_entry_recess_tip_angles_follow_the_target_release() {
    let Some(cli) = solidcraft() else { return };
    let modern = dutchman_solidcraft::SolidCraftVersion::detected(&Verification::With(cli.clone()))
        >= dutchman_solidcraft::SolidCraftVersion::V0_22_1;
    let dir = tempfile::tempdir().unwrap();
    let bore = std::f64::consts::PI * 4.0;
    // Older releases ignore the angle; 0.22.1 and newer cut a cone unless the tip is flat.
    for (kind, extra) in [
        ("counterbore", std::f64::consts::PI * 12.0 * 2.0),
        (
            "countersink",
            std::f64::consts::PI * 2.0 / 3.0 * 28.0 - bore * 2.0,
        ),
    ] {
        let mut previous = None;
        for tip in [None, Some("90"), Some("118"), Some("180")] {
            let mut params = json!({"position": [5, 5, 10], "direction": [0, 0, -1],
                "diameter": "4 mm", "depth": "6 mm", "type": kind,
                "cb_diameter": "8 mm", "cb_depth": "2 mm",
                "cs_diameter": "8 mm", "cs_angle": "90"});
            if let Some(tip) = tip {
                params["tip_angle"] = json!(tip);
            }
            let script = json!([
                {"command": "solid.box", "params": {"length": "10 mm", "width": "10 mm", "height": "10 mm"}},
                {"command": "solid.hole", "params": params},
            ]);
            let input = dir.path().join("hole.json");
            std::fs::write(&input, script.to_string()).unwrap();
            let measured = blend_measurement(&cli, &input, &dir.path().join("eval.json"));
            let volume = measured["volume_mm3"].as_f64().unwrap();
            let flat = 1000.0 - bore * 6.0 - extra;
            let point = if modern {
                tip.map_or(0.0, |angle| {
                    std::f64::consts::PI * 8.0
                        / (3.0 * (angle.parse::<f64>().unwrap().to_radians() / 2.0).tan())
                })
            } else {
                0.0
            };
            let expected = flat - point;
            // Tessellated volumes have a small error, well below either missing cone.
            assert!(
                (volume - expected).abs() < 0.05,
                "{kind} {tip:?}: {volume} vs {expected}"
            );
            if !modern || tip == Some("180") {
                if let Some(previous) = previous {
                    assert_eq!(volume, previous, "{kind} {tip:?}");
                }
            }
            if tip.is_none() {
                previous = Some(volume);
            }
        }
    }
}

#[test]
fn solidcraft_drills_the_holes() {
    let Some(cli) = solidcraft() else { return };
    let close = |got: &[f64], want: f64| {
        assert_eq!(got.len(), 1, "{got:?}");
        assert!((got[0] - want).abs() / want < 1e-3, "{got:?} vs {want}");
    };
    let bore = std::f64::consts::PI * 2.0_f64.powi(2);
    // Through all.
    close(
        &volumes(&cli, &written(&holed_cube(4.0, |_| {})).0),
        1000.0 - bore * 10.0,
    );
    // The same 118° point, with depth measured either to its shoulder or its tip.
    let angle = 118_f64.to_radians();
    let point = 2.0 / (angle / 2.0).tan();
    for to_tip in [false, true] {
        let blind = holed_cube(4.0, |hole| {
            hole["extent"] = json!({"kind": "blind", "length": 6.0});
            hole["bottom"] =
                json!({"kind": "angled", "included_angle": angle, "depth_to_tip": to_tip});
        });
        let shoulder = if to_tip { 6.0 - point } else { 6.0 };
        close(
            &volumes(&cli, &written(&blind).0),
            1000.0 - bore * shoulder - bore * point / 3.0,
        );
    }
}

/// A spatial placement takes its position from the feature, without a sketch.
fn directed_hole(hole: &mut Value) {
    hole.as_object_mut().unwrap().remove("profile");
    hole.as_object_mut().unwrap().remove("profile_filter");
    hole["placements"] = json!([{"kind": "directed",
        "position": {"x": 5.0, "y": 5.0, "z": 10.0},
        "direction": {"x": 0.0, "y": 0.0, "z": -2.0}}]);
}

fn axis_hole(hole: &mut Value, sign: f64) {
    directed_hole(hole);
    hole["face"] = json!({"kind": "faces", "value": ["synthetic:cube:face#top"]});
    hole["placements"] = json!([{"kind": "axis",
        "origin": {"x": 5.0, "y": 5.0, "z": 6.0},
        "axis": {"x": 0.0, "y": 0.0, "z": sign}}]);
}

fn to_face_hole(hole: &mut Value) {
    hole["extent"] = json!({"kind": "to_face", "face": {
        "kind": "resolved", "value": {"faces": ["synthetic:cube:face#bottom"], "native": "bottom"}}});
    hole["bottom"] = json!({"kind": "flat"});
}

#[test]
fn spatial_holes_keep_their_entry_position_and_inward_direction() {
    for model in [
        holed_cube(4.0, directed_hole),
        holed_cube(4.0, |hole| axis_hole(hole, 1.0)),
        holed_cube(4.0, |hole| axis_hole(hole, -1.0)),
        holed_cube(4.0, |hole| {
            directed_hole(hole);
            hole["face"] = json!({"kind": "faces", "value": ["synthetic:cube:face#top"]});
        }),
    ] {
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).starts_with("4 of 4 features rebuilt"),
            "{}",
            history_line(&report)
        );
        let hole = &document["features"][3];
        assert_eq!(hole["position"], json!([5.0, 5.0, 10.0]));
        let direction: Vec<f64> = hole["direction"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert_eq!(direction, vec![0.0, 0.0, -1.0]);
        assert!(hole.get("points").is_none());
    }
}

#[test]
fn a_hole_to_an_earlier_cap_keeps_a_point_on_that_face() {
    let (document, report) = convert(
        &holed_cube(4.0, to_face_hole),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("4 of 4 features rebuilt"),
        "{}",
        history_line(&report)
    );
    let hole = &document["features"][3];
    let to = hole["to"].as_array().unwrap();
    assert_eq!(to[2], 0.0);
    assert!(to[0].as_f64().unwrap() > 0.0 && to[0].as_f64().unwrap() < 10.0);
    assert!(to[1].as_f64().unwrap() > 0.0 && to[1].as_f64().unwrap() < 10.0);
    assert!(hole.get("depth").is_none());
}

#[test]
fn a_hole_can_select_sketch_points_or_circle_centres() {
    for point in [false, true] {
        let mut model = holed_cube(4.0, |hole| {
            hole["profile"] = json!({"kind": "sketch_entities", "value": {
                "sketch": HOLES, "entities": ["f3d:model:sketch-entity#9"]}});
            hole["profile_filter"] = json!(if point { "points" } else { "circles" });
        });
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let entities = wire["model"]["sketch_entities"].as_array_mut().unwrap();
        if point {
            entities.last_mut().unwrap()["geometry"] =
                json!({"kind": "point", "position": {"u": 5.0, "v": 5.0}});
        }
        // An unselected entity of the same family must not create a second hole.
        entities.push(json!({
            "id": "f3d:model:sketch-entity#10", "sketch": HOLES, "construction": false,
            "geometry": if point {
                json!({"kind": "point", "position": {"u": 3.0, "v": 3.0}})
            } else {
                json!({"kind": "circle", "center": {"u": 3.0, "v": 3.0}, "radius": 1.0})
            },
        }));
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).starts_with("4 of 4 features rebuilt"),
            "{}",
            history_line(&report)
        );
        let hole = &document["features"][3];
        assert_eq!(hole["position"], json!([5.0, 5.0, 10.0]));
        assert_eq!(hole["points"]["ids"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn a_drilled_form_keeps_its_included_angle_and_shoulder_depth() {
    let (document, report) = convert(
        &holed_cube(4.0, |hole| {
            hole["shape"]["construction"]["kind"] = json!({"kind": "simple_drilled",
                "drill_point_angle": std::f64::consts::FRAC_PI_2});
            hole["extent"] = json!({"kind": "blind", "length": 6.0});
        }),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("4 of 4 features rebuilt"),
        "{}",
        history_line(&report)
    );
    assert_eq!(
        document["features"][3]["hole"],
        json!({"type": "drilled", "tip_angle": "90"})
    );
    assert_eq!(document["features"][3]["depth"], "6 mm");
}

fn cosmetic_hole_thread() -> Value {
    json!({"kind": "threaded", "standard": "ISO", "designation": "M6",
        "modeled": false, "cosmetic": true, "hand": "right", "depth": {"kind": "hole_depth"}})
}

#[test]
fn cosmetic_hole_threads_keep_the_designation_and_resolved_bore() {
    for blind in [false, true] {
        let model = holed_cube(4.0, |hole| {
            hole["shape"]["construction"]["specification"] = cosmetic_hole_thread();
            if blind {
                hole["extent"] = json!({"kind": "blind", "length": 6.0});
            }
        });
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).starts_with("4 of 4 features rebuilt"),
            "{}",
            history_line(&report)
        );
        let hole = &document["features"][3];
        assert_eq!(hole["thread"], "M6x1");
        assert_eq!(hole["diameter"], "4 mm");
        assert_eq!(hole["hole"], json!({"type": "simple"}));
        assert_eq!(hole["points"]["sketch"], document["features"][2]["id"]);
        if blind {
            assert_eq!(hole["depth"], "6 mm");
        } else {
            assert!(hole.get("depth").is_none());
        }
    }
}

#[test]
fn solidcraft_keeps_the_bore_of_a_cosmetic_tapped_hole() {
    let Some(cli) = solidcraft() else { return };
    for depth in [None, Some(6.0)] {
        let model = holed_cube(4.0, |hole| {
            hole["shape"]["construction"]["specification"] = cosmetic_hole_thread();
            if let Some(depth) = depth {
                hole["extent"] = json!({"kind": "blind", "length": depth});
            }
        });
        let got = volumes(&cli, &written(&model).0);
        let want = 1000.0 - std::f64::consts::PI * 4.0 * depth.unwrap_or(10.0);
        assert_eq!(got.len(), 1, "{got:?}");
        assert!((got[0] - want).abs() / want < 1e-3, "{got:?} vs {want}");
    }
}

#[test]
fn hole_thread_details_solidcraft_lacks_are_reported_not_blocking() {
    let thread_note = |change: Value| {
        let model = holed_cube(4.0, |hole| {
            let mut specification = cosmetic_hole_thread();
            specification
                .as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            specification
                .as_object_mut()
                .unwrap()
                .retain(|_, v| !v.is_null());
            hole["shape"]["construction"]["specification"] = specification;
        });
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).starts_with("4 of 4 features rebuilt"),
            "{change}: {}",
            history_line(&report)
        );
        let note = report
            .losses()
            .iter()
            .find(|loss| loss.detail.contains("thread"))
            .map(|loss| loss.detail.clone())
            .unwrap_or_default();
        (document["features"][3]["thread"].clone(), note)
    };
    let (label, note) = thread_note(json!({"hand": "left", "class": "6H",
        "depth": {"kind": "blind", "depth": 3.0}}));
    assert_eq!(label, "M6x1");
    assert_eq!(
        note,
        "its thread is kept as a cosmetic M6x1 label over the whole hole, without its \
         left-hand direction, its 6H tolerance class, and its 3 mm thread length"
    );
    let (label, note) = thread_note(json!({"designation": "M5", "pitch": 0.5}));
    assert_eq!(label, "M5x0.5");
    assert!(note.is_empty(), "{note}");
    let (label, _) = thread_note(json!({"designation": "M6 x 0.75-6H"}));
    assert_eq!(label, "M6x0.75");
    for (change, why) in [
        (
            json!({"designation": "1/4-20 UNC"}),
            "ISO metric threads only",
        ),
        (json!({"designation": "M12"}), "does not fit its 4 mm bore"),
        (json!({"designation": null}), "its thread size is not given"),
    ] {
        let (label, note) = thread_note(change);
        assert!(label.is_null(), "{label}");
        assert!(
            note.contains(why) && note.contains("the bore is kept"),
            "{note}"
        );
    }
    let model = holed_cube(4.0, |hole| {
        let mut specification = cosmetic_hole_thread();
        specification["modeled"] = json!(true);
        hole["shape"]["construction"]["specification"] = specification;
    });
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).contains("a hole with a modeled thread"));
}

#[test]
fn unsupported_hole_forms_have_specific_reasons() {
    for (form, reason) in [
        (
            json!({"kind": "unresolved", "form": "chamfer"}),
            "its hole entry form or dimensions are not resolved",
        ),
        (
            json!({"kind": "partial_counterbore", "dimension": {"kind": "diameter", "diameter": 8.0}}),
            "its counterbore diameter or depth is not resolved",
        ),
        (
            json!({"kind": "partial_countersink", "dimension": {"kind": "angle", "angle": 1.5}}),
            "its countersink diameter or angle is not resolved",
        ),
        (
            json!({"kind": "counterbore_drilled", "diameter": 8.0, "depth": 2.0, "drill_point_angle": 1.5}),
            "SolidCraft holes take a drill point only on a plain drilled hole",
        ),
        (
            json!({"kind": "counterdrill", "diameter": 8.0, "entry_diameter": 9.0, "depth": 2.0, "angle": 1.5}),
            "a counterdrill with a conical entry and cylindrical recess is not mapped yet",
        ),
    ] {
        let model = holed_cube(4.0, |hole| {
            hole["shape"]["construction"]["kind"] = form.clone()
        });
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
    for kind in ["counterbore", "countersink", "chamfer"] {
        for blind in [false, true] {
            let model = holed_cube(4.0, |hole| {
                hole["shape"]["construction"]["kind"] = if kind == "counterbore" {
                    json!({"kind": kind, "diameter": 8.0, "depth": 2.0})
                } else {
                    json!({"kind": kind, "diameter": 8.0, "angle": 1.5})
                };
                if blind {
                    hole["extent"] = json!({"kind": "blind", "length": 6.0});
                }
                hole["bottom"] =
                    json!({"kind": "angled", "included_angle": 1.5, "depth_to_tip": false});
            });
            // SolidCraft before 0.22.1 drills entry-recess holes flat: refused.
            let (_, report) = convert(
                &model,
                &SolidCraftWriter::new()
                    .verification(Verification::Off)
                    .target_version(Some("0.22.0".parse().unwrap())),
            );
            assert!(history_line(&report)
                .contains("SolidCraft holes take a drill point only on a plain drilled hole"));
            // SolidCraft 0.22.1 and later keep the entry form and its drill point. Name the
            // version: the default target follows the installed SolidCraft, which can be older.
            let (_, report) = convert(
                &model,
                &SolidCraftWriter::new()
                    .verification(Verification::Off)
                    .target_version(Some("0.22.1".parse().unwrap())),
            );
            assert!(
                !history_line(&report).contains("drill point"),
                "{}",
                history_line(&report)
            );
        }
    }
}

#[test]
fn unsupported_hole_ends_have_specific_reasons() {
    for (extent, reason) in [
        (
            json!({"kind": "unresolved"}),
            "its hole termination is not resolved",
        ),
        (
            json!({"kind": "through_next"}),
            "a hole through the next material region",
        ),
        (
            json!({"kind": "to_first"}),
            "a hole ending at the first encountered face",
        ),
        (
            json!({"kind": "to_last"}),
            "a hole ending at the last encountered face",
        ),
        (
            json!({"kind": "to_vertex", "vertex": {"kind": "unresolved"}}),
            "a hole ending at a source vertex",
        ),
        (
            json!({"kind": "offset_from_face", "offset": 1.0,
            "face": {"kind": "faces", "value": ["synthetic:cube:face#bottom"]}}),
            "a hole that ends offset from a face",
        ),
        (
            json!({"kind": "to_shape", "target": {"kind": "native", "value": "body"}}),
            "a hole ending at a selected shape",
        ),
    ] {
        let model = holed_cube(4.0, |hole| hole["extent"] = extent.clone());
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn native_threads_and_exit_treatments_are_refused() {
    for (change, reason) in [
        (
            json!({"shape": {"diameter": 4.0, "construction": {"construction": "native_thread",
                "major_diameter": 6.0, "thread_depth": 6.0, "pitch": 1.0,
                "drill_point_angle": 1.5}}}),
            "a hole with native thread geometry is not mapped yet",
        ),
        (
            json!({"shape": {"diameter": 4.0, "construction": {"construction": "form",
                "kind": {"kind": "simple"}}, "exit_kind": {"kind": "chamfer", "diameter": 6.0,
                "angle": 1.5}}}),
            "a hole with an exit treatment is not mapped yet",
        ),
    ] {
        let model = holed_cube(4.0, |hole| {
            hole.as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
        });
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn holes_refuse_unproven_faces_and_unsupported_geometry() {
    for (change, reason) in [
        (
            json!({"taper_angle": 0.1}),
            "a tapered hole is not mapped yet",
        ),
        (
            json!({"shape": {"diameter": 4.0, "construction": {"construction": "form",
            "kind": {"kind": "simple"}, "specification": {"kind": "threaded", "standard": "ISO",
                "modeled": true, "cosmetic": false, "hand": "right", "depth": {"kind": "hole_depth"}}}}}),
            "a hole with a modeled thread is not mapped yet",
        ),
        (
            json!({"placements": [{"kind": "axis", "origin": {"x": 5.0, "y": 5.0, "z": 6.0},
            "axis": {"x": 0.0, "y": 0.0, "z": 1.0}}]}),
            "its face is not resolved",
        ),
        (
            json!({"extent": {"kind": "to_face", "face": {"kind": "native", "value": "face"}}}),
            "its face has no resolved geometry",
        ),
        (
            json!({"extent": {"kind": "to_face", "offset": 1.0,
            "face": {"kind": "faces", "value": ["synthetic:cube:face#bottom"]}}}),
            "a hole that ends offset from a face is not mapped yet",
        ),
    ] {
        let model = holed_cube(4.0, |hole| {
            hole.as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
        });
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
    // The saved top is at 10 mm, while the only earlier solid stops at 8 mm.
    // Neither an entry nor an end can use it merely because it is in the final B-rep.
    for entry in [true, false] {
        let mut model = holed_cube(4.0, |hole| {
            if entry {
                axis_hole(hole, 1.0);
            } else {
                directed_hole(hole);
                hole["placements"][0]["position"]["z"] = json!(0.0);
                hole["placements"][0]["direction"]["z"] = json!(1.0);
                hole["extent"] = json!({"kind": "to_face", "face": {
                    "kind": "faces", "value": ["synthetic:cube:face#top"]}});
            }
        });
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["features"][1]["definition"]["extent"]["side"]["termination"]["length"] =
            json!(8.0);
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains("its face is not proven to exist before the hole"),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn solidcraft_drills_spatial_and_to_face_holes_and_drilled_forms() {
    let Some(cli) = solidcraft() else { return };
    let bore = std::f64::consts::PI * 4.0;
    for (model, removed) in [
        (holed_cube(4.0, directed_hole), bore * 10.0),
        (holed_cube(4.0, |hole| axis_hole(hole, 1.0)), bore * 10.0),
        (holed_cube(4.0, to_face_hole), bore * 10.0),
        (
            holed_cube(4.0, |hole| {
                hole["profile"] = json!({"kind": "sketch_entities", "value": {
                "sketch": HOLES, "entities": ["f3d:model:sketch-entity#9"]}});
            }),
            bore * 10.0,
        ),
        (
            holed_cube(4.0, |hole| {
                hole["shape"]["construction"]["kind"] = json!({"kind": "simple_drilled",
                "drill_point_angle": std::f64::consts::FRAC_PI_2});
                hole["extent"] = json!({"kind": "blind", "length": 6.0});
            }),
            bore * (6.0 + 2.0 / 3.0),
        ),
    ] {
        let got = volumes(&cli, &written(&model).0);
        let want = 1000.0 - removed;
        assert_eq!(got.len(), 1, "{got:?}");
        assert!((got[0] - want).abs() / want < 1e-3, "{got:?} vs {want}");
    }
}

/// The example cube's geometry with a right-triangle history: a fixed 40 mm horizontal base,
/// a vertical side, and the hypotenuse at `theta` from the base (shown as 36.87°), extruded by
/// 10 mm.
fn triangle_history() -> Model {
    let mut model = cube_with_history(10.0, whole_sketch(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    m["sketches"][0]["profiles"] = json!([]);
    m["sketch_entities"] = json!([
        line(1, [0.0, 0.0], [40.0, 0.0]),
        line(2, [0.0, 0.0], [40.0, 30.0]),
        line(3, [40.0, 0.0], [40.0, 30.0]),
    ]);
    let constraint = |key: u32, definition: Value| {
        json!({"id": format!("f3d:model:sketch-constraint#{key}"), "sketch": SKETCH,
               "definition": definition})
    };
    let start = json!({"kind": "start", "entity": "f3d:model:sketch-entity#1"});
    m["sketch_constraints"] = json!([
        constraint(
            1,
            json!({"kind": "coincident_loci",
            "loci": [start, {"kind": "start", "entity": "f3d:model:sketch-entity#2"}]})
        ),
        constraint(
            2,
            json!({"kind": "horizontal", "entity": "f3d:model:sketch-entity#1"})
        ),
        constraint(
            3,
            json!({"kind": "vertical", "entity": "f3d:model:sketch-entity#3"})
        ),
        constraint(
            4,
            json!({"kind": "distance", "entities": ["f3d:model:sketch-entity#1"],
                              "parameter": "f3d:model:parameter#2"})
        ),
        constraint(
            5,
            json!({"kind": "angle", "first": "f3d:model:sketch-entity#1",
                              "second": "f3d:model:sketch-entity#2",
                              "parameter": "f3d:model:parameter#3"})
        ),
    ]);
    m["parameters"][1]["expression"] = json!("40 mm");
    m["parameters"][1]["value"] = json!({"kind": "length", "value": 40.0});
    let theta = 30.0_f64.atan2(40.0);
    m["parameters"].as_array_mut().unwrap().push(json!({
        "id": "f3d:model:parameter#3",
        "owner": "f3d:model:feature#1",
        "ordinal": 2,
        "name": "theta",
        "expression": format!("{} deg", theta.to_degrees()),
        "value": {"kind": "angle", "value": theta},
    }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn a_converted_angle_drives_the_geometry_in_solidcraft() {
    let Some(cli) = solidcraft() else { return };
    let (bytes, report) = written(&triangle_history());
    assert!(
        !report
            .losses()
            .iter()
            .any(|loss| loss.detail.contains("constraints not carried")),
        "{:?}",
        report.losses()
    );
    let close = |got: Vec<f64>, want: f64| {
        assert_eq!(got.len(), 1, "{got:?}");
        assert!((got[0] - want).abs() < 1e-6, "{got:?} vs {want}");
    };
    close(volumes(&cli, &bytes), 6000.0);
    close(
        volumes(&cli, &with_param(&bytes, "theta", "45 deg")),
        8000.0,
    );
}

/// A square crossed by both diagonals, whose source loop retraces those diagonals,
/// beside a second square which the extrude deliberately leaves out.
fn crossed_profile_history(region: bool) -> Model {
    let profile = if region {
        json!({"kind": "sketch_regions", "value": {"sketch": SKETCH,
            "regions": [{"region": "loops", "outer": 0}]}})
    } else {
        first_profile()
    };
    let mut model = cube_with_history(10.0, profile, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    let member = |key: u32, reversed: bool| json!({"entity": format!("f3d:model:sketch-entity#{key}"), "reversed": reversed});
    m["sketches"][0]["profiles"] = json!([
        [
            member(1, false),
            member(5, false),
            member(5, true),
            member(2, false),
            member(3, false),
            member(4, false),
            member(6, false),
            member(6, true)
        ],
        [
            member(7, false),
            member(8, false),
            member(9, false),
            member(10, false)
        ],
    ]);
    m["sketch_entities"].as_array_mut().unwrap().extend([
        line(5, [10.0, 0.0], [0.0, 10.0]),
        line(6, [0.0, 0.0], [10.0, 10.0]),
        line(7, [20.0, 0.0], [30.0, 0.0]),
        line(8, [30.0, 0.0], [30.0, 10.0]),
        line(9, [30.0, 10.0], [20.0, 10.0]),
        line(10, [20.0, 10.0], [20.0, 0.0]),
    ]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn a_selected_crossed_loop_maps_to_its_atomic_regions() {
    for region in [false, true] {
        let model = crossed_profile_history(region);
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
        let selection = &document["features"][1]["profiles"];
        assert_eq!(selection["type"], "curves");
        let mut loops: Vec<Vec<String>> =
            serde_json::from_value(selection["loops"].clone()).unwrap();
        for ring in &mut loops {
            ring.sort();
        }
        loops.sort();
        assert_eq!(
            loops,
            vec![
                vec!["l1", "l5", "l6"],
                vec!["l2", "l5", "l6"],
                vec!["l3", "l5", "l6"],
                vec!["l4", "l5", "l6"],
            ]
        );
        // The source curves and their ids survive: constraints and later references
        // still address the same geometry. SolidCraft splits their pieces on replay.
        let curves = document["features"][0]["sketch"]["curves"]
            .as_array()
            .unwrap();
        assert_eq!(curves.len(), 10);
        // The separate square (curves 7 to 10) bounds nothing the extrude takes, whether it
        // selects a region or a list of loops, so it is construction; the crossed loop's
        // curves stay solid.
        for curve in curves {
            let id = curve["id"].as_str().unwrap();
            let unused = ["l7", "l8", "l9", "l10"].contains(&id);
            assert_eq!(curve["construction"] == true, unused, "{region} {id}");
        }
        assert!(!document["features"][0]["sketch"]["constraints"]
            .as_array()
            .unwrap()
            .is_empty());
    }
}

#[test]
fn solidcraft_replays_only_the_selected_crossed_region() {
    let Some(cli) = solidcraft() else { return };
    for region in [false, true] {
        let volume = volumes(&cli, &written(&crossed_profile_history(region)).0);
        assert_eq!(volume.len(), 1);
        assert!((volume[0] - 1000.0).abs() < 1e-6, "{volume:?}");
    }
}

/// The 10 mm square loop (curves 1 to 4) extruded as region `{outer: 0}`, with curves no loop
/// uses: a line through the square past both sides, and an open segment inside it.
fn stray_curve_history(change: impl FnOnce(&mut Value)) -> Model {
    let profile = json!({"kind": "sketch_regions", "value": {"sketch": SKETCH,
        "regions": [{"region": "loops", "outer": 0}]}});
    let mut model = cube_with_history(10.0, profile, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketch_entities"]
        .as_array_mut()
        .unwrap()
        .extend([
            line(5, [-5.0, 5.0], [15.0, 5.0]),
            line(6, [2.0, 2.0], [4.0, 3.0]),
        ]);
    change(&mut wire["model"]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

fn construction_flags(document: &Value) -> Vec<(String, bool)> {
    document["features"][0]["sketch"]["curves"]
        .as_array()
        .unwrap()
        .iter()
        .map(|curve| {
            (
                curve["id"].as_str().unwrap().to_owned(),
                curve["construction"] == true,
            )
        })
        .collect()
}

#[test]
fn curves_bounding_no_profile_a_feature_uses_are_written_as_construction() {
    let (document, report) = convert(
        &stray_curve_history(|_| {}),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    let flags = construction_flags(&document);
    assert_eq!(flags.len(), 6);
    assert_eq!(
        flags
            .iter()
            .filter(|(_, construction)| *construction)
            .count(),
        2
    );
    for (id, construction) in &flags {
        assert_eq!(*construction, id == "l5" || id == "l6", "{id}");
    }
    assert!(
        report.losses().iter().any(|loss| loss.detail
            == "2 sketch curves bounding no profile a feature uses written as construction"),
        "{report:?}"
    );
    // With the strays construction, the square's four curves are all the sketch has.
    assert_eq!(document["features"][1]["profiles"]["type"], "all");
}

#[test]
fn solidcraft_replays_a_region_beside_curves_no_loop_uses() {
    let Some(cli) = solidcraft() else { return };
    let volume = volumes(&cli, &written(&stray_curve_history(|_| {})).0);
    assert_eq!(volume.len(), 1);
    assert!((volume[0] - 1000.0).abs() < 1e-6, "{volume:?}");
    // An open segment alone, inside the region.
    let only_open = stray_curve_history(|model| {
        model["sketch_entities"].as_array_mut().unwrap().remove(4);
    });
    let volume = volumes(&cli, &written(&only_open).0);
    assert_eq!(volume.len(), 1);
    assert!((volume[0] - 1000.0).abs() < 1e-6, "{volume:?}");
}

fn circle(key: u32, center: [f64; 2], radius: f64) -> Value {
    entity(
        key,
        json!({"kind": "circle", "center": {"u": center[0], "v": center[1]}, "radius": radius}),
    )
}

/// The 10 mm square loop taken by its extrude, beside loops no feature takes: with `tab`, a
/// 4 by 4 mm tab whose inner edge lies along part of the square's right side; else a 1 mm
/// hole the region keeps (loop 1) inside a 2 mm ring (loop 2).
fn unused_loop_history(tab: bool) -> Model {
    let region = if tab {
        json!({"region": "loops", "outer": 0})
    } else {
        json!({"region": "loops", "outer": 0, "holes": [1]})
    };
    let profile =
        json!({"kind": "sketch_regions", "value": {"sketch": SKETCH, "regions": [region]}});
    let mut model = cube_with_history(10.0, profile, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let id = |key: u32| json!({"entity": format!("f3d:model:sketch-entity#{key}")});
    let (entities, loops) = if tab {
        (
            vec![
                line(7, [10.0, 3.0], [14.0, 3.0]),
                line(8, [14.0, 3.0], [14.0, 7.0]),
                line(9, [14.0, 7.0], [10.0, 7.0]),
                line(10, [10.0, 7.0], [10.0, 3.0]),
            ],
            vec![json!([id(7), id(8), id(9), id(10)])],
        )
    } else {
        (
            vec![circle(11, [5.0, 5.0], 1.0), circle(12, [5.0, 5.0], 2.0)],
            vec![json!([id(11)]), json!([id(12)])],
        )
    };
    let source = &mut wire["model"];
    source["sketch_entities"]
        .as_array_mut()
        .unwrap()
        .extend(entities);
    source["sketches"][0]["profiles"]
        .as_array_mut()
        .unwrap()
        .extend(loops);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn a_loop_no_feature_takes_is_written_as_construction() {
    for (tab, unused) in [(true, 4), (false, 1)] {
        let (document, report) = convert(
            &unused_loop_history(tab),
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).starts_with("2 of 2 features rebuilt"),
            "{tab}"
        );
        let flags = construction_flags(&document);
        let construction = flags
            .iter()
            .filter(|(_, construction)| *construction)
            .count();
        assert_eq!(construction, unused, "{tab}: {flags:?}");
        let noun = if unused == 1 { "curve" } else { "curves" };
        let detail = format!(
            "{unused} sketch {noun} bounding no profile a feature uses written as construction"
        );
        assert!(
            report.losses().iter().any(|loss| loss.detail == detail),
            "{tab}: {report:?}"
        );
    }
}

#[test]
fn solidcraft_replays_a_region_beside_a_loop_no_feature_takes() {
    let Some(cli) = solidcraft() else { return };
    // The tab shares part of the square's side: solid, SolidCraft splits that side and finds
    // no profile bounded by the square's four curves.
    let volume = volumes(&cli, &written(&unused_loop_history(true)).0);
    assert_eq!(volume.len(), 1);
    assert!((volume[0] - 1000.0).abs() < 1e-6, "{volume:?}");
    // The ring around the hole: solid, the square's face ends at the ring, not at the hole.
    let volume = volumes(&cli, &written(&unused_loop_history(false)).0);
    assert_eq!(volume.len(), 1);
    // The hole's 1 mm circle, measured on SolidCraft's mesh; the ring's 2 mm would leave 874.
    let expected = 10.0 * (100.0 - std::f64::consts::PI);
    assert!((volume[0] - expected).abs() < 0.05, "{volume:?}");
}

/// The square and the tab beside it, both taken by the extrude.
fn tab_taken_history() -> Model {
    let mut model = unused_loop_history(true);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][1]["definition"]["profile"]["value"]["regions"] =
        json!([{"region": "loops", "outer": 0}, {"region": "loops", "outer": 1}]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn loops_that_share_part_of_an_edge_are_named_by_points() {
    let (document, report) = convert(
        &tab_taken_history(),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    // Both loops are taken, so their curves stay solid.
    assert!(construction_flags(&document)
        .iter()
        .all(|(_, construction)| !construction));
    let profiles = &document["features"][1]["profiles"];
    assert_eq!(profiles["type"], "points", "{profiles}");
    assert_eq!(
        profiles["points"].as_array().unwrap().len(),
        2,
        "{profiles}"
    );
}

#[test]
fn solidcraft_replays_loops_that_share_part_of_an_edge() {
    let Some(cli) = solidcraft() else { return };
    // The square's 1000 mm³ and the tab's 160, as one body.
    let volume = volumes(&cli, &written(&tab_taken_history()).0);
    assert_eq!(volume.len(), 1, "{volume:?}");
    assert!((volume[0] - 1160.0).abs() < 1e-6, "{volume:?}");
}

/// The 10 mm square taken with two holes that cross: a 2 mm circle and a 3.4 mm square, both
/// centred in it, so the square's corners stick out of the circle (a nut pocket over a bolt hole).
fn crossing_holes_history() -> Model {
    let profile = json!({"kind": "sketch_regions", "value": {"sketch": SKETCH,
        "regions": [{"region": "loops", "outer": 0, "holes": [1, 2]}]}});
    let mut model = cube_with_history(10.0, profile, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let id = |key: u32| json!({"entity": format!("f3d:model:sketch-entity#{key}")});
    let source = &mut wire["model"];
    source["sketch_entities"].as_array_mut().unwrap().extend([
        circle(11, [5.0, 5.0], 2.0),
        line(12, [3.3, 3.3], [6.7, 3.3]),
        line(13, [6.7, 3.3], [6.7, 6.7]),
        line(14, [6.7, 6.7], [3.3, 6.7]),
        line(15, [3.3, 6.7], [3.3, 3.3]),
    ]);
    source["sketches"][0]["profiles"]
        .as_array_mut()
        .unwrap()
        .extend([json!([id(11)]), json!([id(12), id(13), id(14), id(15)])]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn a_region_whose_holes_cross_is_named_by_a_point() {
    let (document, report) = convert(
        &crossing_holes_history(),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    let profiles = &document["features"][1]["profiles"];
    assert_eq!(profiles["type"], "points", "{profiles}");
    assert_eq!(
        profiles["points"].as_array().unwrap().len(),
        1,
        "{profiles}"
    );
}

#[test]
fn solidcraft_replays_a_region_whose_holes_cross() {
    let Some(cli) = solidcraft() else { return };
    let volume = volumes(&cli, &written(&crossing_holes_history()).0);
    assert_eq!(volume.len(), 1, "{volume:?}");
    // The hole is the union: the circle and the square's corners outside it.
    let (half, radius) = (1.7_f64, 2.0_f64);
    let steps = 200_000;
    let corners: f64 = (0..steps)
        .map(|i| {
            let x = -half + 2.0 * half * (f64::from(i) + 0.5) / f64::from(steps);
            let inside = (radius * radius - x * x).max(0.0).sqrt();
            2.0 * (half - inside).max(0.0) * 2.0 * half / f64::from(steps)
        })
        .sum();
    let hole = std::f64::consts::PI * radius * radius + corners;
    let expected = 10.0 * (100.0 - hole);
    assert!((volume[0] - expected).abs() < 0.05, "{volume:?} {expected}");
}

/// The square taken through a list of profile loops, with two circles about (5, 5), 3 mm and
/// 1.75 mm, that no feature takes, and a line from the corner to their centre in no loop.
fn listed_square_history() -> Model {
    let profile = json!({"kind": "sketch_profiles", "value": {"sketch": SKETCH,
        "profiles": [0]}});
    let mut model = cube_with_history(10.0, profile, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let id = |key: u32| json!({"entity": format!("f3d:model:sketch-entity#{key}")});
    let source = &mut wire["model"];
    source["sketch_entities"].as_array_mut().unwrap().extend([
        circle(11, [5.0, 5.0], 3.0),
        circle(12, [5.0, 5.0], 1.75),
        line(13, [0.0, 0.0], [5.0, 5.0]),
    ]);
    source["sketches"][0]["profiles"]
        .as_array_mut()
        .unwrap()
        .extend([json!([id(11)]), json!([id(12)])]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn a_list_of_profile_loops_leaves_unused_curves_construction() {
    let (document, report) = convert(
        &listed_square_history(),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("2 of 2 features rebuilt"),
        "{}",
        history_line(&report)
    );
    // The circles and the line to their centre bound nothing the extrude takes; the square's
    // four sides stay solid. (Written curves are numbered by kind: circles c*, lines l*.)
    let flags = construction_flags(&document);
    assert_eq!(flags.len(), 7, "{flags:?}");
    for (id, construction) in &flags {
        let unused = id.starts_with('c') || id == "l5";
        assert_eq!(*construction, unused, "{flags:?}");
    }
}

#[test]
fn solidcraft_replays_a_listed_loop_beside_curves_no_feature_takes() {
    let Some(cli) = solidcraft() else { return };
    let volume = volumes(&cli, &written(&listed_square_history()).0);
    assert_eq!(volume.len(), 1, "{volume:?}");
    // Solid, the circles and the line split the square, and no profile has its boundary.
    assert!((volume[0] - 1000.0).abs() < 1e-6, "{volume:?}");
}

#[test]
fn a_curve_another_feature_names_stays_real_geometry() {
    // A sweep along curve 5 of the same sketch needs it as geometry, so nothing in the
    // sketch is demoted.
    let model = stray_curve_history(|model| {
        let section = json!({"kind": "sketch_regions", "value": {"sketch": SKETCH,
            "regions": [{"region": "loops", "outer": 0}]}});
        model["features"][1]["definition"] = json!({
            "definition": "sweep",
            "shape": {"mode": "solid", "op": "new_body",
                      "section": {"kind": "profile", "value": section}},
            "path": {"kind": "sketch_curves", "value": {
                "sketch": SKETCH, "curves": ["f3d:model:sketch-entity#5"]}},
            "path_tangent": false,
            "linearize": false,
            "path_extent": {"along_fraction": 1.0, "against_fraction": 0.0},
        });
    });
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(construction_flags(&document)
        .iter()
        .all(|(_, construction)| !construction));
    assert!(!report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("bounding no profile")));
}

#[test]
fn overlapping_crossing_curves_keep_the_refusal() {
    let mut model = crossed_profile_history(false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    // A second line along a diagonal, in the crossed loop too: a line in no loop would bound
    // nothing selected and be construction.
    wire["model"]["sketch_entities"]
        .as_array_mut()
        .unwrap()
        .push(line(11, [0.0, 0.0], [10.0, 10.0]));
    let member =
        |reversed: bool| json!({"entity": "f3d:model:sketch-entity#11", "reversed": reversed});
    let crossed = wire["model"]["sketches"][0]["profiles"][0]
        .as_array_mut()
        .unwrap();
    crossed.extend([member(false), member(true)]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_reference_sketches(&document, 1);
    assert!(
        history_line(&report).contains("a profile crossed by its own sketch's curves"),
        "{}",
        history_line(&report)
    );
}

/// A bow tie uses each curve once but intersects itself between curve endpoints.
fn bow_tie_profile() -> Model {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketch_constraints"] = json!([]);
    wire["model"]["sketch_entities"] = json!([
        line(1, [0.0, 0.0], [10.0, 10.0]),
        line(2, [10.0, 10.0], [0.0, 10.0]),
        line(3, [0.0, 10.0], [10.0, 0.0]),
        line(4, [10.0, 0.0], [0.0, 0.0]),
    ]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn a_self_intersection_between_endpoints_splits_the_profile() {
    let (document, report) = convert(
        &bow_tie_profile(),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    let mut loops: Vec<Vec<String>> =
        serde_json::from_value(document["features"][1]["profiles"]["loops"].clone()).unwrap();
    loops.sort();
    assert_eq!(loops, vec![vec!["l1", "l2", "l3"], vec!["l1", "l3", "l4"]]);
}

#[test]
fn solidcraft_replays_both_lobes_of_a_self_intersecting_profile() {
    let Some(cli) = solidcraft() else { return };
    let volume: f64 = volumes(&cli, &written(&bow_tie_profile()).0).iter().sum();
    assert!((volume - 500.0).abs() < 1e-6, "{volume}");
}

/// A native selection with a known sketch but no decoded region pick. Two nested
/// circles identify one annulus; a third creates a separate material island.
fn nested_native_profile(radii: &[f64]) -> Model {
    let profile = json!({"kind": "sketch_selection", "value": {
        "sketch": SKETCH, "selections": ["inventor:native:profile-selection#1"],
    }});
    let mut model = cube_with_history(10.0, profile, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    m["sketch_constraints"] = json!([]);
    let entities: Vec<Value> = radii
        .iter()
        .enumerate()
        .map(|(i, radius)| {
            entity(
                u32::try_from(i + 1).unwrap(),
                json!({"kind": "circle", "center": {"u": 0.0, "v": 0.0}, "radius": radius}),
            )
        })
        .collect();
    let profiles: Vec<Value> = radii
        .iter()
        .enumerate()
        .map(|(i, _)| json!([{"entity": format!("f3d:model:sketch-entity#{}", i + 1)}]))
        .collect();
    m["sketch_entities"] = json!(entities);
    m["sketches"][0]["profiles"] = json!(profiles);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn native_selections_resolve_one_even_odd_region() {
    let (document, report) = convert(
        &nested_native_profile(&[5.0, 3.0]),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    assert_eq!(
        document["features"][1]["profiles"],
        json!({"type": "whole_boundaries", "profiles": [{"outer": ["c1"], "holes": [["c2"]]}]})
    );
    let (reversed, _) = convert(
        &nested_native_profile(&[3.0, 5.0]),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(
        reversed["features"][1]["profiles"],
        json!({"type": "whole_boundaries", "profiles": [{"outer": ["c2"], "holes": [["c1"]]}]})
    );
    for radii in [&[5.0, 3.0, 1.0][..], &[5.0, 5.0][..], &[5.0, 4.99][..]] {
        let (document, report) = convert(
            &nested_native_profile(radii),
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert_reference_sketches(&document, 1);
        assert!(history_line(&report).contains("a profile cadmpeg does not resolve"));
    }
}

#[test]
fn solidcraft_replays_a_native_annulus_with_its_hole() {
    let Some(cli) = solidcraft() else { return };
    let volume = volumes(&cli, &written(&nested_native_profile(&[5.0, 3.0])).0);
    let expected = std::f64::consts::PI * 16.0 * 10.0;
    assert_eq!(volume.len(), 1);
    assert!((volume[0] - expected).abs() / expected < dutchman_solidcraft::VOLUME_TOLERANCE);
}

/// A synthetic panel with twenty circular holes centred on an unused inner rectangle.
/// The region selects the panel and circles; the inner rectangle is a layout helper.
fn panel_region() -> Model {
    let profile = json!({"kind": "sketch_regions", "value": {"sketch": SKETCH,
        "regions": [{"region": "loops", "outer": 0,
            "holes": (2..22).rev().collect::<Vec<_>>()}]}});
    let mut model = cube_with_history(10.0, profile, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    m["sketch_constraints"] = json!([]);
    let mut entities = vec![
        line(1, [0.0, 0.0], [100.0, 0.0]),
        line(2, [100.0, 0.0], [100.0, 100.0]),
        line(3, [100.0, 100.0], [0.0, 100.0]),
        line(4, [0.0, 100.0], [0.0, 0.0]),
        line(5, [10.0, 10.0], [90.0, 10.0]),
        line(6, [90.0, 10.0], [90.0, 90.0]),
        line(7, [90.0, 90.0], [10.0, 90.0]),
        line(8, [10.0, 90.0], [10.0, 10.0]),
    ];
    let boundary = |keys: Vec<u32>| {
        json!(keys
            .into_iter()
            .map(|key| json!({
                "entity": format!("f3d:model:sketch-entity#{key}")
            }))
            .collect::<Vec<_>>())
    };
    let mut profiles = vec![boundary((1..5).collect()), boundary((5..9).collect())];
    for side in 0..4 {
        for i in 0..5 {
            let along = 20.0 + f64::from(i) * 15.0;
            let center = match side {
                0 => [along, 10.0],
                1 => [90.0, along],
                2 => [along, 90.0],
                _ => [10.0, along],
            };
            let key = 9 + side * 5 + i;
            entities.push(entity(
                key,
                json!({"kind": "circle",
                "center": {"u": center[0], "v": center[1]}, "radius": 1.0}),
            ));
            profiles.push(boundary(vec![key]));
        }
    }
    m["sketch_entities"] = json!(entities);
    m["sketches"][0]["profiles"] = json!(profiles);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn explicit_regions_keep_unused_layout_lines_out_of_their_holes() {
    let model = panel_region();
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, report) = convert(&model, &writer);
    let curves = document["features"][0]["sketch"]["curves"]
        .as_array()
        .unwrap();
    assert_eq!(
        curves.iter().filter(|c| c["construction"] == true).count(),
        4
    );
    assert!(curves[4..8].iter().all(|c| c["construction"] == true));
    assert!(curves[8..].iter().all(|c| c["construction"] == false));
    assert!(report
        .losses()
        .iter()
        .any(|loss| { loss.severity == Severity::Info && loss.detail.contains("4 lines") }));

    // Other selections, including an opaque pick depending on the sketch feature, prevent
    // inference. Suppressed consumers do not change the sketch.
    for profile in [
        whole_sketch(),
        json!({"kind": "native", "value": "unresolved profile"}),
        json!({"kind": "feature", "value": "f3d:model:feature#1"}),
        json!({"kind": "sketch_regions", "value": {"sketch": SKETCH,
            "regions": [{"region": "loops", "outer": 1, "holes": []}]}}),
        json!({"kind": "sketch_regions", "value": {"sketch": SKETCH,
        "regions": [{"region": "trimmed", "outer_boundary": [{
            "entity": "f3d:model:sketch-entity#9", "parameter_range": [0.0, 1.0]
        }]}]}}),
    ] {
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let mut other = wire["model"]["features"][1].clone();
        other["id"] = json!("synthetic:panel:feature#3");
        other["ordinal"] = json!(2);
        other["definition"]["profile"] = profile;
        if other["definition"]["profile"]["kind"] == "feature" {
            other["dependencies"] = json!([]);
        }
        wire["model"]["features"]
            .as_array_mut()
            .unwrap()
            .push(other);
        let mut shared = model.clone();
        shared.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, report) = convert(&shared, &writer);
        if wire["model"]["features"][2]["definition"]["profile"]["kind"] == "native" {
            assert!(history_line(&report).contains("native profiles are not mapped yet"));
        }
        assert!(document["features"][0]["sketch"]["curves"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["construction"] == false));
        wire["model"]["features"][2]["suppressed"] = json!(true);
        shared.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, _) = convert(&shared, &writer);
        assert!(document["features"][0]["sketch"]["curves"]
            .as_array()
            .unwrap()[4..8]
            .iter()
            .all(|c| c["construction"] == true));
    }
}

#[test]
fn solidcraft_replays_a_panel_region_with_twenty_circular_holes() {
    let Some(cli) = solidcraft() else { return };
    let volume = volumes(&cli, &written(&panel_region()).0);
    let expected = (10_000.0 - 20.0 * std::f64::consts::PI) * 10.0;
    assert_eq!(volume.len(), 1);
    assert!((volume[0] - expected).abs() / expected < dutchman_solidcraft::VOLUME_TOLERANCE);
}

/// The shape cadmpeg 0.6's Inventor decoder gives an extrude: its profile as native profile
/// selections of the sketch, an explicit direction, a symmetric or through-next extent. In a
/// sketch of one profile the selection can only be that profile; multiple loops must
/// identify one material region by even-odd nesting, otherwise the history is refused.
#[test]
fn an_inventor_shaped_extrude_maps_only_when_its_sketch_has_one_profile() {
    let inventor_profile = json!({"kind": "sketch_selection",
        "value": {"sketch": SKETCH, "selections": ["inventor:native:profile-selection#1"]}});
    let extent = json!({"kind": "symmetric",
        "side": {"termination": {"kind": "blind", "length": 10.0}}});
    let model = cube_with_history(10.0, inventor_profile.clone(), Some(extent.clone()));
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, _) = convert(&model, &writer);
    assert_eq!(
        types(&document),
        [
            ("sketch".to_owned(), false),
            ("extrude".to_owned(), false),
            ("import".to_owned(), true)
        ]
    );
    // The same sketch with its loop listed twice, as two profiles.
    let mut model = cube_with_history(10.0, inventor_profile, Some(extent));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let profiles = wire["model"]["sketches"][0]["profiles"].clone();
    wire["model"]["sketches"][0]["profiles"] = json!([profiles[0], profiles[0]]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(&model, &writer);
    assert_reference_sketches(&document, 1);
    assert_eq!(
        history_line(&report),
        "flattened: feature \"Block\": a profile cadmpeg does not resolve, in a sketch of 2 \
         profiles, is not mapped yet"
    );
    let outcome = report.outcome().unwrap();
    let statuses: Vec<_> = outcome
        .features
        .iter()
        .map(|f| (f.kind.as_str(), f.status))
        .collect();
    assert_eq!(
        statuses,
        [
            ("sketch", dutchman_core::FeatureStatus::Mapped),
            ("extrude", dutchman_core::FeatureStatus::Blocked)
        ]
    );
}

/// Two parts with the cube history (and so the same parameter names), `First` placed at the
/// origin and `Second` twice, 30 mm and 60 mm along X. The assembly's own file holds no
/// sketches, as an Inventor assembly's holds none of its parts'.
fn assembly_of_histories() -> Model {
    let part = |name: &str| Model {
        name: name.to_owned(),
        source_file: Some(format!("{name}.ipt")),
        source_step: None,
        ..cube_with_history(10.0, first_profile(), None)
    };
    let at = |x: f64| {
        [
            [1.0, 0.0, 0.0, x],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ]
    };
    let placed = |part: usize, x: f64| dutchman_core::Placement {
        hidden: false,
        name: None,
        part,
        transform: at(x),
    };
    Model {
        name: "pair".to_owned(),
        source_file: Some("pair.iam".to_owned()),
        source_step: None,
        assembly: Some(dutchman_core::Assembly {
            own_part: None,
            joints: Vec::new(),
            left_out: Vec::new(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
            parts: vec![part("First"), part("Second")],
            placements: vec![placed(0, 0.0), placed(1, 30.0), placed(1, 60.0)],
        }),
        ir: cadmpeg_ir::examples::unit_cube().unwrap(),
        ..part("pair")
    }
}

#[test]
fn each_sketch_sits_under_the_part_that_owns_it_and_a_consumed_one_stays_shown() {
    let (document, _) = convert(
        &assembly_of_histories(),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let features = document["features"].as_array().unwrap();
    let components = document["components"].as_array().unwrap();
    for name in ["First", "Second"] {
        let id = components.iter().find(|c| c["name"] == name).unwrap()["id"].clone();
        let sketches: Vec<_> = features
            .iter()
            .filter(|f| f["type"] == "sketch" && f["component"] == id)
            .collect();
        assert_eq!(sketches.len(), 1, "{name} holds its sketch");
    }
    assert!(
        !features
            .iter()
            .any(|f| f["type"] == "sketch" && f["component"] == 0),
        "no sketch is left in the assembly root"
    );
    // The extrude of each part is built on its sketch, so both start shown.
    assert!(document.get("visibility").is_none_or(|v| v
        .get("hidden_sketches")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)));
}

#[test]
fn parts_keep_their_histories_inside_an_assembly() {
    let (document, report) = convert(
        &assembly_of_histories(),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let components = document["components"].as_array().unwrap();
    let second = components.iter().find(|c| c["name"] == "Second").unwrap()["id"].clone();
    let in_second: Vec<_> = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["component"] == second)
        .map(|f| {
            (
                f["type"].as_str().unwrap().to_owned(),
                f["suppressed"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        in_second,
        [
            ("sketch".to_owned(), false),
            ("extrude".to_owned(), false),
            ("import".to_owned(), true)
        ]
    );
    // Feature ids are unique, and each extrude still points at its own sketch.
    let features = document["features"].as_array().unwrap();
    let mut ids: Vec<u64> = features.iter().map(|f| f["id"].as_u64().unwrap()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), features.len());
    for extrude in features.iter().filter(|f| f["type"] == "extrude") {
        let sketch = features
            .iter()
            .find(|f| f["id"] == extrude["sketch"])
            .unwrap();
        assert_eq!(sketch["component"], extrude["component"]);
    }
    // The second part's parameters are renamed, and its dimension follows.
    let names: Vec<&str> = document["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["d1", "side", "Second_d1", "Second_side"]);
    let second_sketch = features
        .iter()
        .find(|f| f["type"] == "sketch" && f["component"] == second)
        .unwrap();
    let linked: Vec<&str> = second_sketch["sketch"]["constraints"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["param"].as_str())
        .collect();
    assert_eq!(linked, ["Second_side"]);
    let lines: Vec<String> = report
        .losses()
        .iter()
        .filter(|loss| loss.subject == "history")
        .map(|loss| loss.detail.clone())
        .collect();
    assert_eq!(
        lines,
        [
            "First.ipt: 2 of 2 features rebuilt; unverified: verification turned off",
            "Second.ipt: 2 of 2 features rebuilt; unverified: verification turned off"
        ]
    );
    assert_eq!(report.outcome().unwrap().mode, dutchman_core::Mode::History);
}

/// [`assembly_of_histories`] with `First`'s cube green and `Second`'s top face blue.
fn assembly_with_face_colours() -> Model {
    let mut model = assembly_of_histories();
    let assembly = model.assembly.as_mut().unwrap();
    for (part, recolour) in assembly.parts.iter_mut().zip([true, false]) {
        let mut wire = serde_json::to_value(&part.ir).unwrap();
        if recolour {
            wire["model"]["bodies"][0]["color"] = json!({"r": 0.0, "g": 1.0, "b": 0.0, "a": 1.0});
        } else {
            for face in wire["model"]["faces"].as_array_mut().unwrap() {
                if face["id"] == "synthetic:cube:face#top" {
                    face["color"] = json!({"r": 0.0, "g": 0.0, "b": 1.0, "a": 1.0});
                }
            }
        }
        part.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    }
    model
}

#[test]
fn parts_inside_an_assembly_keep_their_own_body_and_face_colours() {
    let (document, report) = convert(
        &assembly_with_face_colours(),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    // Both parts name their body "unit cube"; the second's is renamed so each keeps its look.
    let bodies = &document["appearances"]["bodies"];
    assert_eq!(bodies["unit cube"]["color"], json!([0, 255, 0]));
    assert_eq!(bodies["unit cube (Second)"]["color"], json!([204, 51, 26]));
    let second = document["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Second")
        .unwrap()["id"]
        .clone();
    let extrude = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "extrude" && f["component"] == second)
        .unwrap();
    assert_eq!(extrude["body_names"], json!(["unit cube (Second)"]));
    // The face look is on the second part's body, at a point in that part's own frame: on its
    // top face, not moved to where it is placed.
    let faces = document["appearances"]["faces"].as_array().unwrap();
    assert_eq!(faces.len(), 1);
    assert_eq!(faces[0]["body"], "unit cube (Second)");
    assert_eq!(faces[0]["look"]["color"], json!([0, 0, 255]));
    let p: Vec<f64> = faces[0]["point"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_f64().unwrap())
        .collect();
    assert!(
        (p[2] - 10.0).abs() < 1e-9 && (0.0..10.0).contains(&p[0]) && (0.0..10.0).contains(&p[1]),
        "{p:?}"
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

#[test]
fn solidcraft_rebuilds_each_parts_history_in_the_assembly() {
    let Some(cli) = solidcraft() else { return };
    let (bytes, _) = written(&assembly_of_histories());
    assert_eq!(volumes(&cli, &bytes), [1000.0, 1000.0, 1000.0]);
    // The second part's own `side` drives only its placements.
    let mut grown = volumes(&cli, &with_param(&bytes, "Second_side", "20 mm"));
    grown.sort_by(f64::total_cmp);
    assert_eq!(grown, [1000.0, 4000.0, 4000.0]);
}

#[test]
fn inspecting_an_assembly_maps_each_parts_history() {
    let mut report = Report::default();
    let outcome = SolidCraftWriter::new().plan(&assembly_of_histories(), &mut report);
    assert_eq!(outcome.mode, dutchman_core::Mode::History);
    let names: Vec<String> = outcome
        .features
        .iter()
        .map(|f| f.name.clone().unwrap_or_default())
        .collect();
    assert_eq!(
        names,
        [
            "First.ipt: Base sketch",
            "First.ipt: Block",
            "Second.ipt: Base sketch",
            "Second.ipt: Block"
        ]
    );
}
/// The cube history followed by a pattern of its extrude, laid out by `layout`.
fn patterned(layout: Value) -> Model {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "f3d:model:feature#3",
            "ordinal": 2,
            "name": "Copies",
            "suppressed": false,
            "definition": {
                "definition": "pattern",
                "seeds": [{"kind": "feature", "value": "f3d:model:feature#2"}],
                "pattern": layout,
            },
        }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

/// Change a synthetic pattern's operands without passing through a native decoder.
fn pattern_seeds(mut model: Model, seeds: Value) -> Model {
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][2]["definition"]["seeds"] = seeds;
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn body_patterns_use_names_assigned_by_earlier_features() {
    for selection in [
        json!({"kind": "bodies", "value": ["synthetic:cube:body#0"]}),
        json!({"kind": "resolved", "value": {
            "bodies": ["synthetic:cube:body#0"], "native": "selection"}}),
        json!({"kind": "resolved_set", "value": {"members": [
            {"body": "synthetic:cube:body#0", "native": "selection"}]}}),
    ] {
        let model = pattern_seeds(
            patterned(json!({"kind": "linear",
                "direction": {"x": 1.0, "y": 0.0, "z": 0.0}, "spacing": 20.0, "count": 3})),
            json!([{"kind": "bodies", "value": selection}]),
        );
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
        assert_eq!(document["features"][2]["bodies"], json!(["unit cube"]));
        assert_eq!(document["features"][2]["features"], json!([]));
        if let Some(cli) = solidcraft() {
            assert!((volumes(&cli, &written(&model).0).iter().sum::<f64>() - 3000.0).abs() < 1e-6);
        }
    }
}

#[test]
fn regular_explicit_pattern_locations_map_to_spacing_and_span() {
    let direction = json!({"x": 1.0, "y": 0.0, "z": 0.0});
    for (layout, field, expected) in [
        (
            json!({"kind": "linear_offsets", "direction": direction, "offsets": [0.0, 20.0, 40.0]}),
            "spacing1",
            "20 mm",
        ),
        (
            json!({"kind": "circular_angles",
            "axis_origin": {"x": -20.0, "y": 0.0, "z": 0.0},
            "axis_dir": {"x": 0.0, "y": 0.0, "z": 1.0},
            "angles": [0.0, std::f64::consts::FRAC_PI_4, std::f64::consts::FRAC_PI_2]}),
            "angle",
            "90 deg",
        ),
    ] {
        let model = patterned(layout);
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
        assert_eq!(document["features"][2]["pattern"][field], expected);
        if let Some(cli) = solidcraft() {
            assert!((volumes(&cli, &written(&model).0).iter().sum::<f64>() - 3000.0).abs() < 1e-6);
        }
    }
    let model = patterned(json!({"kind": "circular_angles",
        "axis_origin": {"x": -20.0, "y": 0.0, "z": 0.0},
        "axis_dir": {"x": 0.0, "y": 0.0, "z": 1.0},
        "angles": [0.0, std::f64::consts::FRAC_PI_2, std::f64::consts::PI,
            3.0 * std::f64::consts::FRAC_PI_2]}));
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(document["features"][2]["pattern"]["angle"], "360 deg");
    assert_eq!(document["features"][2]["pattern"]["count"], "4");
    if let Some(cli) = solidcraft() {
        assert!((volumes(&cli, &written(&model).0).iter().sum::<f64>() - 4000.0).abs() < 1e-6);
    }
}

#[test]
fn two_linear_stages_become_a_rectangular_pattern() {
    let model = patterned(json!({"kind": "composite", "stages": [
        {"pattern": {"kind": "linear", "direction": {"x": 1.0, "y": 0.0, "z": 0.0},
            "spacing": 20.0, "count": 3}},
        {"pattern": {"kind": "linear", "direction": {"x": 0.0, "y": 1.0, "z": 0.0},
            "spacing": 30.0, "count": 2}},
    ]}));
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
    assert_eq!(
        document["features"][2]["pattern"],
        json!({
        "type": "rectangular", "dir1": [1.0, 0.0, 0.0], "count1": "3", "spacing1": "20 mm",
        "dir2": [0.0, 1.0, 0.0], "count2": "2", "spacing2": "30 mm"})
    );
    if let Some(cli) = solidcraft() {
        assert!((volumes(&cli, &written(&model).0).iter().sum::<f64>() - 6000.0).abs() < 1e-6);
    }
}

#[test]
fn full_turn_endpoints_keep_only_distinct_instances() {
    for (angles, count) in [
        (vec![0.0, std::f64::consts::TAU], 1),
        (vec![0.0, std::f64::consts::PI, std::f64::consts::TAU], 2),
        (
            (0..=4)
                .map(|i| i as f64 * std::f64::consts::FRAC_PI_2)
                .collect(),
            4,
        ),
    ] {
        let model = patterned(json!({"kind": "circular_angles",
            "axis_origin": {"x": -20.0, "y": 0.0, "z": 0.0},
            "axis_dir": {"x": 0.0, "y": 0.0, "z": 1.0}, "angles": angles}));
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
        assert_eq!(
            document["features"][2]["pattern"]["count"],
            count.to_string()
        );
        assert_eq!(document["features"][2]["pattern"]["angle"], "360 deg");
        if let Some(cli) = solidcraft() {
            let v = volumes(&cli, &written(&model).0);
            assert_eq!(v.len(), count);
            assert!((v.iter().sum::<f64>() - count as f64 * 1000.0).abs() < 1e-6);
        }
    }
}

#[test]
fn composite_mirrors_multiply_the_previous_instance_set_in_order() {
    let mirror = json!({"pattern": {"kind": "mirror",
        "plane_origin": {"x": 20.0, "y": 0.0, "z": 0.0},
        "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}}});
    let linear = json!({"pattern": {"kind": "linear",
        "direction": {"x": 1.0, "y": 0.0, "z": 0.0}, "spacing": 60.0, "count": 3}});
    for (stages, expected) in [
        (
            json!([mirror, linear]),
            vec![0.0, 30.0, 60.0, 90.0, 120.0, 150.0],
        ),
        (
            json!([linear, mirror]),
            vec![-90.0, -30.0, 0.0, 30.0, 60.0, 120.0],
        ),
    ] {
        let model = patterned(json!({"kind": "composite", "stages": stages}));
        let (document, report) = convert(&model, &composite_writer());
        assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
        assert_eq!(document["features"][2]["features"], json!(["Block"]));
        assert_eq!(
            document["features"][3]["features"],
            json!(["Block", "Copies stage 1"])
        );
        assert_eq!(document["features"][3]["name"], "Copies");
        if let Some(cli) = composite_cli() {
            let design = written_with(&model, &composite_writer()).0;
            let v = volumes(&cli, &design);
            assert_eq!(v.len(), 6);
            assert!((v.iter().sum::<f64>() - 6000.0).abs() < 1e-6);
            let mut x: Vec<f64> = bounds(&cli, &design)
                .iter()
                .map(|(min, _)| min[0])
                .collect();
            x.sort_by(f64::total_cmp);
            assert_eq!(x, expected);
        }
    }
}

fn composite_writer() -> SolidCraftWriter {
    SolidCraftWriter::new()
        .verification(Verification::Off)
        .target_version(Some(dutchman_solidcraft::SolidCraftVersion::V0_25_1))
}

fn composite_cli() -> Option<PathBuf> {
    let cli = solidcraft()?;
    let version =
        dutchman_solidcraft::SolidCraftVersion::detected(&Verification::With(cli.clone()));
    if version < dutchman_solidcraft::SolidCraftVersion::V0_25_1 {
        eprintln!("skipped: composite chains need SolidCraft 0.25 patch 1 or newer");
        return None;
    }
    Some(cli)
}

#[test]
fn compatibility_floor_blocks_composite_chains_but_keeps_single_stages() {
    let mirror = json!({"kind": "mirror",
        "plane_origin": {"x": 20.0, "y": 0.0, "z": 0.0},
        "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}});
    let linear = json!({"kind": "linear",
        "direction": {"x": 1.0, "y": 0.0, "z": 0.0}, "spacing": 60.0, "count": 3});
    let writer = SolidCraftWriter::new()
        .verification(Verification::Off)
        .target_version(Some("0.18.2".parse().unwrap()));
    for stages in [
        json!([{"pattern": mirror}, {"pattern": linear}]),
        json!([{"pattern": linear}, {"pattern": mirror}]),
        json!([{"pattern": mirror}, {"pattern": mirror}]),
    ] {
        let (document, report) = convert(
            &patterned(json!({"kind": "composite", "stages": stages})),
            &writer,
        );
        // The extrusion before the blocked pattern stays as partial history.
        assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
        let feature = report
            .outcome()
            .unwrap()
            .features
            .iter()
            .find(|feature| feature.kind == "pattern")
            .unwrap();
        assert_eq!(
            feature.reason.as_deref(),
            Some("needs SolidCraft 0.25 patch 1 or newer (target 0.18)")
        );
        assert!(history_line(&report).contains(feature.reason.as_deref().unwrap()));
        assert!(!types(&document)
            .iter()
            .any(|(kind, _)| kind == "mirror" || kind == "pattern"));
    }
    for layout in [
        mirror.clone(),
        linear.clone(),
        json!({"kind": "composite", "stages": [{"pattern": mirror}]}),
        json!({"kind": "composite", "stages": [{"pattern": linear}]}),
    ] {
        let (document, report) = convert(&patterned(layout.clone()), &writer);
        assert_eq!(report.outcome().unwrap().mode, Mode::History);
        assert!(types(&document)
            .iter()
            .any(|(kind, _)| kind == "mirror" || kind == "pattern"));
        let mut wire = serde_json::to_value(&patterned(layout.clone()).ir).unwrap();
        wire["model"]["features"]
            .as_array_mut()
            .unwrap()
            .push(json!({
            "id": "f3d:model:feature#4", "ordinal": 3, "name": "Later copies", "suppressed": false,
            "definition": {"definition": "pattern",
                "seeds": [{"kind": "feature", "value": "f3d:model:feature#3"}],
                "pattern": {"kind": "mirror", "plane_origin": {"x": 0.0, "y": 20.0, "z": 0.0},
                    "plane_normal": {"x": 0.0, "y": 1.0, "z": 0.0}}}}));
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..patterned(layout)
        };
        let (_, report) = convert(&model, &writer);
        // The features before the gated later pattern stay as partial history.
        assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
        assert!(
            history_line(&report).contains("needs SolidCraft 0.25 patch 1 or newer (target 0.18)")
        );
        let (_, report) = convert(&model, &composite_writer());
        assert_eq!(report.outcome().unwrap().mode, Mode::History);
    }
}

#[test]
fn composite_mirrors_chain_multiple_reflections_and_circular_copies() {
    let mirror = json!({"pattern": {"kind": "mirror",
        "plane_origin": {"x": 20.0, "y": 0.0, "z": 0.0},
        "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}}});
    let other = json!({"pattern": {"kind": "mirror",
        "plane_origin": {"x": 0.0, "y": 20.0, "z": 0.0},
        "plane_normal": {"x": 0.0, "y": 1.0, "z": 0.0}}});
    let circular = json!({"pattern": {"kind": "circular",
        "axis_origin": {"x": -1000.0, "y": -1000.0, "z": 0.0},
        "axis_dir": {"x": 0.0, "y": 0.0, "z": 1.0},
        "angle": std::f64::consts::FRAC_PI_2, "count": 3}});
    for (stages, count) in [
        (json!([mirror, other]), 4),
        (json!([mirror, other, circular]), 12),
        (json!([circular, mirror]), 6),
    ] {
        let model = patterned(json!({"kind": "composite", "stages": stages}));
        let (_, report) = convert(&model, &composite_writer());
        assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
        if let Some(cli) = composite_cli() {
            let v = volumes(&cli, &written_with(&model, &composite_writer()).0);
            assert_eq!(v.len(), count);
            assert!((v.iter().sum::<f64>() - count as f64 * 1000.0).abs() < 1e-5);
        }
    }
}

#[test]
fn composite_mirror_chains_refuse_unrepresentable_stages_and_excessive_products() {
    let mirror = json!({"pattern": {"kind": "mirror",
        "plane_origin": {"x": 20.0, "y": 0.0, "z": 0.0},
        "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}}});
    let model = patterned(json!({"kind": "composite", "stages": [mirror,
        {"pattern": {"kind": "linear", "direction": {"x": 1.0, "y": 0.0, "z": 0.0},
            "spacing": 20.0, "count": 5001}}]}));
    let (document, report) = convert(&model, &composite_writer());
    assert!(
        history_line(&report).contains("more than 10000 instances"),
        "{}",
        history_line(&report)
    );
    // The sketch and extrusion before the refused pattern stay as partial history beside
    // the imported final shape.
    assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
    assert_eq!(
        types(&document),
        [
            ("sketch".to_owned(), false),
            ("extrude".to_owned(), false),
            ("import".to_owned(), false)
        ]
    );
    let model = pattern_seeds(
        patterned(json!({"kind": "composite", "stages": [mirror, mirror]})),
        json!([{"kind": "bodies", "value": {"kind": "bodies", "value": ["synthetic:cube:body#0"]}}]),
    );
    let (_, report) = convert(&model, &composite_writer());
    assert!(history_line(&report).contains("intermediate copies have no mapped body names"));
}

#[test]
fn later_patterns_can_select_a_whole_composite_mirror_result() {
    let model = patterned(json!({"kind": "composite", "stages": [
        {"pattern": {"kind": "mirror", "plane_origin": {"x": 20.0, "y": 0.0, "z": 0.0},
            "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}}},
        {"pattern": {"kind": "linear", "direction": {"x": 1.0, "y": 0.0, "z": 0.0},
            "spacing": 60.0, "count": 3}}
    ]}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    // Colliding source and helper names must still select the right timeline features.
    wire["model"]["features"][0]["name"] = json!("Copies stage 1");
    wire["model"]["features"].as_array_mut().unwrap().push(json!({
        "id": "f3d:model:feature#4", "ordinal": 3, "name": "Copies", "suppressed": false,
        "definition": {"definition": "pattern", "seeds": [{"kind": "feature", "value": "f3d:model:feature#3"}],
            "pattern": {"kind": "linear", "direction": {"x": 0.0, "y": 1.0, "z": 0.0},
                "spacing": 30.0, "count": 2}}}));
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    };
    let (document, report) = convert(&model, &composite_writer());
    assert!(history_line(&report).starts_with("4 of 4 features rebuilt"));
    assert_eq!(document["features"][2]["name"], "Copies stage 1 (2)");
    assert_eq!(document["features"][4]["name"], "Copies (2)");
    assert_eq!(
        document["features"][4]["features"],
        json!(["Block", "Copies stage 1 (2)", "Copies"])
    );
    if let Some(cli) = composite_cli() {
        let v = volumes(&cli, &written_with(&model, &composite_writer()).0);
        assert_eq!(v.len(), 12);
        assert!((v.iter().sum::<f64>() - 12000.0).abs() < 1e-6);
    }
}

#[test]
fn unfinished_pattern_replays_keep_the_original_geometry() {
    let Some(cli) = solidcraft() else { return };
    for layout in [
        json!({"kind": "circular_angles", "axis_origin": {"x": 0.0, "y": 0.0, "z": 0.0},
            "axis_dir": {"x": 0.0, "y": 0.0, "z": 1.0}, "angles": [0.0, std::f64::consts::TAU]}),
        json!({"kind": "composite", "stages": [{"pattern": {"kind": "mirror",
            "plane_origin": {"x": 20.0, "y": 0.0, "z": 0.0},
            "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}}},
            {"pattern": {"kind": "linear", "direction": {"x": 0.0, "y": 1.0, "z": 0.0},
                "spacing": 30.0, "count": 2}}]}),
    ] {
        let (document, report) = convert(
            &patterned(layout),
            &SolidCraftWriter::new()
                .verification(Verification::With(cli.clone()))
                .target_version(Some(dutchman_solidcraft::SolidCraftVersion::V0_25_1))
                .timeout(std::time::Duration::ZERO),
        );
        // The features before the unverified pattern stay as partial history.
        assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
        assert!(
            history_line(&report).contains("pattern instance set could not be verified"),
            "{}",
            history_line(&report)
        );
        assert!(history_line(&report).contains("timed out"));
        // The unverified timeline (sketch, extrusion, then the pattern's mirror or pattern
        // stages) stays hidden beside the visible original shape.
        let kinds: Vec<String> = types(&document).into_iter().map(|(kind, _)| kind).collect();
        assert_eq!(kinds[..2], ["sketch", "extrude"], "{kinds:?}");
        assert_eq!(
            kinds.last().map(String::as_str),
            Some("import"),
            "{kinds:?}"
        );
        let stages = &kinds[2..kinds.len() - 1];
        assert!(!stages.is_empty(), "{kinds:?}");
        assert!(
            stages
                .iter()
                .all(|kind| kind == "mirror" || kind == "pattern"),
            "{kinds:?}"
        );
    }
}

#[test]
fn cc0_freecad_mirror_and_full_turn_match_recipe_volumes() {
    for (file, count, volume) in [
        ("S13c.FCStd", 7, 11660.70799341),
        ("freecad-1.1.4/S13c.FCStd", 7, 11660.70799341),
        ("P08b.FCStd", 5, 13006.19358586),
    ] {
        let cli = if file == "P08b.FCStd" {
            solidcraft()
        } else {
            composite_cli()
        };
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(file);
        let model = dutchman_core::read(
            dutchman_core::Format::FreeCadDocument,
            &path,
            &mut Report::default(),
        )
        .unwrap();
        let (document, report) = convert(&model, &composite_writer());
        assert!(
            history_line(&report).starts_with(&format!("{count} of {count} features rebuilt")),
            "{file}: {}",
            history_line(&report)
        );
        assert!(types(&document).iter().any(|(kind, _)| kind == "pattern"));
        let model = if file == "P08b.FCStd" {
            assert_eq!(document["features"][4]["pattern"]["count"], "7");
            assert_eq!(document["features"][4]["pattern"]["angle"], "360 deg");
            // The pinned decoder resolves Z_Axis as -Y and includes a duplicate endpoint
            // where the recipe has eight distinct holes. Never repair native decoding
            // in the mapper: expose both gaps and isolate them in this neutral test model.
            assert!(
                (document["features"][4]["pattern"]["axis"][1]
                    .as_f64()
                    .unwrap()
                    + 1.0)
                    .abs()
                    < 1e-9
            );
            let mut wire = serde_json::to_value(&model.ir).unwrap();
            for feature in wire["model"]["features"].as_array_mut().unwrap() {
                if feature["name"] == "PolarPattern" {
                    feature["definition"]["operation"]["pattern"]["axis_dir"] =
                        json!({"x": 0.0, "y": 0.0, "z": 1.0});
                }
            }
            if let Some(cli) = cli.clone() {
                let axis_only = Model {
                    ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
                    ..model.clone()
                };
                let (_, report) = convert(
                    &axis_only,
                    &SolidCraftWriter::new().verification(Verification::With(cli)),
                );
                // A differing replay keeps the mapped timeline as partial history.
                assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
                assert!(
                    history_line(&report).contains("replay differs"),
                    "{}",
                    history_line(&report)
                );
                assert!(history_line(&report).contains("volume"));
            }
            for feature in wire["model"]["features"].as_array_mut().unwrap() {
                if feature["name"] == "PolarPattern" {
                    feature["definition"]["operation"]["pattern"]["angles"] = json!((0..8)
                        .map(|i| i as f64 * std::f64::consts::TAU / 8.0)
                        .collect::<Vec<_>>());
                }
            }
            Model {
                ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
                ..model
            }
        } else {
            model
        };
        if let Some(cli) = cli {
            let (design, report) = written_with(
                &model,
                &SolidCraftWriter::new()
                    .verification(Verification::With(cli.clone()))
                    .target_version(Some(dutchman_solidcraft::SolidCraftVersion::V0_25_1)),
            );
            assert_eq!(
                report.outcome().unwrap().mode,
                Mode::History,
                "{file}: {}",
                history_line(&report)
            );
            assert!(
                history_line(&report).contains("verified against the original"),
                "{file}: {}",
                history_line(&report)
            );
            let v = volumes(&cli, &design);
            assert_eq!(v.len(), 1);
            assert!((v[0] - volume).abs() / volume < 1e-4, "{file}: {v:?}");
        }
    }
}

/// Reuse the sweep fixture's mapped path sketch, lengthened to contain three copies.
fn path_pattern(whole_sketch: bool) -> Model {
    let mut model = sweep_history(|_| {});
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    for row in wire["model"]["sketch_entities"].as_array_mut().unwrap() {
        if row["id"] == "f3d:model:sketch-entity#21" {
            row["geometry"]["end"]["v"] = json!(60.0);
        }
    }
    let features = wire["model"]["features"].as_array_mut().unwrap();
    features[2] = serde_json::to_value(&cube_with_history(10.0, first_profile(), None).ir).unwrap()
        ["model"]["features"][1]
        .clone();
    features[2]["ordinal"] = json!(2);
    let path = if whole_sketch {
        json!({"kind": "sketch", "value": PATH})
    } else {
        json!({"kind": "sketch_curves", "value": {"sketch": PATH,
            "curves": ["f3d:model:sketch-entity#21"]}})
    };
    features.push(json!({"id": "f3d:model:feature#4", "ordinal": 3,
        "name": "Copies", "suppressed": false, "definition": {
            "definition": "pattern", "seeds": [{"kind": "feature", "value": "f3d:model:feature#2"}],
            "pattern": {"kind": "curve_driven", "path": path, "spacing": 20.0, "count": 3}}}));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn path_patterns_use_mapped_sketch_curves_and_arc_length_spacing() {
    for whole_sketch in [false, true] {
        let model = path_pattern(whole_sketch);
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(history_line(&report).starts_with("4 of 4 features rebuilt"));
        let pattern = &document["features"][3]["pattern"];
        assert_eq!(pattern["type"], "path");
        assert_eq!(pattern["path_sketch"], document["features"][1]["id"]);
        assert_eq!(pattern["count"], "3");
        assert_eq!(pattern["spacing"], "20 mm");
        assert_eq!(pattern["path"].as_array().unwrap().len(), 1);
        if let Some(cli) = solidcraft() {
            let mut starts: Vec<f64> = bounds(&cli, &written(&model).0)
                .iter()
                .map(|(min, _)| min[2])
                .collect();
            starts.sort_by(f64::total_cmp);
            assert_eq!(starts, [0.0, 20.0, 40.0]);
        }
    }
}

#[test]
fn pattern_refusals_identify_the_unrepresentable_option() {
    for (layout, reason) in [
        (
            json!({"kind": "linear_offsets", "direction": {"x": 1.0, "y": 0.0, "z": 0.0},
            "offsets": [0.0, 20.0, 60.0]}),
            "nonuniform linear offsets",
        ),
        (
            json!({"kind": "circular_angles", "axis_origin": {"x": 0.0, "y": 0.0, "z": 0.0},
            "axis_dir": {"x": 0.0, "y": 0.0, "z": 1.0}, "angles": [0.0, 0.5, 2.0]}),
            "nonuniform circular angles",
        ),
        (
            json!({"kind": "curve_driven", "path": {"kind": "native", "value": "path"},
            "spacing": 20.0, "count": 3}),
            "pattern paths from native",
        ),
        (
            json!({"kind": "curve_driven", "spacing": 20.0, "count": 3}),
            "its pattern path is not resolved",
        ),
        (
            json!({"kind": "unresolved", "form": "curve_driven"}),
            "cadmpeg does not decode its curve driven pattern layout yet",
        ),
        (
            json!({"kind": "composite", "stages": [
                {"pattern": {"kind": "circular", "axis_origin": {"x": 0.0, "y": 0.0, "z": 0.0},
                    "axis_dir": {"x": 0.0, "y": 0.0, "z": 1.0}, "angle": 1.0, "count": 2}},
                {"pattern": {"kind": "circular", "axis_origin": {"x": 5.0, "y": 0.0, "z": 0.0},
                    "axis_dir": {"x": 0.0, "y": 0.0, "z": 1.0}, "angle": 1.0, "count": 2}}
            ]}),
            "2 stages (circular then circular)",
        ),
        (
            json!({"kind": "scale", "center": {"kind": "first_seed_centroid"},
            "final_factor": 2.0, "count": 3}),
            "progressively scaled patterns",
        ),
        (
            json!({"kind": "linear", "direction": {"x": 1.0, "y": 0.0, "z": 0.0},
            "spacing": 20.0, "count": 10001}),
            "more than 10000 instances",
        ),
    ] {
        let (_, report) = convert(
            &patterned(layout),
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn unresolved_pattern_seeds_and_mixed_seed_kinds_have_specific_refusals() {
    let layout = json!({"kind": "linear",
        "direction": {"x": 1.0, "y": 0.0, "z": 0.0}, "spacing": 20.0, "count": 3});
    for (seeds, reason) in [
        (
            json!([{"kind": "bodies", "value": {"kind": "unresolved"}}]),
            "cadmpeg does not resolve which bodies",
        ),
        (
            json!([{"kind": "bodies", "value": {"kind": "native", "value": "body"}}]),
            "cadmpeg does not resolve which bodies",
        ),
        (
            json!([{"kind": "bodies", "value": {"kind": "native_set", "value": ["body"]}}]),
            "cadmpeg does not resolve which bodies",
        ),
        (
            json!([{"kind": "faces", "value": {"kind": "native", "value": "face"}}]),
            "SolidCraft patterns repeat whole features or bodies",
        ),
        (
            json!([{"kind": "feature", "value": "f3d:model:feature#2"},
            {"kind": "bodies", "value": {"kind": "bodies", "value": ["synthetic:cube:body#0"]}}]),
            "SolidCraft requires one seed kind",
        ),
    ] {
        let model = pattern_seeds(patterned(layout.clone()), seeds);
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn patterns_refuse_unnamed_bodies_and_excessive_rectangular_products() {
    let model = pattern_seeds(
        patterned(
            json!({"kind": "linear", "direction": {"x": 1.0, "y": 0.0, "z": 0.0},
            "spacing": 20.0, "count": 3}),
        ),
        json!([{"kind": "bodies", "value": {"kind": "bodies", "value": ["synthetic:cube:body#0"]}}]),
    );
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][1]["outputs"] = json!([]);
    let unnamed = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    };
    let (_, report) = convert(
        &unnamed,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).contains("body without an earlier mapped name"));

    let too_many = patterned(json!({"kind": "linear",
        "direction": {"x": 1.0, "y": 0.0, "z": 0.0}, "spacing": 20.0, "count": 101,
        "second": {"direction": {"x": 0.0, "y": 1.0, "z": 0.0}, "spacing": 20.0, "count": 100}}));
    let (_, report) = convert(
        &too_many,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).contains("more than 10000 instances"));
}

#[test]
fn a_feature_pattern_names_its_seed() {
    let model = patterned(json!({"kind": "linear",
        "direction": {"x": 1.0, "y": 0.0, "z": 0.0}, "spacing": 20.0, "count": 3}));
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("3 of 3 features rebuilt"),
        "{}",
        history_line(&report)
    );
    let pattern = &document["features"][2];
    assert_eq!(pattern["type"], "pattern");
    assert_eq!(pattern["features"], json!(["Block"]));
}

#[test]
fn solidcraft_replays_feature_patterns() {
    let Some(cli) = solidcraft() else { return };
    let linear = patterned(json!({"kind": "linear",
        "direction": {"x": 1.0, "y": 0.0, "z": 0.0}, "spacing": 20.0, "count": 3}));
    let mut copies = volumes(&cli, &written(&linear).0);
    copies.sort_by(f64::total_cmp);
    assert_eq!(copies.iter().sum::<f64>(), 3000.0, "{copies:?}");
    // Spacing is between instances: the third copy spans 40 to 50 mm.
    let mut starts: Vec<f64> = bounds(&cli, &written(&linear).0)
        .iter()
        .map(|(min, _)| min[0])
        .collect();
    starts.sort_by(f64::total_cmp);
    assert_eq!(starts, [0.0, 20.0, 40.0]);

    let around = patterned(json!({"kind": "circular",
        "axis_origin": {"x": -20.0, "y": 0.0, "z": 0.0},
        "axis_dir": {"x": 0.0, "y": 0.0, "z": 1.0},
        "angle": std::f64::consts::TAU, "count": 4}));
    let copies = volumes(&cli, &written(&around).0);
    assert!(
        (copies.iter().sum::<f64>() - 4000.0).abs() < 1e-6,
        "{copies:?}"
    );
}

#[test]
fn partial_circular_patterns_include_both_ends() {
    let model = patterned(json!({"kind": "circular",
        "axis_origin": {"x": -20.0, "y": 0.0, "z": 0.0},
        "axis_dir": {"x": 0.0, "y": 0.0, "z": 2.0},
        "angle": std::f64::consts::FRAC_PI_2, "count": 3}));
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
    assert_eq!(document["features"][2]["features"], json!(["Block"]));
    assert_eq!(
        document["features"][2]["pattern"],
        json!({"type": "circular", "origin": [-20.0, 0.0, 0.0],
            "axis": [0.0, 0.0, 1.0], "count": "3", "angle": "90 deg"})
    );

    let Some(cli) = solidcraft() else { return };
    let design = written(&model).0;
    let mut boxes = bounds(&cli, &design);
    boxes.sort_by(|(a, _), (b, _)| b[0].total_cmp(&a[0]));
    let root2 = std::f64::consts::SQRT_2;
    // Rotate the seed's [0, 10]^3 box about (-20, 0, 0) by 0, 45 and 90 degrees.
    // The middle box distinguishes a total span from a per-instance angle.
    let expected = [
        ([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
        (
            [-20.0 + 5.0 * root2, 10.0 * root2, 0.0],
            [-20.0 + 15.0 * root2, 20.0 * root2, 10.0],
        ),
        ([-30.0, 20.0, 0.0], [-20.0, 30.0, 10.0]),
    ];
    assert_eq!(boxes.len(), expected.len());
    for ((min, max), (want_min, want_max)) in boxes.iter().zip(expected) {
        for axis in 0..3 {
            assert!((min[axis] - want_min[axis]).abs() < 1e-6, "{boxes:?}");
            assert!((max[axis] - want_max[axis]).abs() < 1e-6, "{boxes:?}");
        }
    }
    let copies = volumes(&cli, &design);
    assert_eq!(copies.len(), 3);
    assert!(copies.iter().all(|volume| (volume - 1000.0).abs() < 1e-6));
}

/// Each body's bounding box (min, max) as SolidCraft evaluates the design.
fn bounds(cli: &Path, design: &[u8]) -> Vec<([f64; 3], [f64; 3])> {
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
    let corner = |v: &Value| [0, 1, 2].map(|i| v[i].as_f64().unwrap());
    result["bodies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| (corner(&b["bbox"]["min"]), corner(&b["bbox"]["max"])))
        .collect()
}

#[test]
fn a_face_colour_becomes_a_face_look_on_that_face() {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    for face in wire["model"]["faces"].as_array_mut().unwrap() {
        if face["id"] == "synthetic:cube:face#top" {
            face["color"] = json!({"r": 0.0, "g": 0.0, "b": 1.0, "a": 1.0});
        }
    }
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let faces = document["appearances"]["faces"].as_array().unwrap();
    assert_eq!(faces.len(), 1, "only the face that differs from its body");
    let look = &faces[0];
    assert_eq!(look["body"], "unit cube");
    assert_eq!(look["look"]["color"], json!([0, 0, 255]));
    let p: Vec<f64> = look["point"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_f64().unwrap())
        .collect();
    assert!(
        (p[2] - 10.0).abs() < 1e-9 && (0.0..10.0).contains(&p[0]) && (0.0..10.0).contains(&p[1]),
        "{p:?}"
    );

    if let Some(cli) = solidcraft() {
        assert_eq!(volumes(&cli, &written(&model).0), [1000.0]);
    }
}

#[test]
fn freecad_periodic_spline_recipes_keep_their_history() {
    // cadmpeg expands compact periodic sketch knots, so both native repros convert in full.
    for name in ["N04a.FCStd", "N04b.FCStd"] {
        let input = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../dutchman-core/tests/fixtures/freecad-1.1.4-N04")
            .join(name);
        let model = dutchman_core::read(
            dutchman_core::Format::FreeCadDocument,
            &input,
            &mut Report::default(),
        )
        .unwrap();
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).starts_with("2 of 2 features rebuilt"),
            "{name}: {}",
            history_line(&report)
        );
        let Some(cli) = solidcraft() else { continue };
        let (_, report) = written_with(
            &model,
            &SolidCraftWriter::new().verification(Verification::With(cli)),
        );
        assert_eq!(report.outcome().unwrap().check, Check::Verified, "{name}");
    }
}

fn nonuniform_spline_geometry() -> Value {
    json!({"kind": "nurbs", "curve": {"degree": 3, "periodic": false,
        "knots": [0.,0.,0.,0.,0.3,1.,1.,1.,1.],
        "poles": {"form": "polynomial", "points": [
            {"u": 0., "v": 0.}, {"u": 1., "v": 4.}, {"u": 4., "v": 6.},
            {"u": 8., "v": 4.}, {"u": 10., "v": 0.}]}}})
}

#[test]
fn nonuniform_spline_profiles_keep_one_closed_loop_and_extrude() {
    for profile in [
        first_profile(),
        json!({"kind": "sketch_entities", "value": {"sketch": SKETCH,
            "entities": ["f3d:model:sketch-entity#1", "f3d:model:sketch-entity#2"]}}),
        json!({"kind": "sketch_regions", "value": {"sketch": SKETCH,
            "regions": [{"region": "loops", "outer": 0, "holes": []}]}}),
    ] {
        let mut model = cube_with_history(10., profile, None);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let m = &mut wire["model"];
        m["sketch_entities"] = json!([
            entity(1, nonuniform_spline_geometry()),
            line(2, [10., 0.], [0., 0.])
        ]);
        m["sketches"][0]["profiles"] = json!([[{"entity": "f3d:model:sketch-entity#1"}, {"entity": "f3d:model:sketch-entity#2"}]]);
        m["sketch_constraints"] = json!([]);
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).starts_with("2 of 2 features rebuilt"),
            "{:?}",
            report.losses()
        );
        let sketch = &document["features"][0]["sketch"];
        let curves = sketch["curves"].as_array().unwrap();
        assert_eq!(curves.len(), 3);
        let first = curves[0]["pts"].as_array().unwrap();
        let second = curves[1]["pts"].as_array().unwrap();
        assert_eq!(first.last(), second.first());
        assert_eq!(first.first().unwrap(), &curves[2]["b"]);
        assert_eq!(second.last().unwrap(), &curves[2]["a"]);
        assert!(sketch["constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["type"] == "tangent" && c["a"] == 0 && c["b"] == 1));
        assert_eq!(document["features"][1]["type"], "extrude");
        let selection = &document["features"][1]["profiles"];
        assert!(
            selection == &json!({"type": "all"})
                || selection == &json!({"type": "curves", "loops": [["s1", "s2", "l1"]]})
                || selection
                    == &json!({"type": "whole_boundaries", "profiles": [{"outer": ["l1", "s1", "s2"], "holes": []}]}),
            "{selection}"
        );
        if let Some(cli) = solidcraft() {
            let volume = volumes(&cli, &written(&model).0);
            assert_eq!(volume.len(), 1, "{volume:?}");
            assert!(volume[0] > 0.0, "{volume:?}");
        }
    }
}

#[test]
fn nonuniform_spline_sweep_paths_expand_the_source_curve() {
    for whole in [false, true] {
        let mut model = sweep_history(|d| {
            d["path"] = if whole {
                json!({"kind": "sketch", "value": PATH})
            } else {
                json!({"kind": "sketch_curves", "value": {"sketch": PATH, "curves": ["f3d:model:sketch-entity#20"]}})
            };
        });
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        for entity in wire["model"]["sketch_entities"].as_array_mut().unwrap() {
            if entity["sketch"] == PATH {
                entity["id"] = json!("f3d:model:sketch-entity#20");
                entity["geometry"] = nonuniform_spline_geometry();
            }
        }
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).starts_with("3 of 3 features rebuilt"),
            "{:?}",
            report.losses()
        );
        let sweep = document["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["type"] == "sweep")
            .unwrap();
        assert_eq!(sweep["path"], json!(["s1", "s2"]));
    }
}

#[test]
fn nonuniform_spline_constraints_follow_chain_endpoints_and_fixed_pieces() {
    let mut model = bounded_conic_history(nonuniform_spline_geometry(), [0., 0.], [10., 0.]);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let mut tangent = line(3, [10., 0.], [12., -4.]);
    tangent["construction"] = json!(true);
    wire["model"]["sketch_entities"]
        .as_array_mut()
        .unwrap()
        .push(tangent);
    wire["model"]["sketch_constraints"] = json!([
        {"id": "f3d:model:sketch-constraint#1", "sketch": SKETCH,
         "definition": {"kind": "fixed", "entity": "f3d:model:sketch-entity#1"}},
        {"id": "f3d:model:sketch-constraint#2", "sketch": SKETCH,
         "definition": {"kind": "coincident_loci", "loci": [
             {"kind": "end", "entity": "f3d:model:sketch-entity#1"},
             {"kind": "start", "entity": "f3d:model:sketch-entity#2"}]}},
        {"id": "f3d:model:sketch-constraint#3", "sketch": SKETCH,
         "definition": {"kind": "distance", "entities": ["f3d:model:sketch-entity#1"], "parameter": "f3d:model:parameter#2"}},
        {"id": "f3d:model:sketch-constraint#4", "sketch": SKETCH,
         "definition": {"kind": "tangent_loci", "first": {"kind": "end", "entity": "f3d:model:sketch-entity#1"},
             "second": {"kind": "start", "entity": "f3d:model:sketch-entity#3"}}},
        {"id": "f3d:model:sketch-constraint#5", "sketch": SKETCH,
         "definition": {"kind": "curvature", "first": "f3d:model:sketch-entity#1", "second": "f3d:model:sketch-entity#3"}}
    ]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("2 of 2 features rebuilt"),
        "{:?}",
        report.losses()
    );
    let curves = document["features"][0]["sketch"]["curves"]
        .as_array()
        .unwrap();
    assert_eq!(curves[0]["fixed"], true);
    assert_eq!(curves[1]["fixed"], true);
    let constraints = document["features"][0]["sketch"]["constraints"]
        .as_array()
        .unwrap();
    for kind in ["tangent", "smooth"] {
        assert!(
            constraints
                .iter()
                .any(|c| c["type"] == kind && c["a"] == 1 && c["b"] == 3),
            "{constraints:?}"
        );
    }
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.severity == dutchman_core::Severity::Info
            && loss
                .detail
                .contains("a constraint on an exact spline chain is not carried")));
    assert!(!report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("constraints not carried")));
}

#[test]
fn nonuniform_spline_projected_copies_keep_every_control_point() {
    let mut model = bounded_conic_history(nonuniform_spline_geometry(), [0., 0.], [10., 0.]);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let mut copy = entity(3, nonuniform_spline_geometry());
    copy["construction"] = json!(true);
    wire["model"]["sketch_entities"]
        .as_array_mut()
        .unwrap()
        .push(copy);
    wire["model"]["sketch_constraints"] = json!([
        {"id": "f3d:model:sketch-constraint#1", "sketch": SKETCH,
         "definition": {"kind": "projected_copy", "source": "f3d:model:sketch-entity#1", "result": "f3d:model:sketch-entity#3"}}
    ]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("2 of 2 features rebuilt"),
        "{:?}",
        report.losses()
    );
    assert!(
        !report
            .losses()
            .iter()
            .any(|loss| loss.detail.contains("not carried")),
        "{:?}",
        report.losses()
    );
    let sketch = &document["features"][0]["sketch"];
    assert_eq!(sketch["curves"].as_array().unwrap().len(), 5);
    assert_eq!(sketch["curves"][3]["construction"], true);
    assert_eq!(sketch["curves"][4]["construction"], true);
    let source: Vec<_> = sketch["curves"][0]["pts"]
        .as_array()
        .unwrap()
        .iter()
        .chain(
            sketch["curves"][1]["pts"]
                .as_array()
                .unwrap()
                .iter()
                .skip(1),
        )
        .collect();
    let result: Vec<_> = sketch["curves"][3]["pts"]
        .as_array()
        .unwrap()
        .iter()
        .chain(
            sketch["curves"][4]["pts"]
                .as_array()
                .unwrap()
                .iter()
                .skip(1),
        )
        .collect();
    let constraints = sketch["constraints"].as_array().unwrap();
    for (p, q) in source.into_iter().zip(result) {
        assert!(
            p == q || constraints.iter().any(|c| c["p"] == *p && c["q"] == *q),
            "{p} -> {q}: {constraints:?}"
        );
    }
}

fn periodic_spline_profile(profile: Value) -> Model {
    let mut model = cube_with_history(10.0, profile, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketches"][0]["profiles"] = json!([[{"entity": "f3d:model:sketch-entity#1"}]]);
    wire["model"]["sketch_constraints"] = json!([]);
    wire["model"]["sketch_entities"] = json!([entity(
        1,
        json!({"kind": "nurbs", "curve": {"degree": 2, "periodic": true,
            "knots": [-2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            "poles": {"form": "polynomial", "points": [
                {"u": -10.0, "v": -10.0}, {"u": 10.0, "v": -10.0},
                {"u": 10.0, "v": 10.0}, {"u": -10.0, "v": 10.0},
                {"u": -10.0, "v": -10.0}, {"u": 10.0, "v": -10.0}]}}})
    )]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn periodic_spline_profiles_keep_editable_history_and_profile_selections() {
    for profile in [whole_sketch(), first_profile()] {
        let model = periodic_spline_profile(profile);
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert_eq!(report.outcome().unwrap().mode, Mode::History, "{report:?}");
        let sketch = &document["features"][0]["sketch"];
        let curve = &sketch["curves"][0];
        assert_eq!(curve["type"], "spline");
        assert_eq!(curve["degree"], 2);
        assert_eq!(curve["control"], true);
        let pts = curve["pts"].as_array().unwrap();
        assert_eq!(pts.first(), pts.last());
        assert_eq!(pts.len(), 6);
        assert!(!history_line(&report).contains("not mapped"));
    }
}

#[test]
fn weighted_periodic_splines_name_solidcrafts_missing_weights() {
    let mut model = periodic_spline_profile(whole_sketch());
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let curve = &mut wire["model"]["sketch_entities"][0]["geometry"]["curve"];
    let points = curve["poles"]["points"].as_array().unwrap().clone();
    curve["poles"] = json!({"form": "rational", "points": points.iter().enumerate()
        .map(|(i, p)| json!({"point": p, "weight": if i % 4 == 0 { 2.0 } else { 1.0 }})).collect::<Vec<_>>()});
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(report.outcome().unwrap().mode, Mode::Import);
    assert!(history_line(&report).contains("weighted periodic splines are not supported yet: SolidCraft control splines have no weights"), "{report:?}");
}

#[test]
fn solidcraft_extrudes_a_closed_periodic_spline_profile() {
    let Some(cli) = solidcraft() else { return };
    for profile in [whole_sketch(), first_profile()] {
        let model = periodic_spline_profile(profile);
        let (bytes, report) = written(&model);
        assert_eq!(report.outcome().unwrap().mode, Mode::History, "{report:?}");
        let got = volumes(&cli, &bytes);
        // A 20 mm square with four quadratic corners removes 4 × 10²/6 mm².
        let expected = (400.0 - 400.0 / 6.0) * 10.0;
        assert_eq!(got.len(), 1, "{got:?}");
        assert!((got[0] - expected).abs() / expected < 1e-3, "{got:?}");
    }
}

#[test]
fn solidcraft_extrudes_a_spline_profile() {
    let Some(cli) = solidcraft() else { return };
    let mut model = cube_with_history(10.0, whole_sketch(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketches"][0]["profiles"] = json!([]);
    wire["model"]["sketch_constraints"] = json!([]);
    wire["model"]["sketch_entities"] = json!([
        line(1, [0.0, 0.0], [30.0, 0.0]),
        entity(
            2,
            json!({"kind": "nurbs", "curve": {"degree": 2, "periodic": false,
            "knots": [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            "poles": {"form": "polynomial", "points": [
                {"u": 0.0, "v": 0.0}, {"u": 15.0, "v": 30.0}, {"u": 30.0, "v": 0.0}]}}})
        ),
    ]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    // Under a quadratic arch: base × control height / 3 = 300 mm², extruded 10 mm.
    let got = volumes(&cli, &written(&model).0);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!((got[0] - 3000.0).abs() / 3000.0 < 1e-3, "{got:?}");
}

#[test]
fn solidcraft_colours_the_face_of_each_parts_own_body() {
    let Some(cli) = solidcraft() else { return };
    let dir = tempfile::tempdir().unwrap();
    let design = dir.path().join("pair.solidcraft");
    std::fs::write(&design, written(&assembly_with_face_colours()).0).unwrap();
    let script = dir.path().join("looks.json");
    let list = |body: &str| json!({"command": "appearance.list", "params": {"body": body}});
    std::fs::write(
        &script,
        json!([list("unit cube (Second)"), list("unit cube")]).to_string(),
    )
    .unwrap();
    let output = dir.path().join("looks.out.json");
    let status = std::process::Command::new(cli)
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
    let results: Value = serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    let face_colours = |result: &Value| -> Vec<String> {
        result["body"]["faces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|face| face["appearance"]["color"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(face_colours(&results[0]), ["#0000ff"], "{results}");
    assert!(face_colours(&results[1]).is_empty(), "{results}");
}

fn mirrored(origin: [f64; 3], normal: [f64; 3]) -> Model {
    patterned(json!({"kind": "mirror",
        "plane_origin": {"x": origin[0], "y": origin[1], "z": origin[2]},
        "plane_normal": {"x": normal[0], "y": normal[1], "z": normal[2]}}))
}

#[test]
fn a_mirror_of_features_becomes_a_solidcraft_mirror() {
    let (document, report) = convert(
        &mirrored([20.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("3 of 3 features rebuilt"),
        "{}",
        history_line(&report)
    );
    let mirror = &document["features"][2];
    assert_eq!(mirror["type"], "mirror");
    assert_eq!(mirror["name"], "Copies");
    assert_eq!(mirror["features"], json!(["Block"]));
    let plane = &mirror["plane"]["plane"];
    assert_eq!(mirror["plane"]["type"], "custom");
    assert_eq!(plane["origin"], json!([20.0, 0.0, 0.0]));
    // The plane is spanned by two directions square to the normal.
    for key in ["x_dir", "y_dir"] {
        assert!(plane[key][0].as_f64().unwrap().abs() < 1e-12, "{plane}");
    }
}

#[test]
fn solidcraft_mirrors_the_features_across_the_plane() {
    let Some(cli) = solidcraft() else { return };
    let h = std::f64::consts::FRAC_1_SQRT_2;
    for (origin, normal, copy_min, copy_max) in [
        // Across x = 20: the copy spans 30 to 40 along X.
        (
            [20.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [30.0, 0.0, 0.0],
            [40.0, 10.0, 10.0],
        ),
        // Across x + y = 0: (x, y) goes to (-y, -x).
        (
            [0.0, 0.0, 0.0],
            [h, h, 0.0],
            [-10.0, -10.0, 0.0],
            [0.0, 0.0, 10.0],
        ),
    ] {
        let design = written(&mirrored(origin, normal)).0;
        let total: f64 = volumes(&cli, &design).iter().sum();
        assert!((total - 2000.0).abs() < 1e-6, "{total}");
        let boxes = bounds(&cli, &design);
        let near = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-6);
        assert!(
            boxes
                .iter()
                .any(|(min, max)| near(*min, copy_min) && near(*max, copy_max)),
            "{normal:?}: {boxes:?}"
        );
    }
}

/// The example cube's model with a history of one primitive feature, `definition`.
fn primitive_history(definition: Value) -> Model {
    let base = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    let model = &mut wire["model"];
    model["features"] = json!([{
        "id": "f3d:model:feature#2",
        "ordinal": 0,
        "name": "Ball",
        "suppressed": false,
        "outputs": ["synthetic:cube:body#0"],
        "definition": definition,
    }]);
    model["parameters"] = json!([]);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    }
}

fn sphere() -> Value {
    json!({"definition": "sphere", "center": {"x": 5.0, "y": 5.0, "z": 5.0}, "radius": 5.0, "op": "new_body"})
}

fn torus(axis: [f64; 3]) -> Value {
    json!({"definition": "torus", "center": {"x": 0.0, "y": 0.0, "z": 2.0},
           "axis": {"x": axis[0], "y": axis[1], "z": axis[2]},
           "major_radius": 10.0, "minor_radius": 2.0, "op": "new_body"})
}

#[test]
fn spheres_and_tori_become_solidcraft_primitives() {
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, report) = convert(&primitive_history(sphere()), &writer);
    let ball = &document["features"][0];
    assert_eq!(ball["type"], "sphere");
    assert_eq!(ball["name"], "Ball");
    assert_eq!(ball["center"], json!([5.0, 5.0, 5.0]));
    assert_eq!(ball["radius"], "5 mm");
    assert_eq!(ball["operation"], "new_body");
    assert_eq!(ball["body_names"], json!(["unit cube"]));
    assert!(
        history_line(&report).contains("1 of 1 features rebuilt"),
        "{}",
        history_line(&report)
    );

    let (document, _) = convert(&primitive_history(torus([0.0, 0.0, -1.0])), &writer);
    let ring = &document["features"][0];
    assert_eq!(ring["type"], "torus");
    assert_eq!(ring["center"], json!([0.0, 0.0, 2.0]));
    assert_eq!(
        (ring["major"].clone(), ring["minor"].clone()),
        (json!("10 mm"), json!("2 mm"))
    );
}

fn placed_primitive(solid: Value) -> Value {
    // Its frame's z axis along model +y, its origin at (1, 2, 3).
    json!({"definition": "primitive", "solid": solid,
           "placement": [[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 2.0], [0.0, -1.0, 0.0, 3.0]],
           "op": "new_body"})
}

#[test]
fn placed_cylinders_and_cones_become_solidcraft_primitives() {
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let full = std::f64::consts::TAU;
    let (document, _) = convert(
        &primitive_history(placed_primitive(
            json!({"kind": "cylinder", "radius": 2.5, "height": 26.0, "angle": full}),
        )),
        &writer,
    );
    let pin = &document["features"][0];
    assert_eq!(pin["type"], "cylinder");
    assert_eq!(pin["base"], json!([1.0, 2.0, 3.0]));
    assert_eq!(pin["axis"], json!([0.0, 1.0, 0.0]));
    assert_eq!(
        (pin["radius"].clone(), pin["height"].clone()),
        (json!("2.5 mm"), json!("26 mm"))
    );

    let cone = placed_primitive(
        json!({"kind": "cone", "radius1": 4.0, "radius2": 1.0, "height": 6.0, "angle": full}),
    );
    // Cones exist from SolidCraft 0.24.1: name that target rather than whichever SolidCraft
    // the machine running the test has.
    let with_cones = SolidCraftWriter::new()
        .verification(Verification::Off)
        .target_version(Some("0.24.1".parse().unwrap()));
    let (document, _) = convert(&primitive_history(cone.clone()), &with_cones);
    let tip = &document["features"][0];
    assert_eq!(tip["type"], "cone");
    assert_eq!(
        (
            tip["radius"].clone(),
            tip["top_radius"].clone(),
            tip["height"].clone()
        ),
        (json!("4 mm"), json!("1 mm"), json!("6 mm"))
    );

    // A cone needs SolidCraft 0.24.1; a part-turn cylinder has no SolidCraft form.
    let older = SolidCraftWriter::new()
        .verification(Verification::Off)
        .target_version(Some("0.23.0".parse().unwrap()));
    let (_, report) = convert(&primitive_history(cone), &older);
    assert!(
        history_line(&report).contains("a cone needs SolidCraft 0.24 patch 1 or newer"),
        "{}",
        history_line(&report)
    );
    let (_, report) = convert(
        &primitive_history(placed_primitive(
            json!({"kind": "cylinder", "radius": 2.5, "height": 26.0, "angle": 3.0}),
        )),
        &writer,
    );
    assert!(
        history_line(&report).contains("a part-turn cylinder is not mapped yet"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn primitives_solidcraft_cannot_place_flatten_the_history() {
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    for (definition, reason) in [
        (
            torus([1.0, 0.0, 0.0]),
            "a torus about an axis other than Z is not mapped yet",
        ),
        (
            json!({"definition": "block", "dimensions": [10.0, 10.0, 10.0],
                   "placement": [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
                   "op": "new_body"}),
            "cadmpeg does not say whether a box's placement is its corner or its centre",
        ),
    ] {
        let (_, report) = convert(&primitive_history(definition), &writer);
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn solidcraft_builds_the_primitives() {
    let Some(cli) = solidcraft() else { return };
    let pi = std::f64::consts::PI;
    for (definition, expected) in [
        (sphere(), 4.0 / 3.0 * pi * 125.0),
        (torus([0.0, 0.0, 1.0]), 2.0 * pi * pi * 10.0 * 4.0),
    ] {
        let volume = volumes(&cli, &written(&primitive_history(definition)).0);
        assert_eq!(volume.len(), 1);
        // SolidCraft measures curved solids numerically (a torus to about 1.5e-4 of the
        // formula); the history check compares SolidCraft with itself, where that cancels.
        assert!(
            (volume[0] - expected).abs() / expected < 1e-3,
            "{volume:?} vs {expected}"
        );
    }
}

/// A coil up Z from the origin, starting along `radial`: 20 mm across, of a 2 mm round
/// section, `extent` long, turning clockwise as CADIR sees it (from its base, along its axis)
/// when `clockwise`.
fn coil(extent: Value, radial: [f64; 3], clockwise: bool) -> Value {
    json!({"definition": "coil",
           "construction": {
               "placement": {"kind": "explicit",
                   "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
                   "axis": {"x": 0.0, "y": 0.0, "z": 1.0},
                   "radial": {"x": radial[0], "y": radial[1], "z": radial[2]}},
               "diameter": 20.0,
               "extent": extent,
               "section": {"kind": "circular", "diameter": 2.0},
               "section_placement": "center",
               "clockwise": clockwise,
               "taper": 0.0},
           "result": {"kind": "new_body"}})
}

fn turns_and_pitch(revolutions: f64, pitch: f64) -> Value {
    json!({"kind": "revolutions_pitch", "revolutions": revolutions, "pitch": pitch})
}

#[test]
fn coils_become_solidcraft_coils() {
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, report) = convert(
        &primitive_history(coil(turns_and_pitch(4.0, 5.0), [1.0, 0.0, 0.0], true)),
        &writer,
    );
    let spring = &document["features"][0];
    assert_eq!(spring["type"], "coil");
    assert_eq!(spring["base"], json!([0.0, 0.0, 0.0]));
    assert_eq!(spring["axis"], json!([0.0, 0.0, 1.0]));
    assert_eq!(
        [&spring["diameter"], &spring["turns"], &spring["pitch"]],
        [&json!("20 mm"), &json!("4"), &json!("5 mm")]
    );
    assert_eq!(
        (&spring["section"], &spring["section_size"]),
        (&json!("circular"), &json!("2 mm"))
    );
    // Clockwise seen from the base is anticlockwise seen from above: SolidCraft's default.
    assert_eq!(spring.get("clockwise"), None);
    assert_eq!(spring.get("start_angle"), None);
    assert_eq!(spring.get("position"), None);
    assert!(
        history_line(&report).contains("1 of 1 features rebuilt"),
        "{}",
        history_line(&report)
    );

    let mut other = coil(
        json!({"kind": "revolutions_height", "revolutions": 2.0, "height": 8.0}),
        [0.0, 1.0, 0.0],
        false,
    );
    other["construction"]["section"] = json!({"kind": "square", "size": 1.5});
    other["construction"]["section_placement"] = json!("inside");
    other["result"] = json!({"kind": "boolean", "operation": "join", "targets": {"kind": "bodies", "value": ["synthetic:cube:body#0"]}});
    let (document, _) = convert(&primitive_history(other), &writer);
    let spring = &document["features"][0];
    assert_eq!(spring["clockwise"], true);
    assert_eq!(spring["start_angle"], "90 deg");
    assert_eq!(spring["position"], -1);
    assert_eq!(spring["pitch"], "(8 mm) / (2)");
    assert_eq!(spring["section"], "square");
    assert_eq!(spring["operation"], "join");
}

#[test]
fn coils_map_signed_extents_and_reverse_the_axis_and_turning_flag_together() {
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    for sign in [1.0, -1.0] {
        for (extent, turns, pitch) in [
            (turns_and_pitch(2.0, sign * 4.0), "2", "4 mm"),
            (
                json!({"kind": "revolutions_height", "revolutions": 2.0, "height": sign * 8.0}),
                "2",
                "(8 mm) / (2)",
            ),
            (
                json!({"kind": "height_pitch", "height": sign * 8.0, "pitch": sign * 4.0}),
                "(8 mm) / (4 mm)",
                "4 mm",
            ),
        ] {
            for clockwise in [true, false] {
                let mut definition = coil(extent.clone(), [0.0, 1.0, 0.0], clockwise);
                definition["construction"]["placement"]["axis"] =
                    json!({"x": 1.0, "y": 0.0, "z": 0.0});
                let (document, report) = convert(&primitive_history(definition), &writer);
                let spring = &document["features"][0];
                assert_eq!(spring["type"], "coil", "{}", history_line(&report));
                assert_eq!(spring["axis"], json!([sign, 0.0, 0.0]));
                assert_eq!(spring["start_angle"], "-90 deg");
                assert_eq!(spring["turns"], turns);
                assert_eq!(spring["pitch"], pitch);
                assert_eq!(
                    spring.get("clockwise"),
                    (clockwise == (sign < 0.0)).then_some(&json!(true))
                );
            }
        }
    }
}

#[test]
fn coils_solidcraft_cannot_build_flatten_the_history() {
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let plain = || coil(turns_and_pitch(4.0, 5.0), [1.0, 0.0, 0.0], true);
    let with = |path: &[&str], value: Value| {
        let mut definition = plain();
        let mut at = &mut definition;
        for key in path {
            at = &mut at[*key];
        }
        *at = value;
        definition
    };
    for (definition, reason) in [
        (
            with(&["construction", "taper"], json!(0.1)),
            "a tapered coil is not mapped yet: SolidCraft's coil has no taper option",
        ),
        (
            with(&["construction", "taper"], json!(-0.1)),
            "a tapered coil is not mapped yet: SolidCraft's coil has no taper option",
        ),
        (
            with(
                &["construction", "extent"],
                json!({"kind": "spiral", "revolutions": 3.0, "radial_pitch": 2.0}),
            ),
            "a flat spiral coil is not mapped yet: SolidCraft's coil has no spiral mode",
        ),
        (
            with(
                &["construction", "extent"],
                json!({"kind": "spiral", "revolutions": 3.0, "radial_pitch": -2.0}),
            ),
            "a flat spiral coil is not mapped yet: SolidCraft's coil has no spiral mode",
        ),
        (
            with(
                &["construction", "extent"],
                json!({"kind": "height_pitch", "height": 8.0, "pitch": -4.0}),
            ),
            "its extent is malformed",
        ),
        (
            with(
                &["construction", "section"],
                json!({"kind": "external_triangle", "size": 2.0}),
            ),
            "a coil of triangular section is not mapped yet: SolidCraft's coil only has \
             circular and square sections",
        ),
        (
            with(
                &["construction", "section"],
                json!({"kind": "internal_triangle", "size": 2.0}),
            ),
            "a coil of triangular section is not mapped yet: SolidCraft's coil only has \
             circular and square sections",
        ),
    ] {
        let (document, report) = convert(&primitive_history(definition), &writer);
        assert_reference_sketches(&document, 1);
        let outcome = report.outcome().unwrap();
        assert_eq!(outcome.mode, Mode::Import);
        assert!(matches!(outcome.check, Check::Flattened(_)));
        assert_eq!(outcome.features.len(), 1);
        assert_eq!(outcome.features[0].kind, "coil");
        assert_eq!(outcome.features[0].reason.as_deref(), Some(reason));
        assert_eq!(
            outcome.features[0].status,
            dutchman_core::FeatureStatus::Blocked
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn solidcraft_turns_the_coil_the_way_cadir_says() {
    let Some(cli) = solidcraft() else { return };
    // Quarter turns expose the start direction and turning sense; a full turn would hide
    // both. Include SolidCraft's alternate start frame close to X, and its usual projected-X
    // frame on either side of that threshold. CADIR requires unit placement vectors.
    for (axis, radial) in [
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
        ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
        ([1.0, 2.0, 3.0], [2.0, -1.0, 0.0]),
        ([1.0, 0.0, 0.0000005], [0.0, 1.0, 0.0]),
        ([1.0, 0.0, 0.000002], [0.0, 1.0, 0.0]),
    ] {
        let normalize = |v: [f64; 3]| {
            let length = v.iter().map(|c| c * c).sum::<f64>().sqrt();
            v.map(|c| c / length)
        };
        let axis = normalize(axis);
        let radial = normalize(radial);
        for clockwise in [true, false] {
            for pitch in [4.0, -4.0] {
                let mut definition = coil(turns_and_pitch(0.25, pitch), radial, clockwise);
                definition["construction"]["placement"]["axis"] =
                    json!({"x": axis[0], "y": axis[1], "z": axis[2]});
                let base = [3.0, -2.0, 5.0];
                definition["construction"]["placement"]["origin"] =
                    json!({"x": base[0], "y": base[1], "z": base[2]});
                let design = written(&primitive_history(definition)).0;
                let boxes = bounds(&cli, &design);
                assert_eq!(boxes.len(), 1, "{boxes:?}");
                let (min, max) = boxes[0];
                // Sample the CADIR helix directly, without using SolidCraft's start-angle frame.
                // Its section is a unit circle in the radial/axis plane at each point.
                let tangent = [
                    axis[1] * radial[2] - axis[2] * radial[1],
                    axis[2] * radial[0] - axis[0] * radial[2],
                    axis[0] * radial[1] - axis[1] * radial[0],
                ];
                let mut expected_min = [f64::INFINITY; 3];
                let mut expected_max = [f64::NEG_INFINITY; 3];
                for step in 0..=1000 {
                    let turns = 0.25 * f64::from(step) / 1000.0;
                    let angle = std::f64::consts::TAU * turns * if clockwise { 1.0 } else { -1.0 };
                    for i in 0..3 {
                        let r = radial[i] * angle.cos() + tangent[i] * angle.sin();
                        let center = base[i] + 10.0 * r + axis[i] * pitch * turns;
                        let half_width = r.hypot(axis[i]);
                        expected_min[i] = expected_min[i].min(center - half_width);
                        expected_max[i] = expected_max[i].max(center + half_width);
                    }
                }
                let case = format!("axis={axis:?}, clockwise={clockwise}, pitch={pitch}");
                for i in 0..3 {
                    assert!(
                        (min[i] - expected_min[i]).abs() < 0.05
                            && (max[i] - expected_max[i]).abs() < 0.05,
                        "{case}: {boxes:?} vs {expected_min:?}, {expected_max:?}"
                    );
                }
                let volume = volumes(&cli, &design);
                assert_eq!(volume.len(), 1);
                // The section's area along the helix: π × 1² × √((π × 20 / 4)² + 1²).
                let pi = std::f64::consts::PI;
                let expected = pi * ((pi * 5.0).powi(2) + 1.0).sqrt();
                assert!(
                    (volume[0] - expected).abs() / expected < 1e-2,
                    "{volume:?} vs {expected}"
                );
            }
        }
    }
}

#[test]
fn a_helix_surface_is_written_to_the_step_and_needs_no_note() {
    let base = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    let model = &mut wire["model"];
    // One of the cube's faces on a coil's swept surface, as Fusion stores a coil's.
    let surface = model["faces"][0]["surface"].clone();
    for row in model["surfaces"].as_array_mut().unwrap() {
        if row["id"] == surface {
            row["geometry"] = json!({"kind": "procedural", "construction": "synthetic:cube:procedural_surface#0"});
        }
    }
    model["procedural_surfaces"] = json!([{
        "id": "synthetic:cube:procedural_surface#0",
        "surface": surface,
        "definition": {"kind": "helix", "construction": {
            "angle_range": [-std::f64::consts::PI, std::f64::consts::PI],
            "dimension_range": [0.0, 250.0],
            "path": {"angle_range": [0.0, 25.0],
                     "center": {"x": 0.0, "y": 0.0, "z": 0.0},
                     "major": {"x": 10.0, "y": 0.0, "z": 0.0},
                     "minor": {"x": 0.0, "y": 10.0, "z": 0.0},
                     "pitch": {"x": 0.0, "y": 0.0, "z": 5.0},
                     "apex_factor": 0.0,
                     "axis": {"x": 0.0, "y": 0.0, "z": 1.0}},
            "profile": {"kind": "circle", "angle": 0.0, "radius": 1.0}}},
        "record_bounds": [null, null, null, null],
    }]);
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    };
    let (_, report) = convert(&model, &SolidCraftWriter::new().import_only(true));
    // cadmpeg writes an evaluable helix surface as a sampled B-spline surface, so nothing is
    // left out and the report names nothing.
    assert!(
        report.losses().iter().all(|loss| loss.subject
            != "step/geometry.procedural-definition-not-written"
            && !loss.detail.contains("has no form cadmpeg writes")),
        "{report:?}"
    );
    assert!(
        report
            .losses()
            .iter()
            .any(|loss| loss.subject == "step/geometry.helix-fitted-to-bspline"),
        "{report:?}"
    );
}

/// The holed cube's history with its sketch and extrude replaced by a base feature holding
/// the cube's body as it is, as Fusion holds a body modelled directly or imported.
fn based_cube() -> Model {
    let mut model = holed_cube(4.0, |_| {});
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    let body = m["bodies"][0]["id"].clone();
    let features = m["features"].as_array_mut().unwrap();
    features.retain(|feature| feature["ordinal"].as_u64() >= Some(2));
    features.push(json!({
        "id": "f3d:model:feature#1",
        "ordinal": 0,
        "name": "Base Feature 1",
        "suppressed": false,
        "definition": {"definition": "base_feature",
            "bodies": {"kind": "resolved", "value": {"bodies": [body], "native": "f3d:base#1"}}},
    }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

/// Synthetic stored geometry uses the same cube body as the base-feature tests.
fn stored_cube(with_hole: bool) -> Model {
    let mut model = based_cube();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let body = wire["model"]["bodies"][0]["id"].clone();
    let features = wire["model"]["features"].as_array_mut().unwrap();
    if !with_hole {
        features.retain(|feature| feature["ordinal"] == 0);
    }
    let base = features.iter_mut().find(|f| f["ordinal"] == 0).unwrap();
    base["definition"] = json!({"definition": "stored_geometry"});
    base["outputs"] = json!([body]);
    base["name"] = json!("Stored solid");
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

const STORED_SOLID_MESSAGE: &str =
    "This part holds an imported solid; there is no feature history to bring over";

#[test]
fn stored_geometry_alone_becomes_an_import_without_lost_history() {
    let model = stored_cube(false);
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, report) = convert(&model, &writer);
    assert_eq!(
        types(&document),
        [
            ("import".to_owned(), false),
            ("sketch".to_owned(), false),
            ("sketch".to_owned(), false),
        ]
    );
    let base = &document["features"][0];
    assert_eq!(base["name"], "Stored solid");
    assert!(base.get("_step").is_none() && base.get("_bodies").is_none());
    let (bytes, _) = written_with(&model, &writer);
    let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut step = String::new();
    archive
        .by_name(base["step_blob"].as_str().unwrap())
        .unwrap()
        .read_to_string(&mut step)
        .unwrap();
    assert!(step.contains("MANIFOLD_SOLID_BREP"), "{step}");
    let outcome = report.outcome().unwrap();
    assert_eq!(outcome.mode, Mode::Import);
    assert_eq!(
        outcome.check,
        Check::NotApplicable(STORED_SOLID_MESSAGE.to_owned())
    );
    assert_eq!(outcome.features.len(), 1);
    assert_eq!(
        outcome.features[0].status,
        dutchman_core::FeatureStatus::Mapped
    );
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.severity == Severity::Info && loss.detail == STORED_SOLID_MESSAGE));
    assert!(report
        .losses()
        .iter()
        .all(|loss| !loss.detail.contains("not mapped yet") && !loss.detail.contains("flattened")));
    let mut planned = Report::default();
    assert_eq!(writer.plan(&model, &mut planned), *outcome);
    assert!(planned
        .losses()
        .iter()
        .any(|loss| loss.detail == STORED_SOLID_MESSAGE));
}

#[test]
fn stored_geometry_without_bodies_is_refused_specifically() {
    let mut model = stored_cube(false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][0]["outputs"] = json!([]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).contains("it holds no bodies"));
    assert!(!history_line(&report).contains("not mapped yet"));
    assert!(report
        .losses()
        .iter()
        .all(|loss| loss.detail != STORED_SOLID_MESSAGE));
}

#[test]
fn stored_geometry_keeps_the_features_after_it() {
    let (document, report) = convert(
        &stored_cube(true),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(
        types(&document),
        [
            ("import".to_owned(), false),
            ("sketch".to_owned(), false),
            ("hole".to_owned(), false),
            ("sketch".to_owned(), false),
            ("import".to_owned(), true),
        ]
    );
    assert!(document["features"][0]["step_blob"].as_str().is_some());
    assert_eq!(report.outcome().unwrap().mode, Mode::History);
    assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.severity == Severity::Info && loss.detail == STORED_SOLID_MESSAGE));
    assert!(report
        .losses()
        .iter()
        .all(|loss| !loss.detail.contains("not mapped yet") && !loss.detail.contains("flattened")));
}

#[test]
fn a_base_feature_is_imported_where_it_stands_in_the_timeline() {
    let (document, report) = convert(
        &based_cube(),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("3 of 3 features rebuilt"),
        "{}",
        history_line(&report)
    );
    let base = &document["features"][0];
    assert_eq!(base["type"], "import");
    assert_eq!(base["name"], "Base Feature 1");
    assert!(base["step_blob"]
        .as_str()
        .is_some_and(|blob| blob.starts_with("blobs/")));
    assert!(base.get("_step").is_none() && base.get("_bodies").is_none());
    // The hole after it keeps its history; the original geometry still ends the timeline.
    assert_eq!(
        types(&document),
        [
            ("import".to_owned(), false),
            ("sketch".to_owned(), false),
            ("hole".to_owned(), false),
            ("sketch".to_owned(), false),
            ("import".to_owned(), true),
        ]
    );
}

#[test]
fn a_base_feature_proven_to_hold_no_bodies_builds_nothing() {
    // A second base feature the source proves empty, after the one holding the cube.
    let mut model = based_cube();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "f3d:model:feature#9",
            "ordinal": 9,
            "name": "Base Feature 2",
            "suppressed": false,
            "definition": {"definition": "base_feature",
                "bodies": {"kind": "resolved", "value": {"bodies": [], "native": "f3d:base#2"}}},
        }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("3 of 3 features rebuilt"),
        "{}",
        history_line(&report)
    );
    let outcome = report.outcome().unwrap();
    let empty = outcome
        .features
        .iter()
        .find(|f| f.name.as_deref() == Some("Base Feature 2"))
        .unwrap();
    assert_eq!(empty.status, dutchman_core::FeatureStatus::NotNeeded);
    assert_eq!(
        types(&document)
            .iter()
            .filter(|(kind, _)| kind == "import")
            .count(),
        2,
        "the base body and the original geometry only"
    );
}

#[test]
fn a_base_feature_holding_every_body_writes_no_second_copy() {
    let mut model = based_cube();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .retain(|feature| feature["ordinal"] == 0);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, _) = convert(&model, &writer);
    assert_eq!(types(&document)[0], ("import".to_owned(), false));
    let features = document["features"].as_array().unwrap();
    assert!(features
        .iter()
        .all(|f| f["name"] != "Original geometry (imported)"));
    let (bytes, _) = written_with(&model, &writer);
    let archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let blobs = archive
        .file_names()
        .filter(|name| name.as_ref().is_ok_and(|n| n.starts_with("blobs/")));
    assert_eq!(blobs.count(), 1, "only the base feature's STEP");
}

#[test]
fn solidcraft_drills_into_the_imported_base_body() {
    let Some(cli) = solidcraft() else { return };
    let bore = std::f64::consts::PI * 2.0_f64.powi(2);
    let got = volumes(&cli, &written(&based_cube()).0);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!((got[0] - (1000.0 - bore * 10.0)).abs() < 1.0, "{got:?}");
}

#[test]
fn a_base_feature_alone_is_verified_against_the_original() {
    let Some(cli) = solidcraft() else { return };
    let mut model = based_cube();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .retain(|feature| feature["ordinal"] == 0);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::With(cli)),
    );
    assert_eq!(
        history_line(&report),
        "1 of 1 features rebuilt; verified against the original in SolidCraft"
    );
}

/// The example cube's history with its extrude replaced by a sheet-metal base flange of the
/// same square, `thickness` thick on `side` of the sketch.
fn flanged_cube(thickness: f64, side: &str) -> Model {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    for feature in wire["model"]["features"].as_array_mut().unwrap() {
        if feature["definition"]["definition"] == "extrude" {
            feature["name"] = json!("BaseFlange 1");
            feature["definition"] = json!({
                "definition": "sheet_metal_base_flange",
                "profile": whole_sketch(),
                "thickness": thickness,
                "side": side,
            });
        }
    }
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn a_base_flange_becomes_a_solidcraft_sheet_under_a_rule_of_its_thickness() {
    let (document, report) = convert(
        &flanged_cube(10.0, "forward"),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("2 of 2 features rebuilt"),
        "{}",
        history_line(&report)
    );
    let flange = &document["features"][1];
    assert_eq!(flange["type"], "sheet_base");
    assert_eq!(flange["rule"], "Sheet 10 mm");
    assert_eq!(flange["profiles"], json!({"type": "all"}));
    assert!(flange.get("flip").is_none());
    assert_eq!(document["sheet"]["active"], "Sheet 10 mm");
    assert_eq!(document["sheet"]["rules"][0]["thickness"], "10 mm");
    // The other side of the sketch flips it; both sides at once is not mapped yet.
    let (document, _) = convert(
        &flanged_cube(10.0, "reverse"),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(document["features"][1]["flip"], true);
    let (_, report) = convert(
        &flanged_cube(10.0, "symmetric"),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains("thick on both sides of its sketch is not mapped yet"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn solidcraft_builds_the_base_flange_as_the_original() {
    let Some(cli) = solidcraft() else { return };
    let (_, report) = convert(
        &flanged_cube(10.0, "forward"),
        &SolidCraftWriter::new().verification(Verification::With(cli)),
    );
    assert_eq!(
        history_line(&report),
        "2 of 2 features rebuilt; verified against the original in SolidCraft"
    );
}

/// A base sheet followed by a synthetic operation on its front top edge.
fn sheet_operation(mut definition: Value) -> Model {
    let mut model = flanged_cube(10.0, "forward");
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    definition["edges"] = json!({"kind": "edges", "value": ["synthetic:cube:edge#4"]});
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "f3d:model:feature#3", "ordinal": 2, "name": "Sheet wall",
            "suppressed": false, "definition": definition,
        }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn edge_flanges_refuse_final_body_edge_selections() {
    // The selected edge exists only on the final body, where it has become the bend.
    let model = sheet_operation(json!({
        "definition": "sheet_metal_edge_flange",
        "height": {"kind": "distance", "value": 30.0},
        "angle": std::f64::consts::FRAC_PI_2,
        "height_datum": "outer_faces",
        "bend_position": "outside",
        "width": {"kind": "full_edge"},
        "bend_radius": 2.0,
    }));
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains("needs this feature's input edge geometry"),
        "{}",
        history_line(&report)
    );
}

/// The final body is deliberately still the original cube. The two symmetric input edges
/// have their own geometry, unrelated to final-body edge numbering.
fn input_edge_flange(back: bool, position: &str, angle: f64, radius: f64) -> Model {
    let mut model = flanged_cube(1.0, "forward");
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let state = "test:sheet:feature-input#wall";
    let edges = [
        "test:sheet:historical-edge#front",
        "test:sheet:historical-edge#back",
    ];
    let faces = [
        "test:sheet:historical-face#top",
        "test:sheet:historical-face#rim",
    ];
    let point = |x, y, z| json!({"x": x, "y": y, "z": z});
    let geometry = [0, 1].map(|i| {
        json!({
            "edge": edges[i], "interior": point(5.0, 10.0 * i as f64, 1.0),
            "start": point(0.0, 10.0 * i as f64, 1.0),
            "end": point(10.0, 10.0 * i as f64, 1.0),
            "tangent": point(1.0, 0.0, 0.0), "faces": faces,
        })
    });
    wire["model"]["feature_input_topologies"] = json!([{
        "id": state, "input_of": "f3d:model:feature#3", "edges": edges, "faces": faces,
        "edge_geometry": geometry,
    }]);
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "f3d:model:feature#3", "ordinal": 2, "name": "Sheet wall",
            "suppressed": false, "definition": {
                "definition": "sheet_metal_edge_flange",
                "edges": {"kind": "historical", "value": {
                    "state": state, "edges": [edges[usize::from(back)]], "native": "synthetic edge",
                }},
                "height": {"kind": "distance", "value": 30.0},
                "angle": angle, "height_datum": "outer_faces", "bend_position": position,
                "width": {"kind": "full_edge"}, "bend_radius": radius,
            },
        }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn edge_flanges_pick_their_own_input_edges_and_keep_dimensions() {
    for back in [false, true] {
        for position in ["inside", "outside"] {
            let (document, report) = convert(
                &input_edge_flange(back, position, std::f64::consts::FRAC_PI_2, 2.0),
                &SolidCraftWriter::new().verification(Verification::Off),
            );
            assert!(
                history_line(&report).starts_with("3 of 3 features rebuilt"),
                "{}",
                history_line(&report)
            );
            let flange = &document["features"][2];
            assert_eq!(flange["type"], "sheet_flange");
            assert_eq!(
                flange["edges"],
                json!([[5.0, if back { 10.0 } else { 0.0 }, 1.0]])
            );
            assert_eq!(flange["height"], "30 mm");
            assert_eq!(flange["angle"], "90 deg");
            assert_eq!(flange["radius"], "2 mm");
            assert_eq!(flange["position"], position);
            assert_eq!(
                document["sheet"]["rules"][0]["corner_relief_shape"],
                "notch"
            );
            assert_eq!(document["sheet"]["rules"][0]["relief_width"], "Thickness");
        }
    }
}

#[test]
fn edge_flanges_refuse_unproven_picks_and_unexpressible_options() {
    let mutations = [
        (
            "/model/feature_input_topologies/0/input_of",
            json!("f3d:model:feature#2"),
            "does not belong",
        ),
        (
            "/model/feature_input_topologies/0/edge_geometry",
            json!([]),
            "input topology has no edge geometry",
        ),
        (
            "/model/feature_input_topologies/0/edges",
            json!([]),
            "input topology has no edges",
        ),
        (
            "/model/feature_input_topologies/0/edge_geometry/0/interior",
            json!({"x": 5.0, "y": 5.0, "z": 1.0}),
            "exactly one rebuilt edge",
        ),
        (
            "/model/feature_input_topologies/0/edge_geometry/0/interior",
            json!({"x": 0.0, "y": 0.0, "z": 1.0}),
            "exactly one rebuilt edge",
        ),
        (
            "/model/feature_input_topologies/0/edge_geometry/0/end",
            json!({"x": 9.0, "y": 0.0, "z": 1.0}),
            "vertices or tangent",
        ),
        (
            "/model/feature_input_topologies/0/edge_geometry/0/tangent",
            json!({"x": 0.0, "y": 1.0, "z": 0.0}),
            "vertices or tangent",
        ),
        (
            "/model/features/2/definition/height_datum",
            json!("inner_faces"),
            "inner faces",
        ),
        (
            "/model/features/2/definition/bend_position",
            json!("adjacent"),
            "bend position",
        ),
        (
            "/model/features/2/definition/bend_position",
            json!("tangent_to_side"),
            "bend position",
        ),
        (
            "/model/features/2/definition/width",
            json!({"kind": "symmetric", "value": {"width": 5.0}}),
            "part of its edge",
        ),
        (
            "/model/features/2/definition/height",
            json!({"kind": "to_object", "value": {
                "target": {"kind": "native", "value": "synthetic"}, "offset": 0.0,
            }}),
            "ending at an object",
        ),
    ];
    for (path, value, reason) in mutations {
        let mut model = input_edge_flange(false, "outside", std::f64::consts::FRAC_PI_2, 2.0);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        *wire.pointer_mut(path).unwrap() = value;
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{path}: {}",
            history_line(&report)
        );
    }
}

#[test]
fn edge_flanges_refuse_bases_without_a_proven_rectangular_body() {
    for extra_body in [false, true] {
        let mut model = input_edge_flange(false, "outside", std::f64::consts::FRAC_PI_2, 2.0);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        if extra_body {
            let mut other = wire["model"]["features"][1].clone();
            other["id"] = json!("f3d:model:feature#4");
            other["ordinal"] = json!(2);
            other["name"] = json!("Another sheet");
            wire["model"]["features"][2]["ordinal"] = json!(3);
            wire["model"]["features"]
                .as_array_mut()
                .unwrap()
                .push(other);
        } else {
            // The base now has an oblique side; a bounding rectangle alone is not proof.
            wire["model"]["sketch_entities"][0]["geometry"]["end"]["u"] = json!(9.0);
            wire["model"]["sketch_entities"][1]["geometry"]["start"]["u"] = json!(9.0);
        }
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let reason = if extra_body {
            "a base sheet that earlier features changed"
        } else {
            "a base sheet that is not one plain rectangle"
        };
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

/// SolidCraft's complete measurements, used to distinguish symmetric but misplaced flanges.
fn sheet_measurements(cli: &Path, input: &Path, output: &Path) -> Value {
    let status = std::process::Command::new(cli)
        .arg("eval")
        .arg(input)
        .arg("--json-out")
        .arg(output)
        .status()
        .unwrap();
    assert!(status.success());
    let result: Value = serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    for entry in result["timeline"].as_array().into_iter().flatten() {
        assert!(entry["error"].is_null(), "{entry}");
    }
    assert_eq!(result["bodies"].as_array().unwrap().len(), 1, "{result}");
    result["bodies"][0].clone()
}

#[test]
fn solidcraft_replays_input_edge_flanges_with_the_expected_shape() {
    let Some(cli) = solidcraft() else { return };
    for (back, position, degrees, radius, both) in [
        (false, "outside", 90.0_f64, 2.0, false),
        (true, "outside", 90.0, 2.0, false),
        (false, "inside", 60.0, 1.5, false),
        (true, "inside", 120.0, 3.0, false),
        (false, "outside", 90.0, 2.0, true),
    ] {
        let mut model = input_edge_flange(back, position, degrees.to_radians(), radius);
        let mut points = vec![[5.0, if back { 10.0 } else { 0.0 }, 1.0]];
        if both {
            let mut wire = serde_json::to_value(&model.ir).unwrap();
            wire["model"]["features"][2]["definition"]["edges"]["value"]["edges"] = json!([
                "test:sheet:historical-edge#front",
                "test:sheet:historical-edge#back",
            ]);
            model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
            points.push([5.0, 10.0, 1.0]);
            let (document, report) = convert(
                &model,
                &SolidCraftWriter::new().verification(Verification::Off),
            );
            assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
            assert_eq!(document["features"][2]["edges"], json!(points));
        }
        let dir = tempfile::tempdir().unwrap();
        let design = dir.path().join("mapped.solidcraft");
        std::fs::write(&design, written(&model).0).unwrap();
        // Build the expected body independently with documented commands and an explicit
        // point on the intended edge. In particular the back case must grow toward +Y.
        let reference = json!([
            {"command": "sketch.create", "params": {"plane": "XY", "name": "Base"}},
            {"command": "sketch.rectangle.two_point", "params": {"p0": [0, 0], "p1": [10, 10]}},
            {"command": "sketch.finish", "params": {}},
            {"command": "sheet.manage_rules", "params": {"name": "Sheet 1 mm", "thickness": "1 mm", "active": true}},
            {"command": "sheet.flange", "params": {"sketch": "Base"}},
            {"command": "sheet.flange", "params": {
                "edges": points, "height": "30 mm",
                "angle": format!("{degrees} deg"), "radius": format!("{radius} mm"), "position": position,
            }},
        ]);
        let script = dir.path().join("expected.json");
        std::fs::write(&script, serde_json::to_vec(&reference).unwrap()).unwrap();
        let actual = sheet_measurements(&cli, &design, &dir.path().join("actual.json"));
        let expected = sheet_measurements(&cli, &script, &dir.path().join("reference.json"));
        let volume = expected["volume_mm3"].as_f64().unwrap();
        assert!((actual["volume_mm3"].as_f64().unwrap() - volume).abs() / volume < 1e-4);
        assert_eq!(actual["faces"], expected["faces"]);
        for corner in ["min", "max"] {
            for axis in 0..3 {
                let actual_at = actual["bbox"][corner][axis].as_f64().unwrap();
                let expected_at = expected["bbox"][corner][axis].as_f64().unwrap();
                assert!(
                    (actual_at - expected_at).abs() < 1e-6,
                    "{back} {position}: {actual} != {expected}"
                );
            }
        }
        if !both && degrees == 90.0 && position == "outside" {
            let corner = if back { "max" } else { "min" };
            let y = if back { 13.0 } else { -3.0 };
            assert!((actual["bbox"][corner][1].as_f64().unwrap() - y).abs() < 1e-6);
            assert_eq!(actual["faces"], 14);
        }
    }
}

#[test]
fn hems_explain_the_form_or_dimension_that_cannot_be_expressed() {
    for (form, direction, reason) in [
        (
            json!({"kind": "flat", "value": {"length": 20.0}}),
            "forward",
            "developed length",
        ),
        (
            json!({"kind": "open", "value": {"gap": 2.0, "length": 20.0}}),
            "reverse",
            "developed length",
        ),
        (
            json!({"kind": "gap_length", "value": {"gap": 2.0, "length": 20.0}}),
            "forward",
            "unresolved flat-or-open",
        ),
        (
            json!({"kind": "rolled", "value": {"radius": 2.0, "angle": 1.0}}),
            "forward",
            "rolled hem",
        ),
        (
            json!({"kind": "teardrop", "value": {"gap": 2.0, "length": 20.0, "radius": 2.0}}),
            "forward",
            "teardrop hem",
        ),
        (
            json!({"kind": "flat", "value": {"length": 20.0}}),
            "unresolved",
            "unresolved fold direction",
        ),
    ] {
        let model = sheet_operation(json!({
            "definition": "sheet_metal_hem", "bend_position": "adjacent",
            "form": form, "direction": direction, "bend_radius": 2.0,
        }));
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

const PLATE: &str = "f3d:model:sketch#41";

/// A 1 mm base sheet, then a plate sketched square to its front edge from `near` to 20 mm
/// up, attached to that edge, then the plate's bend of `radius`. `extra` adds a line to the
/// plate sketch, which makes it more than one rectangle.
fn attached_plate(near: f64, radius: f64, extra: bool) -> Model {
    let mut model = flanged_cube(1.0, "forward");
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    let point = |x, y, z| json!({"x": x, "y": y, "z": z});
    let edges = [
        "test:sheet:historical-edge#front",
        "test:sheet:historical-edge#back",
    ];
    let faces = [
        "test:sheet:historical-face#top",
        "test:sheet:historical-face#rim",
    ];
    let geometry = [0, 1].map(|i| {
        json!({
            "edge": edges[i], "interior": point(5.0, 10.0 * i as f64, 1.0),
            "start": point(0.0, 10.0 * i as f64, 1.0),
            "end": point(10.0, 10.0 * i as f64, 1.0),
            "tangent": point(1.0, 0.0, 0.0), "faces": faces,
        })
    });
    let state = |name: &str, owner: &str| {
        json!({"id": format!("test:sheet:feature-input#{name}"), "input_of": owner,
               "edges": edges, "faces": faces, "edge_geometry": geometry})
    };
    m["feature_input_topologies"] = json!([
        state("plate", "f3d:model:feature#4"),
        state("bend", "f3d:model:feature#5"),
    ]);
    let attachment = |name: &str| {
        json!({"kind": "historical", "value": {
            "state": format!("test:sheet:feature-input#{name}"),
            "edges": [edges[0]], "native": "synthetic edge",
        }})
    };
    m["sketches"].as_array_mut().unwrap().push(json!({
        "id": PLATE, "name": "Plate",
        "placement": {"kind": "resolved", "frame": {
            "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
            "normal": {"x": 0.0, "y": -1.0, "z": 0.0},
            "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0},
        }},
        "profiles": [],
    }));
    let (lo, hi) = (near, 20.0);
    let mut lines = vec![
        line(41, [0.0, lo], [10.0, lo]),
        line(42, [10.0, lo], [10.0, hi]),
        line(43, [10.0, hi], [0.0, hi]),
        line(44, [0.0, hi], [0.0, lo]),
    ];
    if extra {
        lines.push(line(45, [0.0, lo], [5.0, hi]));
    }
    for mut l in lines {
        l["sketch"] = json!(PLATE);
        m["sketch_entities"].as_array_mut().unwrap().push(l);
    }
    let features = m["features"].as_array_mut().unwrap();
    features.push(json!({
        "id": "f3d:model:feature#3", "ordinal": 2, "name": "Plate sketch", "suppressed": false,
        "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": PLATE}},
    }));
    features.push(json!({
        "id": "f3d:model:feature#4", "ordinal": 3, "name": "Plate", "suppressed": false,
        "definition": {
            "definition": "sheet_metal_attached_plate",
            "profile": {"kind": "sketch", "value": PLATE},
            "thickness": 1.0, "side": "forward", "attachment": attachment("plate"),
        },
    }));
    features.push(json!({
        "id": "f3d:model:feature#5", "ordinal": 4, "name": "Bend", "suppressed": false,
        "definition": {
            "definition": "sheet_metal_bend", "plate": "f3d:model:feature#4",
            "attachment": attachment("bend"), "inside_radius": radius,
            "relief": {"form": "style_default", "width": 1.0, "depth": 0.5,
                       "minimum_remnant": 0.0, "seam_gap": 0.0},
            "transition": {"form": "style_default", "radius": 0.0},
        },
    }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn an_attached_plate_and_its_bend_become_one_edge_flange() {
    let (document, report) = convert(
        &attached_plate(3.0, 2.0, false),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("4 of 4 features rebuilt"),
        "{}",
        history_line(&report)
    );
    let flanges: Vec<_> = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["type"] == "sheet_flange")
        .collect();
    assert_eq!(flanges.len(), 1);
    let flange = flanges[0];
    assert_eq!(flange["edges"], json!([[5.0, 0.0, 1.0]]));
    assert_eq!(flange["height"], "20 mm");
    assert_eq!(flange["angle"], "90 deg");
    assert_eq!(flange["radius"], "2 mm");
    assert_eq!(flange["position"], "inside");
}

#[test]
fn an_attached_plate_that_is_not_a_plain_edge_flange_is_refused() {
    for (model, reason) in [
        (attached_plate(3.0, 2.0, true), "not one rectangle"),
        (
            attached_plate(4.0, 2.0, false),
            "does not start where its 90 degree bend ends",
        ),
    ] {
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let line = history_line(&report);
        assert!(line.contains(reason), "{line}");
    }
}

/// The example cube under a timeline of `features`, each `(name, definition)`.
fn cube_under(features: &[(&str, Value)]) -> Model {
    let ir = cadmpeg_ir::examples::unit_cube().unwrap();
    let mut wire = serde_json::to_value(&ir).unwrap();
    wire["model"]["features"] = features
        .iter()
        .enumerate()
        .map(|(i, (name, definition))| {
            json!({"id": format!("inventor:model:feature#{i}"), "ordinal": i, "name": name,
                   "suppressed": false, "definition": definition})
        })
        .collect();
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        fidelity: None,
        name: "cube".to_owned(),
        source_file: Some("cube.ipt".to_owned()),
        source_step: None,
        assembly: None,
    }
}

#[test]
fn a_design_cadmpeg_decodes_no_feature_of_has_no_history_to_flatten() {
    // An Inventor part made of a base solid: its one feature is kept in its native form.
    let native = json!({"definition": "native", "kind": "InventorClass:0843c7c2"});
    let model = cube_under(&[("Base1", native.clone())]);
    let mut report = Report::default();
    let outcome = SolidCraftWriter::new().plan(&model, &mut report);
    assert_eq!(outcome.mode, Mode::Import);
    assert_eq!(
        outcome.check,
        Check::NotApplicable(
            "no feature history cadmpeg decodes: its one feature is kept in its native form"
                .to_owned()
        )
    );
    let model = cube_under(&[("Base1", native.clone()), ("Composite1", native.clone())]);
    let mut report = Report::default();
    SolidCraftWriter::new()
        .verification(Verification::Off)
        .write(&model, &mut Vec::new(), &mut report)
        .unwrap();
    let written = report.outcome().unwrap();
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.detail.starts_with("not written: no feature history")));
    assert_eq!(
        written.check,
        Check::NotApplicable(
            "no feature history cadmpeg decodes: its 2 features are kept in their native form"
                .to_owned()
        )
    );
    // A native feature after a valid extrusion retains that editable prefix.
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    if let Some(features) = wire["model"]["features"].as_array_mut() {
        features.push(
            json!({"id": "f3d:model:feature#9", "ordinal": 9, "name": "Base1",
                             "suppressed": false, "definition": native}),
        );
    }
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let mut report = Report::default();
    let outcome = SolidCraftWriter::new().plan(&model, &mut report);
    assert!(
        matches!(outcome.check, Check::Partial(_)),
        "{:?}",
        outcome.check
    );
    assert_eq!(outcome.mode, Mode::PartialHistory);
    assert_eq!(
        outcome
            .features
            .iter()
            .filter(|f| f.status == dutchman_core::FeatureStatus::Mapped)
            .count(),
        2
    );
}

/// A design of user parameters and no geometry, as a Fusion design other designs take their
/// parameters from: `W1` = 20 mm, `half` = W1 / 2 and an angle.
fn parameters_alone() -> Model {
    let ir = cadmpeg_ir::examples::unit_cube().unwrap();
    let mut wire = serde_json::to_value(&ir).unwrap();
    if let Some(arenas) = wire["model"].as_object_mut() {
        for rows in arenas.values_mut() {
            if rows.is_array() {
                *rows = json!([]);
            }
        }
    }
    wire["model"]["parameters"] = json!([
        {"id": "f3d:model:parameter#1", "ordinal": 0, "name": "W1", "expression": "20 mm",
         "value": {"kind": "length", "value": 20.0}},
        {"id": "f3d:model:parameter#2", "ordinal": 1, "name": "half", "expression": "W1 / 2",
         "value": {"kind": "length", "value": 10.0},
         "dependencies": ["f3d:model:parameter#1"]},
        {"id": "f3d:model:parameter#3", "ordinal": 2, "name": "tilt", "expression": "15 deg",
         "value": {"kind": "angle", "value": 15.0_f64.to_radians()}},
    ]);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        fidelity: None,
        name: "Parameters".to_owned(),
        source_file: Some("Parameters.f3d".to_owned()),
        source_step: None,
        assembly: None,
    }
}

#[test]
fn a_design_of_parameters_alone_is_written_with_its_parameters() {
    let model = parameters_alone();
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    assert_eq!(writer.convertible(&model), Ok(()));
    let reason = "the design holds 3 parameters and no geometry".to_owned();
    let mut report = Report::default();
    let planned = writer.plan(&model, &mut report);
    assert_eq!(planned.check, Check::NotApplicable(reason.clone()));
    let (document, report) = convert(&model, &writer);
    assert_eq!(
        document["params"],
        json!([
            {"name": "W1", "expr": "20 mm", "unit": "mm", "model": false},
            {"name": "half", "expr": "W1 / 2", "unit": "mm", "model": false},
            {"name": "tilt", "expr": "15 deg", "unit": "deg", "model": false},
        ])
    );
    assert_eq!(document["features"], json!([]));
    let outcome = report.outcome().unwrap();
    assert_eq!(outcome.mode, Mode::Import);
    assert_eq!(outcome.check, Check::NotApplicable(reason));
    assert!(report.losses().iter().any(|loss| loss.subject == "geometry"
        && loss.detail == "none: the design holds 3 parameters and no geometry"));

    // A design with neither geometry nor parameters still has nothing to write.
    let mut empty = parameters_alone();
    let mut wire = serde_json::to_value(&empty.ir).unwrap();
    wire["model"]["parameters"] = json!([]);
    empty.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    assert!(writer.convertible(&empty).is_err());
}

#[test]
fn solidcraft_opens_a_design_of_parameters_alone() {
    let Some(cli) = solidcraft() else { return };
    assert_eq!(
        volumes(&cli, &written(&parameters_alone()).0),
        Vec::<f64>::new()
    );
}

/// Change only the extrusion definition of the synthetic cube history.
fn changed_extrude(change: impl FnOnce(&mut Value)) -> Model {
    let mut model = cube_with_history(10.0, whole_sketch(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    change(&mut wire["model"]["features"][1]["definition"]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn one_sided_extrude_draft_keeps_its_sign_in_both_directions() {
    for direction in ["profile_normal", "reversed_profile_normal"] {
        for degrees in [-10.0_f64, 10.0] {
            let model = changed_extrude(|definition| {
                definition["direction"] = json!({"kind": direction});
                definition["extent"]["side"]["draft"] = json!(degrees.to_radians());
            });
            let (document, report) = convert(
                &model,
                &SolidCraftWriter::new().verification(Verification::Off),
            );
            assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
            let extent = &document["features"][1]["extent"];
            assert_eq!(extent["taper"], format!("{degrees} deg"));
            let expected_direction = if direction == "profile_normal" {
                "positive"
            } else {
                "negative"
            };
            assert_eq!(extent["direction"], expected_direction);
        }
    }
}

#[test]
fn solidcraft_rebuilds_drafted_extrude_volumes() {
    let Some(cli) = solidcraft() else { return };
    for direction in ["profile_normal", "reversed_profile_normal"] {
        for degrees in [-10.0_f64, 10.0] {
            let model = changed_extrude(|definition| {
                definition["direction"] = json!({"kind": direction});
                definition["extent"]["side"]["draft"] = json!(degrees.to_radians());
            });
            let (bytes, _) = written(&model);
            // Positive taper grows a 10 mm square outward as it travels 10 mm.
            let top = 10.0 + 20.0 * degrees.to_radians().tan();
            let expected = 10.0 / 3.0 * (100.0 + 10.0 * top + top * top);
            let actual = volumes(&cli, &bytes);
            assert_eq!(actual.len(), 1);
            assert!(
                (actual[0] - expected).abs() < 1e-4,
                "{actual:?}, {expected}"
            );
        }
    }
}

/// A resolved parallel plane end, optionally also present as a placed datum helper.
fn plane_ended_extrude(z: f64, datum: bool, change: impl FnOnce(&mut Value)) -> Model {
    let frame = json!({
        "origin": {"x": 37.0, "y": -23.0, "z": z},
        "normal": {"x": 0.0, "y": 0.0, "z": -2.0},
        "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0}
    });
    let mut model = changed_extrude(|definition| {
        definition["extent"]["side"]["termination"] = json!({
            "kind": "to_plane", "plane": frame, "native": "synthetic plane"
        });
        change(definition);
    });
    if datum {
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        wire["model"]["features"][1]["ordinal"] = json!(2);
        wire["model"]["features"][1]["dependencies"] = json!(["f3d:model:feature#3"]);
        wire["model"]["features"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "f3d:model:feature#3", "ordinal": 1, "name": "End plane",
                "suppressed": false, "dependencies": [], "outputs": [],
                "definition": {"definition": "datum_plane", "frame": frame}
            }));
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    }
    model
}

#[test]
fn parallel_plane_ended_extrudes_use_free_to_object_points() {
    for datum in [false, true] {
        for reverse in [false, true] {
            let z = if reverse { -10.0 } else { 10.0 };
            let model = plane_ended_extrude(z, datum, |definition| {
                if reverse {
                    definition["direction"] = json!({"kind": "reversed_profile_normal"});
                }
            });
            let (document, report) = convert(
                &model,
                &SolidCraftWriter::new().verification(Verification::Off),
            );
            assert!(
                history_line(&report).starts_with("2 of 2 features rebuilt"),
                "{}",
                history_line(&report)
            );
            assert_eq!(
                document["features"][1]["extent"]["to"],
                json!([37.0, -23.0, z])
            );
            assert_eq!(document["features"][1]["extent"]["distance"], "0 mm");
            if let Some(cli) = solidcraft() {
                let actual = volumes(&cli, &written(&model).0);
                assert_eq!(actual.len(), 1);
                assert!((actual[0] - 1000.0).abs() < 1e-4, "{actual:?}");
            }
        }
    }
}

#[test]
fn plane_ended_extrudes_follow_a_placed_sketch_normal() {
    let mut model = plane_ended_extrude(10.0, false, |_| {});
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketches"][0]["placement"]["frame"] = json!({
        "origin": {"x": 5.0, "y": 2.0, "z": 3.0},
        "normal": {"x": 1.0, "y": 0.0, "z": 0.0},
        "u_axis": {"x": 0.0, "y": 1.0, "z": 0.0}
    });
    wire["model"]["features"][1]["definition"]["extent"]["side"]["termination"]["plane"] = json!({
        "origin": {"x": 15.0, "y": -23.0, "z": 37.0},
        "normal": {"x": 2.0, "y": 0.0, "z": 0.0},
        "u_axis": {"x": 0.0, "y": 0.0, "z": 1.0}
    });
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    assert_eq!(
        document["features"][1]["extent"]["to"],
        json!([15.0, -23.0, 37.0])
    );
    if let Some(cli) = solidcraft() {
        let actual = volumes(&cli, &written(&model).0);
        assert_eq!(actual.len(), 1);
        assert!((actual[0] - 1000.0).abs() < 1e-4, "{actual:?}");
    }
}

#[test]
fn plane_ended_extrudes_keep_start_offsets_and_two_sided_travel() {
    let model = plane_ended_extrude(10.0, false, |definition| {
        definition["start"] = json!({"kind": "offset_profile_plane", "offset": 3.0});
        let first = definition["extent"]["side"].clone();
        definition["extent"] = json!({"kind": "two_sided", "first": first,
            "second": {"termination": {"kind": "blind", "length": 2.0}}});
    });
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    assert_eq!(document["features"][1]["extent"]["start_offset"], "3 mm");
    assert_eq!(document["features"][1]["extent"]["distance2"], "2 mm");
    if let Some(cli) = solidcraft() {
        let actual = volumes(&cli, &written(&model).0);
        assert_eq!(actual.len(), 1);
        assert!((actual[0] - 900.0).abs() < 1e-4, "{actual:?}");
    }
}

#[test]
fn unsupported_plane_and_vertex_extrude_ends_name_the_blocker() {
    for (termination, reason) in [
        (
            json!({"kind": "to_plane", "native": "synthetic tilted plane", "plane": {
                "origin": {"x": 0.0, "y": 0.0, "z": 10.0},
                "normal": {"x": 0.0, "y": 1.0, "z": 1.0},
                "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0}
            }}),
            "terminating plane not parallel",
        ),
        (
            json!({"kind": "to_vertex", "vertex": {"kind": "native", "value": "synthetic vertex"}}),
            "to-vertex extrude needs a resolved input vertex position",
        ),
    ] {
        let model = changed_extrude(|definition| {
            definition["extent"]["side"]["termination"] = termination;
        });
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let line = history_line(&report);
        assert!(line.contains("Block") && line.contains(reason), "{line}");
    }
    for (z, change, reason) in [
        (-10.0, "wrong_side", "not along the selected extrude side"),
        (10.0, "symmetric", "symmetric extrude to an object"),
        (
            10.0,
            "negative_end",
            "non-blind extrude end against the sketch normal",
        ),
    ] {
        let model = plane_ended_extrude(z, false, |definition| {
            let side = definition["extent"]["side"].clone();
            if change == "symmetric" {
                definition["extent"] = json!({"kind": "symmetric", "side": side});
            } else if change == "negative_end" {
                definition["extent"] = json!({"kind": "two_sided",
                    "first": {"termination": {"kind": "blind", "length": 2.0}}, "second": side});
            }
        });
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

/// An earlier cube followed by another extrusion using its top face as an end or start.
fn face_ended_extrude(from_face: bool, offset: f64, resolved: bool) -> Model {
    let mut model = cube_with_history(10.0, whole_sketch(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let selection = if resolved {
        json!({"kind": "resolved", "value": {
            "faces": ["synthetic:cube:face#top"], "native": "synthetic top face"
        }})
    } else {
        json!({"kind": "faces", "value": ["synthetic:cube:face#top"]})
    };
    let mut next = wire["model"]["features"][1].clone();
    next["id"] = json!("f3d:model:feature#3");
    next["ordinal"] = json!(2);
    next["name"] = json!("Face extent");
    next["outputs"] = json!([]);
    next["dependencies"] = json!(["f3d:model:feature#2"]);
    if from_face {
        next["definition"]["start"] = json!({
            "kind": "from_face", "face": selection, "offset": offset
        });
        next["definition"]["extent"]["side"]["termination"]["length"] = json!(2.0);
    } else {
        next["definition"]["start"] = json!({"kind": "offset_profile_plane", "offset": 12.0});
        next["definition"]["direction"] = json!({"kind": "reversed_profile_normal"});
        next["definition"]["extent"]["side"]["termination"] = json!({
            "kind": "to_face", "face": selection, "offset": offset
        });
    }
    wire["model"]["features"].as_array_mut().unwrap().push(next);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

/// [`face_ended_extrude`] with its second extrude ending at a plane at height `z`, whose
/// source (`face_surface`, `work_plane`, ...) is `source`.
fn extrude_ending_at_plane(z: f64, source: Option<&str>) -> Model {
    let mut model = face_ended_extrude(false, 0.0, false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let mut termination = json!({
        "kind": "to_plane", "native": "synthetic plane", "plane": {
            "origin": {"x": 0.0, "y": 5.0, "z": z},
            "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
            "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0}
        }
    });
    if let Some(source) = source {
        termination["source"] = json!(source);
    }
    wire["model"]["features"][2]["definition"]["extent"]["side"]["termination"] = termination;
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn plane_ended_extrudes_after_bodies_end_on_a_face_surface() {
    // The plane is the surface of a face: a SolidCraft pick there ends at the same plane.
    let (document, report) = convert(
        &extrude_ending_at_plane(10.0, Some("face_surface")),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).starts_with("3 of 3 features rebuilt"),
        "{}",
        history_line(&report)
    );
    assert_eq!(
        document["features"][2]["extent"]["to"],
        json!([0.0, 5.0, 10.0])
    );
}

#[test]
fn plane_started_extrudes_start_at_the_plane_offset() {
    // The fixture's second extrude starts 12 mm above the sketch and runs down to the top
    // face; the same start given as a plane gives the same start offset.
    let mut model = face_ended_extrude(false, 0.0, false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][2]["definition"]["start"] = json!({
        "kind": "from_plane", "native": "synthetic plane", "source": "face_surface", "plane": {
            "origin": {"x": 0.0, "y": 0.0, "z": 12.0},
            "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
            "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0}
        }
    });
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let line = history_line(&report);
    assert!(!line.contains("extrude starts are not mapped"), "{line}");
    assert_eq!(
        document["features"][2]["extent"]["start_offset"], "12 mm",
        "{line}\n{}",
        document["features"]
    );
}

#[test]
fn plane_ended_extrudes_after_bodies_need_input_face_geometry() {
    for (z, source) in [(10.0, None), (5.0, None), (10.0, Some("work_plane"))] {
        let model = extrude_ending_at_plane(z, source);
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let line = history_line(&report);
        assert!(
            line.contains("Face extent")
                && line.contains("CADIR does not supply the input face geometry"),
            "{line}"
        );
        assert_partial(&document, &report, 1);
    }
}

#[test]
fn resolved_face_ends_and_starts_use_solidcraft_picks() {
    for resolved in [false, true] {
        for from in [false, true] {
            let (document, report) = convert(
                &face_ended_extrude(from, -0.5, resolved),
                &SolidCraftWriter::new().verification(Verification::Off),
            );
            assert!(
                history_line(&report).starts_with("3 of 3 features rebuilt"),
                "{}",
                history_line(&report)
            );
            let extent = &document["features"][2]["extent"];
            let pick = &extent[if from { "from" } else { "to" }];
            assert_eq!(pick[2], 10.0);
            assert!(pick[0].as_f64().unwrap() > 0.0 && pick[0].as_f64().unwrap() < 10.0);
            assert!(pick[1].as_f64().unwrap() > 0.0 && pick[1].as_f64().unwrap() < 10.0);
            if from {
                assert_eq!(extent["start_offset"], "-0.5 mm");
                assert_eq!(extent["distance"], "2 mm");
            } else {
                assert_eq!(extent["start_offset"], "12 mm");
                assert_eq!(extent["to_offset"], "-0.5 mm");
                assert_eq!(extent["distance"], "0 mm");
            }
        }
    }
}

#[test]
fn solidcraft_rebuilds_face_ended_and_face_started_extrudes() {
    let Some(cli) = solidcraft() else { return };
    for (from, expected) in [(false, 150.0), (true, 200.0)] {
        let (bytes, report) = written(&face_ended_extrude(from, -0.5, false));
        assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
        let mut actual = volumes(&cli, &bytes);
        actual.sort_by(f64::total_cmp);
        assert_eq!(actual.len(), 2);
        assert!((actual[0] - expected).abs() < 1e-4, "{actual:?}");
        assert!((actual[1] - 1000.0).abs() < 1e-4, "{actual:?}");
    }
}

#[test]
fn reverse_start_offsets_and_two_sided_distances_follow_the_normal() {
    let model = changed_extrude(|definition| {
        definition["direction"] = json!({"kind": "reversed_profile_normal"});
        definition["start"] = json!({"kind": "offset_profile_plane", "offset": 3.0});
        definition["extent"] = json!({"kind": "two_sided",
            "first": {"termination": {"kind": "blind", "length": 7.0}},
            "second": {"termination": {"kind": "blind", "length": 2.0}}});
    });
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    assert_eq!(
        document["features"][1]["extent"],
        json!({
            "direction": "positive", "distance": "2 mm", "distance2": "7 mm", "start_offset": "3 mm"
        })
    );
    if let Some(cli) = solidcraft() {
        let (bytes, _) = written(&model);
        let actual = volumes(&cli, &bytes);
        assert_eq!(actual.len(), 1);
        assert!((actual[0] - 900.0).abs() < 1e-4);
    }
}

#[test]
fn equal_two_sided_revolves_become_symmetric() {
    let model = ring_history(json!({"kind": "two_sided",
        "first": {"kind": "angle", "angle": std::f64::consts::FRAC_PI_2},
        "second": {"kind": "angle", "angle": std::f64::consts::FRAC_PI_2}}));
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("2 of 2 features rebuilt"));
    assert_eq!(document["features"][1]["symmetric"], true);
    assert_eq!(document["features"][1]["angle"], "180 deg");
    if let Some(cli) = solidcraft() {
        let (bytes, _) = written(&model);
        let actual = volumes(&cli, &bytes);
        assert_eq!(actual.len(), 1);
        assert!((actual[0] - 375.0 * std::f64::consts::PI).abs() < 1e-3);
    }
}

#[test]
fn unsupported_end_combinations_keep_specific_refusals() {
    for kind in ["symmetric", "two_sided"] {
        let model = changed_extrude(|definition| {
            let side = json!({"termination": {"kind": "blind", "length": 4.0}, "draft": 0.1});
            definition["extent"] = if kind == "symmetric" {
                json!({"kind": kind, "side": side})
            } else {
                json!({"kind": kind, "first": side, "second": side})
            };
        });
        let (_, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(history_line(&report).contains("draft on a symmetric or two-sided extrude"));
    }
    let model = ring_history(json!({"kind": "one_sided", "termination": {
        "kind": "to_face", "face": {"kind": "faces", "value": ["synthetic:cube:face#top"]}
    }}));
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).contains("CADIR does not state which way it turns"));
}

#[test]
fn face_selections_that_cannot_be_placed_stop_the_history() {
    for (selection, reason) in [
        (
            json!({"kind": "native", "value": "synthetic top face"}),
            "a native face reference is not supported yet",
        ),
        (
            json!({"kind": "faces", "value": []}),
            "without exactly one face",
        ),
        (
            json!({"kind": "faces", "value": ["synthetic:cube:face#top", "synthetic:cube:face#bottom"]}),
            "without exactly one face",
        ),
        (
            json!({"kind": "faces", "value": ["synthetic:cube:face#front"]}),
            "not parallel to its sketch",
        ),
    ] {
        let mut model = face_ended_extrude(false, 0.0, false);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let termination =
            &mut wire["model"]["features"][2]["definition"]["extent"]["side"]["termination"];
        termination["face"] = selection;
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert_partial(&document, &report, 1);
        assert_eq!(
            types(&document),
            [
                ("sketch".to_owned(), false),
                ("extrude".to_owned(), false),
                ("import".to_owned(), false),
            ]
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
    let model = changed_extrude(|definition| {
        definition["extent"]["side"]["termination"] = json!({"kind": "to_face",
            "face": {"kind": "faces", "value": ["synthetic:cube:face#top"]}});
    });
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).contains("not on an earlier named body"));
}

#[test]
fn a_two_sided_extrude_can_end_at_a_face_along_the_positive_normal() {
    let mut model = face_ended_extrude(false, 0.0, false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let definition = &mut wire["model"]["features"][2]["definition"];
    let first = definition["extent"]["side"].clone();
    definition["direction"] = json!({"kind": "profile_normal"});
    definition["start"] = json!({"kind": "offset_profile_plane", "offset": 5.0});
    definition["extent"] = json!({"kind": "two_sided", "first": first,
        "second": {"termination": {"kind": "blind", "length": 2.0}}});
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).starts_with("3 of 3 features rebuilt"));
    assert_eq!(document["features"][2]["extent"]["distance2"], "2 mm");
    assert_eq!(document["features"][2]["extent"]["to"][2], 10.0);
    if let Some(cli) = solidcraft() {
        let (bytes, _) = written(&model);
        let mut actual = volumes(&cli, &bytes);
        actual.sort_by(f64::total_cmp);
        assert_eq!(actual.len(), 2);
        assert!((actual[0] - 700.0).abs() < 1e-4, "{actual:?}");
    }
}

#[test]
fn a_face_end_does_not_silently_reverse_the_extrude() {
    let mut model = face_ended_extrude(false, 0.0, false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][2]["definition"]["direction"] = json!({"kind": "profile_normal"});
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(history_line(&report).contains("not along the selected extrude side"));
}

/// Synthetic sketch additions on the example cube; every relation is tested alone so
/// another relation cannot hide a missing mapping by already determining the sketch.
fn sketch_relation_history(definition: Value) -> Model {
    let mut model = cube_with_history(10.0, whole_sketch(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let rows = wire["model"]["sketch_entities"].as_array_mut().unwrap();
    let mut extra = vec![
        entity(
            5,
            json!({"kind": "circle", "center": {"u": 20.0, "v": 20.0}, "radius": 2.0}),
        ),
        entity(
            6,
            json!({"kind": "circle", "center": {"u": 20.0, "v": 20.0}, "radius": 2.0}),
        ),
        entity(
            7,
            json!({"kind": "ellipse", "center": {"u": 20.0, "v": 20.0},
            "major_angle": 0.0, "major_radius": 4.0, "minor_radius": 2.0}),
        ),
        entity(
            8,
            json!({"kind": "point", "position": {"u": 5.0, "v": 5.0}}),
        ),
        entity(
            9,
            json!({"kind": "arc", "center": {"u": 10.0, "v": 2.0},
            "radius": 2.0, "start_angle": -std::f64::consts::FRAC_PI_2, "end_angle": 0.0}),
        ),
        line(10, [10.0, 0.0], [20.0, 0.0]),
    ];
    for row in &mut extra {
        row["construction"] = json!(true);
    }
    rows.extend(extra);
    wire["model"]["sketch_constraints"] = json!([{
        "id": "f3d:model:sketch-constraint#1", "sketch": SKETCH, "definition": definition,
    }]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

fn sketch_locus(kind: &str, key: u32) -> Value {
    json!({"kind": kind, "entity": format!("f3d:model:sketch-entity#{key}")})
}

#[test]
fn additional_sketch_relations_keep_their_intent_and_cube_volume() {
    let id = |key| format!("f3d:model:sketch-entity#{key}");
    let parameter = "f3d:model:parameter#2";
    let cases = vec![
        (
            json!({"kind": "same_coordinate", "relation": {
            "first": sketch_locus("start", 1), "second": sketch_locus("end", 3), "axis": "u"}}),
            "vertical_points",
        ),
        (
            json!({"kind": "point_coordinate_values", "point": sketch_locus("entity", 8),
            "values": [5.0, 5.0]}),
            "fix",
        ),
        (
            json!({"kind": "at_intersection", "point": sketch_locus("end", 1),
            "first": id(1), "second": id(2)}),
            "point_on_curve",
        ),
        (
            json!({"kind": "coradial", "first": id(5), "second": id(6)}),
            "concentric",
        ),
        (
            json!({"kind": "tangent_loci", "first": sketch_locus("end", 1),
            "second": sketch_locus("start", 9)}),
            "tangent",
        ),
        (
            json!({"kind": "curvature", "first": id(1), "second": id(10)}),
            "smooth",
        ),
        (
            json!({"kind": "point_symmetric", "first": sketch_locus("start", 1),
            "second": sketch_locus("start", 3), "center": sketch_locus("entity", 8)}),
            "midpoint",
        ),
        (
            json!({"kind": "polygon", "polygon": {"entities": [id(1), id(2), id(3), id(4)]}}),
            "equal",
        ),
        (
            json!({"kind": "distance_loci_value", "first": sketch_locus("start", 1),
            "second": sketch_locus("end", 1), "distance": 10.0, "parameter": parameter}),
            "distance",
        ),
        (
            json!({"kind": "repeated_length", "entities": [id(1), id(2)], "parameter": parameter}),
            "length",
        ),
        (
            json!({"kind": "repeated_distance", "measurements": [
            {"kind": "horizontal", "first": sketch_locus("start", 1), "second": sketch_locus("end", 2)},
            {"kind": "vertical", "first": sketch_locus("start", 4), "second": sketch_locus("end", 10)}],
            "parameter": parameter}),
            "distance_x",
        ),
        (
            json!({"kind": "arc_angle", "entity": id(9), "angle": std::f64::consts::FRAC_PI_2}),
            "arc_sweep",
        ),
        (
            json!({"kind": "polar_distance", "first": sketch_locus("start", 1),
            "second": sketch_locus("end", 1), "distance": 10.0, "angle": 0.0,
            "distance_parameter": parameter}),
            "length",
        ),
        (
            json!({"kind": "equal_distance", "first": {"first": sketch_locus("start", 1), "second": sketch_locus("end", 1)},
            "second": {"first": sketch_locus("start", 2), "second": sketch_locus("end", 2)}}),
            "equal",
        ),
        (
            json!({"kind": "midpoint_coordinate", "first": sketch_locus("start", 1),
            "second": sketch_locus("end", 1), "axis": "u", "value": 5.0}),
            "midpoint",
        ),
        (
            json!({"kind": "parallel_line_set_distance", "first": [id(1), id(10)],
            "second": [id(3)], "parameter": parameter}),
            "parallel",
        ),
    ];
    let cli = solidcraft();
    for (definition, expected) in cases {
        let model = sketch_relation_history(definition.clone());
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert_eq!(
            document["features"][0]["type"],
            "sketch",
            "{definition}: {:?}",
            report.losses()
        );
        assert!(
            !report
                .losses()
                .iter()
                .any(|loss| loss.detail.contains("constraints not carried")),
            "{definition}: {:?}",
            report.losses()
        );
        let constraints = document["features"][0]["sketch"]["constraints"]
            .as_array()
            .unwrap();
        assert_eq!(constraints[0]["type"], expected, "{definition}");
        if definition["parameter"] == parameter || definition["distance_parameter"] == parameter {
            assert!(
                constraints.iter().any(|c| c["param"] == "side"),
                "{definition}: {constraints:?}"
            );
        }
        if let Some(cli) = &cli {
            let got = volumes(cli, &written(&model).0);
            assert_eq!(got.len(), 1, "{definition}: {got:?}");
            assert!((got[0] - 1000.0).abs() < 1e-6, "{definition}: {got:?}");
        }
    }
}

#[test]
fn fixed_curves_hold_the_radius_and_ellipse_radius_dimensions_use_the_minor_radius() {
    for key in [5, 7] {
        let model = sketch_relation_history(
            json!({"kind": "fixed", "entity": format!("f3d:model:sketch-entity#{key}")}),
        );
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(!report
            .losses()
            .iter()
            .any(|loss| loss.detail.contains("constraints not carried")));
        assert_eq!(
            document["features"][0]["sketch"]["curves"][key - 1]["fixed"],
            true
        );
        if let Some(cli) = solidcraft() {
            assert_eq!(volumes(&cli, &written(&model).0), [1000.0]);
        }
    }
    for kind in ["radius", "repeated_radius", "repeated_diameter"] {
        let mut definition = json!({"kind": kind, "parameter": "f3d:model:parameter#3"});
        if kind == "radius" {
            definition["entity"] = json!("f3d:model:sketch-entity#7");
        } else {
            definition["entities"] =
                json!(["f3d:model:sketch-entity#5", "f3d:model:sketch-entity#6"]);
        }
        let mut model = sketch_relation_history(definition);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let value = if kind == "repeated_diameter" {
            4.0
        } else {
            2.0
        };
        wire["model"]["parameters"].as_array_mut().unwrap().push(json!({
            "id": "f3d:model:parameter#3", "name": "round_size", "expression": format!("{value} mm"),
            "value": {"kind": "length", "value": value},
        }));
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(!report
            .losses()
            .iter()
            .any(|loss| loss.detail.contains("constraints not carried")));
        let constraints = document["features"][0]["sketch"]["constraints"]
            .as_array()
            .unwrap();
        assert_eq!(constraints.len(), if kind == "radius" { 1 } else { 2 });
        for c in constraints {
            assert_eq!(c["value"], value);
            assert_eq!(c["param"], "round_size");
        }
        if let Some(cli) = solidcraft() {
            assert_eq!(volumes(&cli, &written(&model).0), [1000.0]);
        }
    }
}

fn bounded_conic_history(geometry: Value, start: [f64; 2], end: [f64; 2]) -> Model {
    let mut model = cube_with_history(10.0, whole_sketch(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketches"][0]["profiles"] = json!([]);
    wire["model"]["sketch_constraints"] = json!([]);
    wire["model"]["sketch_entities"] = json!([entity(1, geometry), line(2, end, start)]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn bounded_conics_and_weighted_beziers_preserve_their_shape() {
    let h = 0.5_f64;
    let cases = vec![
        (
            json!({"kind": "ellipse", "center": {"u": 0.0, "v": 0.0}, "major_angle": 0.0,
            "major_radius": 4.0, "minor_radius": 2.0, "bounds": [0.0, std::f64::consts::FRAC_PI_2]}),
            [4.0, 0.0],
            [0.0, 2.0],
            20.0 * (std::f64::consts::PI - 2.0),
        ),
        (
            json!({"kind": "parabola", "vertex": {"u": 0.0, "v": 0.0}, "axis_angle": 0.0,
            "focal_length": 1.0, "bounds": [-2.0, 2.0]}),
            [1.0, -2.0],
            [1.0, 2.0],
            80.0 / 3.0,
        ),
        (
            json!({"kind": "hyperbola", "center": {"u": 0.0, "v": 0.0}, "major_angle": 0.0,
            "major_radius": 4.0, "minor_radius": 2.0, "bounds": [-h, h]}),
            [4.0 * h.cosh(), -2.0 * h.sinh()],
            [4.0 * h.cosh(), 2.0 * h.sinh()],
            80.0 * (h.sinh() * h.cosh() - h),
        ),
        (
            json!({"kind": "nurbs", "curve": {"degree": 2, "periodic": false,
            "knots": [0.0, 0.0, 0.0, 1.0, 1.0, 1.0], "poles": {"form": "rational", "points": [
                {"point": {"u": 4.0, "v": 0.0}, "weight": 1.0},
                {"point": {"u": 4.0, "v": 2.0}, "weight": std::f64::consts::FRAC_1_SQRT_2},
                {"point": {"u": 0.0, "v": 2.0}, "weight": 1.0}]}}}),
            [4.0, 0.0],
            [0.0, 2.0],
            20.0 * (std::f64::consts::PI - 2.0),
        ),
    ];
    let cli = solidcraft();
    for (geometry, start, end, expected_volume) in cases {
        let model = bounded_conic_history(geometry.clone(), start, end);
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let sketch = &document["features"][0]["sketch"];
        assert_eq!(
            document["features"][0]["type"],
            "sketch",
            "{geometry}: {:?}",
            report.losses()
        );
        assert_eq!(sketch["curves"][0]["type"], "conic");
        assert_eq!(sketch["curves"][0]["a"], sketch["curves"][1]["b"]);
        assert_eq!(sketch["curves"][0]["b"], sketch["curves"][1]["a"]);
        let rho = sketch["curves"][0]["rho"].as_f64().unwrap();
        let weight = match geometry["kind"].as_str().unwrap() {
            "parabola" => 1.0,
            "hyperbola" => h.cosh(),
            _ => std::f64::consts::FRAC_1_SQRT_2,
        };
        assert!((rho - weight / (1.0 + weight)).abs() < 1e-12);
        let apex = usize::try_from(sketch["curves"][0]["apex"].as_u64().unwrap()).unwrap();
        let expected_apex = match geometry["kind"].as_str().unwrap() {
            "parabola" => [-1.0, 0.0],
            "hyperbola" => [4.0 / h.cosh(), 0.0],
            _ => [4.0, 2.0],
        };
        for (index, expected) in expected_apex.into_iter().enumerate() {
            assert!(
                (sketch["points"][apex]["pos"][index].as_f64().unwrap() - expected).abs() < 1e-12
            );
        }
        if let Some(cli) = &cli {
            let got = volumes(cli, &written(&model).0);
            assert_eq!(got.len(), 1, "{geometry}: {got:?}");
            assert!(
                (got[0] - expected_volume).abs() / expected_volume < 1e-3,
                "{geometry}: {got:?} vs {expected_volume}"
            );
        }
    }
}

#[test]
fn static_pattern_and_projected_copy_relations_map_to_point_and_radius_associations() {
    let id = |key| format!("f3d:model:sketch-entity#{key}");
    let cases = [
        json!({"kind": "projected_copy", "source": id(5), "result": id(6)}),
        json!({"kind": "rectangular_pattern", "pattern": {
            "directions": [{"direction": [1.0, 0.0], "spacing": 10.0},
                {"direction": [0.0, 1.0], "spacing": 10.0}],
            "rows": [[{"entities": [id(5)]}], [{"entities": [id(6)]}]],
        }}),
        json!({"kind": "circular_pattern", "pattern": {"center": id(8), "angle": std::f64::consts::PI,
            "seed": [id(5)], "instances": [{"angle": std::f64::consts::PI, "entities": [id(6)]}],
        }}),
    ];
    let cli = solidcraft();
    for definition in cases {
        let kind = definition["kind"].as_str().unwrap();
        let mut model = sketch_relation_history(definition.clone());
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        if kind == "rectangular_pattern" {
            wire["model"]["sketch_entities"][5]["geometry"]["center"] =
                json!({"u": 30.0, "v": 20.0});
        } else if kind == "circular_pattern" {
            wire["model"]["sketch_entities"][5]["geometry"]["center"] =
                json!({"u": -10.0, "v": -10.0});
        }
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            !report
                .losses()
                .iter()
                .any(|loss| loss.detail.contains("constraints not carried")),
            "{definition}: {:?}",
            report.losses()
        );
        let constraints = document["features"][0]["sketch"]["constraints"]
            .as_array()
            .unwrap();
        assert!(constraints.iter().any(|c| c["type"] == "pattern_radius"));
        if kind != "projected_copy" {
            let point = constraints
                .iter()
                .find(|c| c["type"] == "pattern_point")
                .unwrap();
            if kind == "rectangular_pattern" {
                assert_eq!(point["offset"], json!([10.0, 0.0]));
            } else {
                assert!(point["center"].is_number());
                assert_eq!(point["angle"], std::f64::consts::PI);
            }
        }
        if let Some(cli) = &cli {
            assert_eq!(volumes(cli, &written(&model).0), [1000.0]);
        }
    }
}

#[test]
fn vertical_axis_angles_use_a_fixed_construction_axis_and_keep_the_parameter() {
    let mut model = sketch_relation_history(json!({"kind": "angle_to_axis", "axis": "vertical",
        "entity": "f3d:model:sketch-entity#1", "parameter": "f3d:model:parameter#3"}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["parameters"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "f3d:model:parameter#3", "name": "theta", "expression": "90 deg",
            "value": {"kind": "angle", "value": std::f64::consts::FRAC_PI_2},
        }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(!report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("constraints not carried")));
    let sketch = &document["features"][0]["sketch"];
    assert_eq!(sketch["constraints"][0]["type"], "angle");
    assert_eq!(sketch["constraints"][0]["param"], "theta");
    assert_eq!(
        sketch["curves"].as_array().unwrap().last().unwrap()["fixed"],
        true
    );
    if let Some(cli) = solidcraft() {
        assert_eq!(volumes(&cli, &written(&model).0), [1000.0]);
    }
}

#[test]
fn solver_only_sketch_relations_are_counted_without_flattening() {
    let mut model =
        sketch_relation_history(json!({"kind": "scalar_equality", "first": 1, "second": 2}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketch_constraints"].as_array_mut().unwrap().push(json!({
        "id": "f3d:model:sketch-constraint#2", "sketch": SKETCH,
        "definition": {"kind": "angle_difference", "first": 1, "second": 2, "difference": 3, "value": 0.0},
    }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(document["features"][0]["type"], "sketch");
    let loss = report
        .losses()
        .iter()
        .find(|loss| loss.detail.contains("2 of 2 constraints not carried"))
        .unwrap();
    assert_eq!(loss.severity, Severity::Degraded);
    assert!(loss
        .detail
        .contains("solver scalar identities have no sketch entity binding"));
}

#[test]
fn equal_weight_rational_splines_are_polynomial_control_splines() {
    let model = bounded_conic_history(
        json!({"kind": "nurbs", "curve": {
        "degree": 3, "periodic": false, "knots": [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        "poles": {"form": "rational", "points": [
            {"point": {"u": 0.0, "v": 0.0}, "weight": 2.0},
            {"point": {"u": 10.0, "v": 20.0}, "weight": 2.0},
            {"point": {"u": 20.0, "v": 20.0}, "weight": 2.0},
            {"point": {"u": 30.0, "v": 0.0}, "weight": 2.0},
        ]}}}),
        [0.0, 0.0],
        [30.0, 0.0],
    );
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(
        document["features"][0]["type"],
        "sketch",
        "{:?}",
        report.losses()
    );
    let curve = &document["features"][0]["sketch"]["curves"][0];
    assert_eq!(curve["type"], "spline");
    assert_eq!(curve["control"], true);
    assert_eq!(curve["degree"], 3);
    assert_eq!(curve["pts"].as_array().unwrap().len(), 4);
    if let Some(cli) = solidcraft() {
        let got = volumes(&cli, &written(&model).0);
        assert_eq!(got.len(), 1);
        assert!((got[0] - 3000.0).abs() / 3000.0 < 1e-3, "{got:?}");
    }
}

#[test]
fn major_elliptical_arcs_are_exact_conic_chains_in_a_whole_sketch() {
    let geometry = json!({"kind": "ellipse", "center": {"u": 0.0, "v": 0.0}, "major_angle": 0.0,
        "major_radius": 4.0, "minor_radius": 2.0, "bounds": [0.0, 3.0 * std::f64::consts::FRAC_PI_2]});
    let model = bounded_conic_history(geometry, [4.0, 0.0], [0.0, -2.0]);
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(
        document["features"][0]["type"],
        "sketch",
        "{:?}",
        report.losses()
    );
    let curves = document["features"][0]["sketch"]["curves"]
        .as_array()
        .unwrap();
    assert_eq!(curves.len(), 5);
    for pair in curves[..4].windows(2) {
        assert_eq!(pair[0]["type"], "conic");
        assert_eq!(pair[0]["b"], pair[1]["a"]);
    }
    assert_eq!(curves[0]["a"], curves[4]["b"]);
    assert_eq!(curves[3]["b"], curves[4]["a"]);
    if let Some(cli) = solidcraft() {
        let got = volumes(&cli, &written(&model).0);
        let expected = 40.0 * (3.0 * std::f64::consts::FRAC_PI_2 + 1.0);
        assert_eq!(got.len(), 1);
        assert!(
            (got[0] - expected).abs() / expected < 1e-3,
            "{got:?} vs {expected}"
        );
    }
}

#[test]
fn virtual_relations_are_counted_and_inactive_or_disabled_relations_are_quiet() {
    let mut model = sketch_relation_history(json!({"kind": "disabled"}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketch_constraints"]
        .as_array_mut()
        .unwrap()
        .extend([
            json!({"id": "f3d:model:sketch-constraint#2", "sketch": SKETCH, "virtual_space": true,
            "definition": {"kind": "horizontal", "entity": "f3d:model:sketch-entity#1"}}),
            json!({"id": "f3d:model:sketch-constraint#3", "sketch": SKETCH, "active": false,
            "definition": {"kind": "scalar_equality", "first": 1, "second": 2}}),
        ]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(document["features"][0]["type"], "sketch");
    assert!(document["features"][0]["sketch"]["constraints"]
        .as_array()
        .unwrap()
        .is_empty());
    let loss = report
        .losses()
        .iter()
        .find(|loss| loss.detail.contains("constraints not carried"))
        .unwrap();
    assert!(loss.detail.starts_with("1 of 2 constraints not carried"));
    assert!(loss.detail.contains("virtual sketch space"));
}

// Body features use only synthetic CADIR and the example cube's topology.
fn modifying(definition: Value) -> Model {
    let base = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "f3d:model:feature#3", "ordinal": 2, "name": "Modify",
            "suppressed": false, "definition": definition,
        }));
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    }
}

fn cube_faces(names: &[&str]) -> Value {
    json!({"kind": "faces", "value": names.iter()
        .map(|name| format!("synthetic:cube:face#{name}")).collect::<Vec<_>>()})
}

fn cube_body() -> Value {
    json!({"kind": "bodies", "value": ["synthetic:cube:body#0"]})
}

fn body_mirror_definition() -> Value {
    json!({"definition": "pattern", "seeds": [{"kind": "bodies", "value": cube_body()}],
        "pattern": {"kind": "mirror", "plane_origin": {"x": 20.0, "y": 0.0, "z": 0.0},
            "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}}})
}

#[test]
fn body_mirrors_select_named_bodies_and_keep_the_plane() {
    let (document, report) = convert(
        &modifying(body_mirror_definition()),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let mirror = &document["features"][2];
    assert_eq!(mirror["type"], "mirror", "{}", history_line(&report));
    assert_eq!(mirror["bodies"], document["features"][1]["body_names"]);
    assert_eq!(mirror["features"], json!([]));
    assert_eq!(mirror["combine"], false);
    assert_eq!(mirror["plane"]["plane"]["origin"], json!([20.0, 0.0, 0.0]));
}

fn combined_spheres(operation: &str, keep_tools: bool) -> Model {
    let base = primitive_history(sphere());
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    let model = &mut wire["model"];
    // A second, independent synthetic topology, with its own arena identities.
    for arena in [
        "points", "vertices", "curves", "edges", "coedges", "loops", "surfaces", "faces", "shells",
        "regions", "bodies",
    ] {
        let text = serde_json::to_string(&model[arena]).unwrap();
        let mut rows: Vec<Value> =
            serde_json::from_str(&text.replace("synthetic:cube:", "synthetic:tool:")).unwrap();
        if arena == "bodies" {
            rows[0]["name"] = json!("Tool");
        }
        model[arena].as_array_mut().unwrap().extend(rows);
    }
    let mut sphere = sphere();
    sphere["center"]["x"] = json!(10.0);
    model["features"].as_array_mut().unwrap().extend([
        json!({"id": "f3d:model:feature#3", "ordinal": 1, "name": "Tool ball",
            "suppressed": false, "outputs": ["synthetic:tool:body#0"], "definition": sphere}),
        json!({"id": "f3d:model:feature#4", "ordinal": 2, "name": "Boolean",
            "suppressed": false, "definition": {"definition": "combine",
                "operands": {"target": cube_body(), "tools": {"kind": "bodies", "value": ["synthetic:tool:body#0"]}},
                "op": operation, "keep_tools": keep_tools}}),
    ]);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    }
}

/// The example cube split by a datum plane at `x` across X, then `after` (if any). The split's
/// result bodies are the cube and, when `shifted`, a copy of it moved 10 mm along X: on either
/// side of a plane through the cube's +X face.
fn split_cube(x: f64, shifted: bool, after: Option<Value>) -> Model {
    let base = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    let model = &mut wire["model"];
    let mut outputs = vec![json!("synthetic:cube:body#0")];
    if shifted {
        for arena in [
            "points", "vertices", "curves", "edges", "coedges", "loops", "surfaces", "faces",
            "shells", "regions", "bodies",
        ] {
            let text = serde_json::to_string(&model[arena]).unwrap();
            let mut rows: Vec<Value> =
                serde_json::from_str(&text.replace("synthetic:cube:", "synthetic:piece:")).unwrap();
            if arena == "points" {
                for row in &mut rows {
                    let moved = row["position"]["x"].as_f64().unwrap() + 10.0;
                    row["position"]["x"] = json!(moved);
                }
            }
            model[arena].as_array_mut().unwrap().extend(rows);
        }
        outputs.insert(0, json!("synthetic:piece:body#0"));
    }
    let frame = json!({"origin": {"x": x, "y": 0.0, "z": 0.0},
        "normal": {"x": 1.0, "y": 0.0, "z": 0.0}, "u_axis": {"x": 0.0, "y": 1.0, "z": 0.0}});
    let features = model["features"].as_array_mut().unwrap();
    features.extend([
        json!({"id": "f3d:model:feature#3", "ordinal": 2, "name": "Cutting plane",
            "suppressed": false, "definition": {"definition": "datum_plane", "frame": frame}}),
        json!({"id": "f3d:model:feature#4", "ordinal": 3, "name": "Split",
            "suppressed": false, "outputs": outputs,
            "definition": {"definition": "split_body", "targets": cube_body(),
                "tool": {"kind": "plane", "value": {"plane": "f3d:model:feature#3"}}}}),
    ]);
    if let Some(definition) = after {
        features.push(
            json!({"id": "f3d:model:feature#5", "ordinal": 4, "name": "After",
            "suppressed": false, "definition": definition}),
        );
    }
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    }
}

/// The largest X any of the example cube's points has.
fn cube_right(model: &Model) -> f64 {
    let wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["position"]["x"].as_f64())
        .fold(f64::MIN, f64::max)
}

#[test]
fn a_body_split_by_a_datum_plane_keeps_each_piece_named() {
    let right = cube_right(&split_cube(0.0, false, None));
    // The moved copy lies on the side the normal points to and keeps the cube's name; the
    // cube itself, behind the plane, becomes the new piece. A mirror after the split names
    // the cube's body, so it mirrors the new piece.
    let mirror = json!({"definition": "pattern",
        "seeds": [{"kind": "bodies", "value": cube_body()}],
        "pattern": {"kind": "mirror", "plane_origin": {"x": 40.0, "y": 0.0, "z": 0.0},
            "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}}});
    let (document, report) = convert(
        &split_cube(right, true, Some(mirror)),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let made = document["features"][1]["body_names"][0].as_str().unwrap();
    let split = &document["features"][2];
    assert_eq!(split["type"], "split", "{}", history_line(&report));
    assert_eq!(split["name"], "Split");
    assert_eq!(split["body"], made);
    assert_eq!(split["plane"]["type"], "custom");
    assert_eq!(split["plane"]["plane"]["origin"], json!([right, 0.0, 0.0]));
    let piece = split["body_names"][0].as_str().unwrap();
    assert_ne!(piece, made);
    let mirror = &document["features"][3];
    assert_eq!(mirror["type"], "mirror", "{}", history_line(&report));
    assert_eq!(mirror["bodies"], json!([piece]));
}

#[test]
fn split_pieces_that_cannot_be_told_apart_are_not_selected_later() {
    // The plane cuts through the cube, so its vertices lie on both sides: neither piece keeps
    // the cube's identity, and a later feature selecting it stops the history.
    let model = split_cube(0.0, false, None);
    let middle = cube_right(&model) - 5.0;
    let mirror = json!({"definition": "pattern",
        "seeds": [{"kind": "bodies", "value": cube_body()}],
        "pattern": {"kind": "mirror", "plane_origin": {"x": 40.0, "y": 0.0, "z": 0.0},
            "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}}});
    let (document, report) = convert(
        &split_cube(middle, false, Some(mirror)),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(document["features"][2]["type"], "split");
    assert!(
        history_line(&report).contains("it selects a body that an earlier feature has not named"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn splitting_by_faces_is_refused_by_name() {
    let mut model = split_cube(0.0, false, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let split = wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|f| f["name"] == "Split")
        .unwrap();
    split["definition"]["tool"] = json!({"kind": "faces", "value": cube_faces(&["top"])});
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains("splitting a body by faces is not supported yet"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn solidcraft_splits_the_body_in_two() {
    let Some(cli) = solidcraft() else { return };
    let model = split_cube(0.0, false, None);
    let middle = cube_right(&model) - 4.0;
    let got = volumes(&cli, &written(&split_cube(middle, false, None)).0);
    assert_eq!(got.len(), 2, "{got:?}");
    let mut got = got;
    got.sort_by(f64::total_cmp);
    assert!((got[0] - 400.0).abs() < 1.0, "{got:?}");
    assert!((got[1] - 600.0).abs() < 1.0, "{got:?}");
}

#[test]
fn booleans_map_target_tools_operation_and_retention() {
    for operation in ["join", "cut", "intersect"] {
        for keep_tools in [false, true] {
            let (document, report) = convert(
                &combined_spheres(operation, keep_tools),
                &SolidCraftWriter::new().verification(Verification::Off),
            );
            let combine = &document["features"][2];
            assert_eq!(combine["type"], "combine", "{}", history_line(&report));
            assert_eq!(combine["target"], document["features"][0]["body_names"][0]);
            assert_eq!(combine["tools"], json!(["Tool"]));
            assert_eq!(combine["operation"], operation);
            assert_eq!(combine["keep_tools"], keep_tools);
        }
    }
}

#[test]
fn a_boolean_referencing_a_removed_tool_reports_a_history_blocker() {
    let mut model = combined_spheres("cut", false);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    // Match core's consumed-body filtering: preserve history, remove the tool's topology.
    for (arena, values) in wire["model"].as_object_mut().unwrap() {
        if arena == "features" {
            continue;
        }
        if let Some(rows) = values.as_array_mut() {
            rows.retain(|row| {
                !row["id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("synthetic:tool:"))
            });
        }
    }
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(report.outcome().unwrap().features.len(), 3);
    assert!(
        report.losses().iter().any(|loss| loss
            .detail
            .contains("it selects a body that an earlier feature has not named")),
        "{}",
        history_line(&report)
    );
}

#[test]
fn unsupported_modify_options_explain_what_stops_history() {
    let mut cases = Vec::new();
    // Shell and draft faces exist only on the final body, which the feature has changed.
    cases.push((
        json!({"definition": "shell", "bodies": cube_body(),
        "removed_faces": cube_faces(&["top"]), "thickness": 1.0, "outward": false}),
        "shell requires faces from its own input topology",
    ));
    cases.push((
        json!({"definition": "draft", "faces": cube_faces(&["right"]),
        "anchor": {"kind": "neutral_plane", "plane": cube_faces(&["bottom"]),
            "pull": {"direction": {"x": 0.0, "y": 0.0, "z": 1.0}}},
        "angle": 0.1, "outward": false}),
        "draft waits on input face geometry",
    ));
    cases.push((
        json!({"definition": "mirror_shape", "source": cube_body(),
        "plane_origin": {"x": 20.0, "y": 0.0, "z": 0.0},
        "plane_normal": {"x": 1.0, "y": 0.0, "z": 0.0}}),
        "SolidCraft mirrors keep the original body",
    ));
    cases.push((
        json!({"definition": "cosmetic_thread", "face": cube_faces(&["right"]),
        "diameter": 8.0, "extent": {"kind": "through"}}),
        "cosmetic threads require a resolved diameter and pitch",
    ));
    for family in ["thread", "detailed_thread"] {
        cases.push((
            json!({"definition": "unresolved", "family": family}),
            "threads whose construction is unresolved are not supported yet",
        ));
    }
    cases.push((
        json!({"definition": "combine", "operands": {
        "target": {"kind": "native", "value": "Body1"},
        "tools": {"kind": "native", "value": "Body2"}}, "op": "cut"}),
        "a native body reference is not supported yet",
    ));
    for (definition, reason) in cases {
        let (_, report) = convert(
            &modifying(definition),
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn solidcraft_replays_body_mirror_and_booleans() {
    let Some(cli) = solidcraft() else { return };
    let mirror = volumes(&cli, &written(&modifying(body_mirror_definition())).0);
    assert_eq!(mirror.len(), 2);
    assert!(
        mirror.iter().all(|v| (v - 1000.0).abs() < 1e-6),
        "{mirror:?}"
    );
    // Two radius-5 spheres with centres 5 mm apart; overlap = 625π/12 mm³. SolidCraft measures
    // curved booleans to about one part in ten thousand, so the check is relative, as for spheres.
    let sphere = 500.0 * std::f64::consts::PI / 3.0;
    let overlap = 625.0 * std::f64::consts::PI / 12.0;
    for (operation, expected) in [
        ("join", 2.0 * sphere - overlap),
        ("cut", sphere - overlap),
        ("intersect", overlap),
    ] {
        let volume = volumes(&cli, &written(&combined_spheres(operation, false)).0);
        assert_eq!(volume.len(), 1);
        assert!(
            (volume[0] - expected).abs() / expected < 1e-3,
            "{operation}: {volume:?} vs {expected}"
        );
    }
}

#[test]
fn later_body_mirrors_require_boolean_tools_to_be_retained() {
    for keep_tools in [false, true] {
        let base = combined_spheres("join", keep_tools);
        let mut wire = serde_json::to_value(&base.ir).unwrap();
        let mut mirror = body_mirror_definition();
        mirror["seeds"][0]["value"] = json!({"kind": "resolved", "value": {
            "bodies": ["synthetic:tool:body#0"], "native": "Tool"}});
        wire["model"]["features"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "f3d:model:feature#5", "ordinal": 3, "name": "Mirror tool",
                "suppressed": false, "definition": mirror,
            }));
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..base
        };
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        if keep_tools {
            assert_eq!(
                document["features"][3]["type"],
                "mirror",
                "{}",
                history_line(&report)
            );
            assert_eq!(document["features"][3]["bodies"], json!(["Tool"]));
        } else {
            assert!(history_line(&report).contains("earlier feature has not named"));
        }
    }
}

/// The stored B-rep deliberately stays an unblended cube. Only the feature's own
/// historical input samples locate the two symmetric edges; no final edge id is used.
fn input_blend(kind: &str, dimension: Value, opposite: bool) -> Model {
    blend_on(
        cube_with_history(10.0, first_profile(), None),
        kind,
        dimension,
        opposite,
        0.0,
    )
}

/// The blend of [`input_blend`] on the cube extruded symmetrically about its sketch (Z from
/// -5 to 5 mm): a base no rebuild without SolidCraft knew the edges of.
fn symmetric_blend(kind: &str, dimension: Value, opposite: bool) -> Model {
    let extent = json!({"kind": "symmetric",
        "side": {"termination": {"kind": "blind", "length": 10.0}}});
    blend_on(
        cube_with_history(10.0, first_profile(), Some(extent)),
        kind,
        dimension,
        opposite,
        -5.0,
    )
}

/// `base` with a blend of one of its two symmetric vertical edges, which run from `z0` to
/// `z0 + 10` mm: at (0, 0), or at (10, 10) when `opposite`.
fn blend_on(base: Model, kind: &str, dimension: Value, opposite: bool, z0: f64) -> Model {
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    let mut group = json!({"edges": {"kind": "historical", "value": {
        "state": "f3d:model:feature-input-topology#1",
        "edges": [if opposite { "opposite" } else { "near" }],
        "native": "synthetic input edge",
    }}});
    group[if kind == "fillet" { "radius" } else { "spec" }] = dimension;
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "f3d:model:feature#3", "ordinal": 2, "name": "Edge treatment",
            "suppressed": false, "dependencies": ["f3d:model:feature#2"],
            "outputs": ["synthetic:cube:body#0"],
            "definition": {"definition": kind, "groups": [group]},
        }));
    let sample = |edge: &str, xy: f64| {
        json!({
            "edge": edge, "interior": {"x": xy, "y": xy, "z": z0 + 5.0},
            "tangent": {"x": 0.0, "y": 0.0, "z": 1.0},
            "start": {"x": xy, "y": xy, "z": z0},
            "end": {"x": xy, "y": xy, "z": z0 + 10.0},
            "faces": if edge == "near" { json!(["front", "left"]) } else { json!(["back", "right"]) },
        })
    };
    wire["model"]["feature_input_topologies"] = json!([{
        "id": "f3d:model:feature-input-topology#1", "input_of": "f3d:model:feature#3",
        "edges": ["near", "opposite"], "faces": ["front", "left", "back", "right"],
        "edge_geometry": [sample("near", 0.0), sample("opposite", 10.0)],
    }]);
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    }
}

fn blend_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("fillet", json!({"kind": "constant", "radius": 1.0})),
        ("chamfer", json!({"kind": "distance", "distance": 1.0})),
        (
            "chamfer",
            json!({"kind": "two_distances", "first": 1.0, "second": 1.0}),
        ),
        (
            "chamfer",
            json!({"kind": "distance_angle", "distance": 1.0,
                           "angle": std::f64::consts::FRAC_PI_4}),
        ),
    ]
}

#[cfg(unix)]
/// The edges SolidCraft's `model.edges` reports for a prism: the polygon `ring` (on XY)
/// extruded from `z0` to `z1`.
fn prism_edges(ring: &[[f64; 2]], z0: f64, z1: f64) -> Vec<Value> {
    let edge = |a: [f64; 3], b: [f64; 3]| {
        let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
        let mid = [0, 1, 2].map(|i| (a[i] + b[i]) / 2.0);
        json!({"start": a, "end": b, "mid": mid, "length": length})
    };
    let mut edges = Vec::new();
    for (i, p) in ring.iter().enumerate() {
        let q = ring[(i + 1) % ring.len()];
        edges.push(edge([p[0], p[1], z0], [q[0], q[1], z0]));
        edges.push(edge([q[0], q[1], z1], [p[0], p[1], z1]));
        edges.push(edge([p[0], p[1], z0], [p[0], p[1], z1]));
    }
    edges
}

#[cfg(unix)]
const SQUARE: [[f64; 2]; 4] = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];

/// A stand-in SolidCraft CLI that answers a replay with `bodies` (name, edges), after
/// sleeping `delay` seconds; the folder holding it lives as long as the value.
#[cfg(unix)]
struct FakeCli {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

#[cfg(unix)]
fn fake_cli(bodies: &[(&str, Vec<Value>)], delay: u32) -> FakeCli {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let inspect = json!([{"features": 3, "marker": 2}, {
        "bodies": bodies.iter().map(|(name, _)| json!({"name": name})).collect::<Vec<_>>(),
        "timeline": [],
    }]);
    let mut edges = vec![json!({"features": 3, "marker": 2})];
    edges.extend(
        bodies
            .iter()
            .map(|(name, edges)| json!({"body": name, "edges": edges})),
    );
    std::fs::write(dir.path().join("bodies.json"), inspect.to_string()).unwrap();
    std::fs::write(
        dir.path().join("edges.json"),
        Value::from(edges).to_string(),
    )
    .unwrap();
    let path = dir.path().join("solidcraft-cli");
    let folder = dir.path().display();
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nfor last; do out=\"$last\"; done\n\
             case \"$1\" in --version) echo 'solidcraft-cli 0.19.0'; exit 0;; esac\n\
             sleep {delay}\n\
             if grep -q model.edges \"$2\"; then cat '{folder}/edges.json' > \"$out\"; \
             else cat '{folder}/bodies.json' > \"$out\"; fi\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    FakeCli { _dir: dir, path }
}

/// A writer that finds blend edges with `cli` and checks nothing else.
fn replaying(cli: &Path) -> SolidCraftWriter {
    SolidCraftWriter::new()
        .verification(Verification::Off)
        .replay(Verification::With(cli.to_path_buf()))
}

#[cfg(unix)]
#[test]
fn blends_select_symmetric_edges_by_their_own_input_geometry() {
    for (symmetric, z0) in [(false, 0.0), (true, -5.0)] {
        let cli = fake_cli(&[("unit cube", prism_edges(&SQUARE, z0, z0 + 10.0))], 0);
        for (kind, dimension) in blend_cases() {
            for opposite in [false, true] {
                let model = if symmetric {
                    symmetric_blend(kind, dimension.clone(), opposite)
                } else {
                    input_blend(kind, dimension.clone(), opposite)
                };
                let (document, report) = convert(&model, &replaying(&cli.path));
                assert!(
                    !history_line(&report).starts_with("flattened"),
                    "{report:?}"
                );
                let blend = &document["features"][2];
                assert_eq!(blend["type"], kind);
                assert_eq!(blend["body"], "unit cube");
                let xy = if opposite { 10.0 } else { 0.0 };
                assert_eq!(blend["edges"], json!([[xy, xy, z0 + 5.0]]));
                if kind == "fillet" {
                    assert_eq!(blend["radius"], "1 mm");
                    assert_eq!(blend["style"], json!({"type": "constant"}));
                } else {
                    assert_eq!(blend["distance"], "1 mm");
                    assert!(blend.get("distance2").is_none());
                    assert!(blend.get("angle").is_none());
                }
            }
        }
    }
}

#[test]
fn blends_without_a_solidcraft_to_replay_the_history_are_refused() {
    let model = input_blend("fillet", json!({"kind": "constant", "radius": 1.0}), false);
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_partial(&document, &report, 1);
    assert_eq!(
        types(&document),
        [
            ("sketch".to_owned(), false),
            ("extrude".to_owned(), false),
            ("import".to_owned(), false)
        ]
    );
    assert!(
        history_line(&report).contains(
            "feature \"Edge treatment\": its input edges are found only on SolidCraft's replay \
             of the history before it, and there is none: verification turned off"
        ),
        "{report:?}"
    );
    let missing = Path::new("/nonexistent/solidcraft-cli");
    let (_, report) = convert(&model, &replaying(missing));
    assert!(history_line(&report).contains("cannot run"), "{report:?}");
    // Inspecting says the same, without running anything.
    let mut report = Report::default();
    let outcome = SolidCraftWriter::new()
        .verification(Verification::Off)
        .plan(&model, &mut report);
    assert_eq!(outcome.mode, Mode::PartialHistory);
    let blend = outcome.features.last().unwrap();
    assert!(
        blend
            .reason
            .as_deref()
            .unwrap_or("")
            .contains("there is none"),
        "{blend:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_replay_that_runs_out_of_time_refuses_the_blend() {
    let cli = fake_cli(&[("unit cube", prism_edges(&SQUARE, 0.0, 10.0))], 5);
    let model = input_blend("fillet", json!({"kind": "constant", "radius": 1.0}), false);
    let started = std::time::Instant::now();
    let (document, report) = convert(
        &model,
        &replaying(&cli.path).timeout(std::time::Duration::from_millis(300)),
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    assert_partial(&document, &report, 1);
    assert_eq!(
        types(&document),
        [
            ("sketch".to_owned(), false),
            ("extrude".to_owned(), false),
            ("import".to_owned(), false)
        ]
    );
    assert!(history_line(&report).contains("timed out"), "{report:?}");
}

#[cfg(unix)]
#[test]
fn blends_refuse_missing_foreign_or_ambiguous_input_geometry() {
    let cube = fake_cli(&[("unit cube", prism_edges(&SQUARE, 0.0, 10.0))], 0);
    let base = input_blend("fillet", json!({"kind": "constant", "radius": 1.0}), false);
    let check = |cli: &Path, change: fn(&mut Value), reason: &str| {
        let mut wire = serde_json::to_value(&base.ir).unwrap();
        change(&mut wire["model"]);
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..base.clone()
        };
        let (document, report) = convert(&model, &replaying(cli));
        let types = types(&document);
        assert_partial(&document, &report, 1);
        assert_eq!(document["features"][1]["type"], "extrude");
        assert!(!types.iter().any(|(kind, _)| kind == "fillet"), "{types:?}");
        assert!(history_line(&report).contains(reason), "{report:?}");
    };
    let cli = cube.path.as_path();
    check(
        cli,
        |m| m["feature_input_topologies"] = json!([]),
        "input topology is missing",
    );
    check(
        cli,
        |m| m["feature_input_topologies"][0]["input_of"] = json!("f3d:model:feature#2"),
        "does not uniquely belong",
    );
    check(
        cli,
        |m| m["feature_input_topologies"][0]["edge_geometry"] = json!([]),
        "input edge geometry is missing",
    );
    check(
        cli,
        |m| {
            m["feature_input_topologies"][0]["edge_geometry"][0]
                .as_object_mut()
                .unwrap()
                .remove("tangent");
        },
        "tangent is missing",
    );
    check(
        cli,
        |m| m["feature_input_topologies"][0]["edges"] = json!(["opposite"]),
        "not a unique member",
    );
    check(
        cli,
        |m| {
            m["feature_input_topologies"][0]["edge_geometry"][0]["interior"] =
                json!({"x": 0.0, "y": 0.0, "z": 0.0})
        },
        "cannot be proven",
    );
    // A midpoint on no rebuilt edge, and ends or a tangent other than the rebuilt edge's.
    check(
        cli,
        |m| {
            m["feature_input_topologies"][0]["edge_geometry"][0]["interior"] =
                json!({"x": 5.0, "y": 5.0, "z": 5.0})
        },
        "cannot be proven",
    );
    check(
        cli,
        |m| m["feature_input_topologies"][0]["edge_geometry"][0]["end"]["z"] = json!(9.0),
        "cannot be proven",
    );
    check(
        cli,
        |m| {
            m["feature_input_topologies"][0]["edge_geometry"][0]["tangent"] =
                json!({"x": 1.0, "y": 0.0, "z": 0.0})
        },
        "cannot be proven",
    );
    // A curve that is neither a line nor a circular arc.
    check(
        cli,
        |m| {
            m["feature_input_topologies"][0]["edge_geometry"][0]["interior"] =
                json!({"x": 1.0, "y": 1.0, "z": 4.0})
        },
        "other than circular arcs",
    );
    check(
        cli,
        |m| {
            m["features"][2]["definition"]["groups"][0]["edges"] =
                json!({"kind": "edges", "value": ["synthetic:cube:edge#8"]})
        },
        "require edges from their own feature input topology",
    );
    check(
        cli,
        |m| {
            m["features"][2]["definition"]["groups"][0]["edges"] = json!({
                "kind": "resolved", "value": {
                    "edges": ["synthetic:cube:edge#8"], "native": "synthetic base selection"}})
        },
        "not proven members of an earlier stored feature result",
    );
    check(
        cli,
        |m| {
            m["features"][2]["definition"]["groups"][0]["edges"] =
                json!({"kind": "native", "value": "feature-property:93"})
        },
        "a native edge reference is not supported yet",
    );
    check(
        cli,
        |m| {
            let edges = &mut m["features"][2]["definition"]["groups"][0]["edges"];
            let value = edges["value"].clone();
            *edges = json!({"kind": "historical_partial", "value": {"state": value["state"],
                "edges": [], "unresolved": ["op:1"], "native": value["native"]}})
        },
        "some of its edges are not resolved in its input topology",
    );
    // Two bodies in the same place: the midpoint lies on an edge of each.
    let twins = fake_cli(
        &[
            ("unit cube", prism_edges(&SQUARE, 0.0, 10.0)),
            ("unit cube (2)", prism_edges(&SQUARE, 0.0, 10.0)),
        ],
        0,
    );
    check(twins.path.as_path(), |_| {}, "cannot be proven");
}

#[test]
fn blends_refuse_unsupported_dimensions_and_unordered_reference_faces() {
    for (kind, dimension, reason) in [
        (
            "fillet",
            json!({"kind": "variable", "points": [
            {"parameter": 0.0, "radius": 1.0}, {"parameter": 1.0, "radius": 2.0}]}),
            "proven chain direction",
        ),
        (
            "fillet",
            json!({"kind": "asymmetric", "offset_one": 1.0, "offset_two": 2.0}),
            "other than constant-radius",
        ),
        (
            "chamfer",
            json!({"kind": "two_distances", "first": 1.0, "second": 2.0}),
            "input reference face ordering",
        ),
        (
            "chamfer",
            json!({"kind": "distance_angle", "distance": 1.0, "angle": 0.6}),
            "input reference face ordering",
        ),
    ] {
        let (_, report) = convert(
            &input_blend(kind, dimension, false),
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(history_line(&report).contains(reason), "{report:?}");
    }
}

/// Full SolidCraft measurements, including topology and spatial information.
fn blend_measurement(cli: &Path, input: &Path, output: &Path) -> Value {
    let status = std::process::Command::new(cli)
        .arg("eval")
        .arg(input)
        .arg("--json-out")
        .arg(output)
        .status()
        .unwrap();
    assert!(status.success());
    let measured: Value = serde_json::from_str(&std::fs::read_to_string(output).unwrap()).unwrap();
    for entry in measured["timeline"].as_array().unwrap() {
        assert!(entry["error"].is_null(), "{entry}");
    }
    assert_eq!(measured["bodies"].as_array().unwrap().len(), 1);
    measured["bodies"][0].clone()
}

/// The blend found on SolidCraft's own replay of the history before it, on the one-sided and
/// on the symmetric cube, against the same blend made directly in SolidCraft: same volume,
/// bounding box, face count and centre of mass, the last telling the symmetric edges apart.
#[test]
fn solidcraft_blends_the_input_edge_and_matches_the_expected_body() {
    let Some(cli) = solidcraft() else { return };
    let dir = tempfile::tempdir().unwrap();
    for (symmetric, z0) in [(false, 0.0), (true, -5.0)] {
        for (kind, dimension) in blend_cases() {
            for opposite in [false, true] {
                let model = if symmetric {
                    symmetric_blend(kind, dimension.clone(), opposite)
                } else {
                    input_blend(kind, dimension.clone(), opposite)
                };
                let (bytes, report) = written_with(&model, &replaying(&cli));
                assert!(
                    !history_line(&report).starts_with("flattened"),
                    "{report:?}"
                );
                let actual_path = dir.path().join("actual.solidcraft");
                std::fs::write(&actual_path, bytes).unwrap();
                let actual = blend_measurement(&cli, &actual_path, &dir.path().join("actual.json"));
                let xy = if opposite { 10.0 } else { 0.0 };
                let mut params = json!({"edges": [[xy, xy, z0 + 5.0]], "body": "Reference"});
                if kind == "fillet" {
                    params["radius"] = json!("1 mm");
                } else {
                    params["distance"] = json!("1 mm");
                    match dimension["kind"].as_str().unwrap() {
                        "two_distances" => params["distance2"] = json!("1 mm"),
                        "distance_angle" => params["angle"] = json!("45 deg"),
                        _ => {}
                    }
                }
                let script = json!([
                    {"command": "solid.box", "params": {"length": "10 mm", "width": "10 mm",
                        "height": "10 mm", "corner": [0, 0, z0], "body_name": "Reference"}},
                    {"command": format!("solid.{kind}"), "params": params},
                ]);
                let expected_path = dir.path().join("expected-script.json");
                std::fs::write(&expected_path, script.to_string()).unwrap();
                let expected =
                    blend_measurement(&cli, &expected_path, &dir.path().join("expected.json"));
                let volume = actual["volume_mm3"].as_f64().unwrap();
                let wanted = expected["volume_mm3"].as_f64().unwrap();
                assert!((volume - wanted).abs() / wanted < dutchman_solidcraft::VOLUME_TOLERANCE);
                let analytic = if kind == "fillet" {
                    1000.0 - 10.0 * (1.0 - std::f64::consts::FRAC_PI_4)
                } else {
                    995.0
                };
                assert!((volume - analytic).abs() < 0.05, "{actual}");
                assert_eq!(
                    actual["faces"].as_u64().unwrap(),
                    expected["faces"].as_u64().unwrap()
                );
                assert_eq!(actual["faces"].as_u64().unwrap(), 7);
                for axis in 0..3 {
                    for corner in ["min", "max"] {
                        assert!(
                            (actual["bbox"][corner][axis].as_f64().unwrap()
                                - expected["bbox"][corner][axis].as_f64().unwrap())
                            .abs()
                                < 1e-6
                        );
                    }
                    assert!(
                        (actual["center_of_mass"][axis].as_f64().unwrap()
                            - expected["center_of_mass"][axis].as_f64().unwrap())
                        .abs()
                            < 1e-5
                    );
                }
                // Volume, bounding box and face count are identical for these symmetric
                // alternatives. The centre of mass proves which corner lost material.
                for axis in 0..2 {
                    let center = actual["center_of_mass"][axis].as_f64().unwrap();
                    assert!(
                        if opposite {
                            center < 4.999
                        } else {
                            center > 5.001
                        },
                        "{actual}"
                    );
                }
            }
        }
    }
}

/// Two bodies where the cube is, made by extruding its profile twice: SolidCraft's replay
/// gives each the edge the blend's midpoint is on, so the blend is refused.
#[test]
fn solidcraft_refuses_a_blend_whose_midpoint_lies_on_two_rebuilt_edges() {
    let Some(cli) = solidcraft() else { return };
    let base = input_blend("fillet", json!({"kind": "constant", "radius": 1.0}), false);
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    let model = &mut wire["model"];
    let mut earlier = model["features"][1].clone();
    earlier["id"] = json!("f3d:model:feature#4");
    earlier["ordinal"] = json!(3);
    model["features"].as_array_mut().unwrap().push(earlier);
    model["features"][2]["ordinal"] = json!(4);
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    };
    let (_, report) = written_with(&model, &replaying(&cli));
    assert!(
        history_line(&report).contains("cannot be proven"),
        "{report:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_distance_angle_chamfer_requires_perpendicular_input_faces() {
    let base = input_blend(
        "chamfer",
        json!({"kind": "distance_angle", "distance": 1.0,
               "angle": std::f64::consts::FRAC_PI_4}),
        false,
    );
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    // A parallelogram has the same selected vertical edge, but oblique support faces.
    let entities = &mut wire["model"]["sketch_entities"];
    entities[1]["geometry"]["end"]["u"] = json!(20.0);
    entities[2]["geometry"]["start"]["u"] = json!(20.0);
    entities[2]["geometry"]["end"]["u"] = json!(10.0);
    entities[3]["geometry"]["start"]["u"] = json!(10.0);
    wire["model"]["sketch_constraints"] = json!([]);
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base
    };
    let parallelogram = [[0.0, 0.0], [10.0, 0.0], [20.0, 10.0], [10.0, 10.0]];
    let cli = fake_cli(&[("unit cube", prism_edges(&parallelogram, 0.0, 10.0))], 0);
    let (_, report) = convert(&model, &replaying(&cli.path));
    assert!(
        history_line(&report).contains("non-perpendicular input faces"),
        "{report:?}"
    );
}

#[cfg(unix)]
#[test]
fn a_fillet_of_tangency_weight_one_maps_as_a_circular_fillet() {
    let base = input_blend("fillet", json!({"kind": "constant", "radius": 1.0}), false);
    let cli = fake_cli(&[("unit cube", prism_edges(&SQUARE, 0.0, 10.0))], 0);
    let writer = replaying(&cli.path);
    for (weight, maps) in [(1.0, true), (0.999, false), (2.0, false)] {
        let mut wire = serde_json::to_value(&base.ir).unwrap();
        wire["model"]["features"][2]["definition"]["groups"][0]["tangency_weight"] = json!(weight);
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..base.clone()
        };
        let (document, report) = convert(&model, &writer);
        let noted = report.losses().iter().any(|loss| {
            loss.severity == Severity::Info
                && loss.detail == "tangency weight 1.0 treated as a circular fillet"
        });
        if maps {
            assert_eq!(document["features"][2]["type"], "fillet", "{report:?}");
            assert_eq!(document["features"][2]["radius"], "1 mm");
            assert_eq!(
                document["features"][2]["style"],
                json!({"type": "constant"})
            );
            assert!(noted, "{report:?}");
        } else {
            assert!(
                history_line(&report)
                    .contains("fillets with a tangency weight are not supported yet"),
                "{weight}: {report:?}"
            );
            assert!(!noted, "{report:?}");
        }
    }
}

#[cfg(unix)]
#[test]
fn blends_keep_multiple_picks_and_groups_and_refuse_repeated_geometry_and_weights() {
    let base = input_blend("fillet", json!({"kind": "constant", "radius": 1.0}), false);
    let mut wire = serde_json::to_value(&base.ir).unwrap();
    wire["model"]["features"][2]["definition"]["groups"][0]["edges"]["value"]["edges"] =
        json!(["near", "opposite"]);
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base.clone()
    };
    let cli = fake_cli(&[("unit cube", prism_edges(&SQUARE, 0.0, 10.0))], 0);
    let writer = replaying(&cli.path);
    let (document, _) = convert(&model, &writer);
    assert_eq!(
        document["features"][2]["edges"],
        json!([[0.0, 0.0, 5.0], [10.0, 10.0, 5.0]])
    );
    assert_eq!(document["features"][2]["body"], "unit cube");
    let sample = wire["model"]["feature_input_topologies"][0]["edge_geometry"][0].clone();
    wire["model"]["feature_input_topologies"][0]["edge_geometry"][1] = sample;
    wire["model"]["feature_input_topologies"][0]["edge_geometry"][1]["edge"] = json!("opposite");
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..base.clone()
    };
    let (_, report) = convert(&model, &writer);
    assert!(
        history_line(&report).contains("repeats a rebuilt edge"),
        "{report:?}"
    );
    for multiple_groups in [false, true] {
        let mut wire = serde_json::to_value(&base.ir).unwrap();
        let groups = &mut wire["model"]["features"][2]["definition"]["groups"];
        if multiple_groups {
            let group = groups[0].clone();
            groups.as_array_mut().unwrap().push(group);
        } else {
            groups[0]["tangency_weight"] = json!(0.5);
        }
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..base.clone()
        };
        let (_, report) = convert(&model, &writer);
        if multiple_groups {
            // Several radius groups map, one blend per group.
            assert!(
                !history_line(&report).contains("independently dimensioned groups"),
                "{report:?}"
            );
        } else {
            assert!(
                history_line(&report).contains("tangency weight"),
                "{report:?}"
            );
        }
    }
}

#[test]
fn signed_axis_dimensions_keep_their_expression_and_drive_positive_magnitudes() {
    for (kind, edge, axis) in [
        ("horizontal_distance", 1, "x"),
        ("vertical_distance", 2, "y"),
    ] {
        let mut model = cube_with_history(10.0, first_profile(), None);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let m = &mut wire["model"];
        let id = format!("f3d:model:sketch-entity#{edge}");
        m["sketch_constraints"][4]["definition"] = json!({
            "kind": kind,
            "first": {"kind": "end", "entity": id},
            "second": {"kind": "start", "entity": id},
            "parameter": "f3d:model:parameter#2",
        });
        m["sketch_entities"].as_array_mut().unwrap().push(entity(
            5,
            json!({"kind": "point", "position": {"u": 0.0, "v": 0.0}}),
        ));
        m["sketch_constraints"].as_array_mut().unwrap().push(json!({
            "id": "f3d:model:sketch-constraint#9", "sketch": SKETCH,
            "definition": {"kind": "fixed", "entity": "f3d:model:sketch-entity#5"},
        }));
        m["parameters"][1]["expression"] = json!("-span");
        m["parameters"][1]["value"]["value"] = json!(-10.0);
        m["parameters"][1]["dependencies"] = json!(["f3d:model:parameter#3"]);
        m["parameters"].as_array_mut().unwrap().push(json!({
            "id": "f3d:model:parameter#3", "name": "span", "expression": "10 mm",
            "value": {"kind": "length", "value": 10.0},
        }));
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, _) = convert(&model, &SolidCraftWriter::new());
        let dimension = &document["features"][0]["sketch"]["constraints"][4];
        assert_eq!(dimension["type"], format!("distance_{axis}"));
        assert_eq!(dimension["value"], 10.0);
        assert_eq!(dimension["p"], if edge == 1 { 2 } else { 3 });
        assert_eq!(dimension["q"], if edge == 1 { 1 } else { 2 });
        assert_ne!(dimension["param"], "side");
        let params = document["params"].as_array().unwrap();
        assert_eq!(
            params.iter().find(|p| p["name"] == "side").unwrap()["expr"],
            "-span"
        );
        assert_eq!(
            params
                .iter()
                .find(|p| p["name"] == dimension["param"])
                .unwrap()["expr"],
            "-(side)"
        );
        let Some(cli) = solidcraft() else { continue };
        let (bytes, _) = written(&model);
        assert_eq!(volumes(&cli, &bytes), [1000.0]);
        let edited = evaluated(&cli, &with_param(&bytes, "span", "20 mm"));
        assert_eq!(edited["bodies"][0]["volume_mm3"], 4000.0);
        assert_eq!(edited["bodies"][0]["bbox"]["min"], json!([0.0, 0.0, 0.0]));
        assert_eq!(
            edited["bodies"][0]["bbox"]["max"],
            json!([20.0, 20.0, 10.0])
        );
    }
}

#[test]
fn used_datum_helpers_keep_the_resolved_dependent_sketch_and_replay() {
    let base = cube_with_history(10.0, first_profile(), None);
    for definition in [
        json!({"definition": "datum_point", "position": {"x": 0.0, "y": 0.0, "z": 0.0}}),
        json!({"definition": "datum_coordinate_system", "frame": {
            "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
            "x_axis": {"x": 1.0, "y": 0.0, "z": 0.0},
            "y_axis": {"x": 0.0, "y": 1.0, "z": 0.0},
            "z_axis": {"x": 0.0, "y": 0.0, "z": 1.0}}}),
    ] {
        let mut wire = serde_json::to_value(&base.ir).unwrap();
        let features = wire["model"]["features"].as_array_mut().unwrap();
        for feature in features.iter_mut() {
            feature["ordinal"] = json!(feature["ordinal"].as_u64().unwrap() + 1);
        }
        features[0]["dependencies"] = json!(["f3d:model:feature#datum"]);
        features.insert(
            0,
            json!({"id": "f3d:model:feature#datum", "ordinal": 0,
            "name": "Attachment", "suppressed": false, "definition": definition}),
        );
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..base.clone()
        };
        let (document, report) = convert(&model, &SolidCraftWriter::default());
        assert_eq!(
            types(&document),
            [
                ("sketch".to_owned(), false),
                ("extrude".to_owned(), false),
                ("import".to_owned(), true)
            ]
        );
        assert!(report
            .losses()
            .iter()
            .any(|loss| loss.detail.contains("its resolved placement is carried")));
        if std::env::var_os(CLI_ENV).is_some() {
            assert_eq!(report.outcome().unwrap().check, Check::Verified);
        }
    }
}

#[test]
fn freecad_cc0_negative_distance_replays_without_flattening() {
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/P09a.FCStd");
    let model = dutchman_core::read(
        dutchman_core::Format::FreeCadDocument,
        &input,
        &mut Report::default(),
    )
    .unwrap();
    let (document, _) = convert(&model, &SolidCraftWriter::new());
    let sketch = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Sketch001")
        .unwrap();
    let constraints = sketch["sketch"]["constraints"].as_array().unwrap();
    let dimension = constraints
        .iter()
        .find(|c| c["type"] == "distance_x")
        .unwrap();
    assert_eq!(dimension["value"], 2.5);
    let magnitude = document["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == dimension["param"])
        .unwrap();
    assert_eq!(magnitude["expr"], "-(Constraint9)");
    let Some(cli) = solidcraft() else { return };
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::With(cli.clone())),
    );
    assert!(
        history_line(&report).contains("verified against the original"),
        "{}",
        history_line(&report)
    );
    let (bytes, _) = written(&model);
    let actual = volumes(&cli, &bytes);
    assert_eq!(actual.len(), 1);
    assert!((actual[0] - 6250.0).abs() < 1e-6);
}

#[test]
fn helper_info_survives_mapping_and_replay_fallbacks() {
    for blocked in [true, false] {
        let base = cube_with_history(if blocked { 10.0 } else { 20.0 }, first_profile(), None);
        let mut wire = serde_json::to_value(&base.ir).unwrap();
        let features = wire["model"]["features"].as_array_mut().unwrap();
        features.push(json!({"id": "f3d:model:feature#unused", "ordinal": 4,
            "name": "Unused point", "suppressed": false, "definition": {
                "definition": "datum_point", "position": {"x": 0.0, "y": 0.0, "z": 0.0}}}));
        if blocked {
            features.push(json!({"id": "f3d:model:feature#unknown", "ordinal": 3,
                "name": "Later", "suppressed": false, "definition": {"definition": "native", "kind": "Mystery"}}));
        }
        let model = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..base
        };
        let (_, report) = convert(&model, &SolidCraftWriter::default());
        assert!(report
            .losses()
            .iter()
            .any(|loss| loss.severity == Severity::Info
                && loss.detail == "helper not converted: nothing uses it"));
        if blocked || std::env::var_os(CLI_ENV).is_some() {
            assert!(matches!(report.outcome().unwrap().check, Check::Partial(_)));
        }
    }
}

/// A valid extrusion, then a blocker, a resolved sketch, and a later extrusion.
fn partial_history_model() -> Model {
    let mut model = reference_sketch_history();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][1]["ordinal"] = json!(3);
    wire["model"]["features"][2]["ordinal"] = json!(2);
    let cube = cube_with_history(10.0, first_profile(), None);
    let cube = serde_json::to_value(&cube.ir).unwrap();
    let mut extrude = cube["model"]["features"][1].clone();
    extrude["id"] = json!("f3d:model:feature#4");
    extrude["ordinal"] = json!(1);
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(extrude.clone());
    extrude["id"] = json!("f3d:model:feature#5");
    extrude["ordinal"] = json!(4);
    extrude["name"] = json!("Later extrusion");
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(extrude);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

fn assert_partial(document: &Value, report: &Report, sketches: usize) {
    assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
    assert!(matches!(report.outcome().unwrap().check, Check::Partial(_)));
    let features = document["features"].as_array().unwrap();
    let original = features.last().unwrap();
    assert_eq!(original["type"], "import");
    assert_eq!(original["suppressed"], false);
    assert!(original.get("component").is_none());
    let component = &document["components"][0]["id"];
    assert!(features
        .iter()
        .take(features.len() - 1)
        .all(|f| &f["component"] == component && f["suppressed"] == false));
    assert_eq!(
        document["visibility"]["hidden_occurrences"],
        json!([[document["occurrences"][0]["id"]]])
    );
    assert_eq!(
        features.iter().filter(|f| f["type"] == "sketch").count(),
        sketches
    );
    assert!(report.losses().iter().any(|l| l.subject == "history"
        && l.severity == Severity::Degraded
        && l.detail.contains("imported final shape")));
    let ids: std::collections::BTreeSet<_> =
        features.iter().map(|f| f["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.len(), features.len());
    assert!(ids
        .iter()
        .all(|id| *id < document["next_id"].as_u64().unwrap()));
}

#[test]
fn partial_history_appearance_bindings_reach_the_visible_original() {
    for named_prefix in [false, true] {
        let mut model = partial_history_model();
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let body = wire["model"]["bodies"][0]["id"].clone();
        if !named_prefix {
            for feature in wire["model"]["features"].as_array_mut().unwrap() {
                if feature["definition"]["definition"] == "extrude" {
                    feature["outputs"] = json!([]);
                }
            }
        }
        wire["model"]["appearances"] = json!([
            {"id":"synthetic:test:appearance#red", "base_color":{"r":1.0,"g":0.0,"b":0.0,"a":1.0}},
            {"id":"synthetic:test:appearance#blue", "base_color":{"r":0.0,"g":0.0,"b":1.0,"a":1.0}}
        ]);
        wire["model"]["appearance_bindings"] = json!([
            {"id":"synthetic:test:binding#body", "target":{"kind":"body","id":body},"appearance":"synthetic:test:appearance#red"},
            {"id":"synthetic:test:binding#face", "target":{"kind":"face","id":"synthetic:cube:face#top"},"appearance":"synthetic:test:appearance#blue"}
        ]);
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (doc, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert_partial(&doc, &report, 2);
        let original = doc["features"].as_array().unwrap().last().unwrap();
        let name = original["body_names"][0].as_str().unwrap();
        assert_eq!(
            doc["appearances"]["bodies"][name]["color"],
            json!([255, 0, 0])
        );
        let face = doc["appearances"]["faces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|face| face["body"] == name)
            .unwrap();
        assert_eq!(face["look"]["color"], json!([0, 0, 255]));
        assert_eq!(name.contains("Original geometry"), named_prefix);
        assert!(
            !report
                .losses()
                .iter()
                .any(|loss| loss.subject == "appearance"),
            "{:?}",
            report.losses()
        );
        if let Some(cli) = solidcraft() {
            let dir = tempfile::tempdir().unwrap();
            let design = dir.path().join("partial.solidcraft");
            let script = dir.path().join("looks.json");
            let output = dir.path().join("looks.out.json");
            std::fs::write(
                &design,
                written_with(
                    &model,
                    &SolidCraftWriter::new().verification(Verification::Off),
                )
                .0,
            )
            .unwrap();
            std::fs::write(
                &script,
                json!([
                    {"command":"document.inspect","params":{"measure":false}},
                    {"command":"appearance.list","params":{"body":name}}
                ])
                .to_string(),
            )
            .unwrap();
            assert!(std::process::Command::new(cli)
                .arg("run")
                .arg(&script)
                .arg("--in")
                .arg(&design)
                .arg("--results")
                .arg("--json-out")
                .arg(&output)
                .status()
                .unwrap()
                .success());
            let results: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
            let bodies = results[0]["bodies"].as_array().unwrap();
            assert_eq!(bodies.len(), 2, "{results}");
            assert!(
                bodies
                    .iter()
                    .any(|b| b["name"] == name && b["visible"] == true),
                "{results}"
            );
            assert_eq!(
                results[1]["body"]["appearance"]["color"], "#ff0000",
                "{results}"
            );
            // SolidCraft 0.18 lists every face; newer releases list only faces with their own
            // look. Either way exactly one face is blue.
            let faces = results[1]["body"]["faces"].as_array().unwrap();
            let blue = faces
                .iter()
                .filter(|face| face["appearance"]["color"] == "#0000ff")
                .count();
            assert_eq!(blue, 1, "{results}");
            assert!(
                faces.iter().all(|face| matches!(
                    face["appearance"]["color"].as_str(),
                    Some("#0000ff" | "#ff0000")
                )),
                "{results}"
            );
        }
    }
}

fn with_later_blocker(mut model: Model) -> Model {
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "f3d:model:feature#99", "ordinal": 99,
            "name": "Later blocker", "suppressed": false,
            "definition": {"definition": "native", "kind": "Mystery"}
        }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn partial_entry_recesses_keep_the_target_in_parts_and_assemblies() {
    for version in ["0.18.2", "0.25.1"] {
        for form in [
            json!({"kind": "counterbore", "diameter": 8.0, "depth": 2.0}),
            json!({"kind": "countersink", "diameter": 8.0, "angle": std::f64::consts::FRAC_PI_2}),
        ] {
            let part = with_later_blocker(holed_cube(4.0, |hole| {
                hole["shape"]["construction"]["kind"] = form;
                hole["extent"] = json!({"kind": "blind", "length": 6.0});
                hole["bottom"] = json!({"kind": "flat"});
            }));
            let mut assembly = assembly_of_histories();
            assembly.assembly.as_mut().unwrap().parts[1].ir = part.ir.clone();
            for model in [&part, &assembly] {
                let writer = SolidCraftWriter::new()
                    .verification(Verification::Auto)
                    .target_version(Some(version.parse().unwrap()));
                let (document, report) = convert(model, &writer);
                assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
                let Check::Partial(why) = &report.outcome().unwrap().check else {
                    panic!("{report:?}");
                };
                if solidcraft().is_some() {
                    assert!(why.contains("retained features replay"), "{why}");
                }
                let hole = document["features"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|feature| feature["type"] == "hole")
                    .unwrap();
                let tip = &hole["hole"]["tip_angle"];
                if version
                    .parse::<dutchman_solidcraft::SolidCraftVersion>()
                    .unwrap()
                    >= dutchman_solidcraft::SolidCraftVersion::V0_22_1
                {
                    assert_eq!(tip, "180");
                } else {
                    assert!(tip.is_null());
                }
                let component = &hole["component"];
                assert!(document["components"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| &c["id"] == component && c["name"] == "Editable history (partial)"));
            }
        }
    }
}

#[test]
fn partial_helices_keep_the_target_in_parts_and_assemblies() {
    let replay = helix_cli().map_or(Verification::Off, Verification::With);
    let part = with_later_blocker(helical_history(|_| {}));
    let mut assembly = assembly_of_histories();
    assembly.assembly.as_mut().unwrap().parts[1].ir = part.ir.clone();
    for model in [&part, &assembly] {
        for version in ["0.18.2", "0.25.1"] {
            let writer = SolidCraftWriter::new()
                .verification(replay.clone())
                .target_version(Some(version.parse().unwrap()));
            let (document, report) = convert(model, &writer);
            let coils: Vec<_> = document["features"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|f| f["type"] == "coil")
                .collect();
            if version == "0.18.2" {
                assert!(coils.is_empty());
                assert!(report.outcome().unwrap().features.iter().any(|f| f
                    .reason
                    .as_deref()
                    .is_some_and(|why| why.contains("needs SolidCraft 0.25 patch 1"))));
            } else {
                assert_eq!(coils.len(), 1);
                assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
                let Check::Partial(why) = &report.outcome().unwrap().check else {
                    panic!("{report:?}");
                };
                if helix_cli().is_some() {
                    assert!(why.contains("retained features replay"), "{why}");
                }
                assert!(document["components"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["id"] == coils[0]["component"]
                        && c["name"] == "Editable history (partial)"));
            }
        }
    }
}

#[test]
fn a_blocker_keeps_the_prefix_and_later_resolved_sketches() {
    let model = partial_history_model();
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, report) = convert(&model, &writer);
    assert_partial(&document, &report, 2);
    assert_eq!(
        types(&document),
        [
            ("sketch".to_owned(), false),
            ("extrude".to_owned(), false),
            ("sketch".to_owned(), false),
            ("import".to_owned(), false)
        ]
    );
    let outcome = report.outcome().unwrap();
    assert_eq!(
        outcome
            .features
            .iter()
            .filter(|f| f.status == dutchman_core::FeatureStatus::Mapped)
            .count(),
        3
    );
    assert_eq!(
        outcome.features.last().unwrap().status,
        dutchman_core::FeatureStatus::Blocked
    );
    assert_eq!(
        writer.plan(&model, &mut Report::default()).mode,
        Mode::PartialHistory
    );
    let (import, _) = convert(&model, &writer.import_only(true));
    assert_eq!(
        document["features"].as_array().unwrap().last().unwrap()["step_blob"],
        import["features"][0]["step_blob"]
    );
}

#[test]
fn partial_history_keeps_a_sketch_without_a_timeline_row() {
    let mut model = partial_history_model();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .retain(|feature| feature["ordinal"] != 3);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_partial(&document, &report, 2);
    assert_eq!(document["features"][1]["type"], "extrude");
    assert_eq!(
        report
            .outcome()
            .unwrap()
            .features
            .iter()
            .filter(|feature| feature.kind == "sketch")
            .count(),
        1
    );
}

#[test]
fn partial_history_is_not_required_to_keep_sketches_without_timeline_rows() {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"] = json!([]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, report) = convert(&model, &writer);
    assert_reference_sketches(&document, 1);
    assert_eq!(report.outcome().unwrap().mode, Mode::Import);
    assert!(matches!(
        report.outcome().unwrap().check,
        Check::NotApplicable(_)
    ));
    wire["model"]["sketches"][0]["placement"] = json!({"kind": "unresolved"});
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = convert(&model, &writer);
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("sketch left out") && loss.detail.contains("plane")));
}

#[test]
fn a_later_unresolved_sketch_is_reported_without_losing_the_prefix() {
    let mut model = partial_history_model();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["sketches"][1]["placement"] = json!({"kind": "unresolved"});
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_partial(&document, &report, 1);
    assert!(report
        .losses()
        .iter()
        .any(|l| l.detail.contains("sketch left out") && l.detail.contains("plane")));
}

#[test]
fn partial_history_replays_beside_the_unchanged_original() {
    let Some(cli) = solidcraft() else { return };
    let model = partial_history_model();
    let writer = SolidCraftWriter::new().verification(Verification::With(cli.clone()));
    let (document, report) = convert(&model, &writer);
    assert_partial(&document, &report, 2);
    assert!(history_line(&report).contains("retained features replay in SolidCraft"));
    // eval includes hidden bodies: one editable 1000 mm³ cube and the original cube.
    let (bytes, _) = written_with(&model, &writer);
    assert_eq!(volumes(&cli, &bytes), [1000.0, 1000.0]);
}

#[test]
fn cc0_freecad_subshape_binder_keeps_the_earlier_pad() {
    if !dutchman_core::Format::FreeCadDocument.can_convert() {
        return;
    }
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/F16b.FCStd");
    let mut report = Report::default();
    let model =
        dutchman_core::read(dutchman_core::Format::FreeCadDocument, &input, &mut report).unwrap();
    let writer = SolidCraftWriter::new();
    let (document, report) = convert(&model, &writer);
    assert_partial(&document, &report, 2);
    let features = document["features"].as_array().unwrap();
    assert_eq!(
        features.iter().filter(|f| f["type"] == "extrude").count(),
        1
    );
    assert!(history_line(&report).contains("DatumReference"));
    if let Some(cli) = solidcraft() {
        assert!(history_line(&report).contains("retained features replay in SolidCraft"));
        let (bytes, _) = written_with(&model, &writer);
        let mut measured = volumes(&cli, &bytes);
        measured.sort_by(f64::total_cmp);
        // The hidden prefix is 6000 mm³. The visible original is two solids,
        // 3000 + 6000 mm³, as the recipe's CC0 manifest records.
        assert_eq!(measured.len(), 3);
        for (actual, expected) in measured.iter().zip([3000.0, 6000.0, 6000.0]) {
            assert!((actual - expected).abs() < 1e-6, "{measured:?}");
        }
        let dir = tempfile::tempdir().unwrap();
        let design = dir.path().join("F16b.solidcraft");
        let script = dir.path().join("visibility.json");
        let output = dir.path().join("visibility.out.json");
        std::fs::write(&design, &bytes).unwrap();
        std::fs::write(
            &script,
            json!([
                {"command": "document.inspect", "params": {"measure": true}},
                {"command": "inspect.silhouette", "params": {}}
            ])
            .to_string(),
        )
        .unwrap();
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
        let bodies = results[0]["bodies"].as_array().unwrap();
        assert_eq!(bodies.len(), 3);
        let visible: Vec<_> = bodies.iter().filter(|b| b["visible"] == true).collect();
        assert_eq!(visible.len(), 2);
        assert_eq!(
            visible
                .iter()
                .map(|b| b["volume_mm3"].as_f64().unwrap())
                .sum::<f64>(),
            9000.0
        );
        assert!(visible
            .iter()
            .all(|b| b["name"].as_str().unwrap().starts_with("F16b")));
        let hidden: Vec<_> = bodies.iter().filter(|b| b["visible"] == false).collect();
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0]["name"], "Body1");
        assert_eq!(hidden[0]["volume_mm3"], 6000.0);
        assert_eq!(results[1]["bodies"], 2);
    }
}

#[test]
fn a_partial_component_is_hidden_in_every_assembly_placement() {
    let mut model = assembly_of_histories();
    let second = &mut model.assembly.as_mut().unwrap().parts[1];
    second.ir = partial_history_model().ir;
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, report) = convert(&model, &writer);
    assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
    assert!(matches!(report.outcome().unwrap().check, Check::Partial(_)));
    let components = document["components"].as_array().unwrap();
    let second = &components.iter().find(|c| c["name"] == "Second").unwrap()["id"];
    let partial = &components
        .iter()
        .find(|c| c["name"] == "Editable history (partial)")
        .unwrap()["id"];
    let features = document["features"].as_array().unwrap();
    let prefix: Vec<_> = features
        .iter()
        .filter(|f| &f["component"] == partial)
        .collect();
    assert_eq!(prefix.len(), 3);
    assert!(prefix.iter().any(|f| f["type"] == "extrude"));
    assert!(prefix.iter().all(|f| f["suppressed"] == false));
    let original = features
        .iter()
        .find(|f| &f["component"] == second && f["type"] == "import")
        .unwrap();
    assert_eq!(original["suppressed"], false);
    let hidden = document["visibility"]["hidden_occurrences"]
        .as_array()
        .unwrap();
    assert_eq!(hidden.len(), 2);
    let occurrences = document["occurrences"].as_array().unwrap();
    for path in hidden {
        assert_eq!(path.as_array().unwrap().len(), 2);
        assert_eq!(
            occurrences.iter().find(|o| o["id"] == path[0]).unwrap()["component"],
            *second
        );
        assert_eq!(
            occurrences.iter().find(|o| o["id"] == path[1]).unwrap()["component"],
            *partial
        );
    }
    let mut ids: Vec<_> = features
        .iter()
        .chain(components)
        .chain(occurrences)
        .map(|v| v["id"].as_u64().unwrap())
        .collect();
    let count = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count);
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.detail == "2 editable-history component placement(s) are placed hidden"));
    assert!(!report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("the source hides")));
    assert_eq!(document["timeline_groups"].as_array().unwrap().len(), 2);

    // Source visibility and retained-history visibility share the same path builder,
    // while their report counts retain their distinct meanings.
    model.assembly.as_mut().unwrap().placements[1].hidden = true;
    let (document, report) = convert(&model, &writer);
    assert_eq!(
        document["visibility"]["hidden_occurrences"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    for detail in [
        "1 component placement(s) the source hides are placed hidden",
        "2 editable-history component placement(s) are placed hidden",
    ] {
        assert!(report.losses().iter().any(|loss| loss.detail == detail));
    }
}

#[test]
fn partial_assembly_history_keeps_reference_sketches_without_a_body_prefix() {
    let mut model = assembly_of_histories();
    model.assembly.as_mut().unwrap().parts[1].ir = reference_sketch_history().ir;
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, report) = convert(&model, &writer);
    let component = &document["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "Second")
        .unwrap()["id"];
    let features: Vec<_> = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| &f["component"] == component)
        .collect();
    assert_eq!(
        features
            .iter()
            .map(|f| f["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["sketch", "sketch", "import"]
    );
    assert_eq!(features.last().unwrap()["suppressed"], false);
    let ids: Vec<_> = features
        .iter()
        .filter(|f| f["type"] == "sketch")
        .map(|f| f["id"].clone())
        .collect();
    assert_eq!(document["visibility"]["hidden_sketches"], json!(ids));
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.subject.contains("Second.ipt")
            && loss.detail.contains("kept 2 sketches for reference")));
    if let Some(cli) = solidcraft() {
        let (bytes, _) = written_with(&model, &writer);
        assert_eq!(volumes(&cli, &bytes), [1000.0, 1000.0, 1000.0]);
    }
}

#[test]
fn a_partial_component_is_hidden_through_repeated_subassemblies() {
    let mut inner = assembly_of_histories();
    inner.assembly.as_mut().unwrap().parts[1].ir = partial_history_model().ir;
    let mut model = assembly_of_histories();
    let assembly = model.assembly.as_mut().unwrap();
    assembly.parts = vec![inner];
    for placement in &mut assembly.placements {
        placement.part = 0;
    }
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (document, report) = convert(&model, &writer);
    assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
    let hidden = document["visibility"]["hidden_occurrences"]
        .as_array()
        .unwrap();
    // Three subassembly placements, each with two placements of the partial part.
    assert_eq!(hidden.len(), 6);
    let occurrences = document["occurrences"].as_array().unwrap();
    let components = document["components"].as_array().unwrap();
    for path in hidden {
        let path = path.as_array().unwrap();
        assert_eq!(path.len(), 3);
        let mut parent = json!(0);
        for id in path {
            let occurrence = occurrences.iter().find(|o| &o["id"] == id).unwrap();
            assert_eq!(occurrence["parent"], parent);
            parent = occurrence["component"].clone();
        }
        assert_eq!(
            components.iter().find(|c| c["id"] == parent).unwrap()["name"],
            "Editable history (partial)"
        );
    }
}

#[test]
fn a_later_sketch_that_fails_replay_does_not_discard_the_valid_body_prefix() {
    let Some(cli) = solidcraft() else { return };
    for (bound, blocked) in [(true, true), (false, true), (false, false)] {
        let mut model = partial_history_model();
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        if !bound {
            wire["model"]["features"]
                .as_array_mut()
                .unwrap()
                .retain(|feature| feature["ordinal"] != 3);
        }
        if !blocked {
            wire["model"]["features"]
                .as_array_mut()
                .unwrap()
                .retain(|feature| feature["ordinal"].as_u64().unwrap() <= 1);
        }
        wire["model"]["parameters"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "f3d:model:parameter#99", "name": "bad_dimension", "expression": "-5 mm",
                "value": {"kind": "length", "value": -5.0}
            }));
        wire["model"]["sketch_constraints"].as_array_mut().unwrap().push(json!({
        "id": "f3d:model:sketch-constraint#99", "sketch": TOP,
        "definition": {"kind": "distance", "entities": ["f3d:model:sketch-entity#11"], "parameter": "f3d:model:parameter#99"}
    }));
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::With(cli.clone())),
        );
        assert_partial(&document, &report, 2);
        assert_eq!(document["features"][1]["type"], "extrude");
        assert_eq!(document["features"][2]["sketch"]["constraints"], json!([]));
        if bound {
            assert!(
                report
                    .losses()
                    .iter()
                    .any(|loss| loss.subject.contains("Top sketch")
                        && loss.detail.contains("not carried after replay failed")),
                "{report:?}"
            );
        } else {
            let line = history_line(&report);
            if blocked {
                assert!(line.contains("native is not mapped yet"), "{line}");
            }
            assert!(line.contains("not carried after replay failed"), "{line}");
        }
    }
}

#[test]
fn partial_history_is_independent_of_the_native_source_format() {
    for extension in ["FCStd", "f3d", "ipt", "sldprt"] {
        let mut model = partial_history_model();
        model.source_file = Some(format!("cube.{extension}"));
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert_partial(&document, &report, 2);
        assert_eq!(
            document["features"].as_array().unwrap().last().unwrap()["file"],
            format!("cube.{extension}")
        );
        assert_eq!(document["features"][1]["type"], "extrude");
    }
}

/// CC0 recipe geometry with the named sketch binding restored in the neutral model.
/// cadmpeg currently exposes the spreadsheet and pad formulas but omits the named
/// sketch constraint expression. This isolates the writer's contract from that decoder gap.
fn recipe_with_neutral_sheet_binding(recipe: &str, sheet: &str, alias: &str) -> Model {
    let input = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "tests/fixtures/recipe-{recipe}-freecad-0.21.2.FCStd"
    ));
    let mut model = dutchman_core::read(
        dutchman_core::Format::FreeCadDocument,
        &input,
        &mut Report::default(),
    )
    .unwrap();
    let cell = model
        .ir
        .model
        .parameters
        .iter()
        .find(|p| p.name == alias)
        .unwrap()
        .id
        .clone();
    let dimension = model
        .ir
        .model
        .parameters
        .iter_mut()
        .find(|p| {
            p.properties
                .get("source_name")
                .is_some_and(|name| name == "width")
        })
        .unwrap();
    dimension.expression = format!("{sheet}.{alias}");
    dimension.dependencies = vec![cell].try_into().unwrap();
    model
}

#[test]
fn a_cc0_sheet_edit_drives_geometry_with_a_synthetic_neutral_sketch_binding() {
    let Some(cli) = solidcraft() else { return };
    let (recipe, sheet, alias, edit, expected) = ("N11a", "Dimensions", "w", "45 mm", 9000.0);
    let model = recipe_with_neutral_sheet_binding(recipe, sheet, alias);
    let (bytes, report) = written_with(
        &model,
        &SolidCraftWriter::new().verification(Verification::With(cli.clone())),
    );
    assert_eq!(
        report.outcome().unwrap().check,
        Check::Verified,
        "{recipe}: {:?}",
        report.losses()
    );
    assert_eq!(volumes(&cli, &bytes), [6000.0], "{recipe}");
    let actual = volumes(&cli, &with_param(&bytes, alias, edit));
    assert_eq!(actual.len(), 1, "{recipe}");
    assert!(
        (actual[0] - expected).abs() < 0.01,
        "{recipe}: {actual:?}, expected {expected}"
    );
}

#[test]
fn spreadsheet_function_expressions_evaluate_in_solidcraft() {
    let Some(cli) = solidcraft() else { return };
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    // The sketch's side parameter drives two dimensions, so doubling it quadruples volume.
    let side = wire["model"]["parameters"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|p| p["name"] == "side")
        .unwrap();
    side["expression"] =
        json!("min(max(20 mm, abs(-30 mm)), sqrt(400) * 1 mm) * sin(30) + 0 * pi * 1 mm");
    side.as_object_mut().unwrap().remove("value");
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let bytes = written(&model).0;
    let initial = volumes(&cli, &bytes);
    assert_eq!(initial.len(), 1);
    assert!((initial[0] - 1000.0).abs() < 0.01, "{initial:?}");
    let grown = with_param(&bytes, "side", "max(20 mm, 1 in)");
    let actual = volumes(&cli, &grown);
    assert_eq!(actual.len(), 1);
    assert!((actual[0] - 25.4 * 25.4 * 10.0).abs() < 0.01, "{actual:?}");
}

#[test]
fn cc0_spreadsheet_formulas_keep_their_decoded_parameter_dependencies() {
    let input = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/recipe-P12a-freecad-0.21.2.FCStd");
    let model = dutchman_core::read(
        dutchman_core::Format::FreeCadDocument,
        &input,
        &mut Report::default(),
    )
    .unwrap();
    let (doc, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let params = doc["params"].as_array().unwrap();
    assert!(params
        .iter()
        .any(|p| p["name"] == "plate_w" && p["expr"] == "20 mm" && p["model"] == false));
    assert!(params
        .iter()
        .any(|p| p["name"] == "Length" && p["expr"] == "plate_w / 2"));
    assert!(!params.iter().any(|p| p["name"] == "A1"));
    assert!(
        !report
            .losses()
            .iter()
            .any(|loss| loss.subject.starts_with("parameter ") && loss.severity != Severity::Info),
        "{:?}",
        report.losses()
    );
}

#[test]
fn sketch_choice_keeps_required_profiles_but_not_reference_sketches() {
    let writer = SolidCraftWriter::new()
        .sketches(false)
        .verification(Verification::Off);
    for before_blocker in [false, true] {
        let mut model = partial_history_model();
        if before_blocker {
            let mut wire = serde_json::to_value(&model.ir).unwrap();
            wire["model"]["features"][1]["ordinal"] = json!(2);
            wire["model"]["features"][2]["ordinal"] = json!(3);
            model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        }
        let (document, report) = convert(&model, &writer);
        assert_partial(&document, &report, 1);
        assert_eq!(
            types(&document),
            [
                ("sketch".to_owned(), false),
                ("extrude".to_owned(), false),
                ("import".to_owned(), false)
            ]
        );
    }
    for model in [loft_history(10.0, |_| {}), sweep_history(|_| {})] {
        let (document, _) = convert(&model, &writer);
        assert_eq!(
            document["features"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|f| f["type"] == "sketch")
                .count(),
            2,
            "section, profile and path sketches stay"
        );
    }
}

/// The example cube (10 mm, built by the base extrude) with one more extrude that takes its
/// top (`up`) or bottom face as it was before that extrude: a historical face, known only by
/// the geometry of the edges around it. `change` edits the extrude's definition.
fn face_extrude(up: bool, change: impl FnOnce(&mut Value)) -> Model {
    let model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let z = if up { 10.0 } else { 0.0 };
    let point = |x: f64, y: f64| json!({"x": x, "y": y, "z": z});
    let state = "test:face:feature-input#3";
    let face = "test:face:historical-face#target";
    let side = |k: usize| format!("test:face:historical-face#side{k}");
    let corners = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
    let geometry: Vec<Value> = (0..4)
        .map(|k| {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            let tangent = [(b.0 - a.0) / 10.0, (b.1 - a.1) / 10.0];
            json!({
                "edge": format!("test:face:historical-edge#{k}"),
                "interior": point((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                "start": point(a.0, a.1), "end": point(b.0, b.1),
                "tangent": {"x": tangent[0], "y": tangent[1], "z": 0.0},
                "faces": [face, side(k)],
            })
        })
        .collect();
    let edges: Vec<String> = (0..4)
        .map(|k| format!("test:face:historical-edge#{k}"))
        .collect();
    let mut faces: Vec<String> = (0..4).map(side).collect();
    faces.push(face.to_owned());
    wire["model"]["feature_input_topologies"] = json!([{
        "id": state, "input_of": "f3d:model:feature#3", "edges": edges, "faces": faces,
        "edge_geometry": geometry,
    }]);
    let mut definition = json!({
        "definition": "extrude",
        "profile": {"kind": "historical_faces", "value": {
            "state": state, "faces": [face], "native": ["synthetic profile group"],
        }},
        "direction": {"kind": "profile_normal"},
        "start": {"kind": "profile_plane"},
        "extent": {"kind": "one_sided", "side": {"termination": {"kind": "blind", "length": 5.0}}},
        "op": "join",
    });
    change(&mut definition);
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "f3d:model:feature#3", "ordinal": 2, "name": "Face extrude",
            "suppressed": false, "dependencies": ["f3d:model:feature#2"], "definition": definition,
        }));
    Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    }
}

/// The example cube with a delete-face feature whose face is the top of the cube as it was
/// before the feature, or a native selection.
fn face_delete(selection: impl FnOnce(&str, &str) -> Value, heal: bool) -> Model {
    face_extrude(true, |definition| {
        *definition = json!({
            "definition": "delete_face",
            "faces": selection("test:face:feature-input#3", "test:face:historical-face#target"),
            "heal": heal,
        });
    })
}

#[test]
fn a_historical_face_is_deleted_by_a_point_inside_it() {
    for heal in [true, false] {
        let model = face_delete(
            |state, face| {
                json!({"kind": "historical", "value": {
                    "state": state, "faces": [face], "native": "synthetic selection"}})
            },
            heal,
        );
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let feature = document["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["type"] == "delete_face")
            .unwrap_or_else(|| panic!("{}", history_line(&report)));
        assert_eq!(feature["heal"], json!(heal));
        let points = feature["faces"].as_array().unwrap();
        assert_eq!(points.len(), 1);
        let p: Vec<f64> = points[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert!((p[2] - 10.0).abs() < 1e-9, "{p:?}");
        assert!(
            p[0] > 1.0 && p[0] < 9.0 && p[1] > 1.0 && p[1] < 9.0,
            "a point well inside the top face: {p:?}"
        );
    }
}

fn face_move(motion: Value) -> Model {
    face_extrude(true, |definition| {
        *definition = json!({
            "definition": "move_face",
            "faces": {"kind": "historical", "value": {
                "state": "test:face:feature-input#3",
                "faces": ["test:face:historical-face#target"], "native": "synthetic selection"}},
            "motion": motion,
        });
    })
}

#[test]
fn a_face_moved_along_its_normal_becomes_an_offset_face_at_a_point_inside_it() {
    let model = face_move(json!({"kind": "offset", "distance": 2.5}));
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let feature = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "offset_face")
        .unwrap_or_else(|| panic!("{}", history_line(&report)));
    assert_eq!(feature["distance"], json!("2.5 mm"));
    let points = feature["faces"].as_array().unwrap();
    assert_eq!(points.len(), 1);
    let p: Vec<f64> = points[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert!((p[2] - 10.0).abs() < 1e-9, "{p:?}");
    assert!(
        p[0] > 1.0 && p[0] < 9.0 && p[1] > 1.0 && p[1] < 9.0,
        "{p:?}"
    );
}

#[test]
fn rotated_and_slid_faces_are_refused_for_their_motion() {
    let rotate = json!({"kind": "rotate",
        "axis_origin": {"x": 0.0, "y": 0.0, "z": 0.0},
        "axis_dir": {"x": 1.0, "y": 0.0, "z": 0.0}, "angle": 0.5});
    let slide = json!({"kind": "translate",
        "direction": {"x": 0.0, "y": 0.0, "z": 1.0}, "distance": 2.0});
    for (motion, reason) in [
        (rotate, "rotating faces is not supported yet"),
        (
            slide,
            "translating faces along a direction is not supported yet",
        ),
    ] {
        let (_, report) = convert(
            &face_move(motion),
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(
            history_line(&report).contains(reason),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn a_native_face_deletion_is_refused_for_its_selection() {
    let model = face_delete(|_, _| json!({"kind": "native", "value": "synthetic"}), true);
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains("a native face reference is not supported yet"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn a_historical_face_extrude_without_a_replay_waits_for_one() {
    let (_, report) = convert(
        &face_extrude(true, |_| {}),
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains("replay of the history before it"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn a_historical_face_without_edge_geometry_is_refused_for_that_reason() {
    let model = face_extrude(true, |_| {});
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["feature_input_topologies"][0]["edge_geometry"] = json!([]);
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    };
    let (_, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert!(
        history_line(&report).contains("carries no edge geometry for the selected face"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn solidcraft_extrudes_a_historical_face_along_its_outward_normal() {
    let Some(cli) = solidcraft() else { return };
    let join = |model: Model| {
        let (bytes, report) = written_with(&model, &replaying(&cli));
        assert!(
            history_line(&report).starts_with("3 of 3 features rebuilt"),
            "{}",
            history_line(&report)
        );
        volumes(&cli, &bytes)
    };
    // Out of the top, and out of the bottom: the sketch is on the outward side either way.
    for up in [true, false] {
        let volume = join(face_extrude(up, |_| {}));
        assert_eq!(volume.len(), 1);
        assert!((volume[0] - 1500.0).abs() < 1e-3, "{up}: {volume:?}");
    }
    // Against the profile normal, a cut goes into the cube.
    for up in [true, false] {
        let volume = join(face_extrude(up, |definition| {
            definition["op"] = json!("cut");
            definition["direction"] = json!({"kind": "reversed_profile_normal"});
            definition["extent"]["side"]["termination"]["length"] = json!(3.0);
        }));
        assert!((volume[0] - 700.0).abs() < 1e-3, "{up}: {volume:?}");
    }
}

#[test]
fn sketch_entity_distances_keep_expressions_driving_geometry() {
    for operands in [
        json!(["f3d:model:sketch-entity#1", "f3d:model:sketch-entity#3"]),
        json!(["f3d:model:sketch-entity#5", "f3d:model:sketch-entity#3"]),
        json!(["f3d:model:sketch-entity#3", "f3d:model:sketch-entity#5"]),
        json!(["f3d:model:sketch-entity#5", "f3d:model:sketch-entity#6"]),
        json!(["f3d:model:sketch-entity#7", "f3d:model:sketch-entity#5"]),
    ] {
        let mut model = cube_with_history(10.0, first_profile(), None);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let m = &mut wire["model"];
        m["sketch_entities"].as_array_mut().unwrap().extend([
            entity(
                5,
                json!({"kind": "point", "position": {"u": 0.0, "v": 0.0}}),
            ),
            entity(
                6,
                json!({"kind": "point", "position": {"u": 10.0, "v": 0.0}}),
            ),
        ]);
        let mut carrier = line(7, [0.0, 20.0], [0.0, 10.0]);
        carrier["construction"] = json!(true);
        m["sketch_entities"].as_array_mut().unwrap().push(carrier);
        m["sketch_constraints"][4]["definition"]["entities"] = operands;
        m["parameters"][1]["expression"] = json!("Width / 2");
        m["parameters"][1]["dependencies"] = json!(["f3d:model:parameter#3"]);
        m["parameters"].as_array_mut().unwrap().push(json!({
            "id": "f3d:model:parameter#3", "name": "Width", "expression": "20 mm",
            "value": {"kind": "length", "value": 20.0}
        }));
        model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
        let (document, _) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(document["params"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "side" && p["expr"] == "Width / 2"));
        assert!(document["features"][0]["sketch"]["constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["param"] == "side"));
        if let Some(cli) = solidcraft() {
            let bytes = written(&model).0;
            assert!((volumes(&cli, &bytes)[0] - 1000.0).abs() < 0.01);
            let edited = volumes(&cli, &with_param(&bytes, "Width", "30 mm"));
            assert!((edited[0] - 2250.0).abs() < 0.01, "{edited:?}");
        }
    }
}

/// `cube_with_history` with the extrude's own parameter `d1` (an AlongDistance, as the source
/// kind names it) reading a user parameter `Height`.
fn cube_with_height_parameter(sign: f64) -> Model {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let parameters = wire["model"]["parameters"].as_array_mut().unwrap();
    parameters[0]["expression"] = json!("Height");
    parameters[0]["value"]["value"] = json!(sign * 10.0);
    parameters[0]["dependencies"] = json!(["f3d:model:parameter#3"]);
    parameters[0]["properties"] = json!({"source_kind": "AlongDistance"});
    parameters.push(json!({
        "id": "f3d:model:parameter#3", "name": "Height", "expression": "10 mm",
        "value": {"kind": "length", "value": 10.0}
    }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn an_extrude_distance_follows_the_parameter_that_sets_it() {
    for sign in [1.0, -1.0] {
        let model = cube_with_height_parameter(sign);
        let (document, _) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let distance = document["features"][1]["extent"]["distance"]
            .as_str()
            .unwrap();
        assert_eq!(distance, if sign > 0.0 { "d1" } else { "abs(d1)" });
        if let Some(cli) = solidcraft() {
            let bytes = written(&model).0;
            assert!((volumes(&cli, &bytes)[0] - 1000.0).abs() < 0.01);
            let edited = volumes(&cli, &with_param(&bytes, "Height", "20 mm"));
            assert!((edited[0] - 2000.0).abs() < 0.01, "{edited:?}");
        }
    }
}

#[test]
fn an_extrude_distance_keeps_its_value_when_no_single_parameter_matches() {
    let mut model = cube_with_height_parameter(1.0);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["parameters"][0]["value"]["value"] = json!(7.0);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(document["features"][1]["extent"]["distance"], "10 mm");
}

#[test]
fn a_partial_revolve_angle_follows_the_parameter_that_sets_it() {
    let mut model = ring_history(json!({"kind": "one_sided",
        "termination": {"kind": "angle", "angle": std::f64::consts::FRAC_PI_2}}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["parameters"] = json!([
        {"id": "f3d:model:parameter#1", "owner": "f3d:model:feature#2", "name": "d1",
         "expression": "Turn", "dependencies": ["f3d:model:parameter#2"],
         "value": {"kind": "angle", "value": std::f64::consts::FRAC_PI_2},
         "properties": {"source_kind": "AlongAngle"}},
        {"id": "f3d:model:parameter#2", "name": "Turn", "expression": "90 deg",
         "value": {"kind": "angle", "value": std::f64::consts::FRAC_PI_2}},
    ]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let angle = document["features"][1]["angle"].as_str().unwrap();
    assert!(angle == "d1" || angle == "-(d1)", "{angle}");
    if let Some(cli) = solidcraft() {
        let bytes = written(&model).0;
        let ring = std::f64::consts::PI * (10.0_f64.powi(2) - 5.0_f64.powi(2)) * 10.0;
        let quarter = volumes(&cli, &bytes)[0];
        assert!((quarter - ring / 4.0).abs() / ring < 1e-3, "{quarter}");
        let half = volumes(&cli, &with_param(&bytes, "Turn", "180 deg"))[0];
        assert!((half - ring / 2.0).abs() / ring < 1e-3, "{half}");
    }
}

#[test]
fn selecting_nested_loops_leaves_the_inner_one_a_hole() {
    let mut model = nested_native_profile(&[5.0, 3.0]);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][1]["definition"]["profile"] =
        json!({"kind": "sketch_profiles", "value": {"sketch": SKETCH, "profiles": [1, 0]}});
    // End where the synthetic final cube has no face: this test is about the written profile,
    // not the end-cap comparison.
    wire["model"]["features"][1]["definition"]["extent"]["side"]["termination"]["length"] =
        json!(5.0);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(
        document["features"][1]["profiles"],
        json!({"type": "whole_boundaries", "profiles": [{"outer": ["c1"], "holes": [["c2"]]}]})
    );
    if let Some(cli) = solidcraft() {
        let volume = volumes(&cli, &written(&model).0);
        let expected = std::f64::consts::PI * 16.0 * 5.0;
        assert!((volume[0] - expected).abs() / expected < 1e-3, "{volume:?}");
    }
}

#[test]
fn a_loose_corner_is_held_so_a_parameter_edit_moves_only_what_it_drives() {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    // The square is drawn away from the origin and nothing pins where: its corner floats.
    for entity in wire["model"]["sketch_entities"].as_array_mut().unwrap() {
        for end in ["start", "end"] {
            for axis in ["u", "v"] {
                let at = entity["geometry"][end][axis].as_f64().unwrap();
                entity["geometry"][end][axis] = json!(at + 5.0);
            }
        }
    }
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let constraints = document["features"][0]["sketch"]["constraints"]
        .as_array()
        .unwrap();
    let holds: Vec<&Value> = constraints
        .iter()
        .filter(|c| {
            matches!(c["type"].as_str(), Some("distance_x" | "distance_y"))
                && c.get("param").is_none()
        })
        .collect();
    assert_eq!(holds.len(), 2, "{constraints:?}");
    assert!(holds.iter().all(|c| c["value"].as_f64().unwrap() >= 5.0));
    if let Some(cli) = solidcraft() {
        let bytes = written(&model).0;
        let edited = volumes(&cli, &with_param(&bytes, "side", "20 mm"));
        assert!((edited[0] - 4000.0).abs() < 0.01, "{edited:?}");
    }
}

/// A block, profiled on XZ and extruded, for a sketch on its end: the profile is 10 wide and
/// `Height` tall with its top left corner tied to the sketch origin by a point entity there.
fn block_with_height_parameter() -> Model {
    let mut model = cube_with_history(10.0, first_profile(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    m["sketches"][0]["placement"]["frame"]["normal"] = json!({"x": 0.0, "y": 1.0, "z": 0.0});
    // On XZ, sketch v runs along -Z.
    m["sketch_entities"] = json!([
        line(1, [0.0, 0.0], [10.0, 0.0]),
        line(2, [10.0, 0.0], [10.0, -10.0]),
        line(3, [10.0, -10.0], [0.0, -10.0]),
        line(4, [0.0, -10.0], [0.0, 0.0]),
        entity(
            11,
            json!({"kind": "point", "position": {"u": 0.0, "v": 0.0}})
        ),
    ]);
    let constraint = |key: u32, definition: Value| {
        json!({"id": format!("f3d:model:sketch-constraint#{key}"), "sketch": SKETCH,
               "definition": definition})
    };
    let at = |key: u32| format!("f3d:model:sketch-entity#{key}");
    let locus = |kind: &str, key: u32| json!({"kind": kind, "entity": at(key)});
    m["sketch_constraints"] = json!([
        constraint(1, json!({"kind": "horizontal", "entity": at(1)})),
        constraint(2, json!({"kind": "vertical", "entity": at(2)})),
        constraint(3, json!({"kind": "horizontal", "entity": at(3)})),
        constraint(4, json!({"kind": "vertical", "entity": at(4)})),
        constraint(
            5,
            json!({"kind": "distance", "entities": [at(1), at(3)],
            "parameter": "f3d:model:parameter#2"})
        ),
        constraint(
            6,
            json!({"kind": "coincident_loci",
            "loci": [locus("start", 1), locus("entity", 11)]})
        ),
        constraint(
            7,
            json!({"kind": "coincident_loci",
            "loci": [locus("end", 4), locus("entity", 11)]})
        ),
    ]);
    m["parameters"] = json!([
        {"id": "f3d:model:parameter#2", "owner": "f3d:model:feature#1", "ordinal": 0,
         "name": "d2", "expression": "Height", "dependencies": ["f3d:model:parameter#3"],
         "value": {"kind": "length", "value": 10.0}},
        {"id": "f3d:model:parameter#3", "name": "Height", "expression": "10 mm",
         "value": {"kind": "length", "value": 10.0}},
    ]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn a_sketch_on_the_end_of_a_feature_moves_with_the_dimensions_that_place_it() {
    let mut model = block_with_height_parameter();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let m = &mut wire["model"];
    let pocket = "f3d:model:sketch#2";
    // The pocket sketch lies at z = Height, drawn with its axes turned half a turn.
    m["sketches"].as_array_mut().unwrap().push(json!({
        "id": pocket, "name": "Pocket",
        "placement": {"kind": "resolved", "frame": {
            "origin": {"x": 0.0, "y": 0.0, "z": 10.0},
            "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
            "u_axis": {"x": -1.0, "y": 0.0, "z": 0.0},
        }},
        "profiles": [[{"entity": "f3d:model:sketch-entity#10"}]],
    }));
    let mut circle = entity(
        10,
        json!({"kind": "circle", "center": {"u": -4.0, "v": -3.0}, "radius": 1.0}),
    );
    circle["sketch"] = json!(pocket);
    m["sketch_entities"].as_array_mut().unwrap().push(circle);
    m["features"].as_array_mut().unwrap().push(json!({
        "id": "f3d:model:feature#3", "ordinal": 2, "name": "Pocket sketch", "suppressed": false,
        "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": pocket}},
    }));
    m["features"].as_array_mut().unwrap().push(json!({
        "id": "f3d:model:feature#4", "ordinal": 3, "name": "Pocket", "suppressed": false,
        "dependencies": ["f3d:model:feature#2", "f3d:model:feature#3"],
        "definition": {
            "definition": "extrude",
            "profile": {"kind": "sketch_profiles", "value": {"sketch": pocket, "profiles": [0]}},
            "direction": {"kind": "reversed_profile_normal"},
            "start": {"kind": "profile_plane"},
            "extent": {"kind": "one_sided",
                "side": {"termination": {"kind": "blind", "length": 2.0}}},
            "op": "cut",
        },
    }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, _) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let sketch = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Pocket sketch")
        .unwrap();
    assert_eq!(sketch["plane"]["type"], "offset", "{sketch}");
    assert_eq!(sketch["plane"]["base"]["name"], "XY");
    let distance = sketch["plane"]["distance"].as_str().unwrap();
    assert!(distance.contains("d2"), "{distance}");
    // Turned half a turn, the circle's centre keeps its place on the model.
    assert_eq!(sketch["sketch"]["points"][1]["pos"], json!([4.0, 3.0]));
    if let Some(cli) = solidcraft() {
        // Block 10 x 10 x 10 less the 2 mm deep pocket of radius 1: and the pocket still
        // opens on the top face after the block is made taller.
        let pocket_volume = std::f64::consts::PI * 2.0;
        let bytes = written(&model).0;
        let first = volumes(&cli, &bytes);
        assert!(
            (first[0] - (1000.0 - pocket_volume)).abs() < 0.05,
            "{first:?}"
        );
        let taller = volumes(&cli, &with_param(&bytes, "Height", "20 mm"));
        assert!(
            (taller[0] - (2000.0 - pocket_volume)).abs() < 0.05,
            "{taller:?}"
        );
    }
}

#[test]
fn an_empty_base_region_does_not_consume_an_import_name() {
    let mut model = based_cube();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let body = wire["model"]["bodies"][0]["id"].clone();
    let empty = json!({"id": "test:model:region#empty", "body": body, "shells": []});
    wire["model"]["regions"]
        .as_array_mut()
        .unwrap()
        .insert(0, empty);
    wire["model"]["bodies"][0]["regions"]
        .as_array_mut()
        .unwrap()
        .insert(0, json!("test:model:region#empty"));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(
        document["features"][0]["name"],
        "Base Feature 1",
        "{}",
        history_line(&report)
    );
    assert_eq!(
        document["features"][0]["body_names"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(document["features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["type"] == "hole"));
    assert!(!history_line(&report).contains("appearance targets"));
}

#[test]
fn an_omitted_base_region_does_not_discard_history_or_shift_body_colours() {
    let mut model = based_cube();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let body = wire["model"]["bodies"][0]["id"].clone();
    // A nonempty region whose shell is unavailable: the exporter omits it.
    wire["model"]["regions"].as_array_mut().unwrap().insert(0,
        json!({"id": "test:model:region#omitted", "body": body, "shells": ["test:model:shell#missing"]}));
    wire["model"]["bodies"][0]["regions"]
        .as_array_mut()
        .unwrap()
        .insert(0, json!("test:model:region#omitted"));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (document, report) = convert(
        &model,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let base = &document["features"][0];
    assert_eq!(base["name"], "Base Feature 1", "{}", history_line(&report));
    assert!(base.get("body_names").is_none());
    assert!(document["features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["type"] == "hole"));
}

#[test]
fn an_offset_datum_with_independent_sketch_consumers_does_not_stop_history() {
    let model = cube_with_history(10.0, whole_sketch(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    for feature in wire["model"]["features"].as_array_mut().unwrap() {
        feature["ordinal"] = json!(feature["ordinal"].as_u64().unwrap() + 1);
    }
    wire["model"]["features"][0]["dependencies"] = json!(["test:model:feature#offset"]);
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!({
        "id": "test:model:feature#offset", "ordinal": 0, "name": "Offset datum", "suppressed": false,
        "definition": {"definition": "datum_offset_plane", "distance": -7.0}}));
    let changed = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    };
    let (document, report) = convert(
        &changed,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let outcome = report.outcome().unwrap();
    assert_eq!(
        outcome.features[0].status,
        dutchman_core::FeatureStatus::NotNeeded,
        "{}",
        history_line(&report)
    );
    assert!(document["features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["type"] == "extrude"));
    let mut unresolved = serde_json::to_value(&changed.ir).unwrap();
    unresolved["model"]["sketches"][0]["placement"] = json!({"kind": "unresolved"});
    let changed = Model {
        ir: cadmpeg_ir::CadIr::from_json(&unresolved.to_string()).unwrap(),
        ..changed
    };
    let (_, report) = convert(
        &changed,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    assert_eq!(
        report.outcome().unwrap().features[0].status,
        dutchman_core::FeatureStatus::Blocked
    );
    assert!(
        history_line(&report).contains("offset plane reference is not resolved"),
        "{}",
        history_line(&report)
    );
}

#[test]
fn a_body_split_uses_the_signed_offset_datum_frame() {
    let model = split_cube(0.0, false, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][2]["definition"] = json!({"definition": "datum_offset_plane",
        "reference": {"reference": "resolved_plane", "frame": {
            "origin": {"x": 9.0, "y": 0.0, "z": 0.0},
            "normal": {"x": 2.0, "y": 0.0, "z": 0.0},
            "u_axis": {"x": 0.0, "y": 3.0, "z": 0.0}}}, "distance": -5.0});
    let changed = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    };
    let (document, report) = convert(
        &changed,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let split = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "split")
        .unwrap_or_else(|| panic!("{}", history_line(&report)));
    assert_eq!(split["plane"]["plane"]["origin"], json!([4.0, 0.0, 0.0]));
    if let Some(cli) = solidcraft() {
        let mut got = volumes(
            &cli,
            &written_with(
                &changed,
                &SolidCraftWriter::new().verification(Verification::Off),
            )
            .0,
        );
        got.sort_by(f64::total_cmp);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(
            (got[0] - 400.0).abs() < 1.0 && (got[1] - 600.0).abs() < 1.0,
            "{got:?}"
        );
    }
}

fn historical_shell(outward: bool) -> Model {
    face_extrude(true, |definition| {
        *definition = json!({"definition": "shell", "removed_faces": {
            "kind": "historical", "value": {"state": "test:face:feature-input#3",
                "faces": ["test:face:historical-face#target"], "native": "synthetic selection"}},
            "thickness": 1.0, "outward": outward});
    })
}

#[test]
fn shells_map_historical_input_faces_and_signed_direction() {
    for outward in [false, true] {
        let model = historical_shell(outward);
        let (document, report) = convert(
            &model,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let shell = document["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["type"] == "shell")
            .unwrap_or_else(|| panic!("{}", history_line(&report)));
        assert_eq!(
            shell["direction"],
            if outward { "outside" } else { "inside" }
        );
        assert_eq!(shell["thickness"], "1 mm");
        assert_eq!(shell["faces"].as_array().unwrap().len(), 1);
        assert_eq!(shell["faces"][0][2], 10.0);
        assert!(shell["body"].is_string());
        if let Some(cli) = solidcraft() {
            let got = volumes(
                &cli,
                &written_with(
                    &model,
                    &SolidCraftWriter::new().verification(Verification::Off),
                )
                .0,
            );
            let expected = if outward {
                12.0 * 12.0 * 11.0 - 1000.0
            } else {
                1000.0 - 8.0 * 8.0 * 9.0
            };
            assert_eq!(got.len(), 1, "{got:?}");
            assert!((got[0] - expected).abs() < 1.0, "{got:?} vs {expected}");
        }
    }
}

#[test]
fn shells_require_their_own_historical_input_faces() {
    let model = historical_shell(false);
    let original = serde_json::to_value(&model.ir).unwrap();
    for missing_geometry in [true, false] {
        let mut wire = original.clone();
        if missing_geometry {
            wire["model"]["feature_input_topologies"][0]["edge_geometry"] = json!([]);
        } else {
            wire["model"]["feature_input_topologies"][0]["input_of"] = json!("f3d:model:feature#2");
        }
        let changed = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model.clone()
        };
        let (document, report) = convert(
            &changed,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        assert!(!document["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["type"] == "shell"));
        assert!(
            history_line(&report).contains(if missing_geometry {
                "no edge geometry"
            } else {
                "does not uniquely belong"
            }),
            "{}",
            history_line(&report)
        );
    }
}

#[test]
fn a_body_the_source_leaves_unnamed_gets_a_name_later_features_can_target() {
    let model = cube_with_history(10.0, whole_sketch(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["bodies"][0]
        .as_object_mut()
        .unwrap()
        .remove("name");
    let changed = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    };
    let (document, report) = convert(
        &changed,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let extrude = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "extrude")
        .unwrap_or_else(|| panic!("{}", history_line(&report)));
    assert_eq!(
        extrude["body_names"],
        json!(["Body 1"]),
        "{}",
        history_line(&report)
    );
}

#[test]
fn a_join_before_any_body_makes_and_names_the_first_body() {
    let model = cube_with_history(10.0, whole_sketch(), None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["bodies"][0]
        .as_object_mut()
        .unwrap()
        .remove("name");
    for feature in wire["model"]["features"].as_array_mut().unwrap() {
        if feature["definition"]["op"] == "new_body" {
            feature["definition"]["op"] = json!("join");
        }
    }
    let changed = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        ..model
    };
    let (document, report) = convert(
        &changed,
        &SolidCraftWriter::new().verification(Verification::Off),
    );
    let extrude = document["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["type"] == "extrude")
        .unwrap_or_else(|| panic!("{}", history_line(&report)));
    assert_eq!(
        extrude["operation"],
        "new_body",
        "{}",
        history_line(&report)
    );
    assert_eq!(
        extrude["body_names"],
        json!(["Body 1"]),
        "{}",
        history_line(&report)
    );
}

#[test]
fn through_all_cuts_require_a_body_before_the_cut_in_ordinal_order() {
    const CUT_SKETCH: &str = "test:model:sketch#cut";
    for with_boss in [false, true] {
        let model = cube_with_history(10.0, whole_sketch(), None);
        let mut wire = serde_json::to_value(&model.ir).unwrap();
        let mut cut = wire["model"]["features"][1].clone();
        cut["id"] = json!("test:model:feature#cut");
        cut["name"] = json!("Through cut");
        cut["ordinal"] = json!(3);
        cut["outputs"] = json!([]);
        cut["dependencies"] = if with_boss {
            json!(["test:model:feature#cut-sketch", "f3d:model:feature#2"])
        } else {
            json!(["test:model:feature#cut-sketch"])
        };
        cut["definition"]["op"] = json!("cut");
        cut["definition"]["extent"] =
            json!({"kind": "symmetric", "side": {"termination": {"kind": "through_all"}}});
        cut["definition"]["profile"] = json!({"kind": "sketch", "value": CUT_SKETCH});
        let mut circle = entity(
            5,
            json!({"kind": "circle", "center": {"u": 5.0, "v": 5.0}, "radius": 1.0}),
        );
        circle["sketch"] = json!(CUT_SKETCH);
        wire["model"]["sketch_entities"]
            .as_array_mut()
            .unwrap()
            .push(circle);
        let placement = wire["model"]["sketches"][0]["placement"].clone();
        wire["model"]["sketches"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id": CUT_SKETCH,
            "name": "Cut sketch", "placement": placement,
            "profiles": [[{"entity": "f3d:model:sketch-entity#5"}]]}));
        let features = wire["model"]["features"].as_array_mut().unwrap();
        if !with_boss {
            features.retain(|f| f["ordinal"] == 0);
        }
        features.push(json!({"id": "test:model:feature#cut-sketch", "ordinal": 2, "name": "Cut sketch", "suppressed": false,
            "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": CUT_SKETCH}}}));
        features.push(cut);
        features.reverse();
        let changed = Model {
            ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
            ..model
        };
        let (document, report) = convert(
            &changed,
            &SolidCraftWriter::new().verification(Verification::Off),
        );
        let cuts: Vec<_> = document["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["operation"] == "cut")
            .collect();
        if with_boss {
            assert_eq!(cuts.len(), 1, "{}", history_line(&report));
            let features = document["features"].as_array().unwrap();
            assert_eq!(features[1]["name"], "Block");
            assert_eq!(features[2]["name"], "Cut sketch");
            assert_eq!(features[3]["name"], "Through cut");
            if let Some(cli) = solidcraft() {
                let got = volumes(
                    &cli,
                    &written_with(
                        &changed,
                        &SolidCraftWriter::new().verification(Verification::Off),
                    )
                    .0,
                );
                assert_eq!(got.len(), 1);
                assert!(
                    (got[0] - (1000.0 - std::f64::consts::PI * 10.0)).abs() < 1.0,
                    "{got:?}"
                );
            }
        } else {
            assert!(cuts.is_empty());
            assert!(
                history_line(&report).contains("through-all cut has no earlier named body"),
                "{}",
                history_line(&report)
            );
        }
    }
}

/// The example cube with a circle of the given radius added to the history's sketch, centred
/// in its square, as a second profile loop.
fn cube_with_circle_profile(length: f64, profile: Value, radius: f64) -> Model {
    let mut model = cube_with_history(length, profile, None);
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let sketch = &mut wire["model"];
    sketch["sketch_entities"]
        .as_array_mut()
        .unwrap()
        .push(entity(
            5,
            json!({"kind": "circle", "center": {"u": 5.0, "v": 5.0}, "radius": radius}),
        ));
    let loops = sketch["sketches"][0]["profiles"][0].clone();
    sketch["sketches"][0]["profiles"] = json!([loops, [{"entity": "f3d:model:sketch-entity#5"}]]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

fn extrude_outcome(model: &Model) -> (dutchman_core::FeatureOutcome, Report) {
    let writer = SolidCraftWriter::new().verification(Verification::Off);
    let (_, report) = convert(model, &writer);
    let extrude = report
        .outcome()
        .unwrap()
        .features
        .iter()
        .find(|f| f.kind == "extrude")
        .unwrap()
        .clone();
    (extrude, report)
}

fn profile_of(indices: &[usize]) -> Value {
    json!({"kind": "sketch_profiles", "value": {"sketch": SKETCH, "profiles": indices}})
}

#[test]
fn an_extrude_whose_region_differs_from_the_source_end_cap_is_blocked() {
    // The sweep covers a Ø4 circle; the model's end face is the whole 10 mm square.
    let model = cube_with_circle_profile(10.0, profile_of(&[1]), 2.0);
    let (extrude, report) = extrude_outcome(&model);
    assert_eq!(extrude.status, dutchman_core::FeatureStatus::Blocked);
    let reason = extrude.reason.unwrap();
    assert!(
        reason.starts_with("its profile region doesn't match the source (swept 12.5 mm²"),
        "{reason}"
    );
    assert!(reason.contains("the source 100.0 mm²)"), "{reason}");
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.code.as_deref() == Some("history/profile-region-mismatch")));
    // A hole in the swept region leaves the end face smaller than the source's.
    let model = cube_with_circle_profile(10.0, profile_of(&[0, 1]), 2.0);
    let (extrude, _) = extrude_outcome(&model);
    assert_eq!(extrude.status, dutchman_core::FeatureStatus::Blocked);
    assert!(extrude.reason.unwrap().contains("swept 87.5 mm²"));
}

#[test]
fn an_extrude_whose_region_matches_the_source_end_cap_is_kept_and_proven() {
    let model = cube_with_circle_profile(10.0, profile_of(&[0]), 2.0);
    let (extrude, report) = extrude_outcome(&model);
    assert_eq!(extrude.status, dutchman_core::FeatureStatus::Mapped);
    assert!(!extrude.profile_unverified);
    assert!(!report
        .losses()
        .iter()
        .any(|loss| loss.code.as_deref() == Some("history/profile-region-mismatch")));
}

#[test]
fn an_extrude_with_no_end_cap_in_the_model_is_kept_but_its_profile_is_unproven() {
    // The model has no face 5 mm up, so nothing says what this sweep covered.
    let model = cube_with_circle_profile(5.0, profile_of(&[1]), 2.0);
    let (extrude, _) = extrude_outcome(&model);
    assert_eq!(extrude.status, dutchman_core::FeatureStatus::Mapped);
    assert!(extrude.profile_unverified);
}

/// Recorded responses for the synthetic cube fixture above. No CLI or temporary tool
/// is used: the same JSON parser handles these and the native CLI's answers.
#[derive(Debug, Default)]
struct RecordedEvaluator {
    calls: std::sync::Mutex<Vec<String>>,
    different_history: bool,
    warning: bool,
}

impl dutchman_solidcraft::SolidCraftEvaluator for RecordedEvaluator {
    fn evaluate(
        &self,
        design: &[u8],
        timeout: std::time::Duration,
        label: &str,
    ) -> Result<Value, String> {
        assert!(!timeout.is_zero());
        let mut archive = ZipArchive::new(Cursor::new(design)).unwrap();
        assert!(archive.by_name("document.json").is_ok());
        self.calls.lock().unwrap().push(label.to_owned());
        Ok(json!({
            "bodies": [{"volume_mm3": if label == "history" && self.different_history {900.0} else {1000.0},
                "area_mm2": 600.0, "bbox": {"min": [0, 0, 0], "max": [10, 10, 10]}}],
            "timeline": [{"name": "Original geometry", "error": null,
                "warning": if self.warning { "1 face(s) could not be meshed" } else { "" }}],
            "total": {"faces": 6}
        }))
    }

    fn run(
        &self,
        design: &[u8],
        commands: &[Value],
        timeout: std::time::Duration,
        label: &str,
    ) -> Result<Vec<Value>, String> {
        assert!(!design.is_empty());
        assert!(!timeout.is_zero());
        self.calls.lock().unwrap().push(label.to_owned());
        commands
            .iter()
            .map(|command| {
                Ok(match command["command"].as_str().unwrap() {
                    "timeline.roll_to" => json!({"marker": command["params"]["feature"]}),
                    "document.inspect" => {
                        json!({"bodies": [{"name": "unit cube"}], "timeline": []})
                    }
                    "model.edges" => json!({"body": "unit cube", "edges": [{
                        "name": "front|left", "start": [0, 0, 0], "end": [0, 0, 10],
                        "mid": [0, 0, 5], "length": 10.0
                    }]}),
                    other => return Err(format!("Unexpected command: {other}")),
                })
            })
            .collect()
    }
}

#[test]
fn a_host_evaluator_proves_a_blend_and_verifies_the_fixture() {
    let host = std::sync::Arc::new(RecordedEvaluator::default());
    let writer = SolidCraftWriter::new()
        // A host must not depend on an installed CLI, even when this path is missing.
        .verification(Verification::With(PathBuf::from("/missing/solidcraft-cli")))
        .replay(Verification::With(PathBuf::from("/missing/solidcraft-cli")))
        .evaluator(host.clone())
        .target_version(Some(dutchman_solidcraft::SolidCraftVersion::default()));
    let model = input_blend("fillet", json!({"kind": "constant", "radius": 1.0}), false);
    let (document, report) = convert(&model, &writer);
    assert_eq!(document["features"][2]["type"], "fillet");
    assert_eq!(document["features"][2]["edges"], json!([[0.0, 0.0, 5.0]]));
    assert_eq!(report.outcome().unwrap().check, Check::Verified);
    assert_eq!(
        *host.calls.lock().unwrap(),
        ["bodies", "edges", "original", "history"]
    );
}

#[test]
fn a_host_evaluator_carries_prefix_measurements_and_final_geometry_warnings() {
    let host = std::sync::Arc::new(RecordedEvaluator {
        different_history: true,
        warning: true,
        ..RecordedEvaluator::default()
    });
    let writer = SolidCraftWriter::new()
        .evaluator(host.clone())
        .target_version(Some(dutchman_solidcraft::SolidCraftVersion::default()));
    let model = cube_with_history(10.0, first_profile(), None);
    let (_, report) = convert(&model, &writer);
    assert_eq!(report.outcome().unwrap().mode, Mode::PartialHistory);
    assert!(matches!(report.outcome().unwrap().check, Check::Partial(_)));
    assert_eq!(
        *host.calls.lock().unwrap(),
        ["original", "history", "prefix", "import"]
    );
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("could not be meshed")));
}
