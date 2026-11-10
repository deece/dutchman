//! End-to-end checks of the `dutchman` binary.

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::Mutex;

// A fork must not inherit a write descriptor for an executable another test will run.
// Hold this through each spawn (which waits for exec) and each executable publication.
static EXECUTABLE_PUBLICATION: Mutex<()> = Mutex::new(());

trait TestCommand {
    fn test_spawn(&mut self) -> std::io::Result<Child>;
    fn test_output(&mut self) -> std::io::Result<Output>;
}

impl TestCommand for Command {
    fn test_spawn(&mut self) -> std::io::Result<Child> {
        let mut retries = 0;
        loop {
            let result = {
                let _guard = EXECUTABLE_PUBLICATION.lock().unwrap();
                self.spawn()
            };
            match result {
                Err(error)
                    if error.kind() == std::io::ErrorKind::ExecutableFileBusy && retries < 5 =>
                {
                    retries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                result => return result,
            }
        }
    }

    fn test_output(&mut self) -> std::io::Result<Output> {
        self.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .test_spawn()?
            .wait_with_output()
    }
}

fn dutchman(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args(args)
        .test_output()
        .unwrap()
}

/// The synthetic cube plus generated padding: only metadata cost changes, not geometry.
fn memory_budget_cube(folder: &std::path::Path) -> String {
    use std::io::Write;
    let path = folder.join("budget-cube.f3d");
    let mut source =
        zip::ZipArchive::new(std::fs::File::open(fixture("generated_unit_cube.f3d")).unwrap())
            .unwrap();
    let mut target = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for index in 0..source.len() {
        let mut member = source.by_index(index).unwrap();
        let name = member.name().unwrap().into_owned();
        target.start_file(name, options).unwrap();
        std::io::copy(&mut member, &mut target).unwrap();
    }
    target.start_file("memory-budget.bin", options).unwrap();
    target.write_all(&vec![0; 1 << 20]).unwrap();
    target.finish().unwrap();
    path.to_str().unwrap().to_owned()
}

/// Preflight fits, but the calibrated extra check does not. Held memory stays below
/// the separate mapping-stop threshold, so the test reaches checking.
fn memory_between_conversion_and_check(input: &str) -> (String, u64) {
    let options = dutchman::ConvertOptions::default();
    let path = std::path::Path::new(input);
    let conversion = dutchman::choices::estimate_with_options(
        path,
        &options.choices(),
        &dutchman::ConvertOptions {
            no_verify: true,
            ..options.clone()
        },
    )
    .unwrap();
    let needed = dutchman::stop::history_check_bytes(path, &options).unwrap();
    assert!(needed > conversion.peak_bytes);
    let available = conversion.peak_bytes + (needed - conversion.peak_bytes) / 2;
    (format!("100000000,{available}"), available)
}

#[cfg(feature = "pilot")]
#[test]
fn pilot_help_names_the_embedded_command() {
    for args in [vec!["pilot", "--help"], vec!["pilot"]] {
        let out = dutchman(&args);
        assert!(out.status.success());
        let help = String::from_utf8_lossy(&out.stdout);
        assert!(help.contains("Usage:\n  dutchman pilot how"), "{help}");
        assert!(!help.contains("dutchman-pilot how"), "{help}");
    }
    let out = dutchman(&["pilot", "model", "--help"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Usage: dutchman pilot model"));
}

#[cfg(feature = "pilot")]
#[test]
fn pilot_dispatches_and_reports_errors() {
    let out = dutchman(&["pilot", "how", "round all edges 3 mm"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("solid.fillet"));

    let dir = tempfile::tempdir().unwrap();
    let missing_log = dir.path().join("missing.jsonl");
    let out = dutchman(&["pilot", "log-summary", missing_log.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("dutchman: "));

    let out = dutchman(&["pilot", "model", "--upstream-timeout", "0"]);
    assert_eq!(out.status.code(), Some(2));
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("--upstream-timeout"), "{error}");
    assert!(error.contains("1..=3600"), "{error}");
}

#[cfg(not(feature = "pilot"))]
#[test]
fn pilot_is_absent_without_the_feature() {
    let out = dutchman(&["--help"]);
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("pilot"));
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args(["pilot", "--help"])
        .env("PATH", "")
        .test_output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8(out.stderr).unwrap(), "dutchman: `dutchman pilot` is not installed; get it at https://github.com/dutchman-cad/pilot\n");
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dutchman-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn convert_reports_unreadable_solidworks_assembly_cleanly() {
    let input = scratch("assembly.sldasm");
    std::fs::write(&input, b"not really an assembly").unwrap();
    let output = scratch("assembly.solidcraft");

    let out = dutchman(&["convert", input.to_str().unwrap(), output.to_str().unwrap()]);

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot read"), "{stderr}");
    assert!(!output.exists(), "no output file is created on failure");
}

#[test]
fn iges_is_refused_before_decoding_in_text_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out.solidcraft");
    let reason =
        "Dutchman doesn't convert IGES yet: open it in SolidCraft directly (File > Import).";
    for suffix in ["igs", "iges", "IGS", "IGES"] {
        let input = dir.path().join(format!("design.{suffix}"));
        std::fs::write(&input, b"not IGES data").unwrap();
        for command in ["convert", "inspect"] {
            for json in [false, true] {
                let mut args = vec![command, input.to_str().unwrap()];
                if command == "convert" {
                    args.push(output.to_str().unwrap());
                }
                if json {
                    args.push("--json");
                }
                let out = dutchman(&args);
                assert_eq!(out.status.code(), Some(1));
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                if json {
                    let report = json_of(&out);
                    assert_eq!(report["ok"], false);
                    assert_eq!(report["input"]["format"], "iges");
                    assert_eq!(report["input"]["extension"], suffix);
                    assert_eq!(report["input"]["words"], "open it in SolidCraft directly");
                    assert_eq!(report["error"]["message"], reason);
                    assert_eq!(report["error"]["stage"], "decode");
                    assert_eq!(report["error"]["upstream"], serde_json::Value::Null);
                } else {
                    assert_text_error_with_progress(&out, &input, &format!("dutchman: {reason}"));
                    if command == "convert" {
                        assert_eq!(
                            stdout,
                            format!("IGES .{suffix}: open it in SolidCraft directly\n")
                        );
                    } else {
                        assert!(stdout.is_empty());
                    }
                }
                assert!(!stdout.contains("Want to help"));
                assert!(!stderr.contains("Want to help"));
                assert!(!output.exists());
            }
        }
    }
}

#[test]
fn batch_refuses_iges_without_inviting_test_files_and_continues_other_formats() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = dir.path().join("inputs");
    let outputs = dir.path().join("outputs");
    std::fs::create_dir(&inputs).unwrap();
    let first = inputs.join("first.igs");
    let second = inputs.join("second.iges");
    for path in [&first, &second] {
        std::fs::write(path, b"not IGES data").unwrap();
    }
    let reason =
        "Dutchman doesn't convert IGES yet: open it in SolidCraft directly (File > Import).";
    for directory in [false, true] {
        for json in [false, true] {
            let mut args = vec!["convert-all", "--out-dir", outputs.to_str().unwrap()];
            if directory {
                args.push(inputs.to_str().unwrap());
            } else {
                args.extend([first.to_str().unwrap(), second.to_str().unwrap()]);
            }
            if json {
                args.push("--json");
            }
            let out = dutchman(&args);
            assert_eq!(out.status.code(), Some(1));
            if json {
                let report = json_of(&out);
                assert_eq!(report["converted"], 0);
                let results = report["results"].as_array().unwrap();
                assert_eq!(results.len(), 2);
                for result in results {
                    assert_eq!(result["ok"], false);
                    assert_eq!(result["input"]["format"], "iges");
                    assert_eq!(result["error"]["message"], reason);
                }
            } else {
                let text = String::from_utf8_lossy(&out.stdout);
                assert_eq!(text.matches(reason).count(), 2);
                assert!(!text.contains("Want to help"));
            }
            assert!(!outputs.join("first.sc").exists());
            assert!(!outputs.join("second.sc").exists());
        }
    }
    std::fs::copy(fixture("generated_unit_cube.f3d"), inputs.join("cube.f3d")).unwrap();
    let out = dutchman(&[
        "convert-all",
        inputs.to_str().unwrap(),
        "--out-dir",
        outputs.to_str().unwrap(),
        "--no-verify",
        "--json",
    ]);
    let report = json_of(&out);
    assert_eq!(report["converted"], 1);
    assert_eq!(report["failed"], 2);
    assert!(outputs.join("cube.sc").is_file());
}

#[test]
fn convert_rejects_wrong_output_extension() {
    let out = dutchman(&["convert", "part.f3d", "part.step"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("output must be a .sc file"));
}

#[test]
fn convert_rejects_unknown_input() {
    let input = scratch("notes.txt");
    std::fs::write(&input, b"hello").unwrap();
    let out = dutchman(&["convert", input.to_str().unwrap(), "x.solidcraft"]);
    assert_eq!(out.status.code(), Some(1));
    let converts = dutchman_core::Format::supported_names();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!(
            "Dutchman does not convert this kind of file. It converts {converts}."
        )),
        "{stderr}"
    );
}

