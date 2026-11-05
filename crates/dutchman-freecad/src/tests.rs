#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use dutchman_core::{FeatureStatus, Mode, Model, Report, Severity, Writer};
use serde_json::{json, Value};

use super::{generate, readable, FreeCadWriter};

// Synthetic CADIR, following dutchman-solidcraft/tests/history.rs. The B-rep is
// cadmpeg's generated cube; the history is authored here to test script text only.
/// A synthetic part: a sketched circle extruded 5 mm, beside the unit cube's final shape. The
/// extrude ends where the final shape has no face, so its profile is reported unverified rather
/// than compared with the cube's 100 mm² top face.
fn model(kind: &str) -> Model {
    let mut wire = serde_json::to_value(cadmpeg_ir::examples::unit_cube().unwrap()).unwrap();
    let arena = &mut wire["model"];
    arena["sketches"] = json!([{
        "id": "synthetic:test:sketch#1",
        "name": "Profile",
        "placement": {"kind": "resolved", "frame": {
            "origin": {"x": 0.0, "y": 0.0, "z": 0.0},
            "normal": {"x": 0.0, "y": 0.0, "z": 1.0},
            "u_axis": {"x": 1.0, "y": 0.0, "z": 0.0}
        }},
        "profiles": [[{"entity": "synthetic:test:sketch-entity#1"}]]
    }]);
    arena["sketch_entities"] = json!([{
        "id": "synthetic:test:sketch-entity#1",
        "sketch": "synthetic:test:sketch#1",
        "construction": false,
        "geometry": {"kind": "circle", "center": {"u": 5.0, "v": 0.0}, "radius": 2.0}
    }]);
    arena["features"] = json!([
        {"id": "synthetic:test:feature#1", "ordinal": 0, "name": "Profile", "suppressed": false,
         "definition": {"definition": "sketch", "sketch": {"space": "planar", "sketch": "synthetic:test:sketch#1"}}},
        {"id": "synthetic:test:feature#2", "ordinal": 1, "name": "Solid", "suppressed": false,
         "definition": {"definition": "extrude", "profile": {"kind": "sketch", "value": "synthetic:test:sketch#1"},
             "direction": {"kind": "profile_normal"}, "start": {"kind": "profile_plane"},
             "extent": {"kind": "one_sided", "side": {"termination": {"kind": "blind", "length": 5.0}}},
             "op": "new_body"}}
    ]);
    if kind == "revolve" {
        arena["features"][1]["definition"] = json!({
            "definition": "revolve", "op": "new_body",
            "construction": {"state": "resolved",
                "profile": {"kind": "sketch", "value": "synthetic:test:sketch#1"},
                "axis": {"origin": {"x": 0.0, "y": 0.0, "z": 0.0}, "direction": {"x": 0.0, "y": 1.0, "z": 0.0}},
                "extent": {"kind": "one_sided", "termination": {"kind": "angle", "angle": std::f64::consts::TAU}}
            }
        });
    }
    if kind == "pattern" {
        arena["features"].as_array_mut().unwrap().push(json!({
            "id": "synthetic:test:feature#3", "ordinal": 2, "name": "Copies", "suppressed": false,
            "definition": {"definition": "pattern", "seeds": [{"kind": "feature", "value": "synthetic:test:feature#2"}],
                "pattern": {"kind": "linear", "direction": {"x": 1.0, "y": 0.0, "z": 0.0}, "spacing": 20.0, "count": 3}}
        }));
    }
    let model = Model {
        ir: cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap(),
        fidelity: None,
        name: "Synthetic".to_owned(),
        source_file: None,
        source_step: None,
        assembly: None,
    };
    if kind == "partial" {
        with_unmapped_suffix(model)
    } else {
        model
    }
}

fn written(model: &Model, writer: FreeCadWriter) -> (String, Report) {
    let mut bytes = Vec::new();
    let mut report = Report::default();
    writer.write(model, &mut bytes, &mut report).unwrap();
    (String::from_utf8(bytes).unwrap(), report)
}

#[test]
fn synthetic_histories_match_macro_goldens() {
    for (kind, golden) in [
        ("pad", include_str!("../tests/golden/pad.FCMacro")),
        ("revolve", include_str!("../tests/golden/revolve.FCMacro")),
        ("pattern", include_str!("../tests/golden/pattern.FCMacro")),
        ("partial", include_str!("../tests/golden/partial.FCMacro")),
    ] {
        let (script, report) = written(&model(kind), FreeCadWriter::new());
        assert_eq!(report.outcome().unwrap().mode, Mode::History);
        // STEP has encoder metadata unrelated to macro generation. Replace only its
        // single escaped literal; compare every other byte of the executable macro.
        let normalized: String = script
            .lines()
            .map(|line| {
                if line.starts_with("original = original_step(") {
                    "original = original_step(\"Synthetic\", \"STEP payload\")\n".to_owned()
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        if std::env::var_os("DUTCHMAN_FREECAD_BLESS").is_some() {
            let path = format!("{}/tests/golden/{kind}.FCMacro", env!("CARGO_MANIFEST_DIR"));
            std::fs::write(path, &normalized).unwrap();
            continue;
        }
        assert_eq!(normalized, golden, "{kind}");
    }
}

fn with_unmapped_suffix(mut model: Model) -> Model {
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "synthetic:test:feature#3", "ordinal": 2, "name": "Unmapped",
            "suppressed": false,
            "definition": {"definition": "native", "kind": "Synthetic unsupported feature"}
        }));
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn partial_history_keeps_pad_and_sketch_beside_the_final_step() {
    let model = with_unmapped_suffix(model("pad"));
    let (script, report) = written(&model, FreeCadWriter::new());
    let outcome = report.outcome().unwrap();
    assert_eq!(outcome.mode, Mode::History);
    assert_eq!(outcome.features[0].status, FeatureStatus::Mapped);
    assert_eq!(outcome.features[1].status, FeatureStatus::Mapped);
    assert_eq!(outcome.features[2].status, FeatureStatus::Blocked);
    assert!(outcome.features[1]
        .reason
        .as_ref()
        .unwrap()
        .contains("reference"));
    assert!(outcome.features[2]
        .reason
        .as_ref()
        .unwrap()
        .contains("not mapped"));
    assert!(report
        .losses()
        .iter()
        .any(|loss| { loss.detail == "rebuilt 2 of 3 features; kept as reference" }));
    assert!(script.contains("group.Label = 'Rebuilt history (partial)'"));
    assert!(script.contains("visible(group, False)\n    visible(original, True)"));
    assert!(script.contains("compare_original(tip.Shape, original.Shape)"));
    let data = script
        .lines()
        .find_map(|line| line.strip_prefix("selected = partial_history(json.loads("))
        .unwrap();
    let literal = serde_json::Deserializer::from_str(data)
        .into_iter::<String>()
        .next()
        .unwrap()
        .unwrap();
    let rows: Value = serde_json::from_str(&literal).unwrap();
    assert_eq!(rows[0]["type"], "sketch");
    assert_eq!(rows[1]["type"], "extrude");
    assert_eq!(rows[1]["extent"]["distance"], "5 mm");
    assert_eq!(rows.as_array().unwrap().len(), 2);
}

#[test]
fn freecad_refusals_stop_the_prefix_and_keep_later_sketches() {
    let mut model = model("pad");
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let mut sketch = wire["model"]["features"][0].clone();
    sketch["id"] = json!("synthetic:test:feature#4");
    sketch["ordinal"] = json!(3);
    sketch["name"] = json!("Later sketch");
    // A second pad with taper maps in SolidCraft but is refused by FreeCAD output.
    let mut tapered = wire["model"]["features"][1].clone();
    tapered["id"] = json!("synthetic:test:feature#3");
    tapered["ordinal"] = json!(2);
    tapered["name"] = json!("Tapered pad");
    tapered["definition"]["op"] = json!("join");
    tapered["definition"]["extent"]["side"]["draft"] = json!(0.1);
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .extend([tapered, sketch]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (script, report) = written(&model, FreeCadWriter::new());
    let features = &report.outcome().unwrap().features;
    assert_eq!(features[1].status, FeatureStatus::Mapped);
    assert_eq!(features[2].status, FeatureStatus::Blocked);
    assert!(features[2].reason.as_ref().unwrap().contains("taper"));
    assert_eq!(features[3].status, FeatureStatus::Mapped);
    assert!(features[3]
        .reason
        .as_ref()
        .unwrap()
        .contains("reference sketch"));
    assert!(script.contains("Later sketch"));
    assert!(report
        .losses()
        .iter()
        .any(|loss| { loss.detail == "rebuilt 2 of 4 features; kept as reference" }));
}

#[test]
fn import_only_does_not_keep_a_partial_prefix() {
    let (script, report) = written(
        &with_unmapped_suffix(model("pad")),
        FreeCadWriter::new().import_only(true),
    );
    assert_eq!(report.outcome().unwrap().mode, Mode::Import);
    assert!(!script.contains("def partial_history("));
}

#[test]
fn shared_mapping_failure_keeps_later_sketches_but_no_later_solids() {
    let mut model = model("partial");
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let mut sketch = wire["model"]["features"][0].clone();
    sketch["id"] = json!("synthetic:test:feature#4");
    sketch["ordinal"] = json!(3);
    sketch["name"] = json!("Later sketch");
    let mut pad = wire["model"]["features"][1].clone();
    pad["id"] = json!("synthetic:test:feature#5");
    pad["ordinal"] = json!(4);
    pad["name"] = json!("Later pad");
    wire["model"]["features"]
        .as_array_mut()
        .unwrap()
        .extend([sketch, pad]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (script, report) = written(&model, FreeCadWriter::new());
    let features = &report.outcome().unwrap().features;
    assert_eq!(features[1].status, FeatureStatus::Mapped);
    assert_eq!(features[2].status, FeatureStatus::Blocked);
    assert_eq!(features[3].status, FeatureStatus::Mapped);
    assert_eq!(features[4].status, FeatureStatus::Blocked);
    assert!(script.contains("Later sketch"));
    assert!(!script.contains("Later pad"));
}

#[test]
fn partial_prefix_uses_timeline_order_and_excludes_suppressed_features() {
    let mut model = model("partial");
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let mut suppressed = wire["model"]["features"][2].clone();
    suppressed["id"] = json!("synthetic:test:feature#4");
    suppressed["ordinal"] = json!(0);
    suppressed["suppressed"] = json!(true);
    let rows = wire["model"]["features"].as_array_mut().unwrap();
    rows[0]["ordinal"] = json!(1);
    rows[1]["ordinal"] = json!(2);
    rows[2]["ordinal"] = json!(3);
    rows.push(suppressed);
    rows.reverse();
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = written(&model, FreeCadWriter::new());
    assert_eq!(
        report.outcome().unwrap().features[0].status,
        FeatureStatus::LeftOut
    );
    assert!(report
        .losses()
        .iter()
        .any(|loss| { loss.detail == "rebuilt 2 of 3 features; kept as reference" }));
}

#[test]
fn partial_history_checks_final_volume_in_freecad_when_available() {
    let Some(cli) = configured_freecad() else {
        return;
    };
    let (mut script, _) = written(&model("partial"), FreeCadWriter::new());
    script.push_str(
        "assert doc.getObject('PartialHistory').Label == 'Rebuilt history (partial)'\n\
         assert abs(original.Shape.Volume - 1000.0) < 1e-6\n\
         assert abs(doc.getObject('Body').Tip.Shape.Volume - 40*math.pi) < 1e-6\n\
         print('DUTCHMAN_PARTIAL_OK')\n",
    );
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("partial.FCMacro");
    std::fs::write(&path, script).unwrap();
    let output = cli
        .process(folder.path())
        .arg("-c")
        .arg(format!(
            "exec(open({}).read())",
            generate::quoted(&path.to_string_lossy()).unwrap()
        ))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("DUTCHMAN_PARTIAL_OK"));
}

#[test]
fn hostile_names_are_data_only() {
    let name = "O'Brien \"quoted\" \\ folder\nnext\rline\t雪 🛠\0\u{2028}";
    let literal = generate::quoted(name).unwrap();
    assert_eq!(serde_json::from_str::<String>(&literal).unwrap(), name);
    assert!(!literal.contains('\n'));
    let mut model = model("pad");
    model.name = name.to_owned();
    let (script, _) = written(&model, FreeCadWriter::new());
    assert!(script.contains(&format!("body.Label = {literal}\n")));
    assert!(!script.contains("exec("));
    assert!(!script.contains("eval("));
    assert!(script.contains("os.unlink(path)"));
}

#[test]
fn import_only_and_unmapped_features_keep_step_and_report_each_feature() {
    let model = model("pad");
    let (script, report) = written(&model, FreeCadWriter::new().import_only(true));
    assert_eq!(report.outcome().unwrap().mode, Mode::Import);
    assert!(report
        .outcome()
        .unwrap()
        .features
        .iter()
        .all(|f| f.status == FeatureStatus::Blocked && f.reason.is_some()));
    assert!(script.contains("ISO-10303-21"));
    assert!(!script.contains("tip = rebuild(json.loads("));

    let mut model = model;
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["features"][1]["definition"] =
        json!({"definition": "native", "kind": "Synthetic unsupported feature"});
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (_, report) = written(&model, FreeCadWriter::new());
    assert_eq!(report.outcome().unwrap().features.len(), 2);
    assert!(report
        .outcome()
        .unwrap()
        .features
        .iter()
        .all(|f| f.reason.is_some()));
}

#[test]
fn fixed_float_data_round_trips_without_turning_names_into_code() {
    let value = json!({"small": 1e-30, "large": 1e30, "name": "\"; raise RuntimeError('name') #"});
    let code = generate::data(&value).unwrap();
    let outer = code
        .strip_prefix("json.loads(")
        .unwrap()
        .strip_suffix(')')
        .unwrap();
    let inner: String = serde_json::from_str(outer).unwrap();
    let round_trip: Value = serde_json::from_str(&inner).unwrap();
    assert_eq!(round_trip, value);
    assert!(inner.contains("e-30"));
}

#[test]
fn huge_pattern_counts_are_reported_before_macro_execution() {
    let rows = [
        json!({"type": "pattern", "name": "Too many", "pattern": {"type": "rectangular", "count1": "10001"}}),
    ];
    assert!(generate::history(&rows)
        .unwrap_err()
        .contains("count exceeds"));
}

#[test]
fn sketch_constraints_keep_their_evaluated_dimensions() {
    let mut model = model("pad");
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["parameters"] = json!([{
        "id": "synthetic:test:parameter#1", "name": "Radius", "expression": "2 mm",
        "owner": "synthetic:test:feature#1", "value": {"kind": "length", "value": 2.0}
    }]);
    wire["model"]["sketch_constraints"] = json!([{
        "id": "synthetic:test:sketch-constraint#1", "sketch": "synthetic:test:sketch#1",
        "definition": {"kind": "radius", "entity": "synthetic:test:sketch-entity#1", "parameter": "synthetic:test:parameter#1"}
    }]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let mapped = dutchman_solidcraft::mapped_history(&model, &mut Report::default());
    let rows = mapped.features.unwrap();
    assert_eq!(rows[0]["sketch"]["constraints"][0]["type"], "radius");
    assert_eq!(rows[0]["sketch"]["constraints"][0]["value"], 2.0);
    let (script, report) = written(&model, FreeCadWriter::new());
    assert_eq!(report.outcome().unwrap().mode, Mode::History);
    assert!(script.contains("parameter_table(json.loads("));
}

#[test]
fn assembly_occurrences_keep_nested_placements_and_independent_bodies() {
    let part = model("pad");
    let placement = dutchman_core::Placement {
        hidden: false,
        name: Some("Placed part".to_owned()),
        part: 0,
        transform: [
            [1.0, 0.0, 0.0, 12.0],
            [0.0, 1.0, 0.0, -3.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    };
    let assembly = Model {
        assembly: Some(dutchman_core::Assembly {
            own_part: None,
            parts: vec![part.clone()],
            placements: vec![placement.clone(), placement],
            joints: Vec::new(),
            left_out: Vec::new(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
        }),
        ..part
    };
    let (script, report) = written(&assembly, FreeCadWriter::new());
    assert_eq!(script.matches("parents.append(container(").count(), 2);
    assert_eq!(script.matches("body.Label = \"Synthetic\"").count(), 2);
    assert_eq!(script.matches("parents.pop()").count(), 2);
    assert_eq!(report.outcome().unwrap().features.len(), 4);
}

#[test]
fn step_freecad_cannot_read_is_refused_before_the_macro_is_written() {
    assert!(readable("#1 = LINE('',#2,#3);\n").is_ok());
    assert!(readable("#29 = CURVE_REPLICA('',#24,#28);\n").is_err());
}

#[test]
fn options_the_script_would_ignore_are_refused_with_a_reason() {
    let base = json!({"id": 2, "name": "F", "type": "extrude", "operation": "join",
        "extent": {"direction": "positive", "distance": "5 mm"}});
    assert!(generate::history(std::slice::from_ref(&base)).is_ok());
    for (key, value) in [
        (
            "extent",
            json!({"direction": "positive", "distance": "5 mm", "taper": "2 deg"}),
        ),
        (
            "extent",
            json!({"direction": "positive", "distance": "0 mm", "to": [0.0, 0.0, 9.0]}),
        ),
    ] {
        let mut row = base.clone();
        row[key] = value;
        let reason = generate::history(&[row]).unwrap_err();
        assert!(
            reason.contains("not supported yet in FreeCAD output"),
            "{reason}"
        );
    }
    let counterbore = json!({"id": 3, "name": "H", "type": "hole", "diameter": "4 mm",
        "hole": {"type": "counterbore", "cb_diameter": "8 mm"}});
    assert!(generate::history(&[counterbore])
        .unwrap_err()
        .contains("counterbored"));
    let tilted = json!({"id": 4, "name": "C", "type": "coil", "axis": [1.0, 0.0, 0.0]});
    assert!(generate::history(&[tilted])
        .unwrap_err()
        .contains("other than Z"));
    let scaled = json!({"id": 5, "name": "S", "type": "sweep", "scale": "2"});
    assert!(generate::history(&[scaled]).is_err());
}

fn joint_model() -> Model {
    use dutchman_core::placement::IDENTITY;
    let part = model("pad");
    let mut placed = IDENTITY;
    placed[2][3] = 10.0;
    Model {
        name: "Hinge".to_owned(),
        assembly: Some(dutchman_core::Assembly {
            own_part: None,
            parts: vec![part.clone()],
            placements: vec![
                dutchman_core::Placement {
                    hidden: false,
                    name: Some("Base".to_owned()),
                    part: 0,
                    transform: IDENTITY,
                },
                dutchman_core::Placement {
                    hidden: false,
                    name: Some("Arm".to_owned()),
                    part: 0,
                    transform: placed,
                },
            ],
            joints: vec![dutchman_core::Joint {
                a: vec![0],
                b: vec![1],
                frame_a: placed,
                frame_b: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, -1.0, 0.0, 0.0],
                    [0.0, 0.0, -1.0, 0.0],
                ],
                suppressed: false,
                motion: dutchman_core::Motion::Revolute {
                    angle: 0.0,
                    limits: None,
                },
                flip: false,
            }],
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
            left_out: Vec::new(),
        }),
        ..part
    }
}

fn raw_joint_model(definition: Value) -> Model {
    let family = definition["kind"].as_str().unwrap_or_default().to_owned();
    let mut model = joint_model();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let identity = dutchman_core::placement::IDENTITY;
    wire["model"]["product_definitions"] = json!([{
        "id": "synthetic:test:product#part", "kind": "part"
    }]);
    wire["model"]["occurrences"] = json!([
        {"id": "synthetic:test:occurrence#base", "ordinal": 0, "name": "Base",
         "prototype": {"scope": "local", "definition": "synthetic:test:product#part"},
         "parent": {"kind": "root"}, "transform": identity, "scale": [1.0, 1.0, 1.0]},
        {"id": "synthetic:test:occurrence#arm", "ordinal": 1, "name": "Arm",
         "prototype": {"scope": "local", "definition": "synthetic:test:product#part"},
         "parent": {"kind": "root"}, "transform": identity, "scale": [1.0, 1.0, 1.0]}
    ]);
    let joint = &model.assembly.as_ref().unwrap().joints[0];
    let connector = |name: &str, frame| {
        json!({
            "operand": {"container": {"container": "occurrence", "occurrence": name}, "object": "Body"},
            "frame": frame, "detached": false
        })
    };
    let mut frame_a = joint.frame_a;
    if family == "perpendicular" {
        frame_a = [
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0, 10.0],
        ];
    }
    wire["model"]["assembly_joints"] = json!([{
        "id": "synthetic:test:joint#hinge", "suppressed": false,
        "operands": {"arity": "pair", "kind": definition, "connectors": [
            connector("synthetic:test:occurrence#base", frame_a),
            connector("synthetic:test:occurrence#arm", joint.frame_b)
        ]}
    }]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    // The core converts only these families; the others reach the writer through CADIR.
    let motion = match family.as_str() {
        "fixed" => Some(dutchman_core::Motion::Rigid),
        "revolute" => Some(dutchman_core::Motion::Revolute {
            angle: 0.0,
            limits: None,
        }),
        "slider" => Some(dutchman_core::Motion::Slider { distance: 0.0 }),
        _ => None,
    };
    let assembly = model.assembly.as_mut().unwrap();
    match motion {
        Some(motion) => assembly.joints[0].motion = motion,
        None => assembly.joints.clear(),
    }
    model
}

/// The top assembly's joints and grounds, as the macro passes them to FreeCAD.
fn assembly_data(script: &str) -> Value {
    let data = macro_data(script);
    data["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|scope| scope["path"] == json!([]))
        .map(|scope| json!({"joints": scope["joints"], "held": scope["held"]}))
        .unwrap_or_else(|| json!({"joints": [], "held": []}))
}

fn macro_data(script: &str) -> Value {
    let line = script
        .lines()
        .rev()
        .find(|line| line.starts_with("finish_assembly("))
        .unwrap();
    let outer = line
        .split_once("json.loads(")
        .unwrap()
        .1
        .strip_suffix("))")
        .unwrap();
    let inner: String = serde_json::from_str(outer).unwrap();
    serde_json::from_str(&inner).unwrap()
}

#[test]
fn hinge_macro_matches_joint_golden_and_infers_one_anchor() {
    let (script, report) = written(&joint_model(), FreeCadWriter::new());
    let data = assembly_data(&script);
    let normalized = format!("{}\n", serde_json::to_string_pretty(&data).unwrap());
    assert_eq!(
        normalized,
        include_str!("../tests/golden/hinge.joints.json")
    );
    assert!(script.contains("JointObject.GroundedJoint(ground, children[path[0]])"));
    assert!(script.contains("joint.Reference1 = joint_reference(children, row['a'])"));
    assert!(script.contains("joint.Detach1 = True"));
    assert!(script.contains("Assembly solve changed the stored part placements"));
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("grounded, inferred")));
    assert!(!report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("joints are not supported")));
}

#[test]
fn converted_joint_families_flip_limits_ground_and_suppression_are_kept() {
    use dutchman_core::{placement::IDENTITY, Motion};
    for (motion, kind) in [
        (Motion::Rigid, "Fixed"),
        (Motion::Slider { distance: 3.0 }, "Slider"),
        (
            Motion::Revolute {
                angle: 0.3,
                limits: Some((-0.5, 1.0)),
            },
            "Revolute",
        ),
    ] {
        let mut model = joint_model();
        let assembly = model.assembly.as_mut().unwrap();
        assembly.joints[0].motion = motion;
        assembly.joints[0].flip = true;
        assembly.joints[0].frame_b = IDENTITY;
        assembly.held = vec![vec![0]];
        let (script, report) = written(&model, FreeCadWriter::new());
        let data = assembly_data(&script);
        assert_eq!(data["joints"][0]["kind"], kind);
        assert_eq!(data["joints"][0]["frame_b"], json!(IDENTITY));
        assert_eq!(data["held"], json!([[0]]));
        assert!(!report
            .losses()
            .iter()
            .any(|loss| loss.detail.contains("inferred")));
        if kind == "Revolute" {
            assert_eq!(
                data["joints"][0]["angular"]["minimum"],
                (-0.5_f64).to_degrees()
            );
        }
        model.assembly.as_mut().unwrap().joints[0].suppressed = true;
        model.assembly.as_mut().unwrap().held.clear();
        let (script, _) = written(&model, FreeCadWriter::new());
        assert_eq!(assembly_data(&script)["joints"][0]["suppressed"], true);
        assert_eq!(assembly_data(&script)["held"], json!([]));
    }
    let (script, _) = written(&joint_model(), FreeCadWriter::new().import_only(true));
    assert_eq!(assembly_data(&script), json!({"joints": [], "held": []}));
}

#[test]
fn cadir_joint_families_and_independent_bounds_are_mapped() {
    for (family, kind) in [
        ("fixed", "Fixed"),
        ("revolute", "Revolute"),
        ("slider", "Slider"),
        ("cylindrical", "Cylindrical"),
        ("ball", "Ball"),
        ("distance", "Distance"),
        ("parallel", "Parallel"),
        ("perpendicular", "Perpendicular"),
        ("angle", "Angle"),
        ("axis_coincidence", "Cylindrical"),
    ] {
        let (script, _) = written(
            &raw_joint_model(json!({"kind": family})),
            FreeCadWriter::new(),
        );
        assert_eq!(assembly_data(&script)["joints"][0]["kind"], kind);
    }
    let model = raw_joint_model(json!({"kind": "cylindrical", "angle": 0.4, "distance": 2.0,
        "angular_limits": {"bounds": "minimum", "minimum": -0.5},
        "linear_limits": {"bounds": "maximum", "maximum": 8.0}}));
    let (script, _) = written(&model, FreeCadWriter::new());
    let data = assembly_data(&script);
    let row = &data["joints"][0];
    assert_eq!(row["angular"], json!({"minimum": (-0.5_f64).to_degrees()}));
    assert_eq!(row["linear"], json!({"maximum": 8.0}));
    assert_eq!(row["distance"], 2.0);
    assert!((row["frame_a"][0][0].as_f64().unwrap() - 0.4_f64.cos()).abs() < 1e-12);
}

#[test]
fn joints_core_left_out_are_reported_only_when_the_macro_cannot_build_them() {
    // Core leaves out a cylindrical joint, which the macro builds from CADIR, and a revolute
    // one it could not place, which the macro builds only from core's converted joints.
    let mut model = raw_joint_model(json!({"kind": "cylindrical"}));
    if let Some(assembly) = &mut model.assembly {
        assembly.joint_notes = vec![
            dutchman_core::JointNote {
                text: "joint 1: cylindrical joints are not converted yet".to_owned(),
                family: Some("cylindrical".to_owned()),
            },
            dutchman_core::JointNote {
                text: "joint 2: it joins a component to itself".to_owned(),
                family: Some("revolute".to_owned()),
            },
        ];
    }
    let (script, report) = written(&model, FreeCadWriter::new());
    assert_eq!(assembly_data(&script)["joints"][0]["kind"], "Cylindrical");
    let dropped: Vec<&str> = report
        .losses()
        .iter()
        .filter(|loss| loss.subject == "joints" && loss.severity == Severity::Dropped)
        .map(|loss| loss.detail.as_str())
        .collect();
    assert_eq!(dropped, ["joint 2: it joins a component to itself"]);
}

#[test]
fn unsupported_joint_families_and_fixed_limits_have_specific_notes() {
    for definition in [
        json!({"kind": "rack_pinion"}),
        json!({"kind": "screw"}),
        json!({"kind": "gears"}),
        json!({"kind": "belt"}),
        json!({"kind": "plane_coincidence", "sense": "opposed"}),
        json!({"kind": "point_on_axis"}),
        json!({"kind": "tangent", "contact": "outside"}),
        json!({"kind": "native", "name": "Special linkage"}),
        json!({"kind": "fixed", "angular_limits": {"bounds": "minimum", "minimum": 0.0}}),
    ] {
        let family = definition["kind"].as_str().unwrap().to_owned();
        let (script, report) = written(&raw_joint_model(definition), FreeCadWriter::new());
        // A fixed joint is still written, from the converted joint, without its limits.
        let kept = usize::from(family == "fixed");
        assert_eq!(
            assembly_data(&script)["joints"].as_array().unwrap().len(),
            kept
        );
        assert!(
            report
                .losses()
                .iter()
                .any(|loss| loss.detail.contains(&family)),
            "{family}: {:?}",
            report.losses()
        );
    }
}

#[test]
fn freecad_solves_synthetic_joints_and_moves_only_the_hinge_arm() {
    let Some(command) = configured_freecad() else {
        return;
    };
    let folder =
        std::env::temp_dir().join(format!("dutchman-freecad-joints-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    for (family, kind) in [
        ("revolute", "Revolute"),
        ("slider", "Slider"),
        ("fixed", "Fixed"),
        ("cylindrical", "Cylindrical"),
        ("ball", "Ball"),
        ("distance", "Distance"),
        ("parallel", "Parallel"),
        ("perpendicular", "Perpendicular"),
        ("angle", "Angle"),
        ("axis_coincidence", "Cylindrical"),
        ("insert", "Revolute"),
    ] {
        let definition = if family == "insert" {
            json!({"kind": family, "sense": "opposed"})
        } else {
            json!({"kind": family})
        };
        let mut model = raw_joint_model(definition);
        model.assembly.as_mut().unwrap().held = vec![vec![0]];
        let (mut script, _) = written(&model, FreeCadWriter::new());
        script.push_str(&format!(
            r#"
assemblies = [obj for obj in doc.Objects if obj.TypeId == 'Assembly::AssemblyObject']
assert len(assemblies) == 1, 'Joint reconstruction fell back'
assembly = assemblies[0]
joint = assembly.Joints[0]
assert joint.JointType == '{kind}'
base = joint.Reference1[0]
arm = joint.Reference2[0]
assert assembly.isPartGrounded(base)
assert same_pose(base.Placement, App.Placement())
assert same_pose(arm.Placement, App.Placement(App.Vector(0, 0, 10), App.Rotation()))
assert assembly.solve() == 0
assert abs(base.Group[0].Shape.Volume - 1000) < 1e-7
if joint.JointType == 'Revolute':
    drive_joint_angle(assembly, joint, 35)
    assert assembly.solve() == 0
    assert abs(arm.Placement.Rotation.Angle - math.radians(35)) < 1e-7
    assert same_pose(base.Placement, App.Placement())
    joint.EnableAngleMin = True
    joint.EnableAngleMax = True
    joint.AngleMin = -10
    joint.AngleMax = 20
    drive_joint_angle(assembly, joint, 50)
    assert abs(arm.Placement.Rotation.Angle - math.radians(20)) < 1e-7
    assert same_pose(base.Placement, App.Placement())
App.Console.PrintMessage('DUTCHMAN_JOINT_OK\n')
"#
        ));
        run_freecad(&command, &folder, family, script);
    }
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn joint_attachment_offsets_grounding_and_operand_refusals_are_explicit() {
    use dutchman_core::placement::IDENTITY;
    let mut model = raw_joint_model(json!({"kind": "cylindrical"}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let mut offset = IDENTITY;
    offset[0][3] = 5.0;
    wire["model"]["assembly_joints"][0]["operands"]["offset_frames"] = json!([offset, IDENTITY]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (script, _) = written(&model, FreeCadWriter::new());
    assert_eq!(assembly_data(&script)["joints"][0]["frame_a"][0][3], 5.0);
    assert_eq!(assembly_data(&script)["joints"][0]["frame_a"][2][3], 10.0);

    for (sense, expected_z) in [("aligned", -1.0), ("opposed", 1.0)] {
        let (script, _) = written(
            &raw_joint_model(json!({"kind": "insert", "sense": sense, "offset": 2.0})),
            FreeCadWriter::new(),
        );
        let data = assembly_data(&script);
        assert_eq!(data["joints"][0]["kind"], "Revolute");
        assert_eq!(data["joints"][0]["frame_a"][2][3], 8.0);
        assert_eq!(data["joints"][0]["frame_b"][2][2], expected_z);
    }
    // Grounding reaches the writer as the core's rigid joint to the assembly itself.
    let mut model = joint_model();
    let assembly = model.assembly.as_mut().unwrap();
    let mut ground = assembly.joints[0].clone();
    ground.a = Vec::new();
    ground.b = vec![0];
    ground.motion = dutchman_core::Motion::Rigid;
    assembly.joints.push(ground);
    let (script, report) = written(&model, FreeCadWriter::new());
    assert_eq!(assembly_data(&script)["held"], json!([[0]]));
    assert!(!report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("inferred")));
    model.assembly.as_mut().unwrap().joints[1].suppressed = true;
    let (_, report) = written(&model, FreeCadWriter::new());
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("suppressed grounding")));

    let mut model = raw_joint_model(json!({"kind": "ball"}));
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    wire["model"]["assembly_joints"][0]["operands"]["connectors"][0]["operand"]["container"]
        ["occurrence_path"] = json!(["NestedPart"]);
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let (script, report) = written(&model, FreeCadWriter::new());
    assert!(assembly_data(&script)["joints"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(report
        .losses()
        .iter()
        .any(|loss| loss.detail.contains("nested or subelement")));
}

#[test]
fn cadir_supplements_keep_the_converted_frame_sense() {
    let mut model = raw_joint_model(json!({"kind": "revolute"}));
    model.assembly.as_mut().unwrap().joints[0].flip = true;
    let (script, _) = written(&model, FreeCadWriter::new());
    let frame = model.assembly.as_ref().unwrap().joints[0].frame_b;
    assert_eq!(assembly_data(&script)["joints"][0]["frame_b"], json!(frame));
}

/// The FreeCAD that `FREECAD_CMD` names (an executable, a wrapper accepting `-c CODE`, or
/// the Flatpak form), or `None` to skip the FreeCAD-backed tests.
fn configured_freecad() -> Option<super::run::FreeCadCommand> {
    std::env::var_os("FREECAD_CMD")?;
    super::run::installed()
}

/// Run a synthetic macro through `FREECAD_CMD` and require its success marker.
fn run_freecad(
    command: &super::run::FreeCadCommand,
    folder: &std::path::Path,
    name: &str,
    script: String,
) {
    let path = folder.join(format!("{name}.FCMacro"));
    std::fs::write(&path, script).unwrap();
    let expression = format!(
        "exec(open({}).read())",
        generate::quoted(&path.to_string_lossy()).unwrap()
    );
    let output = command
        .process(folder)
        .arg("-c")
        .arg(expression)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{name}: {stderr}");
    assert!(
        stdout.contains("DUTCHMAN_JOINT_OK"),
        "{name}: {stdout}\n{stderr}"
    );
    assert!(
        !stdout.contains("Assembly joints failed") && !stderr.contains("Assembly joints failed"),
        "{name}: {stdout}\n{stderr}"
    );
}

/// A two-part assembly whose joint axis is the world X axis through (5, 3, 10): the base
/// at the origin, the arm 10 mm up. `ground` names the explicitly grounded part, if any.
fn off_axis_model(definition: Value, ground: Option<usize>) -> Model {
    use dutchman_core::placement::{compose, inverse, IDENTITY};
    let mut model = raw_joint_model(definition);
    let axis = [
        [0.0, 0.0, 1.0, 5.0],
        [0.0, 1.0, 0.0, 3.0],
        [-1.0, 0.0, 0.0, 10.0],
    ];
    let facing = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, -1.0, 0.0],
    ];
    let mut arm = IDENTITY;
    arm[2][3] = 10.0;
    // CADIR connectors face each other where the parts are placed.
    let frame_b = compose(&compose(&inverse(&arm), &axis), &facing);
    let assembly = model.assembly.as_mut().unwrap();
    assembly.joints[0].frame_a = axis;
    assembly.joints[0].frame_b = frame_b;
    assembly.held = ground.map(|part| vec![vec![part]]).unwrap_or_default();
    let mut wire = serde_json::to_value(&model.ir).unwrap();
    let connectors = &mut wire["model"]["assembly_joints"][0]["operands"]["connectors"];
    connectors[0]["frame"] = json!(axis);
    connectors[1]["frame"] = json!(frame_b);
    if let Some(part) = ground {
        let name = ["base", "arm"][part];
        wire["model"]["assembly_joints"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "id": "synthetic:test:joint#ground", "suppressed": false,
                "operands": {"arity": "grounded", "connector": {
                    "operand": {"container": {"container": "occurrence",
                        "occurrence": format!("synthetic:test:occurrence#{name}")}, "object": "Body"},
                    "frame": IDENTITY, "detached": false}}
            }));
    }
    model.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    model
}

#[test]
fn freecad_moves_hinges_and_sliders_about_their_own_axes() {
    let Some(command) = configured_freecad() else {
        return;
    };
    let folder = std::env::temp_dir().join(format!("dutchman-freecad-axes-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    // `still` is the part that must never move (grounded), `moving` the other one.
    for (name, family, ground, inferred) in [
        ("hinge", "revolute", Some(0), false),
        ("hinge-inferred", "revolute", None, true),
        ("hinge-arm-grounded", "revolute", Some(1), false),
        ("slider", "slider", Some(0), false),
        ("fixed", "fixed", Some(0), false),
    ] {
        let (mut script, report) = written(
            &off_axis_model(json!({"kind": family}), ground),
            FreeCadWriter::new(),
        );
        assert_eq!(
            report
                .losses()
                .iter()
                .any(|loss| loss.detail.contains("grounded, inferred")),
            inferred,
            "{name}: {:?}",
            report.losses()
        );
        // An inferred ground holds the second side, as in the SolidCraft output.
        let still = ground.unwrap_or(1);
        script.push_str(&format!(
            r#"
assemblies = [obj for obj in doc.Objects if obj.TypeId == 'Assembly::AssemblyObject']
assert len(assemblies) == 1, 'Joint reconstruction fell back'
assembly = assemblies[0]
assert len(assembly.Joints) == 1
joint = assembly.Joints[0]
parts = [joint.Reference1[0], joint.Reference2[0]]
still, moving = parts[{still}], parts[1 - {still}]
assert assembly.isPartGrounded(still) and not assembly.isPartGrounded(moving)
placed = [App.Placement(), App.Placement(App.Vector(0, 0, 10), App.Rotation())]
for part, pose in zip(parts, placed):
    assert same_pose(part.Placement, pose), (part.Label, part.Placement)
assert assembly.solve() == 0
for part, pose in zip(parts, placed):
    assert same_pose(part.Placement, pose), ('moved by the solve', part.Label, part.Placement)
start = App.Placement(moving.Placement)
def about_axis(degrees):
    return App.Placement(App.Vector(), App.Rotation(App.Vector(1, 0, 0), degrees), App.Vector(5, 3, 10)).multiply(start)
def solved(pose):
    moving.Placement = pose
    assert assembly.solve() == 0, 'solver error'
    assert same_pose(still.Placement, placed[{still}]), 'the grounded part moved'
    return App.Placement(moving.Placement)
kind = '{family}'
if kind == 'revolute':
    # A turn about the joint axis is free; a turn about another axis is pulled back.
    assert same_pose(solved(about_axis(35)), about_axis(35))
    tilted = App.Placement(App.Vector(), App.Rotation(App.Vector(0, 1, 0), 20), App.Vector(5, 3, 10)).multiply(start)
    back = solved(tilted)
    assert not same_pose(back, tilted)
    axis_point = back.multVec(start.inverse().multVec(App.Vector(5, 3, 10)))
    assert (axis_point - App.Vector(5, 3, 10)).Length < 1e-6, axis_point
    turned = back.Rotation.multiply(start.Rotation.inverted())
    assert abs(abs(turned.Axis.x) - 1) < 1e-6 or turned.Angle < 1e-9, turned
    # Driving turns whichever side is not grounded, the second side positively.
    moving.Placement = start
    drive_joint_angle(assembly, joint, 35)
    assert same_pose(moving.Placement, about_axis(35 if {still} == 0 else -35)), moving.Placement
    assert same_pose(still.Placement, placed[{still}])
elif kind == 'slider':
    along = App.Placement(App.Vector(7, 0, 0), App.Rotation()).multiply(start)
    assert same_pose(solved(along), along)
    back = solved(App.Placement(App.Vector(0, 4, -2), App.Rotation()).multiply(start))
    assert abs(back.Base.y) < 1e-6 and abs(back.Base.z - 10) < 1e-6, back
    assert solved(about_axis(30)).Rotation.isSame(start.Rotation, 1e-6)
else:
    assert same_pose(solved(App.Placement(App.Vector(1, 2, 3), App.Rotation()).multiply(start)), start)
App.Console.PrintMessage('DUTCHMAN_JOINT_OK\n')
"#
        ));
        run_freecad(&command, &folder, &format!("axes-{name}"), script);
    }
    std::fs::remove_dir_all(folder).unwrap();
}

/// A base part, and a sub-assembly of two parts stacked above it: the sub-assembly's
/// parts turn on a hinge between them, and its lower part is fixed to the base.
fn nested_model() -> Model {
    use dutchman_core::placement::IDENTITY;
    let part = model("pad");
    let at = |z: f64| {
        let mut placed = IDENTITY;
        placed[2][3] = z;
        placed
    };
    let facing = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, -1.0, 0.0, 0.0],
        [0.0, 0.0, -1.0, 0.0],
    ];
    let placement = |name: &str, part: usize, transform| dutchman_core::Placement {
        hidden: false,
        name: Some(name.to_owned()),
        part,
        transform,
    };
    let joint = |a: Vec<usize>, b: Vec<usize>, frame_a, motion| dutchman_core::Joint {
        a,
        b,
        frame_a,
        frame_b: facing,
        suppressed: false,
        motion,
        flip: false,
    };
    let sub = Model {
        name: "Arm".to_owned(),
        assembly: Some(dutchman_core::Assembly {
            own_part: None,
            parts: vec![part.clone()],
            placements: vec![
                placement("Lower", 0, at(10.0)),
                placement("Upper", 0, at(20.0)),
            ],
            joints: Vec::new(),
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
            left_out: Vec::new(),
        }),
        ..part.clone()
    };
    Model {
        name: "Nested".to_owned(),
        assembly: Some(dutchman_core::Assembly {
            own_part: None,
            parts: vec![part.clone(), sub],
            placements: vec![
                placement("Base", 0, IDENTITY),
                placement("Arm", 1, IDENTITY),
            ],
            joints: vec![
                joint(
                    vec![1, 0],
                    vec![1, 1],
                    at(10.0),
                    dutchman_core::Motion::Revolute {
                        angle: 0.0,
                        limits: None,
                    },
                ),
                joint(vec![0], vec![1, 0], at(10.0), dutchman_core::Motion::Rigid),
            ],
            held: Vec::new(),
            joint_notes: Vec::new(),
            stored_placements: Vec::new(),
            identities: Vec::new(),
            definitions: Vec::new(),
            left_out: Vec::new(),
        }),
        ..part
    }
}

#[test]
fn joints_inside_a_sub_assembly_are_solved_there() {
    let (script, report) = written(&nested_model(), FreeCadWriter::new());
    let scopes = macro_data(&script)["scopes"].clone();
    // Deepest first: the sub-assembly's hinge, then the top's joint into the sub-assembly.
    assert_eq!(scopes[0]["path"], json!([1]));
    assert_eq!(scopes[0]["joints"][0]["kind"], "Revolute");
    assert_eq!(scopes[0]["joints"][0]["a"], json!([0]));
    assert_eq!(scopes[0]["joints"][0]["b"], json!([1]));
    assert_eq!(scopes[0]["held"], json!([[1]]));
    assert_eq!(scopes[1]["path"], json!([]));
    assert_eq!(scopes[1]["joints"][0]["a"], json!([0]));
    assert_eq!(scopes[1]["joints"][0]["b"], json!([1, 0]));
    assert_eq!(scopes[1]["held"], json!([[1]]));
    let details: Vec<_> = report.losses().iter().map(|loss| &loss.detail).collect();
    assert!(details
        .iter()
        .any(|detail| detail.contains("occurrence 2.2 is grounded, inferred")));
    assert!(!details.iter().any(|detail| detail.contains("nested")));
}

#[test]
fn freecad_solves_a_sub_assembly_inside_its_assembly() {
    let Some(command) = configured_freecad() else {
        return;
    };
    let folder =
        std::env::temp_dir().join(format!("dutchman-freecad-nested-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let (mut script, _) = written(&nested_model(), FreeCadWriter::new());
    script.push_str(
        r#"
assemblies = [obj for obj in doc.Objects if obj.TypeId == 'Assembly::AssemblyObject']
assert len(assemblies) == 2, 'Joint reconstruction fell back'
inner = [a for a in assemblies if a.getParentGeoFeatureGroup() is not None and a.getParentGeoFeatureGroup().TypeId == 'App::Part'][0]
top = [a for a in assemblies if a is not inner][0]
hinge = inner.Joints[0]
lower, upper = hinge.Reference1[0], hinge.Reference2[0]
assert inner.isPartGrounded(upper) and not inner.isPartGrounded(lower)
arm = [obj for obj in top.Group if obj.TypeId == 'App::Part' and obj.Label == 'Arm'][0]
assert top.isPartGrounded(arm)
assert same_pose(lower.Placement, App.Placement(App.Vector(0, 0, 10), App.Rotation()))
assert same_pose(upper.Placement, App.Placement(App.Vector(0, 0, 20), App.Rotation()))
# The hinge turns the lower part about the vertical axis through the upper part.
turned = App.Placement(App.Vector(0, 0, 10), App.Rotation(App.Vector(0, 0, 1), 30))
lower.Placement = turned
assert inner.solve() == 0
assert same_pose(lower.Placement, turned), lower.Placement
assert same_pose(upper.Placement, App.Placement(App.Vector(0, 0, 20), App.Rotation()))
lower.Placement = App.Placement(App.Vector(0, 0, 10), App.Rotation(App.Vector(1, 0, 0), 20))
assert inner.solve() == 0
assert abs(lower.Placement.Rotation.multVec(App.Vector(0, 0, 1)).z - 1) < 1e-6, lower.Placement
App.Console.PrintMessage('DUTCHMAN_JOINT_OK\n')
"#,
    );
    run_freecad(&command, &folder, "nested", script);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn native_document_rebuilds_a_synthetic_pad_when_freecad_is_configured() {
    if std::env::var_os("FREECAD_CMD").is_none() {
        return;
    }
    // A square pad with exactly the same shape as cadmpeg's synthetic 10 mm cube.
    let mut source = model("pad");
    let mut wire = serde_json::to_value(&source.ir).unwrap();
    let entities = (0..4)
        .map(|i| {
            let points = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
            let a = points[i];
            let b = points[(i + 1) % 4];
            json!({"id": format!("synthetic:test:sketch-entity#{}", i + 1),
                "sketch": "synthetic:test:sketch#1", "construction": false,
                "geometry": {"kind": "line", "start": {"u": a[0], "v": a[1]}, "end": {"u": b[0], "v": b[1]}}})
        })
        .collect::<Vec<_>>();
    wire["model"]["sketch_entities"] = json!(entities);
    let profile = (1..=4)
        .map(|i| json!({"entity": format!("synthetic:test:sketch-entity#{i}")}))
        .collect::<Vec<_>>();
    wire["model"]["sketches"][0]["profiles"] = json!([profile]);
    wire["model"]["features"][1]["definition"]["extent"]["side"]["termination"]["length"] =
        json!(10.0);
    source.ir = cadmpeg_ir::CadIr::from_json(&wire.to_string()).unwrap();
    let runner = super::run::FreeCadRunner::new(
        FreeCadWriter::new(),
        true,
        true,
        super::run::DEFAULT_TIMEOUT,
    );
    let mut bytes = Vec::new();
    let mut report = Report::default();
    runner.write(&source, &mut bytes, &mut report).unwrap();
    assert!(bytes.starts_with(b"PK"));
    assert_eq!(
        report.outcome().unwrap().check,
        dutchman_core::Check::Verified
    );
    assert_eq!(report.outcome().unwrap().mode, Mode::History);
    // Reopened without the GUI, only the rebuilt body is visible: the saved document
    // property, not just the view, hides the original STEP.
    let dir = tempfile::tempdir().unwrap();
    let saved = dir.path().join("pad.FCStd");
    std::fs::write(&saved, &bytes).unwrap();
    let check = format!(
        "import FreeCAD as App\n\
         d = App.openDocument({})\n\
         print('VISIBLE=' + ','.join(sorted(o.Name for o in d.Objects \
             if o.TypeId == 'PartDesign::Body' and o.Visibility)))\n",
        generate::quoted(&saved.to_string_lossy()).unwrap()
    );
    let output = configured_freecad()
        .unwrap()
        .process(dir.path())
        .arg("-c")
        .arg(format!("exec({})", generate::quoted(&check).unwrap()))
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.lines().any(|line| line == "VISIBLE=Body"),
        "{stdout}"
    );
}