#[test]
fn convert_reports_missing_input() {
    let out = dutchman(&["convert", "/nonexistent/part.f3d", "x.solidcraft"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn an_unreadable_archive_suggests_recovery_in_text_and_json() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("incomplete.f3d");
    let output = dir.path().join("out.solidcraft");
    std::fs::write(&input, b"not an archive").unwrap();
    let args = [input.to_str().unwrap(), output.to_str().unwrap()];
    let text = dutchman(&["convert", args[0], args[1]]);
    assert_eq!(text.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&text.stderr);
    assert!(stderr.contains("download or copy it again"), "{stderr}");
    assert!(stderr.contains("check its extension"), "{stderr}");
    assert!(
        stderr.contains("template=file-did-not-convert.yml"),
        "{stderr}"
    );
    let json = dutchman(&["convert", "--json", args[0], args[1]]);
    assert_eq!(json.status.code(), Some(1));
    assert_progress_json(&json);
    let report = json_of(&json);
    assert_eq!(report["error"]["stage"], "decode");
    assert_eq!(report["error"]["upstream"], "cadmpeg");
    let message = report["error"]["message"].as_str().unwrap();
    assert_text_error_with_progress(&text, &input, &format!("dutchman: {message}"));
    assert!(!output.exists());
}

#[test]
fn missing_arguments_is_a_usage_error() {
    let out = dutchman(&["convert"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn convert_writes_a_solidcraft_design_from_f3d() {
    let input = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../dutchman-core/tests/fixtures/generated_unit_cube.f3d"
    );
    let output = scratch("cube.solidcraft");

    let out = dutchman(&["convert", input, output.to_str().unwrap()]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read(&output).unwrap().starts_with(b"PK\x03\x04"));
}

#[test]
fn convert_leaves_no_file_when_the_input_has_no_geometry() {
    let input = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../dutchman-core/tests/fixtures/container_metadata_only.f3d"
    );
    let output = scratch("empty.solidcraft");

    let out = dutchman(&["convert", input, output.to_str().unwrap()]);

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no geometry could be read"), "{stderr}");
    assert!(!output.exists(), "no output file is created on failure");
}

#[test]
fn convert_reports_what_became_of_the_history() {
    let input = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../dutchman-core/tests/fixtures/generated_unit_cube.f3d"
    );

    let out = dutchman(&[
        "convert",
        input,
        scratch("flat.solidcraft").to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("history: flattened: the design carries no feature history"),
        "{stderr}"
    );

    let out = dutchman(&[
        "convert",
        "--import-only",
        input,
        scratch("import.solidcraft").to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Info: history: not written (--import-only)"),
        "{stderr}"
    );
}

#[test]
fn solidcraft_and_no_verify_conflict() {
    let out = dutchman(&[
        "convert",
        "--no-verify",
        "--solidcraft",
        "/x",
        "a.f3d",
        "b.solidcraft",
    ]);
    assert_eq!(out.status.code(), Some(2));
}

fn fixture(name: &str) -> String {
    format!(
        "{}/../dutchman-core/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn json_of(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn write_replay_writes_the_script_beside_the_design() {
    let output = scratch("replay.sc");
    let script = output.with_extension("replay.json");
    let _ = std::fs::remove_file(&script);
    let out = dutchman(&[
        "convert",
        "--json",
        "--no-verify",
        "--write-replay",
        &fixture("synthetic_cube.ipt"),
        output.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    let report = json_of(&out);
    assert_eq!(report["replay_script"], script.display().to_string());
    let text = std::fs::read_to_string(&script).unwrap();
    let commands: serde_json::Value = serde_json::from_str(&text).unwrap();
    let commands = commands.as_array().unwrap();
    assert_eq!(commands.len() % 2, 0);
    for pair in commands.chunks(2) {
        assert_eq!(pair[0]["command"], "timeline.roll_to");
        assert_eq!(pair[1]["command"], "document.inspect");
    }
}

#[test]
fn convert_json_reports_what_was_written() {
    let output = scratch("json.solidcraft");
    let out = dutchman(&[
        "convert",
        "--json",
        "--no-verify",
        &fixture("synthetic_cube.ipt"),
        output.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    assert_progress_json(&out);
    let report = json_of(&out);
    assert_eq!(report["schema"], "dutchman.convert/1");
    assert_eq!(report["ok"], true);
    assert_eq!(report["input"]["format"], "inventor_part");
    assert_eq!(
        report["input"]["words"],
        dutchman_core::Format::InventorPart.words()
    );
    assert_eq!(report["output"]["path"], output.to_str().unwrap());
    assert_eq!(report["output"]["format"], "solidcraft/23");
    assert_eq!(report["mode"], "import");
    assert_eq!(report["verification"]["status"], "not_applicable");
    assert_eq!(report["history"]["total"], 0);
    let loss = &report["losses"][0];
    for key in ["level", "code", "subject", "message"] {
        assert!(loss.get(key).is_some(), "{key}: {loss}");
    }
}

#[test]
fn convert_json_reports_a_failure() {
    let out = dutchman(&[
        "convert",
        "--json",
        &fixture("container_metadata_only.f3d"),
        scratch("never.solidcraft").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let report = json_of(&out);
    assert_eq!(report["ok"], false);
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("no geometry could be read"));
}

#[test]
fn inspect_json_shows_what_would_map_without_writing() {
    let out = dutchman(&[
        "inspect",
        "--json",
        &fixture("design_sketch_constraints.f3d"),
    ]);
    assert_eq!(out.status.code(), Some(0));
    let report = json_of(&out);
    assert_eq!(report["schema"], "dutchman.inspect/1");
    assert_eq!(report["geometry"]["bodies"], 1);
    assert_eq!(report["mode"], "import");
    assert_eq!(report["verification"]["status"], "flattened");
    assert_eq!(report["history"]["mapped"], 1);
    assert_eq!(report["history"]["total"], 1);
    let feature = &report["history"]["features"][0];
    assert_eq!(feature["kind"], "sketch");
    assert_eq!(feature["status"], "mapped");
    // Its one body is a sketch's, which no conversion can write; inspect says so.
    assert_eq!(
        report["conversion"],
        serde_json::json!({"possible": false,
            "reason": "the model has no solids or surfaces SolidCraft can import"})
    );
    let text = dutchman(&["inspect", &fixture("design_sketch_constraints.f3d")]);
    assert!(String::from_utf8_lossy(&text.stdout).contains(
        "would write: nothing; converting it fails: the model has no solids or surfaces"
    ));
    let cube = json_of(&dutchman(&[
        "inspect",
        "--json",
        &fixture("generated_unit_cube.f3d"),
    ]));
    assert_eq!(
        cube["conversion"],
        serde_json::json!({"possible": true, "reason": null})
    );
}

#[test]
fn inspect_json_names_what_blocks_a_solidworks_history() {
    let out = dutchman(&["inspect", "--json", &fixture("body_history.sldprt")]);
    assert_eq!(out.status.code(), Some(0));
    let report = json_of(&out);
    assert_eq!(report["input"]["format"], "solidworks_part");
    assert_eq!(
        report["geometry"],
        serde_json::json!({"bodies": 1, "faces": 1})
    );
    assert_eq!(
        report["conversion"],
        serde_json::json!({"possible": false,
            "reason": "the model has no solids or surfaces SolidCraft can import"})
    );
    assert_eq!(report["mode"], "import");
    assert_eq!(report["verification"]["status"], "flattened");
    assert_eq!(
        report["verification"]["reason"],
        "feature \"Boss\": unresolved profiles are not mapped yet"
    );
    let features: Vec<_> = report["history"]["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["name"].as_str().unwrap(),
                f["kind"].as_str().unwrap(),
                f["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        features,
        [
            ("Boss", "extrude", "blocked"),
            ("Profile", "native", "blocked")
        ]
    );
}

#[test]
fn inspect_prints_a_text_summary() {
    let out = dutchman(&["inspect", &fixture("generated_unit_cube.f3d")]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("Fusion archive (.f3d): 1 body, 6 faces\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("would write: geometry without history"),
        "{stdout}"
    );
}

#[test]
fn formats_markdown_matches_the_formats_page() {
    let out = dutchman(&["formats", "--markdown"]);
    assert!(out.status.success(), "{out:?}");
    let table = String::from_utf8(out.stdout).unwrap();
    let page = include_str!("../../../docs/formats.md");
    let block = page
        .split_once("<!-- formats: generated from dutchman formats --markdown -->")
        .unwrap()
        .1
        .split_once("<!-- /formats -->")
        .unwrap()
        .0;
    // Ignore padding, and the FreeCAD row in a build without FreeCAD: the page shows the
    // default build.
    let rows = |text: &str| {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter(|line| cfg!(feature = "freecad") || !line.starts_with("| FreeCAD document |"))
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        rows(&table),
        rows(block),
        "Update the table in docs/formats.md: dutchman formats --markdown"
    );
}

#[test]
fn formats_lists_what_converts() {
    let out = dutchman(&["formats", "--json"]);
    assert_eq!(out.status.code(), Some(0));
    let report = json_of(&out);
    assert_eq!(report["schema"], "dutchman.formats/1");
    let fusion = report["formats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "fusion_archive")
        .unwrap();
    assert_eq!(fusion["status"], "converts");
    assert_eq!(fusion["level"], "L5");
    let unclassified = report["formats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "unclassified_part")
        .unwrap();
    assert_eq!(unclassified.get("level"), Some(&serde_json::Value::Null));
    assert_eq!(fusion["extension"], "f3d");
    let solidworks = report["formats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "solidworks_part")
        .unwrap();
    assert_eq!(solidworks["status"], "converts");
    let assembly = report["formats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "solidworks_assembly")
        .unwrap();
    assert_eq!(assembly["status"], "converts");
    assert_eq!(assembly["words"], "reads geometry");
    assert!(assembly["converts"]
        .as_str()
        .unwrap()
        .contains("sub-assemblies"));

    let iges = report["formats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "iges")
        .unwrap();
    assert_eq!(iges["status"], "not_supported");
    assert_eq!(iges["words"], "open it in SolidCraft directly");
    assert_eq!(
        iges["converts"],
        "SolidCraft imports IGES itself (File > Import)"
    );

    let text = dutchman(&["formats"]);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.contains("Inventor assembly (.iam)"), "{stdout}");
    assert!(stdout.contains("IGES (.igs, .iges)  open it in SolidCraft directly  L9  not yet: SolidCraft imports IGES itself (File > Import)"), "{stdout}");
}

#[test]
fn a_multi_document_archive_converts_like_a_fusion_archive() {
    let input = scratch("design.f3z");
    std::fs::copy(fixture("generated_unit_cube.f3d"), &input).unwrap();
    let output = scratch("from_f3z.solidcraft");
    let out = dutchman(&[
        "convert",
        "--json",
        "--no-verify",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(json_of(&out)["input"]["format"], "fusion_multi_document");

    let noise = scratch("noise.f3z");
    std::fs::write(&noise, [0xffu8; 512]).unwrap();
    let out = dutchman(&["convert", noise.to_str().unwrap(), output.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot read this design"));
}

#[test]
fn convert_all_converts_a_folder_and_reports_each_file() {
    let root = std::env::temp_dir().join(format!("dutchman-batch-{}", std::process::id()));
    let project = root.join("project");
    let nested = project.join("parts");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::copy(fixture("generated_unit_cube.f3d"), project.join("cube.f3d")).unwrap();
    std::fs::copy(fixture("synthetic_cube.ipt"), nested.join("block.ipt")).unwrap();
    std::fs::write(nested.join("broken.f3d"), [0xffu8; 256]).unwrap();
    std::fs::write(project.join("notes.txt"), b"not a design").unwrap();
    let out_dir = root.join("out");
    let missing = root.join("missing.f3d");

    let out = dutchman(&[
        "convert-all",
        "--json",
        "--no-verify",
        "--jobs",
        "2",
        "--out-dir",
        out_dir.to_str().unwrap(),
        project.to_str().unwrap(),
        missing.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1), "a failure makes the run fail");
    let report = json_of(&out);
    assert_eq!(report["schema"], "dutchman.convert-all/1");
    assert_eq!(report["ok"], false);
    assert_eq!(report["converted"], 2);
    assert_eq!(report["failed"], 2);
    assert_eq!(report["skipped"], 1, "notes.txt");
    let results = report["results"].as_array().unwrap();
    assert_eq!(
        results.len(),
        4,
        "the text file is not a design and is skipped"
    );
    for result in results {
        assert_eq!(result["schema"], "dutchman.convert/1");
    }
    assert!(out_dir.join("cube.sc").is_file());
    assert!(out_dir.join("parts").join("block.sc").is_file());
    // The whole text report, every loss in full, beside the designs.
    let log = std::fs::read_to_string(out_dir.join("dutchman.log")).unwrap();
    assert!(
        log.contains("converted") && log.contains("cube.f3d"),
        "{log}"
    );
    assert!(log.contains("missing.f3d: no such file or folder"), "{log}");
    assert!(log.ends_with("2 converted, 2 failed, 1 skipped\n"), "{log}");
    assert!(!out_dir.join("parts").join("broken.sc").exists());

    let text = dutchman(&[
        "convert-all",
        "--quiet",
        "--no-verify",
        "--out-dir",
        out_dir.to_str().unwrap(),
        project.join("cube.f3d").to_str().unwrap(),
    ]);
    assert_eq!(text.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.ends_with("1 converted, 0 failed\n"), "{stdout}");
    // Each converted file's losses are listed beneath it, as `convert` prints them, after the
    // SolidCraft version it targets.
    let lines = without_target_line(&stdout);
    assert_eq!(lines[0], "0 assemblies, 1 part, 0 STEP files");
    let lines = &lines[1..];
    assert_eq!(lines[0], "Fusion .f3d: reads geometry");
    assert!(lines[1].starts_with("converted  "), "{stdout}");
    assert!(
        lines[2..lines.len() - 1]
            .iter()
            .all(|line| line.starts_with("           ") && line.contains(": ")),
        "{stdout}"
    );
    assert!(
        stdout.contains("Degraded: f3d/material.not-transferred"),
        "{stdout}"
    );

    // A folder with a file Dutchman does not know says so in the counts.
    let text = dutchman(&[
        "convert-all",
        "--quiet",
        "--no-verify",
        "--out-dir",
        out_dir.to_str().unwrap(),
        nested.to_str().unwrap(),
        project
            .join("notes.txt")
            .parent()
            .unwrap()
            .join("cube.f3d")
            .to_str()
            .unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.ends_with("2 converted, 1 failed\n"), "{stdout}");
    let text = dutchman(&[
        "convert-all",
        "--quiet",
        "--no-verify",
        "--out-dir",
        out_dir.to_str().unwrap(),
        project.to_str().unwrap(),
    ]);
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(
        stdout.ends_with("2 converted, 1 failed, 1 skipped (not in a format Dutchman knows)\n"),
        "{stdout}"
    );
}

#[test]
fn convert_says_what_it_wrote_and_cuts_long_notes_short() {
    let output = scratch("plain.solidcraft");
    let cube = fixture("solidworks_cube.sldprt");
    let out = dutchman(&[
        "convert",
        "--quiet",
        "--no-verify",
        &cube,
        output.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        without_target_line(&stdout).join("\n"),
        format!(
            "SolidWorks .sldprt: reads geometry\nconverted  {cube} -> {} (geometry without history)",
            output.display()
        )
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(
            "step/native.passthrough-record-omitted: 2 uninterpreted passthrough record(s)"
        ),
        "{stderr}"
    );
    assert!(
        stderr.contains("step/native.source-record-omitted: 1 source-native record(s)"),
        "{stderr}"
    );
    assert!(
        stderr.contains("so no declared identity was verified. …\n"),
        "{stderr}"
    );
    assert!(!stderr.contains("Agreement is consistency"), "{stderr}");
    assert!(
        stderr.ends_with("--verbose prints every note in full)\n"),
        "{stderr}"
    );

    let out = dutchman(&[
        "convert",
        "--no-verify",
        "--verbose",
        &cube,
        output.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Agreement is consistency, not a declaration."),
        "{stderr}"
    );
    assert!(!stderr.contains("--verbose prints"), "{stderr}");
}

#[test]
fn no_clobber_keeps_an_existing_design() {
    let dir = std::env::temp_dir().join(format!("dutchman-noclobber-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("cube.sc");
    std::fs::write(&output, b"SENTINEL").unwrap();
    let cube = fixture("synthetic_cube.ipt");

    let out = dutchman(&[
        "convert",
        "--no-verify",
        "--no-clobber",
        &cube,
        output.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already exists"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read(&output).unwrap(), b"SENTINEL");

    // Without it, the existing file is replaced, as `--help` says.
    let out = dutchman(&["convert", "--no-verify", &cube, output.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert_ne!(std::fs::read(&output).unwrap(), b"SENTINEL");
    let help = dutchman(&["convert", "--help"]);
    assert!(String::from_utf8_lossy(&help.stdout).contains("An explicit output is replaced"));

    // convert-all fails that one file and converts the rest.
    let out_dir = dir.join("all");
    std::fs::create_dir_all(&out_dir).unwrap();
    std::fs::write(out_dir.join("synthetic_cube.sc"), b"SENTINEL").unwrap();
    let out = dutchman(&[
        "convert-all",
        "--json",
        "--no-verify",
        "--no-clobber",
        "--out-dir",
        out_dir.to_str().unwrap(),
        &cube,
        &fixture("generated_unit_cube.f3d"),
    ]);
    let report = json_of(&out);
    assert_eq!(report["converted"], 1);
    assert_eq!(report["failed"], 1);
    assert_eq!(
        std::fs::read(out_dir.join("synthetic_cube.sc")).unwrap(),
        b"SENTINEL"
    );
    assert!(out_dir.join("generated_unit_cube.sc").is_file());
}

/// A library whose folder, files and parts carry names that must not reach a shared report.
fn private_library(root: &std::path::Path) -> PathBuf {
    let library = root.join("AcmeClient_Confidential");
    let nested = library.join("Prototype_Gearbox_X93");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::copy(
        fixture("generated_unit_cube.f3d"),
        library.join("Housing_Rev7_Secret.f3d"),
    )
    .unwrap();
    std::fs::copy(
        fixture("synthetic_cube.ipt"),
        nested.join("Bracket_K42.ipt"),
    )
    .unwrap();
    std::fs::write(nested.join("Broken_Widget_Q8.f3d"), [0xffu8; 256]).unwrap();
    std::fs::write(library.join("pricing_notes.txt"), b"not a design").unwrap();
    library
}

#[test]
fn the_anonymous_report_names_no_file_and_matches_its_ids_to_the_files() {
    let root = std::env::temp_dir().join(format!("dutchman-anon-{}", std::process::id()));
    let library = private_library(&root);
    let out_dir = root.join("Converted_For_AcmeClient");
    let anon = root.join("report-anon.json");
    let key = root.join("report-key.json");
    let missing = library.join("Missing_Part_Z5.f3d");

    let out = dutchman(&[
        "convert-all",
        "--no-verify",
        "--out-dir",
        out_dir.to_str().unwrap(),
        "--report-anon",
        anon.to_str().unwrap(),
        "--report-key",
        key.to_str().unwrap(),
        library.to_str().unwrap(),
        missing.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let salt = stderr
        .split("the salt ")
        .nth(1)
        .and_then(|rest| rest.get(..32))
        .unwrap()
        .to_owned();

    let text = std::fs::read_to_string(&anon).unwrap();
    for secret in [
        root.to_str().unwrap(),
        "AcmeClient",
        "Confidential",
        "Prototype_Gearbox",
        "Gearbox",
        "Housing_Rev7",
        "Secret",
        "Bracket_K42",
        "Broken_Widget",
        "Missing_Part",
        "pricing",
        "Converted_For",
        &salt,
    ] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["schema"], "dutchman.report-anon/1");
    assert_eq!(report["totals"]["files"], 4);
    assert_eq!(report["totals"]["converted"], 2);
    assert_eq!(report["totals"]["failed"], 2);
    assert_eq!(report["totals"]["skipped"], 1);
    assert_eq!(report["skipped_by_extension"]["txt"], 1);
    assert_eq!(report["formats"]["fusion_archive"]["files"], 3);
    assert_eq!(report["formats"]["inventor_part"]["converted"], 1);
    assert_eq!(report["stages"]["decode"]["failed"], 1, "{report}");
    assert_eq!(report["stages"]["input"]["failed"], 1, "{report}");
    assert!(report["versions"]["dutchman"].is_string());
    assert!(report["versions"]["cadmpeg"]
        .as_str()
        .unwrap()
        .starts_with("0."));
    assert_eq!(
        report["timing"]["count"], 3,
        "the missing file was never timed"
    );
    assert!(report["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|loss| loss["code"] == "f3d/material.not-transferred" && loss["stage"] == "decode"));

    // The key, kept by whoever ran the batch, says which file each id is; the same salt
    // gives the same ids again.
    let key: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&key).unwrap()).unwrap();
    assert_eq!(key["salt"], salt.as_str());
    let failed = report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|error| error["stage"] == "decode")
        .unwrap();
    let id = failed["file_ids"][0].as_str().unwrap();
    assert!(key["files"][id]
        .as_str()
        .unwrap()
        .ends_with("Broken_Widget_Q8.f3d"));
    let again = root.join("again.json");
    dutchman(&[
        "convert-all",
        "--no-verify",
        "--out-dir",
        out_dir.to_str().unwrap(),
        "--report-anon",
        again.to_str().unwrap(),
        "--report-salt",
        &salt,
        library.to_str().unwrap(),
    ]);
    let again = std::fs::read_to_string(&again).unwrap();
    assert!(again.contains(id), "{again}");
}

#[test]
fn a_journal_lets_a_stopped_run_carry_on_without_converting_files_again() {
    let root = std::env::temp_dir().join(format!("dutchman-journal-{}", std::process::id()));
    let library = private_library(&root);
    let out_dir = root.join("out");
    let journal = root.join("journal.jsonl");
    let run = || {
        json_of(&dutchman(&[
            "convert-all",
            "--json",
            "--no-verify",
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--journal",
            journal.to_str().unwrap(),
            library.to_str().unwrap(),
        ]))
    };
    let first = run();
    assert_eq!(first["resumed"], 0);
    assert_eq!(first["converted"], 2);
    assert_eq!(
        std::fs::read_to_string(&journal).unwrap().lines().count(),
        3
    );
    let second = run();
    assert_eq!(second["resumed"], 3, "{second}");
    assert_eq!(second["converted"], 2);
    assert_eq!(second["failed"], 1);

    // A design removed since is made again; a changed file is converted again.
    std::fs::remove_file(out_dir.join("Housing_Rev7_Secret.sc")).unwrap();
    let third = run();
    assert_eq!(third["resumed"], 2, "{third}");
    assert!(out_dir.join("Housing_Rev7_Secret.sc").is_file());
}

#[test]
fn a_file_over_its_time_or_memory_is_stopped_and_the_batch_carries_on() {
    let root = std::env::temp_dir().join(format!("dutchman-limits-{}", std::process::id()));
    let library = private_library(&root);
    let out = dutchman(&[
        "convert-all",
        "--json",
        "--no-verify",
        "--max-memory",
        "1",
        "--out-dir",
        root.join("out").to_str().unwrap(),
        library.to_str().unwrap(),
    ]);
    let report = json_of(&out);
    // Every file still has a report: a converter stopped for its memory fails alone.
    assert_eq!(report["results"].as_array().unwrap().len(), 3, "{report}");
    for result in report["results"].as_array().unwrap() {
        if let Some(message) = result["error"]["message"].as_str() {
            if message.starts_with("stopped when it held") {
                assert_ne!(result["error"]["stage"], serde_json::Value::Null);
            }
        }
    }
}

#[cfg(feature = "freecad")]
#[test]
fn a_freecad_document_converts() {
    let out_dir = tempfile::tempdir().unwrap();
    let output = out_dir.path().join("product.solidcraft");
    let out = dutchman(&[
        "convert",
        "--json",
        "--no-verify",
        &fixture("core_design_product.FCStd"),
        output.to_str().unwrap(),
    ]);
    let report = json_of(&out);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["input"]["format"], "freecad_document");
    assert!(output.is_file());
    let formats = json_of(&dutchman(&["formats", "--json"]));
    let freecad = formats["formats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"] == "freecad_document")
        .unwrap();
    assert_eq!(freecad["status"], "converts");
}

#[cfg(not(feature = "freecad"))]
#[test]
fn a_build_without_freecad_says_where_freecad_files_convert() {
    let out_dir = tempfile::tempdir().unwrap();
    let output = out_dir.path().join("product.solidcraft");
    let out = dutchman(&[
        "convert",
        "--json",
        &fixture("core_design_product.FCStd"),
        output.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let report = json_of(&out);
    assert_eq!(
        report["error"]["message"],
        "this build leaves FreeCAD files out (built without the `freecad` feature); \
         Dutchman's release builds convert them: https://github.com/dutchman-cad/dutchman/releases"
    );
    assert_eq!(report["error"]["upstream"], serde_json::Value::Null);
    assert!(!output.exists());
    let text = String::from_utf8(dutchman(&["formats"]).stdout).unwrap();
    let line = text.lines().find(|l| l.starts_with("FreeCAD")).unwrap();
    assert!(
        line.contains("not yet") && line.contains("left out of this build"),
        "{line}"
    );
}

#[test]
fn freecad_macro_output_is_available_without_freecad_input_support() {
    let dir = tempfile::tempdir().unwrap();
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures/generated_unit_cube.f3d");
    let output = dir.path().join("out.FCMacro");
    let result = dutchman(&[
        "convert",
        input.to_str().unwrap(),
        "--to",
        "freecad",
        "-o",
        output.to_str().unwrap(),
        "--json",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["output"]["format"], "freecad-macro/1");
    let script = std::fs::read_to_string(output).unwrap();
    assert!(script.contains("original_step("));
    assert!(script.contains("ISO-10303-21"));
}

#[test]
fn unknown_tool_explains_where_to_install_it() {
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .arg("unknown-tool-for-test")
        .env("PATH", "")
        .test_output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8(out.stderr).unwrap(), "dutchman: `dutchman unknown-tool-for-test` is not installed; get it at https://github.com/dutchman-cad/unknown-tool-for-test\n");
}

#[test]
fn builtin_typos_keep_clap_suggestions() {
    let out = dutchman(&["conver"]);
    assert_eq!(out.status.code(), Some(2));
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("similar subcommand"), "{text}");
    assert!(text.contains("convert"), "{text}");
}

#[cfg(unix)]
mod external_tools {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &std::path::Path, name: &str, body: &str) {
        let path = dir.join(format!("dutchman-{name}"));
        executable(&path, |file| {
            write!(file, "#!/bin/sh\n{body}\n").unwrap();
        });
    }

    fn executable(path: &std::path::Path, write: impl FnOnce(&mut std::fs::File)) {
        let _guard = EXECUTABLE_PUBLICATION.lock().unwrap();
        let mut temporary = tempfile::NamedTempFile::new_in(path.parent().unwrap()).unwrap();
        write(temporary.as_file_mut());
        temporary.as_file().sync_all().unwrap();
        // Close the writer before chmod and rename. Publication alone cannot prevent
        // inherited writers; the lock also excludes forks until the writer is closed.
        let temporary = temporary.into_temp_path();
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&temporary, path).unwrap();
    }

    #[test]
    fn memory_skipping_launches_no_rebuild_and_forcing_really_checks() {
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("solidcraft-cli");
        let marker = dir.path().join("replay-called");
        executable(&cli, |file| {
            file.write_all(
                br#"#!/bin/sh
if [ "$1" = "--version" ]; then
    printf 'solidcraft-cli 0.41.0\n'
    exit 0
fi
printf 'replay\n' >> "$DUTCHMAN_TEST_REPLAY_MARKER"
exit 1
"#,
            )
            .unwrap();
        });
        let input = memory_budget_cube(dir.path());
        let (memory, _) = memory_between_conversion_and_check(&input);
        let output = dir.path().join("cube.sc");
        let args = [
            "convert",
            &input,
            "-o",
            output.to_str().unwrap(),
            "--solidcraft-version",
            "0.41.0",
            "--json",
        ];
        // Preflight fits while the calibrated check does not.
        let env = [
            ("DUTCHMAN_TEST_MEMORY", memory.as_str()),
            ("SOLIDCRAFT_CLI", cli.to_str().unwrap()),
            ("DUTCHMAN_TEST_REPLAY_MARKER", marker.to_str().unwrap()),
        ];
        let skipped = dutchman_with(&args, &env);
        assert!(skipped.status.success());
        assert!(json_of(&skipped)["history_check_skipped"].is_object());
        assert!(!marker.exists());
        // The old 3.15x resident-set reserve falsely skipped this case. A check
        // that fits its metadata-based allowance must actually launch replay.
        let ample = [
            ("DUTCHMAN_TEST_MEMORY", "14500000000,15600000000"),
            ("SOLIDCRAFT_CLI", cli.to_str().unwrap()),
            ("DUTCHMAN_TEST_REPLAY_MARKER", marker.to_str().unwrap()),
        ];
        let checked_by_default = dutchman_with(&args, &ample);
        assert!(checked_by_default.status.success());
        assert!(json_of(&checked_by_default)
            .get("history_check_skipped")
            .is_none());
        assert!(marker.exists());
        std::fs::remove_file(&marker).unwrap();
        let mut forced = args.to_vec();
        forced.push("--no-memory-check");
        let checked = dutchman_with(&forced, &env);
        assert!(checked.status.success());
        assert!(json_of(&checked).get("history_check_skipped").is_none());
        assert!(marker.exists());
        std::fs::remove_file(&marker).unwrap();
        forced.push("--no-verify");
        assert!(dutchman_with(&forced, &env).status.success());
        assert!(!marker.exists());
    }

    #[test]
    fn dispatch_preserves_stdin_arguments_and_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        script(
            dir.path(),
            "foo",
            "printf '<%s>\\n' \"$@\"; IFS= read -r line; printf 'stdin:%s\\n' \"$line\"; exit 7",
        );
        let mut child = Command::new(env!("CARGO_BIN_EXE_dutchman"))
            .args(["foo", "a", "b c"])
            .env("PATH", dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .test_spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"hello world\n")
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(7));
        assert_eq!(out.stdout, b"<a>\n<b c>\nstdin:hello world\n");
    }

    #[test]
    fn printer_commands_dispatch_to_the_print_companion() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("dutchman");
        executable(&binary, |file| {
            let mut source = std::fs::File::open(env!("CARGO_BIN_EXE_dutchman")).unwrap();
            std::io::copy(&mut source, file).unwrap();
        });
        script(dir.path(), "print", "printf '<%s>\\n' \"$@\"; exit 7");
        for (args, expected) in [
            (
                vec!["printers", "--json", "--config", "a b.toml"],
                "<printers>\n<--json>\n<--config>\n<a b.toml>\n",
            ),
            (
                vec!["--verbose", "printers", "--json"],
                "<printers>\n<--json>\n",
            ),
            (
                vec!["print", "a b.3mf", "--printer", "demo", "--yes"],
                "<print>\n<a b.3mf>\n<--printer>\n<demo>\n<--yes>\n",
            ),
        ] {
            let out = Command::new(&binary)
                .args(args)
                .env("PATH", dir.path())
                .test_output()
                .unwrap();
            assert_eq!(out.status.code(), Some(7));
            assert_eq!(out.stdout, expected.as_bytes());
        }
    }

    #[test]
    fn installed_pilot_accepts_its_own_flags_before_clap() {
        let dir = tempfile::tempdir().unwrap();
        script(
            dir.path(),
            "pilot",
            "printf 'installed Pilot'; printf ' <%s>' \"$@\"; printf '\\n'",
        );
        for args in [
            vec!["pilot", "--version"],
            vec!["--verbose", "pilot", "--own-flag", "b c"],
        ] {
            let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
                .args(&args)
                .env("PATH", dir.path())
                .test_output()
                .unwrap();
            assert!(out.status.success());
            let expected = if args.len() == 2 {
                "installed Pilot <--version>\n"
            } else {
                "installed Pilot <--own-flag> <b c>\n"
            };
            assert_eq!(out.stdout, expected.as_bytes());
        }
    }

    #[test]
    fn adjacent_tool_is_preferred_and_help_lists_it() {
        let adjacent = tempfile::tempdir().unwrap();
        let path = tempfile::tempdir().unwrap();
        let binary = adjacent.path().join("dutchman");
        executable(&binary, |file| {
            let mut source = std::fs::File::open(env!("CARGO_BIN_EXE_dutchman")).unwrap();
            std::io::copy(&mut source, file).unwrap();
        });
        script(adjacent.path(), "foo", "echo adjacent-1.0");
        script(path.path(), "foo", "echo path-2.0");
        let out = Command::new(&binary)
            .arg("foo")
            .env("PATH", path.path())
            .test_output()
            .unwrap();
        assert_eq!(out.stdout, b"adjacent-1.0\n");
        for args in [vec![], vec!["--help"]] {
            let out = Command::new(&binary)
                .args(args)
                .env("PATH", path.path())
                .test_output()
                .unwrap();
            assert!(out.status.success());
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                text.contains("Installed Dutchman CAD tools:\n  foo  adjacent-1.0"),
                "{text}"
            );
            assert!(!text.contains("path-2.0"), "{text}");
        }
    }

    #[test]
    fn non_utf8_arguments_pass_through() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        script(dir.path(), "foo", "printf '%s' \"$1\"");
        let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
            .arg("foo")
            .arg(std::ffi::OsString::from_vec(vec![0xff, b'a']))
            .env("PATH", dir.path())
            .test_output()
            .unwrap();
        assert_eq!(out.stdout, [0xff, b'a']);
    }

    #[test]
    fn resolution_prefers_builtins_then_installed_tools_then_files() {
        let dir = tempfile::tempdir().unwrap();
        script(dir.path(), "formats", "echo external-formats");
        script(dir.path(), "foo", "echo external-foo");
        std::fs::write(dir.path().join("foo"), b"sentinel").unwrap();
        let run = |args: &[&str]| {
            Command::new(env!("CARGO_BIN_EXE_dutchman"))
                .current_dir(dir.path())
                .env("PATH", dir.path())
                .args(args)
                .test_output()
                .unwrap()
        };
        let out = run(&["formats"]);
        assert!(out.status.success());
        assert!(!String::from_utf8_lossy(&out.stdout).contains("external-formats"));
        assert_eq!(run(&["foo"]).stdout, b"external-foo\n");
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../dutchman-core/tests/fixtures/generated_unit_cube.f3d"
            ),
            dir.path().join("pilot"),
        )
        .unwrap();
        let out = run(&["convert", "pilot", "pilot.solidcraft", "--no-verify"]);
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains("does not convert this kind of file"));
    }

    #[test]
    fn installed_tool_with_builtin_typo_name_runs() {
        let dir = tempfile::tempdir().unwrap();
        script(dir.path(), "conver", "echo installed");
        let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
            .arg("conver")
            .env("PATH", dir.path())
            .test_output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout, b"installed\n");
    }
}

#[cfg(feature = "pilot")]
#[test]
fn pilot_version_falls_back_to_builtin() {
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args(["pilot", "--version"])
        .env("PATH", "")
        .test_output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("0.1.0-beta.1"));
}

#[test]
fn bare_and_explicit_conversion_derive_and_protect_default_outputs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../dutchman-core/tests/fixtures/generated_unit_cube.f3d"
        ),
        dir.path().join("part.f3d"),
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_dutchman"))
            .current_dir(dir.path())
            .args(args)
            .env("PATH", "")
            .test_output()
            .unwrap()
    };
    for prefix in [vec![], vec!["convert"], vec!["--verbose"]] {
        let mut args = prefix;
        args.extend(["part.f3d", "--no-verify"]);
        let out = run(&args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let output = dir.path().join("part.sc");
        assert!(std::fs::read(&output).unwrap().starts_with(b"PK"));
        std::fs::remove_file(output).unwrap();
    }
    let output = dir.path().join("part.sc");
    std::fs::write(&output, b"sentinel").unwrap();
    let modified = output.metadata().unwrap().modified().unwrap();
    let out = run(&["part.f3d"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out.stderr, b"dutchman: part.sc already exists; choose another output with -o PATH, or replace it with --force\n");
    assert_eq!(std::fs::read(&output).unwrap(), b"sentinel");
    assert_eq!(output.metadata().unwrap().modified().unwrap(), modified);
    assert!(run(&["part.f3d", "--force", "--no-verify"])
        .status
        .success());
    assert_ne!(std::fs::read(&output).unwrap(), b"sentinel");
    std::fs::write(&output, b"sentinel").unwrap();
    assert!(run(&["part.f3d", "part.sc", "--no-verify"])
        .status
        .success());
    assert_ne!(std::fs::read(&output).unwrap(), b"sentinel");
    assert_eq!(
        run(&["part.f3d", "--force", "--no-clobber"]).status.code(),
        Some(2)
    );
    assert!(run(&["part.f3d", "-o", "other.sc", "--no-verify"])
        .status
        .success());
}

#[test]
fn default_output_extensions_and_input_protection() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_dutchman"))
            .current_dir(dir.path())
            .args(args)
            .test_output()
            .unwrap()
    };
    std::fs::write(dir.path().join("part.FCStd"), b"sentinel").unwrap();
    for force in [vec![], vec!["--force"]] {
        let mut args = vec!["convert", "part.FCStd", "--to", "fcstd"];
        args.extend(force);
        let out = run(&args);
        assert_eq!(out.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&out.stderr).contains("is the input file"));
        assert_eq!(
            std::fs::read(dir.path().join("part.FCStd")).unwrap(),
            b"sentinel"
        );
    }
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../dutchman-core/tests/fixtures/generated_unit_cube.f3d"
        ),
        dir.path().join("part.F3D"),
    )
    .unwrap();
    assert!(run(&["part.F3D", "--to", "freecad"]).status.success());
    assert!(dir.path().join("part.FCMacro").is_file());
    for ext in ["step", "StP", "IPT", "f3z", "sldprt"] {
        let out = run(&[&format!("missing.{ext}")]);
        assert_eq!(out.status.code(), Some(1));
        assert!(!String::from_utf8_lossy(&out.stderr).contains("is not installed"));
    }
}

#[test]
fn builtin_command_wins_over_a_same_named_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("formats"), b"sentinel").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .current_dir(dir.path())
        .arg("formats")
        .test_output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Fusion"));
}

#[test]
fn format_status_and_volunteer_invitation_follow_run_options() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.prt");
    let second = dir.path().join("second.prt");
    for path in [&first, &second] {
        std::fs::write(path, b"#UGC:2").unwrap();
    }
    let output = dir.path().join("out.sc");
    for option in [None, Some("--quiet"), Some("--json")] {
        let mut args = vec!["convert", first.to_str().unwrap(), output.to_str().unwrap()];
        args.extend(option);
        let out = dutchman(&args);
        assert_eq!(out.status.code(), Some(1));
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        if option == Some("--json") {
            let report = json_of(&out);
            assert_eq!(report["input"]["format"], "creo_part");
            assert_eq!(report["input"]["words"], "opens, no geometry yet");
            assert!(!stderr.contains("Want to help"));
        } else {
            assert!(stdout.starts_with("Creo .prt: opens, no geometry yet\n"));
        }
        assert_eq!(
            stdout.matches("Want to help").count(),
            usize::from(option.is_none())
        );
        assert!(!stdout.contains("unknown format"));
        assert!(!output.exists());
        let mut args = vec![
            "convert-all",
            first.to_str().unwrap(),
            second.to_str().unwrap(),
            "--out-dir",
            dir.path().to_str().unwrap(),
        ];
        args.extend(option);
        let out = dutchman(&args);
        assert_eq!(out.status.code(), Some(1));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            stdout.matches("Want to help").count(),
            usize::from(option.is_none())
        );
        if option.is_none() {
            let pitch = stdout
                .lines()
                .find(|l| l.starts_with("Want to help"))
                .unwrap();
            assert!(pitch.starts_with("Want to help Dutchman read Creo better?"));
            assert!(!pitch.contains("first") && !pitch.contains("second"));
        }
    }
}

#[test]
fn every_listed_format_has_reader_status() {
    let report = json_of(&dutchman(&["formats", "--json"]));
    let formats = report["formats"].as_array().unwrap();
    assert_eq!(formats.len(), dutchman_core::Format::ALL.len());
    for format in dutchman_core::Format::ALL {
        let row = formats.iter().find(|row| row["id"] == format.id()).unwrap();
        assert_eq!(row["words"], format.words());
        assert_eq!(
            row.get("level"),
            Some(&serde_json::json!(format.headline_level()))
        );
    }
}

#[test]
fn an_inconclusive_part_is_recognised_and_refused() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("inconclusive.prt");
    std::fs::write(&input, b"no recognised signature").unwrap();
    let output = dir.path().join("out.solidcraft");
    let out = dutchman(&[
        "convert",
        "--json",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let report = json_of(&out);
    assert_eq!(report["input"]["format"], "unclassified_part");
    assert_eq!(report["input"]["words"], "recognised, not read yet");
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not supported yet"));
    assert!(!output.exists());
}

#[test]
fn numbered_saves_select_the_newest_and_report_the_older_revision() {
    let dir = tempfile::tempdir().unwrap();
    let inputs = dir.path().join("inputs");
    let outputs = dir.path().join("outputs");
    std::fs::create_dir_all(&inputs).unwrap();
    for name in ["bracket.prt.3", "bracket.prt.7"] {
        std::fs::write(inputs.join(name), b"#UGC:2").unwrap();
    }
    for json in [false, true] {
        let mut args = vec![
            "convert-all",
            inputs.to_str().unwrap(),
            "--out-dir",
            outputs.to_str().unwrap(),
        ];
        if json {
            args.push("--json");
        }
        let out = dutchman(&args);
        // Creo is recognised but does not supply geometry yet; only the newest is attempted.
        assert_eq!(out.status.code(), Some(1));
        if json {
            let report = json_of(&out);
            assert_eq!(report["results"].as_array().unwrap().len(), 1);
            assert_eq!(
                report["results"][0]["input"]["path"],
                inputs.join("bracket.prt.7").to_str().unwrap()
            );
            assert_eq!(report["results"][0]["input"]["extension"], "prt.7");
            assert_eq!(report["skipped"], 1);
            assert_eq!(
                report["skipped_revisions"][0]["path"],
                inputs.join("bracket.prt.3").to_str().unwrap()
            );
        } else {
            let text = String::from_utf8(out.stdout).unwrap();
            assert!(text.contains("Creo .prt.7: opens, no geometry yet"));
            assert!(text.contains("bracket.prt.3: older revision; using "));
            assert!(text.contains("1 skipped (older revisions)"));
            assert!(!text.contains("Creo .prt.3:"));
        }
    }
}

#[test]
fn numbered_default_outputs_use_the_base_name_and_keep_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("bracket.prt.7");
    let output = dir.path().join("bracket.sc");
    std::fs::write(&input, b"#UGC:2").unwrap();
    std::fs::write(&output, b"keep this design").unwrap();
    let out = dutchman(&[input.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("bracket.sc already exists"));
    assert_eq!(std::fs::read(output).unwrap(), b"keep this design");
    assert!(!dir.path().join("bracket.prt.sc").exists());
}

/// Off a terminal the text mode prints no progress lines: the error is the only stderr line.
fn assert_text_error_with_progress(out: &Output, _input: &std::path::Path, expected: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines: Vec<_> = stderr.lines().collect();
    assert_eq!(lines, [expected]);
}

fn assert_progress_json(out: &Output) -> Vec<serde_json::Value> {
    let text = std::str::from_utf8(&out.stderr).unwrap();
    let events: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(!events.is_empty());
    for event in &events {
        assert_eq!(event["schema"], "dutchman.progress/1");
        assert!(event["elapsed_ms"].is_u64());
        assert!(event["designs_read"].is_u64());
        assert!(event["heartbeat"].is_boolean());
    }
    assert!(events
        .windows(2)
        .all(|pair| pair[0]["elapsed_ms"].as_u64() <= pair[1]["elapsed_ms"].as_u64()));
    events
}

#[test]
fn conversion_progress_stays_on_stderr_and_quiet_suppresses_it() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("progress.sc");
    for batch in [false, true] {
        for json in [false, true] {
            for quiet in [false, true] {
                let mut args = vec![if batch { "convert-all" } else { "convert" }, "--no-verify"];
                if json {
                    args.push("--json");
                }
                if quiet {
                    args.push("--quiet");
                }
                let input = fixture("generated_unit_cube.f3d");
                args.push(&input);
                if batch {
                    args.push("--out-dir");
                }
                args.push(if batch {
                    dir.path().to_str().unwrap()
                } else {
                    output.to_str().unwrap()
                });
                let out = dutchman(&args);
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "{}",
                    String::from_utf8_lossy(&out.stderr)
                );
                let stdout = String::from_utf8_lossy(&out.stdout);
                assert!(!stdout.contains("s elapsed"));
                assert!(!stdout.contains("dutchman.progress/1"));
                if quiet {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    assert!(!stderr.contains("s elapsed"));
                    assert!(!stderr.contains("dutchman.progress/1"));
                    if json {
                        assert!(out.stderr.is_empty());
                    }
                } else if json {
                    let events = assert_progress_json(&out);
                    assert!(events.iter().any(|event| event["stage"] == "decode"));
                    assert!(events.iter().any(|event| event["stage"] == "project"));
                    let last = events.last().unwrap();
                    assert_eq!(last["stage"], "write");
                    assert_eq!(last["designs_read"], 1);
                    assert_eq!(last["designs_total"], 1);
                } else {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    // Piped stderr is not a terminal: no progress lines.
                    assert!(!stderr.contains("designs read"));
                    assert!(!stderr.contains("s elapsed"));
                    assert!(!stderr.contains('\r'));
                    assert!(!stderr.contains('\x1b'));
                }
                if json {
                    let report = json_of(&out);
                    assert_eq!(report["ok"], true);
                    assert_eq!(
                        report["schema"],
                        if batch {
                            "dutchman.convert-all/1"
                        } else {
                            "dutchman.convert/1"
                        }
                    );
                    assert!(report.get("heartbeat").is_none());
                }
            }
        }
    }
}

#[test]
fn inspection_progress_leaves_the_final_json_report_on_stdout() {
    for quiet in [false, true] {
        let input = fixture("generated_unit_cube.f3d");
        let mut args = vec!["inspect", "--json", &input];
        if quiet {
            args.push("--quiet");
        }
        let out = dutchman(&args);
        assert_eq!(out.status.code(), Some(0));
        let report = json_of(&out);
        assert_eq!(report["schema"], "dutchman.inspect/1");
        assert_eq!(report["ok"], true);
        assert!(report.get("heartbeat").is_none());
        if quiet {
            assert!(out.stderr.is_empty());
        } else {
            let events = assert_progress_json(&out);
            assert_eq!(events.last().unwrap()["designs_read"], 1);
            assert_eq!(events.last().unwrap()["stage"], "map");
        }
    }
}

#[cfg(feature = "freecad")]
#[test]
fn techdraw_dxf_exports_page_text_and_reports_missing_ir_geometry() {
    let formats = json_of(&dutchman(&["formats", "--json"]));
    assert_eq!(formats["drawings"][0]["input"], "freecad_document");
    assert_eq!(formats["drawings"][0]["output"], "dxf");
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures/techdraw.FCStd");
    let folder = tempfile::tempdir().unwrap();
    let input = folder.path().join("box.FCStd");
    std::fs::copy(fixture, &input).unwrap();
    std::fs::create_dir(folder.path().join("box.dxf")).unwrap();
    let result = dutchman(&["convert", input.to_str().unwrap(), "--to", "dxf", "--json"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["pages"][0]["name"], "Page");
    assert_eq!(
        report["pages"][0]["dimension_representation"],
        "lines_and_text"
    );
    assert_eq!(report["pages"][0]["entities_per_layer"]["DIMENSIONS"], 8);
    assert_eq!(report["pages"][0]["entities_per_layer"]["FRAME"], 6);
    assert_eq!(report["pages"][0]["entities_per_layer"]["TITLE_BLOCK"], 1);
    assert_eq!(report["pages"][0]["entities_per_layer"]["ANNOTATIONS"], 2);
    assert_eq!(report["pages"][0]["entities_per_layer"]["VIEW_2_Front"], 0);
    let losses = report["pages"][0]["losses"].as_array().unwrap();
    for subject in ["Front", "Top"] {
        assert!(
            losses.iter().any(|l| l["subject"] == subject
                && l["level"] == "Dropped"
                && l["upstream"] == "cadmpeg"),
            "{losses:?}"
        );
    }
    let output = folder.path().join("box-Page.dxf");
    let text = std::fs::read_to_string(&output).unwrap();
    let lines: Vec<_> = text.lines().collect();
    let (records, remainder) = lines.as_chunks::<2>();
    assert!(remainder.is_empty());
    let pairs: Vec<_> = records
        .iter()
        .map(|[code, value]| (*code, *value))
        .collect();
    assert_eq!(pairs.iter().filter(|p| **p == ("0", "TEXT")).count(), 4);
    assert_eq!(
        pairs.iter().filter(|p| **p == ("8", "ANNOTATIONS")).count(),
        2
    );
    let annotation_entities: Vec<_> = text
        .split("0\nTEXT\n")
        .filter(|entity| entity.contains("8\nANNOTATIONS\n"))
        .collect();
    assert_eq!(annotation_entities.len(), 2);
    assert!(annotation_entities[0].contains("20\n100\n40\n5\n"));
    assert!(annotation_entities[1].contains("20\n93\n40\n5\n"));
    assert!(pairs.contains(&("10", "120")));
    assert!(text.contains("Synthetic box"));
    assert!(!text.contains("%.2w"));
    let retry = dutchman(&["convert", input.to_str().unwrap(), "--to", "dxf", "--json"]);
    assert!(!retry.status.success());
    assert_eq!(std::fs::read_to_string(&output).unwrap(), text);
    let forced = dutchman(&[
        "convert",
        input.to_str().unwrap(),
        "--to",
        "dxf",
        "--force",
        "--page",
        "Page",
        "--json",
    ]);
    assert!(forced.status.success());
    assert_eq!(std::fs::read_to_string(&output).unwrap(), text);
    let mut model =
        dutchman_core::read_drawings(&input, &mut dutchman_core::Report::default()).unwrap();
    let note = model
        .ir
        .model
        .drawings
        .iter()
        .find(|drawing| drawing.object.ends_with("#Note"))
        .unwrap()
        .object
        .clone();
    model.ir.model.view_presentations.push(
        serde_json::from_value(serde_json::json!({
            "id": "test:dxf:presentation#note", "object": note, "order": 0, "visible": false
        }))
        .unwrap(),
    );
    let pages = dutchman_dxf::pages(&model, None, &mut dutchman_core::Report::default()).unwrap();
    assert_eq!(pages[0].entities_per_layer["ANNOTATIONS"], 0);
    assert!(pages[0]
        .report
        .losses()
        .iter()
        .any(|loss| loss.subject == "Note" && loss.detail.contains("hidden")));
    model.ir.model.view_presentations[0].visible = Some(true);
    model
        .ir
        .model
        .semantic_annotations
        .iter_mut()
        .find(|annotation| annotation.object.ends_with("#Note"))
        .unwrap()
        .text = vec!["first".into(), "".into(), "third".into()];
    let pages = dutchman_dxf::pages(&model, None, &mut dutchman_core::Report::default()).unwrap();
    assert_eq!(pages[0].entities_per_layer["ANNOTATIONS"], 2);
    let text = String::from_utf8(pages[0].bytes.clone()).unwrap();
    assert!(text.contains("20\n86\n"));
    assert!(!text.contains("20\n93\n"));
}

#[cfg(feature = "freecad")]
#[test]
fn drawing_only_pages_support_selection_and_output_rules() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures/techdraw-pages.FCStd");
    let folder = tempfile::tempdir().unwrap();
    let result = dutchman(&[
        "convert",
        fixture.to_str().unwrap(),
        "--to",
        "dxf",
        "-o",
        folder.path().to_str().unwrap(),
        "--json",
    ]);
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["pages"].as_array().unwrap().len(), 2);
    assert!(report["pages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["dimension_representation"] == "none"));
    for name in ["First", "Second"] {
        assert!(folder
            .path()
            .join(format!("techdraw-pages-{name}.dxf"))
            .exists());
    }
    let selected = folder.path().join("selected.dxf");
    let result = dutchman(&[
        "convert",
        fixture.to_str().unwrap(),
        "--to",
        "dxf",
        "--page",
        "Second",
        "-o",
        selected.to_str().unwrap(),
        "--json",
    ]);
    assert!(result.status.success());
    let text = std::fs::read_to_string(&selected).unwrap();
    assert!(text.contains("Second"));
    assert!(!text.contains("First"));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["pages"].as_array().unwrap().len(), 1);
    let missing = dutchman(&[
        "convert",
        fixture.to_str().unwrap(),
        "--to",
        "dxf",
        "--page",
        "Absent",
        "-o",
        selected.to_str().unwrap(),
        "--json",
    ]);
    assert!(!missing.status.success());
    assert_eq!(std::fs::read_to_string(&selected).unwrap(), text);
    let part = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures/design_history.FCStd");
    let no_page = dutchman(&[
        "convert",
        part.to_str().unwrap(),
        "--to",
        "dxf",
        "-o",
        selected.to_str().unwrap(),
        "--json",
    ]);
    assert!(!no_page.status.success());
    let report: serde_json::Value = serde_json::from_slice(&no_page.stdout).unwrap();
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("no TechDraw page"));
}

#[cfg(feature = "freecad")]
#[test]
fn neutral_techdraw_fixtures_keep_templates_dimensions_and_units() {
    for (fixture, value, dimension_entities, frame_entities) in [
        ("techdraw", Some("30"), 8, 6),
        ("techdraw-pages", None, 0, 6),
        ("cadmpeg-techdraw-views", Some("30"), 24, 8),
        ("cadmpeg-techdraw-dimensions", Some("R7"), 35, 8),
        ("cadmpeg-techdraw-variants", Some("R11"), 43, 8),
    ] {
        let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../dutchman-core/tests/fixtures/{fixture}.FCStd"));
        let model =
            dutchman_core::read_drawings(&input, &mut dutchman_core::Report::default()).unwrap();
        let pages =
            dutchman_dxf::pages(&model, None, &mut dutchman_core::Report::default()).unwrap();
        assert_eq!(
            pages.len(),
            if value.is_none() { 2 } else { 1 },
            "{fixture}"
        );
        for page in &pages {
            let text = std::str::from_utf8(&page.bytes).unwrap();
            assert!(text.contains("$INSUNITS\n70\n4\n"), "{fixture}");
            assert!(text.contains("$LIMMAX\n10\n210\n20\n297\n"), "{fixture}");
            assert_eq!(
                page.entities_per_layer["FRAME"], frame_entities,
                "{fixture}"
            );
            assert_eq!(
                page.entities_per_layer["DIMENSIONS"], dimension_entities,
                "{fixture}"
            );
            assert_eq!(page.entities_per_layer["TITLE_BLOCK"], 1, "{fixture}");
            assert!(page.entities_per_layer["ANNOTATIONS"] > 0, "{fixture}");
            assert!(text.contains("100\nAcDbText\n"), "{fixture}");
            assert!(text.contains("40\n4\n"), "template text height: {fixture}");
            if let Some(value) = value {
                assert!(text.contains(&format!("1\n{value}\n")), "{fixture}: {text}");
                assert!(page.entities_per_layer["DIMENSIONS"] > 1, "{fixture}");
                assert!(
                    text.contains("100\nAcDbEntity\n8\nDIMENSIONS\n100\nAcDbLine\n"),
                    "dimension lines: {fixture}"
                );
                assert!(
                    page.report
                        .losses()
                        .iter()
                        .any(|loss| loss.subject == "Front"
                            && loss.detail.contains("view curves are unavailable")),
                    "{fixture}"
                );
            }
            if fixture.contains("variants") {
                for label in ["42", "\\U+230022", "40"] {
                    assert!(text.contains(&format!("1\n{label}\n")));
                }
            }
            if fixture.contains("views")
                || fixture.contains("dimensions")
                || fixture.contains("variants")
            {
                assert!(text.contains("1\nEdited title\n"));
                assert!(text.contains("0\nCIRCLE\n"));
                assert!(text.contains("0\nARC\n"));
            }
            if fixture.contains("dimensions") || fixture.contains("variants") {
                assert_eq!(page.dimension_representation, "partial_lines_and_text");
                assert!(text.contains("1\n90\\U+00B0\n"));
                assert!(page
                    .report
                    .losses()
                    .iter()
                    .any(|loss| loss.detail.contains("text only")));
            }
        }
    }
}

#[cfg(feature = "freecad")]
#[test]
fn neutral_view_curves_keep_placement_visibility_and_source_layers() {
    let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures/techdraw.FCStd");
    let mut model =
        dutchman_core::read_drawings(&input, &mut dutchman_core::Report::default()).unwrap();
    let view = model
        .ir
        .model
        .drawings
        .iter_mut()
        .find(|d| d.object.ends_with("#Front"))
        .unwrap();
    for (source, visibility, displayed) in [
        ("cosmetic", "hidden", true),
        ("center_line", "visible", true),
        ("projected_model_edge", "visible", false),
    ] {
        view.curves.push(
            serde_json::from_value(serde_json::json!({
                "geometry": {"kind": "line", "start": [1,2], "end": [3,4]},
                "source": source, "visibility": visibility, "displayed": displayed
            }))
            .unwrap(),
        );
    }
    let pages = dutchman_dxf::pages(&model, None, &mut dutchman_core::Report::default()).unwrap();
    let page = &pages[0];
    assert_eq!(page.entities_per_layer["VIEW_2_Front_COSMETIC_HIDDEN"], 1);
    assert_eq!(page.entities_per_layer["VIEW_2_Front_CENTER"], 1);
    assert_eq!(page.entities_per_layer["VIEW_2_Front"], 0);
    let text = std::str::from_utf8(&page.bytes).unwrap();
    assert!(text.contains(
        "8\nVIEW_2_Front_COSMETIC_HIDDEN\n100\nAcDbLine\n10\n67\n20\n184\n11\n71\n21\n188\n"
    ));
    assert!(text.contains("2\nVIEW_2_Front_COSMETIC_HIDDEN\n70\n0\n62\n7\n6\nHIDDEN\n"));
    assert!(text.contains("2\nVIEW_2_Front_CENTER\n70\n0\n62\n7\n6\nCENTER\n"));
}

#[test]
fn an_estimate_above_free_memory_does_not_refuse() {
    let Some(available) = dutchman::choices::available_memory() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("synthetic.step");
    let output = dir.path().join("never-written.sc");
    // Sparse metadata is enough for the estimator; no design bytes are read or allocated.
    std::fs::File::create(&input)
        .unwrap()
        .set_len(available.saturating_add(2_000_000_000))
        .unwrap();
    let out = dutchman(&[
        "convert",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--preset",
        "full",
        "--json",
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "the decoder's file-size guard takes over: {out:?}"
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(!report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("needs about"));
}

#[cfg(all(debug_assertions, unix))]
#[test]
fn a_check_stopped_for_memory_keeps_the_design_unverified() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("solidcraft-cli");
    std::fs::write(
        &cli,
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'solidcraft-cli 0.41.1'; exit 0; fi\nsleep 30\n",
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let input = fixture("generated_unit_cube.f3d");
    let output = dir.path().join("kept.sc");
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args(["convert", &input, output.to_str().unwrap(), "--json"])
        .args(["--solidcraft", cli.to_str().unwrap()])
        .env("DUTCHMAN_TEST_CHECK_LOW_MEMORY", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["ok"], true);
    assert_ne!(report["verification"]["status"], "verified", "{report}");
    assert!(
        report.to_string().contains("History check stopped"),
        "{report}"
    );
    assert!(output.exists());
    // Memory checks turned off leave the check alone: this one then runs out its own time.
    let forced = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args(["convert", &input, output.to_str().unwrap(), "--json"])
        .args(["--solidcraft", cli.to_str().unwrap()])
        .args(["--no-memory-check", "--verify-timeout", "1", "--force"])
        .env("DUTCHMAN_TEST_CHECK_LOW_MEMORY", "1")
        .output()
        .unwrap();
    assert!(forced.status.success(), "{forced:?}");
    assert!(
        !String::from_utf8_lossy(&forced.stdout).contains("History check stopped"),
        "{forced:?}"
    );
}

#[cfg(debug_assertions)]
#[test]
fn running_low_on_memory_stops_a_conversion_with_exit_two_or_a_failed_result() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("synthetic.f3d");
    std::fs::copy(fixture("generated_unit_cube.f3d"), &input).unwrap();
    let low = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_dutchman"))
            .args(args)
            .env("DUTCHMAN_TEST_LOW_MEMORY", "1")
            .output()
            .unwrap()
    };
    let output = dir.path().join("out.sc");
    let single = low(&[
        "convert",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(single.status.code(), Some(2), "{single:?}");
    let report: serde_json::Value = serde_json::from_slice(&single.stdout).unwrap();
    assert!(report["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("stopped before this computer ran out of memory"));
    let batch = low(&[
        "convert-all",
        input.to_str().unwrap(),
        "--out-dir",
        dir.path().join("batch").to_str().unwrap(),
        "--json",
    ]);
    let report: serde_json::Value = serde_json::from_slice(&batch.stdout).unwrap();
    assert_eq!(report["failed"], 1, "{report}");
    assert!(report["results"][0]["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("stopped before this computer ran out of memory"));
    let off = low(&[
        "convert",
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        "--no-memory-check",
        "--no-verify",
        "--json",
    ]);
    assert_eq!(off.status.code(), Some(0), "{off:?}");
}

#[test]
fn part_positions_reports_keep_estimates_text_only_and_respect_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let input = PathBuf::from(fixture("synthetic_cube.ipt"));
    let output = dir.path().join("cube.solidcraft");
    for (flags, mode) in [
        (vec![], "smart"),
        (vec!["--quiet"], "smart"),
        (vec!["--json"], "smart"),
        (vec!["--json", "--stored-placements"], "first-placed"),
        (
            vec!["--json", "--part-positions", "last-shown"],
            "last-shown",
        ),
    ] {
        let mut args = vec![
            "convert",
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "--no-verify",
            "--preset",
            "fast",
        ];
        args.extend(flags.iter().copied());
        let out = dutchman(&args);
        assert!(out.status.success(), "{out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            text.contains("Estimate: under 0.1 GB, under a minute"),
            flags.is_empty(),
            "{text}"
        );
        if flags.contains(&"--json") {
            let report: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert!(report["estimate"]["peak_bytes"].is_u64());
            assert!(report["estimate"]["seconds"].is_u64());
            assert_eq!(report["part_positions"]["mode"], mode);
            assert_eq!(report["part_positions"]["total"], 0);
            assert_eq!(
                report["choices"],
                serde_json::json!({"preset": "fast", "history": false, "sketches": false, "joints": false, "part_positions":mode, "stored_placements":mode=="first-placed"})
            );
        }
    }
}

#[test]
fn part_positions_and_narrowing_choices_invalidate_journal_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture("synthetic_cube.ipt");
    let out_dir = dir.path().join("out");
    let journal = dir.path().join("journal.jsonl");
    let run = |sketches: bool, positions: &str| {
        let mut args = vec![
            "convert-all",
            &input,
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--journal",
            journal.to_str().unwrap(),
            "--preset",
            "full",
            "--part-positions",
            positions,
            "--no-history",
            "--no-joints",
            "--no-memory-check",
            "--no-verify",
            "--json",
        ];
        if !sketches {
            args.push("--no-sketches");
        }
        json_of(&dutchman(&args))
    };
    let first = run(true, "smart");
    assert_eq!(first["converted"], 1, "{first}");
    assert_eq!(first["resumed"], 0);
    assert_eq!(
        first["results"][0]["choices"],
        serde_json::json!({"preset": "full", "history": false, "sketches": true, "joints": false, "part_positions":"smart", "stored_placements":false})
    );
    assert_eq!(run(true, "smart")["resumed"], 1);
    let changed = run(false, "smart");
    assert_eq!(
        changed["resumed"], 0,
        "changing a choice must reconvert: {changed}"
    );
    assert_eq!(changed["results"][0]["choices"]["sketches"], false);
    let moved = run(false, "first-placed");
    assert_eq!(
        moved["resumed"], 0,
        "changing part positions must reconvert"
    );
    assert_eq!(
        moved["results"][0]["choices"]["part_positions"],
        "first-placed"
    );
    assert_eq!(moved["results"][0]["choices"]["stored_placements"], true);
    assert_eq!(run(false, "first-placed")["resumed"], 1);
}

#[test]
fn threads_accepts_serial_and_auto_on_conversion_commands() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture("generated_unit_cube.f3d");
    for threads in ["0", "1", "4"] {
        let output = dir.path().join(format!("threads-{threads}.sc"));
        let out = dutchman(&[
            "convert",
            input.as_str(),
            output.to_str().unwrap(),
            "--threads",
            threads,
            "--no-verify",
            "--json",
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            report["threads"]["requested"],
            threads.parse::<u64>().unwrap()
        );
        assert_eq!(report["threads"]["used"], 1);
        for args in [
            vec![
                "convert-all",
                input.as_str(),
                "--out-dir",
                dir.path().to_str().unwrap(),
                "--threads",
                threads,
                "--help",
            ],
            vec![
                "watch",
                dir.path().to_str().unwrap(),
                "--out-dir",
                dir.path().to_str().unwrap(),
                "--threads",
                threads,
                "--help",
            ],
        ] {
            assert!(dutchman(&args).status.success());
        }
    }
}

#[cfg(feature = "freecad")]
#[test]
fn part_threads_keep_assembly_json_and_design_bytes_identical() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture("freecad-1.1.4-A01/A01a.FCStd");
    let output = dir.path().join("assembly.solidcraft");
    let mut previous = None;
    for threads in ["1", "4"] {
        let out = dutchman(&[
            "convert",
            &input,
            output.to_str().unwrap(),
            "--threads",
            threads,
            "--no-verify",
            "--solidcraft-version",
            "0.26.0",
            "--preset",
            "full",
            "--no-memory-check",
            "--json",
            "--quiet",
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        report.as_object_mut().unwrap().remove("threads");
        let design = std::fs::read(&output).unwrap();
        if let Some((before_report, before_design)) = previous {
            assert_eq!(before_report, report);
            assert_eq!(before_design, design);
        }
        previous = Some((report, design));
    }
}

/// Conversion lines without the target line and optional installed-version notice.
fn without_target_line(stdout: &str) -> Vec<&str> {
    let (target, rest): (Vec<&str>, Vec<&str>) = stdout
        .lines()
        .partition(|line| line.starts_with("Target SolidCraft "));
    assert_eq!(target.len(), 1, "{stdout}");
    rest.into_iter()
        .filter(|line| {
            !(line.starts_with("SolidCraft ")
                && line.contains("is newer than this Dutchman knows."))
        })
        .collect()
}

#[test]
fn solidcraft_output_extensions_follow_the_target_and_accept_explicit_names() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixture("generated_unit_cube.f3d"),
        dir.path().join("part.f3d"),
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_dutchman"))
            .current_dir(dir.path())
            .args(args)
            .env("PATH", "")
            .env_remove("SOLIDCRAFT_CLI")
            .test_output()
            .unwrap()
    };
    for (target, extension) in [("0.25.0", "solidcraft"), ("0.25.1", "sc"), ("0.27.0", "sc")] {
        let out = run(&[
            "convert",
            "part.f3d",
            "--solidcraft-version",
            target,
            "--no-verify",
            "--json",
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let output = format!("part.{extension}");
        assert_eq!(json_of(&out)["output"]["path"], output);
        assert!(std::fs::read(dir.path().join(&output))
            .unwrap()
            .starts_with(b"PK"));
        std::fs::remove_file(dir.path().join(output)).unwrap();
    }
    let newest = run(&["part.f3d", "--no-verify", "--json"]);
    assert!(newest.status.success());
    assert_eq!(json_of(&newest)["output"]["path"], "part.sc");
    for extension in ["sc", "solidcraft", "solvecraft", "SC"] {
        for target in ["0.25.0", "0.25.1"] {
            let output = format!("explicit.{extension}");
            let out = run(&[
                "convert",
                "part.f3d",
                "-o",
                &output,
                "--solidcraft-version",
                target,
                "--no-verify",
                "--json",
            ]);
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let report = json_of(&out);
            assert_eq!(report["output"]["path"], output);
            assert_eq!(report["output"]["format"], "solidcraft/23");
            assert!(std::fs::read(dir.path().join(output))
                .unwrap()
                .starts_with(b"PK"));
        }
    }
}

#[test]
fn repro_cli_writes_a_private_kit_and_reports_success_or_failure() {
    for (name, status) in [
        ("generated_unit_cube.f3d", "no_failure_to_reproduce"),
        ("container_metadata_only.f3d", "failure"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let output = dutchman(&[
            "repro",
            &fixture(name),
            "--out",
            dir.path().to_str().unwrap(),
            "--json",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["schema"], "dutchman.repro/1");
        assert_eq!(result["anonymous"]["status"], status);
        assert!(result.get("local_key").is_none());
        assert!(
            result["summary"]
                .as_str()
                .unwrap()
                .contains("Do not share")
                .then_some(())
                .is_some()
                || result["summary"].as_str().unwrap().contains("Never share")
        );
        let kit = dir.path().join("kit");
        assert!(kit.join("anonymous.json").is_file());
        assert!(kit.join("local-key.json").is_file());
        assert!(kit.join("next-steps.md").is_file());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        let again = dutchman(&[
            "repro",
            &fixture(name),
            "--out",
            dir.path().to_str().unwrap(),
        ]);
        assert_eq!(again.status.code(), Some(1));
    }
}

// ---- dutchman diagnose, convert --part and the memory stop ----

fn dutchman_with(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dutchman"));
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    command.test_output().unwrap()
}

#[cfg(feature = "freecad")]
fn diagnose_report(out: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(out.join("diagnose/report.json")).unwrap()).unwrap()
}

/// A multi-part public fixture diagnosed once: its report and the private names of its parts.
#[cfg(feature = "freecad")]
fn diagnosed_assembly(dir: &std::path::Path) -> (serde_json::Value, Vec<(u64, String)>) {
    let out = dutchman(&[
        "diagnose",
        &fixture("product_assembly.FCStd"),
        "--out",
        dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let key: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("diagnose/local-key.json")).unwrap())
            .unwrap();
    let names = key["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["part"].as_u64().unwrap(),
                p["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    (diagnose_report(dir), names)
}

/// The number and name of a part of the fixture that has geometry of its own.
#[cfg(feature = "freecad")]
fn leaf_part(report: &serde_json::Value, names: &[(u64, String)]) -> (u64, String) {
    let part = report["parts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["kind"] == "part" && p["entities"].as_u64().unwrap() > 0)
        .unwrap();
    let number = part["part"].as_u64().unwrap();
    let name = names.iter().find(|(n, _)| *n == number).unwrap().1.clone();
    (number, name)
}

#[test]
#[cfg(feature = "freecad")]
fn diagnose_ranks_the_failing_part_first_and_part_reproduces_it_alone() {
    let clean = tempfile::tempdir().unwrap();
    let (report, names) = diagnosed_assembly(clean.path());
    assert_eq!(report["schema"], "dutchman.diagnose/1");
    assert_eq!(report["status"], "complete");
    assert!(report["parts"].as_array().unwrap().len() >= 3);
    assert_eq!(report["summary"]["failed"], 0);
    let (target, name) = leaf_part(&report, &names);

    // The writer is made to fail on that one part, here and in `convert --part`.
    let env = [("DUTCHMAN_TEST_FAIL_MODEL", name.as_str())];
    let dir = tempfile::tempdir().unwrap();
    let out = dutchman_with(
        &[
            "diagnose",
            &fixture("product_assembly.FCStd"),
            "--out",
            dir.path().to_str().unwrap(),
            "--json",
        ],
        &env,
    );
    let result = json_of(&out);
    assert_eq!(result["schema"], "dutchman.diagnose/1");
    let report = diagnose_report(dir.path());
    let parts = report["parts"].as_array().unwrap();
    // Parts that share a name fail together.
    let failing: Vec<u64> = names
        .iter()
        .filter(|(n, part_name)| {
            *part_name == name
                && report["parts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p["part"] == *n && p["entities"].as_u64().unwrap() > 0)
        })
        .map(|(n, _)| *n)
        .collect();
    assert!(failing.contains(&target));
    assert!(
        failing.contains(&parts[0]["part"].as_u64().unwrap()),
        "{report}"
    );
    assert_eq!(parts[0]["rank"], 1);
    assert_eq!(parts[0]["outcome"], "failed");
    assert_eq!(parts[0]["error_kind"], "restate");
    assert_eq!(parts[0]["stage"], "write");
    assert_eq!(report["summary"]["failed"], failing.len());
    for other in &parts[failing.len()..] {
        assert!(
            matches!(other["outcome"].as_str(), Some("ok" | "container")),
            "{other}"
        );
    }
    // Parts that fail alike rank by how they ran, so the first of them can be either.
    let first = parts[0]["part"].as_u64().unwrap();
    let text = std::fs::read_to_string(dir.path().join("diagnose/report.txt")).unwrap();
    assert!(
        text.contains(&format!("--part {first}")),
        "the text names the part: {text}"
    );
    assert!(result["summary"]
        .as_str()
        .unwrap()
        .contains(&format!("--part {first}")));

    // That part alone fails; the others convert.
    let scratch = tempfile::tempdir().unwrap();
    for (number, _) in &names {
        let output = scratch.path().join(format!("part{number}.sc"));
        let out = dutchman_with(
            &[
                "convert",
                &fixture("product_assembly.FCStd"),
                "--part",
                &number.to_string(),
                "-o",
                output.to_str().unwrap(),
                "--no-verify",
                "--json",
            ],
            &env,
        );
        let result = json_of(&out);
        let is_container = report["parts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["part"] == *number && p["outcome"] == "container");
        if failing.contains(number) {
            assert_eq!(result["ok"], false, "{result}");
            assert_eq!(result["error"]["kind"], "restate");
        } else if !is_container {
            assert_eq!(result["ok"], true, "part {number}: {result}");
            assert!(output.is_file());
        }
    }
}

#[test]
#[cfg(feature = "freecad")]
fn diagnose_files_never_hold_names_paths_or_the_file_name() {
    let dir = tempfile::tempdir().unwrap();
    let (report, names) = diagnosed_assembly(dir.path());
    let (_, name) = leaf_part(&report, &names);
    // Fail one part with a message that names it, so a leak through errors would show.
    let secret_dir = tempfile::tempdir().unwrap();
    let out = dutchman_with(
        &[
            "diagnose",
            &fixture("product_assembly.FCStd"),
            "--out",
            secret_dir.path().to_str().unwrap(),
        ],
        &[("DUTCHMAN_TEST_FAIL_MODEL", name.as_str())],
    );
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("report.json"), "{stdout}");
    assert!(stdout.contains("local-key.json"), "{stdout}");
    for shared in ["report.json", "report.txt"] {
        let text =
            std::fs::read_to_string(secret_dir.path().join("diagnose").join(shared)).unwrap();
        for (_, part_name) in &names {
            // Names the words of the report itself could match are not tested.
            if part_name.chars().count() > 5 {
                assert!(
                    !text.contains(part_name.as_str()),
                    "{part_name} in {shared}"
                );
            }
        }
        assert!(!text.contains("product_assembly"), "{shared}");
        assert!(!text.contains("FCStd"), "{shared}");
        assert!(
            !text.contains(secret_dir.path().to_str().unwrap()),
            "{shared}"
        );
        assert!(!text.contains("injected failure"), "{shared}");
    }
    let key = std::fs::read_to_string(secret_dir.path().join("diagnose/local-key.json")).unwrap();
    assert!(key.contains(&name) && key.contains("Do not share"));
    // A second run into the same folder is refused and changes nothing.
    let again = dutchman(&[
        "diagnose",
        &fixture("product_assembly.FCStd"),
        "--out",
        secret_dir.path().to_str().unwrap(),
    ]);
    assert_eq!(again.status.code(), Some(1));
}

// ---- dutchman diagnose --narrow, --same-failure and --make-report ----

/// The fault tests use, on a fixture every build reads: part 1 of the design fails once its
/// history reaches `feature`.
fn narrow_fault(feature: usize) -> String {
    format!("1:feature:{feature}:fail")
}

fn narrow(file: &str, out: &std::path::Path, env: &[(&str, &str)], extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dutchman"));
    command
        .args(["diagnose", "--narrow", file, "--out"])
        .arg(out)
        .args(extra);
    for (key, value) in env {
        command.env(key, value);
    }
    command.test_output().unwrap()
}

fn narrow_result(out: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(out.join("diagnose/narrow.json")).unwrap()).unwrap()
}

/// How many features the history of `body_history.sldprt` has.
fn history_of_fixture() -> u64 {
    let dir = tempfile::tempdir().unwrap();
    // A fault at the first feature makes the run say how many there are.
    let file = fixture("body_history.sldprt");
    let fault = narrow_fault(1);
    let out = narrow(
        &file,
        dir.path(),
        &[("DUTCHMAN_DIAGNOSE_TEST_FAULT", &fault)],
        &[],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    narrow_result(dir.path())["features"].as_u64().unwrap()
}

#[test]
fn narrow_finds_the_first_failing_feature_and_says_each_step() {
    let total = history_of_fixture();
    assert!(total >= 2, "the fixture needs a history: {total}");
    let dir = tempfile::tempdir().unwrap();
    let file = fixture("body_history.sldprt");
    let fault = narrow_fault(2);
    let out = narrow(
        &file,
        dir.path(),
        &[("DUTCHMAN_DIAGNOSE_TEST_FAULT", &fault)],
        &[],
    );
    let said = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{said}");
    for step in 1..=5 {
        assert!(said.contains(&format!("step {step} of 5:")), "{said}");
    }
    let found = narrow_result(dir.path());
    assert_eq!(found["schema"], "dutchman.narrow/1");
    assert_eq!(found["status"], "narrowed");
    assert_eq!(found["part"], 1);
    assert_eq!(found["first_failing_feature"], 2);
    let shared = [
        std::fs::read_to_string(dir.path().join("diagnose/narrow.json")).unwrap(),
        std::fs::read_to_string(dir.path().join("diagnose/signature.json")).unwrap(),
        std::fs::read_to_string(dir.path().join("diagnose/narrow-state.json")).unwrap(),
        std::fs::read_to_string(dir.path().join("diagnose/kit/anonymous.json")).unwrap(),
        std::fs::read_to_string(dir.path().join("diagnose/kit/next-steps.md")).unwrap(),
    ];
    for text in &shared {
        assert!(!text.contains("body_history"), "{text}");
        assert!(!text.contains(dir.path().to_str().unwrap()), "{text}");
    }
    assert!(shared[4].contains("--same-failure"));
}

#[test]
fn narrow_resumes_after_it_is_killed_without_repeating_finished_trials() {
    let file = fixture("body_history.sldprt");
    let fault = narrow_fault(2);
    let dir = tempfile::tempdir().unwrap();
    // Dies, as a crash or a reboot would, once two trials are on disk.
    let stopped = narrow(
        &file,
        dir.path(),
        &[
            ("DUTCHMAN_DIAGNOSE_TEST_FAULT", &fault),
            ("DUTCHMAN_NARROW_TEST_ABORT_AFTER", "2"),
        ],
        &[],
    );
    assert!(!stopped.status.success());
    assert!(!dir.path().join("diagnose/narrow.json").exists());
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("diagnose/narrow-state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["trials"].as_array().unwrap().len(), 2);
    let out = narrow(
        &file,
        dir.path(),
        &[("DUTCHMAN_DIAGNOSE_TEST_FAULT", &fault)],
        &[],
    );
    let said = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{said}");
    assert!(
        said.contains("carrying on from narrow-state.json"),
        "{said}"
    );
    assert!(!said.contains("step 1 of 5"), "{said}");

    let whole = tempfile::tempdir().unwrap();
    assert!(narrow(
        &file,
        whole.path(),
        &[("DUTCHMAN_DIAGNOSE_TEST_FAULT", &fault)],
        &[]
    )
    .status
    .success());
    assert_eq!(narrow_result(dir.path()), narrow_result(whole.path()));

    // Another design in the same folder is refused.
    let other = narrow(&fixture("generated_unit_cube.f3d"), dir.path(), &[], &[]);
    assert!(!other.status.success());
    assert!(String::from_utf8_lossy(&other.stderr).contains("fresh --out"));
}

#[test]
fn narrow_survives_a_feature_that_crashes_or_hangs_the_converter() {
    let file = fixture("body_history.sldprt");
    // A crash can take a while to end on a loaded machine, as the system saves a core dump.
    for (fault, outcome, limit) in [("crash", "failed", "60"), ("hang", "timed_out", "2")] {
        let dir = tempfile::tempdir().unwrap();
        let fault = format!("1:feature:2:{fault}");
        let out = narrow(
            &file,
            dir.path(),
            &[("DUTCHMAN_DIAGNOSE_TEST_FAULT", &fault)],
            &["--time-limit", limit],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let found = narrow_result(dir.path());
        assert_eq!(found["first_failing_feature"], 2, "{found}");
        assert_eq!(found["signature"]["error"]["outcome"], outcome, "{found}");
    }
}

#[test]
fn convert_cuts_a_history_at_the_feature_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("cut.sc");
    let cut = dutchman(&[
        "convert",
        &fixture("generated_unit_cube.f3d"),
        "--part",
        "1",
        "--stop-at-feature",
        "0",
        "--no-verify",
        "-o",
        output.to_str().unwrap(),
    ]);
    let said =
        String::from_utf8_lossy(&cut.stdout).into_owned() + &String::from_utf8_lossy(&cut.stderr);
    assert!(cut.status.success(), "{said}");
    let past = dutchman(&[
        "convert",
        &fixture("generated_unit_cube.f3d"),
        "--part",
        "1",
        "--stop-at-feature",
        "99",
        "--no-verify",
        "-o",
        output.to_str().unwrap(),
    ]);
    assert!(!past.status.success());
    assert!(String::from_utf8_lossy(&past.stderr).contains("there is no feature 99"));
    // It needs a part to cut.
    let alone = dutchman(&[
        "convert",
        &fixture("generated_unit_cube.f3d"),
        "--stop-at-feature",
        "1",
    ]);
    assert_eq!(alone.status.code(), Some(2));
}

#[test]
fn a_design_that_converts_has_nothing_to_narrow() {
    let dir = tempfile::tempdir().unwrap();
    let out = narrow(&fixture("generated_unit_cube.f3d"), dir.path(), &[], &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(narrow_result(dir.path())["status"], "nothing_to_narrow");
    assert!(!dir.path().join("diagnose/signature.json").exists());
}

#[cfg(feature = "freecad")]
/// Copies the archive `from` to `to` with a comment on it: the same design in different bytes.
fn repack(from: &std::path::Path, to: &std::path::Path) {
    use std::io::{Read, Write};
    let mut archive = zip::ZipArchive::new(std::fs::File::open(from).unwrap()).unwrap();
    let mut copy = zip::ZipWriter::new(std::fs::File::create(to).unwrap());
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    copy.set_comment("rebuilt").unwrap();
    for at in 0..archive.len() {
        let mut member = archive.by_index(at).unwrap();
        let name = member.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        copy.start_file(name, options).unwrap();
        copy.write_all(&bytes).unwrap();
    }
    copy.finish().unwrap();
}

#[test]
#[cfg(feature = "freecad")]
fn a_small_file_is_checked_and_the_report_gathered_only_when_it_fails_the_same_way() {
    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("results");
    let design = fixture("design_history.FCStd");
    let fault = narrow_fault(2);
    let env = [("DUTCHMAN_DIAGNOSE_TEST_FAULT", fault.as_str())];
    assert!(narrow(&design, &out_dir, &env, &[]).status.success());
    let diagnose = |flag: &str, file: &std::path::Path, env: &[(&str, &str)]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dutchman"));
        command
            .args(["diagnose", flag])
            .arg(file)
            .arg("--out")
            .arg(&out_dir)
            .arg("--json");
        for (key, value) in env {
            command.env(key, value);
        }
        command.test_output().unwrap()
    };

    // The design itself is not a rebuild of it.
    let refused = diagnose("--same-failure", std::path::Path::new(&design), &env);
    assert!(!refused.status.success());

    // A rebuild: the same design, stored differently, so it is a file of its own.
    let small = dir.path().join("small.FCStd");
    repack(std::path::Path::new(&design), &small);

    // Without the fault it converts fine, and there is no report to make.
    let fine = diagnose("--same-failure", &small, &[]);
    assert!(!fine.status.success());
    let verdict = json_of(&fine);
    assert_eq!(verdict["schema"], "dutchman.same-failure/1");
    assert_eq!(verdict["verdict"], "converts_fine", "{verdict}");
    assert!(verdict["summary"].as_str().unwrap().contains("Try again"));
    assert!(!diagnose("--make-report", &small, &[]).status.success());

    // With the fault it fails the same way, and the report can be made.
    let same = diagnose("--same-failure", &small, &env);
    let verdict = json_of(&same);
    assert_eq!(verdict["verdict"], "same", "{verdict}");
    assert!(verdict["summary"]
        .as_str()
        .unwrap()
        .contains("this is the one to send"));
    assert!(same.status.success());
    let made = diagnose("--make-report", &small, &[]);
    assert!(made.status.success());
    let made = json_of(&made);
    assert_eq!(made["schema"], "dutchman.made-report/1");
    let zip = std::fs::File::open(made["paths"]["zip"].as_str().unwrap()).unwrap();
    let zip = zip::ZipArchive::new(zip).unwrap();
    let mut names: Vec<String> = zip
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["anonymous.json", "signature.json", "small.FCStd"]);
}

#[test]
#[cfg(feature = "freecad")]
fn diagnose_stops_a_part_over_its_time_limit_and_carries_on() {
    let clean = tempfile::tempdir().unwrap();
    let (report, names) = diagnosed_assembly(clean.path());
    let (target, _) = leaf_part(&report, &names);
    let dir = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    // Far above any real part's time, even on a slow shared runner, so only the hang (an
    // hour) reaches it: with 1 s, a busy Windows runner timed out a real part too.
    let limit = 15;
    let out = dutchman_with(
        &[
            "diagnose",
            &fixture("product_assembly.FCStd"),
            "--out",
            dir.path().to_str().unwrap(),
            "--time-limit",
            &limit.to_string(),
        ],
        &[("DUTCHMAN_DIAGNOSE_TEST_FAULT", &format!("{target}:hang"))],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The hang would have lasted an hour.
    assert!(started.elapsed() < std::time::Duration::from_secs(120));
    let report = diagnose_report(dir.path());
    let parts = report["parts"].as_array().unwrap();
    assert_eq!(parts[0]["part"], target);
    assert_eq!(parts[0]["outcome"], "timed_out");
    assert_eq!(parts[0]["stage"], "map");
    assert!(parts[0]["time_ms"].as_u64().unwrap() >= limit * 1000);
    assert_eq!(report["summary"]["timed_out"], 1, "{report}");
    assert_eq!(report["summary"]["restarts"], 1);
    assert_eq!(report["summary"]["not_run"], 0);
    assert_eq!(report["limits"]["time_limit_s"], limit as f64);
    // Every other part still got its turn after the converter was started again.
    assert!(parts[1..]
        .iter()
        .all(|p| matches!(p["outcome"].as_str(), Some("ok" | "container"))));
}

#[test]
#[cfg(feature = "freecad")]
fn diagnose_survives_a_part_that_crashes_the_converter() {
    let clean = tempfile::tempdir().unwrap();
    let (report, names) = diagnosed_assembly(clean.path());
    let (target, _) = leaf_part(&report, &names);
    let dir = tempfile::tempdir().unwrap();
    let out = dutchman_with(
        &[
            "diagnose",
            &fixture("product_assembly.FCStd"),
            "--out",
            dir.path().to_str().unwrap(),
        ],
        &[("DUTCHMAN_DIAGNOSE_TEST_FAULT", &format!("{target}:crash"))],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = diagnose_report(dir.path());
    assert_eq!(report["parts"][0]["part"], target);
    assert_eq!(report["parts"][0]["outcome"], "failed");
    assert_eq!(report["parts"][0]["error_kind"], "crash");
    assert_eq!(report["summary"]["restarts"], 1);
    assert_eq!(report["summary"]["not_run"], 0);
}

#[test]
#[cfg(feature = "freecad")]
fn convert_part_refuses_a_number_the_design_does_not_have() {
    let dir = tempfile::tempdir().unwrap();
    let out = dutchman(&[
        "convert",
        &fixture("product_assembly.FCStd"),
        "--part",
        "99",
        "-o",
        dir.path().join("x.sc").to_str().unwrap(),
        "--no-verify",
    ]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("there is no part 99"), "{stderr}");
    assert!(stderr.contains("numbered 1 to"), "{stderr}");
    assert!(!dir.path().join("x.sc").exists());
    let out = dutchman(&[
        "convert",
        &fixture("product_assembly.FCStd"),
        "--with-children",
    ]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
#[cfg(feature = "freecad")]
fn a_conversion_the_machine_cannot_finish_stops_early_naming_the_largest_parts() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("x.sc");
    let clean = tempfile::tempdir().unwrap();
    let (report, names) = diagnosed_assembly(clean.path());
    let (leaf, _) = leaf_part(&report, &names);
    let leaf = leaf.to_string();
    let args = [
        "convert",
        &fixture("product_assembly.FCStd"),
        "-o",
        output.to_str().unwrap(),
        "--no-verify",
        "--preset",
        "full",
    ];
    // The design holds 12 GB once read and 2 GB are free.
    let env = [("DUTCHMAN_TEST_MEMORY", "12000000000,1000000000")];
    let out = dutchman_with(&args, &env);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("this conversion needs about 1.8 GB more memory and 1.0 GB are free"),
        "{stderr}"
    );
    assert!(stderr.contains("are the largest"), "{stderr}");
    assert!(stderr.contains("dutchman diagnose"), "{stderr}");
    assert!(stderr.contains("--part N"), "{stderr}");
    assert!(!output.exists());
    // The override converts anyway; so does converting one part.
    let mut forced = args.to_vec();
    forced.push("--no-memory-check");
    assert!(dutchman_with(&forced, &env).status.success());
    let mut one = args.to_vec();
    one.extend(["--part", leaf.as_str()]);
    assert!(dutchman_with(&one, &env).status.success());
}

#[test]
#[cfg(feature = "freecad")]
fn repro_follows_a_diagnose_report_to_the_failing_part() {
    let clean = tempfile::tempdir().unwrap();
    let (report, names) = diagnosed_assembly(clean.path());
    let (_, name) = leaf_part(&report, &names);
    let env = [("DUTCHMAN_TEST_FAIL_MODEL", name.as_str())];
    let dir = tempfile::tempdir().unwrap();
    let out = dutchman_with(
        &[
            "diagnose",
            &fixture("product_assembly.FCStd"),
            "--out",
            dir.path().to_str().unwrap(),
        ],
        &env,
    );
    assert!(out.status.success());
    // Parts that share a name fail together; the first ranked is the one repro follows.
    let target = diagnose_report(dir.path())["parts"][0]["part"]
        .as_u64()
        .unwrap();
    // The report in --out is found; so is one named with --from-diagnose.
    let out = dutchman_with(
        &[
            "repro",
            &fixture("product_assembly.FCStd"),
            "--out",
            dir.path().to_str().unwrap(),
            "--json",
        ],
        &env,
    );
    let result = json_of(&out);
    assert_eq!(result["anonymous"]["status"], "failure", "{result}");
    assert_eq!(result["anonymous"]["narrowing"]["from_diagnose"], true);
    assert_eq!(result["anonymous"]["narrowing"]["path"][0]["unit"], "part");
    assert_eq!(
        result["anonymous"]["narrowing"]["path"][0]["ordinal"],
        target
    );
    let steps = std::fs::read_to_string(dir.path().join("kit/next-steps.md")).unwrap();
    assert!(steps.contains(&format!("part {target} of")), "{steps}");
    let shared = result["anonymous"].to_string() + &steps;
    assert!(
        !shared.contains(&name) || name.chars().count() <= 5,
        "{shared}"
    );
    let elsewhere = tempfile::tempdir().unwrap();
    let out = dutchman_with(
        &[
            "repro",
            &fixture("product_assembly.FCStd"),
            "--out",
            elsewhere.path().join("kit-here").to_str().unwrap(),
            "--from-diagnose",
            dir.path().to_str().unwrap(),
            "--json",
        ],
        &env,
    );
    assert_eq!(
        json_of(&out)["anonymous"]["narrowing"]["path"][0]["ordinal"],
        target
    );
    // A folder with no report is refused.
    let out = dutchman(&[
        "repro",
        &fixture("product_assembly.FCStd"),
        "--out",
        elsewhere.path().join("another").to_str().unwrap(),
        "--from-diagnose",
        elsewhere.path().to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn folders_and_zips_convert_with_automatic_sibling_output_folders() {
    use serde_json::Value;
    use std::io::Write as _;
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let bytes = std::fs::read(fixture("generated_unit_cube.f3d")).unwrap();
    std::fs::write(project.join("cube.f3d"), &bytes).unwrap();
    for (args, output) in [
        (
            vec![
                "convert",
                project.to_str().unwrap(),
                "--json",
                "--no-verify",
            ],
            "project (SolidCraft)",
        ),
        (
            vec![project.to_str().unwrap(), "--json", "--no-verify"],
            "project (SolidCraft 2)",
        ),
    ] {
        let result = dutchman(&args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["schema"], "dutchman.convert-all/1");
        assert_eq!(report["converted"], 1);
        assert!(dir.path().join(output).join("cube.sc").is_file());
    }
    let archive = dir.path().join("archive.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    zip.start_file("cube.f3d", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&bytes).unwrap();
    zip.finish().unwrap();
    let result = dutchman(&[archive.to_str().unwrap(), "--json", "--no-verify"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["converted"], 1);
    assert!(dir
        .path()
        .join("archive (SolidCraft)/archive/cube.sc")
        .is_file());
}

#[test]
fn history_check_skips_for_low_memory_but_off_and_force_remain_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("cube.sc");
    let input = memory_budget_cube(dir.path());
    let (memory, available) = memory_between_conversion_and_check(&input);
    let env = [("DUTCHMAN_TEST_MEMORY", memory.as_str())];
    let args = [
        "convert",
        &input,
        "-o",
        output.to_str().unwrap(),
        "--solidcraft-version",
        "0.41.0",
        "--json",
    ];
    let out = dutchman_with(&args, &env);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = json_of(&out);
    assert!(output.is_file());
    let needed = dutchman::stop::history_check_bytes(
        std::path::Path::new(&input),
        &dutchman::ConvertOptions::default(),
    )
    .unwrap();
    assert_eq!(report["history_check_skipped"]["needed_bytes"], needed);
    assert_eq!(
        report["history_check_skipped"]["available_bytes"],
        available
    );
    let reason = report["history_check_skipped"]["reason"].as_str().unwrap();
    assert_eq!(
        reason,
        dutchman::stop::history_check_message(needed, available)
    );
    assert!(dutchman::present::outcome(&report).contains(reason));
    for flags in [
        vec!["--no-verify"],
        vec!["--no-memory-check", "--no-verify"],
        vec!["--no-memory-check"],
    ] {
        let mut selected = args.to_vec();
        selected.extend(flags);
        let out = dutchman_with(&selected, &env);
        assert!(out.status.success());
        assert!(json_of(&out).get("history_check_skipped").is_none());
    }
    // The shared batch path also skips each input independently.
    let out = dutchman_with(
        &[
            "convert-all",
            &input,
            "--out-dir",
            dir.path().to_str().unwrap(),
            "--solidcraft-version",
            "0.41.0",
            "--json",
        ],
        &env,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(json_of(&out)["results"][0]["history_check_skipped"].is_object());
}

#[test]
#[cfg(feature = "freecad")]
fn low_memory_keeps_editable_history_unverified_and_the_original_shape_visible() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("history.sc");
    let input = fixture("design_history.FCStd");
    let (memory, _) = memory_between_conversion_and_check(&input);
    let out = dutchman_with(
        &[
            "convert",
            &input,
            "-o",
            output.to_str().unwrap(),
            "--solidcraft-version",
            "0.41.0",
            "--part",
            "1",
            "--stop-at-feature",
            "3",
            "--json",
        ],
        &[("DUTCHMAN_TEST_MEMORY", memory.as_str())],
    );
    assert!(
        out.status.success(),
        "{:?}\n{}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let report = json_of(&out);
    assert_eq!(report["verification"]["status"], "unverified");
    assert!(report["history_check_skipped"].is_object());
    assert!(report["history"]["mapped"].as_u64().unwrap() > 0);
    let mut zip = zip::ZipArchive::new(std::fs::File::open(output).unwrap()).unwrap();
    let doc: serde_json::Value =
        serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    assert!(doc.to_string().contains("Editable history (unverified)"));
    assert!(doc["features"]
        .as_array()
        .unwrap()
        .iter()
        .any(|feature| feature["type"] == "import" && feature["suppressed"] == false));
    assert!(doc
        .pointer("/visibility/hidden_occurrences")
        .as_ref()
        .is_some_and(|v| !v.as_array().unwrap().is_empty()));
}

#[test]
#[cfg(feature = "freecad")]
fn automatic_assembly_workers_use_the_synthetic_free_memory_budget() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("assembly.sc");
    let out = dutchman_with(
        &[
            "convert",
            &fixture("product_assembly.FCStd"),
            "-o",
            output.to_str().unwrap(),
            "--no-verify",
            "--solidcraft-version",
            "0.41.0",
            "--json",
        ],
        &[("DUTCHMAN_TEST_MEMORY", "100000000,500000000")],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = json_of(&out);
    assert_eq!(report["threads"]["requested"], 0);
    assert_eq!(report["threads"]["used"], 1);
    assert!(report.get("history_check_skipped").is_none());
}

#[test]
fn report_previews_a_converted_result_and_refuses_unconfirmed_sending() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("converted-report");
    let folder_text = folder.to_str().unwrap();
    let output = dutchman(&[
        "report",
        "--folder",
        folder_text,
        "--report-id",
        "f0123456789abcdef",
        "--source-format",
        "f3d",
        "--signature",
        "converted-with-problem",
        "--json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let preview: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(preview["signature"], "converted-with-problem");
    assert_eq!(preview["default_choice"], "summary");
    assert_eq!(preview["bundle_available"], false);
    assert_eq!(preview["metadata"]["consent"]["choice"], "summary");
    assert_eq!(preview["metadata"]["consent"]["public_test"], false);
    assert!(preview["metadata"].get("description").is_none());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(folder_text));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("claim_secret"));
    assert!(folder.join("prepared.json").exists());
    assert!(!folder.join("submission.json").exists());
    let reopened = dutchman(&["report", "--folder", folder_text]);
    assert!(reopened.status.success());
    let reopened: serde_json::Value = serde_json::from_slice(&reopened.stdout).unwrap();
    assert_eq!(reopened["metadata"], preview["metadata"]);
    let refused = dutchman(&["report", "--folder", folder_text, "--send-summary"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(!folder.join("submission.json").exists());
}

#[test]
fn verbose_asks_for_plain_progress_lines_off_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("verbose.sc");
    let out = dutchman(&[
        "convert",
        "--no-verify",
        "--verbose",
        &fixture("generated_unit_cube.f3d"),
        output.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("s elapsed"), "{stderr}");
}

#[test]
fn convert_all_warns_about_a_missing_argument_that_looks_like_an_output_folder() {
    let dir = tempfile::tempdir().unwrap();
    let wanted = dir.path().join("Converted");
    let out = dutchman(&[
        "convert-all",
        "--no-verify",
        &fixture("generated_unit_cube.f3d"),
        wanted.to_str().unwrap(),
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let warning = format!(
        "dutchman: {} does not exist; every positional argument is an input",
        wanted.display()
    );
    assert!(stderr.contains(&warning), "{stderr}");
    assert!(stderr.contains("--out-dir"), "{stderr}");
    // The missing input is still reported as before.
    assert_eq!(out.status.code(), Some(1));
    assert!(!wanted.exists());
}

#[test]
fn convert_all_help_says_every_positional_is_an_input() {
    let out = dutchman(&["convert-all", "--help"]);
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("Every"), "{help}");
    assert!(help.contains("--out-dir for the output folder"), "{help}");
}

#[test]
fn convert_writes_the_anonymous_report_of_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("one.sc");
    let anon = dir.path().join("anon.json");
    let key = dir.path().join("key.json");
    let input = fixture("generated_unit_cube.f3d");
    let out = dutchman(&[
        "convert",
        "--no-verify",
        "--report-anon",
        anon.to_str().unwrap(),
        "--report-key",
        key.to_str().unwrap(),
        &input,
        output.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("anonymous report written to"), "{stderr}");
    assert!(stderr.contains("the salt "), "{stderr}");
    let text = std::fs::read_to_string(&anon).unwrap();
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["schema"], "dutchman.report-anon/1");
    assert_eq!(report["totals"]["files"], 1);
    assert_eq!(report["totals"]["converted"], 1);
    assert!(!text.contains("generated_unit_cube"), "{text}");
    assert!(!text.contains(dir.path().to_str().unwrap()), "{text}");
    let key: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&key).unwrap()).unwrap();
    assert_eq!(key["files"].as_object().unwrap().len(), 1);
}

#[test]
fn investigate_writes_a_summary_with_nothing_from_the_design_and_sends_nothing_unasked() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Private Client 5521");
    std::fs::create_dir_all(&folder).unwrap();
    let input = folder.join("Secret Widget 9137.f3d");
    std::fs::copy(fixture("container_metadata_only.f3d"), &input).unwrap();
    let out = dir.path().join("Scratch Area 8810");
    let args = [
        "investigate",
        input.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--no-compare",
        "--json",
    ];
    // No terminal and no answer given: nothing is sent and nothing is asked.
    let output = dutchman(&args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["schema"], "dutchman.investigate/1");
    assert_eq!(result["sent"], false);
    assert_eq!(result["asked"], false);
    assert_eq!(result["report"], "converted");
    let summary = std::fs::read_to_string(out.join("summary.md")).unwrap();
    assert!(summary.contains("- Outcome: failed"), "{summary}");
    for private in [
        "Secret",
        "Widget",
        "9137",
        "Private",
        "Client",
        "5521",
        "Scratch",
        "8810",
        dir.path().to_str().unwrap(),
    ] {
        assert!(!summary.contains(private), "{private} in:\n{summary}");
    }
    assert!(out.join("local-key.json").is_file());
    // Answered no in advance: the earlier summary is reused, still nothing sent.
    let mut no = args.to_vec();
    no.push("--no-send");
    let output = dutchman(&no);
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["sent"], false);
    assert_eq!(
        std::fs::read_to_string(out.join("summary.md")).unwrap(),
        summary
    );
    // Both answers at once are refused.
    let mut both = no.clone();
    both.push("--yes");
    assert_eq!(dutchman(&both).status.code(), Some(2));
}

#[test]
fn stand_up_cli_flags_control_occurrence_transform() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture("generated_unit_cube.f3d");
    let output_default = dir.path().join("default.sc");
    let output_stand_up = dir.path().join("stand_up.sc");
    let output_no_stand_up = dir.path().join("no_stand_up.sc");

    let res = dutchman(&["convert", &input, output_default.to_str().unwrap()]);
    assert!(res.status.success());
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&output_default).unwrap()).unwrap();
    let doc_file = archive.by_name("document.json").unwrap();
    let doc: serde_json::Value = serde_json::from_reader(doc_file).unwrap();
    assert_eq!(
        doc["occurrences"][0]["transform"],
        serde_json::json!([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    );

    let res_stand_up = dutchman(&[
        "convert",
        "--stand-up",
        &input,
        output_stand_up.to_str().unwrap(),
    ]);
    assert!(res_stand_up.status.success());
    let mut archive_su =
        zip::ZipArchive::new(std::fs::File::open(&output_stand_up).unwrap()).unwrap();
    let doc_file_su = archive_su.by_name("document.json").unwrap();
    let doc_su: serde_json::Value = serde_json::from_reader(doc_file_su).unwrap();
    assert_eq!(
        doc_su["occurrences"][0]["transform"],
        serde_json::json!([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    );

    let res2 = dutchman(&[
        "convert",
        "--no-stand-up",
        &input,
        output_no_stand_up.to_str().unwrap(),
    ]);
    assert!(res2.status.success());
    let mut archive2 =
        zip::ZipArchive::new(std::fs::File::open(&output_no_stand_up).unwrap()).unwrap();
    let doc_file2 = archive2.by_name("document.json").unwrap();
    let doc2: serde_json::Value = serde_json::from_reader(doc_file2).unwrap();
    assert_eq!(
        doc2["occurrences"][0]["transform"],
        serde_json::json!([
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    );
}
