//! Synthetic-only end-to-end folder checks, checkpoint reuse and matching exports.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures/generated_unit_cube.f3d")
}

fn command() -> Command {
    #[cfg(unix)]
    {
        let mut command = Command::new("nice");
        command
            .args([
                "-n",
                "10",
                "bash",
                "-c",
                "ulimit -v 16000000; exec \"$@\"",
                "dutchman-test",
            ])
            .arg(env!("CARGO_BIN_EXE_dutchman"));
        command
    }
    #[cfg(not(unix))]
    {
        Command::new(env!("CARGO_BIN_EXE_dutchman"))
    }
}

fn report(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn folder_checkpoints_resume_and_changed_source_is_checked_again() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("private-project");
    let out = dir.path().join("checked");
    std::fs::create_dir(&input).unwrap();
    std::fs::copy(fixture(), input.join("private-file.f3d")).unwrap();
    let run = || {
        report(
            command()
                .arg("doctor")
                .arg(&input)
                .arg("--out")
                .arg(&out)
                .args([
                    "--jobs",
                    "1",
                    "--memory-mb",
                    "2048",
                    "--no-verify",
                    "--preset",
                    "fast",
                    "--json",
                ])
                .output()
                .unwrap(),
        )
    };
    let first = run();
    assert_eq!(first["files"][0]["converted"], true);
    assert_eq!(first["files"][0]["check_status"], "complete");
    assert!(!first.to_string().contains("private-"));
    let checkpoint = std::fs::read_dir(out.join("private-checkpoints"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("complete.json");
    let changed = std::fs::metadata(&checkpoint).unwrap().modified().unwrap();
    assert_eq!(run()["files"], first["files"]);
    assert_eq!(
        std::fs::metadata(&checkpoint).unwrap().modified().unwrap(),
        changed
    );
    std::fs::write(input.join("second.f3d"), b"synthetic refused input").unwrap();
    let changed = run();
    assert_eq!(changed["files"].as_array().unwrap().len(), 2);
    assert!(!changed["findings"].as_array().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn diagnose_discovers_step_and_reports_a_planted_measurement_difference() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("private-part.f3d");
    std::fs::copy(fixture(), &input).unwrap();
    let model = dutchman_core::read_in(
        dutchman_core::Format::FusionArchive,
        &input,
        &dutchman_core::PartSearch::default(),
        &mut dutchman_core::Report::default(),
    )
    .unwrap();
    let step =
        dutchman_solidcraft::restate_step(&model, &mut dutchman_core::Report::default()).unwrap();
    std::fs::write(input.with_extension("STEP"), step).unwrap();
    let cli = dir.path().join("synthetic-cli");
    std::fs::write(&cli,r#"#!/bin/bash
if [ "$1" = --version ]; then echo 'solidcraft-cli 0.40.1'; exit; fi
volume=1000
case "$2" in *reference.solidcraft) volume=500;; esac
while [ "$#" -gt 0 ]; do
  if [ "$1" = --json-out ]; then shift; output="$1"; fi
  shift
done
printf '{"bodies":[{"name":"private-cli-name","volume_mm3":%s,"area_mm2":600,"bbox":{"min":[0,0,0],"max":[10,10,10]},"faces":6}],"timeline":[]}' "$volume" > "$output"
"#).unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = report(
        command()
            .arg("diagnose")
            .arg(&input)
            .arg("--out")
            .arg(dir.path().join("checked"))
            .args(["--part", "1", "--json"])
            .env("SOLIDCRAFT_CLI", cli)
            .output()
            .unwrap(),
    );
    let anonymous = &result["report"];
    assert_eq!(anonymous["target_part"], 1);
    assert_eq!(anonymous["step_comparison"]["status"], "measured");
    assert_eq!(
        anonymous["step_comparison"]["parts"][0]["looks_wrong"],
        true
    );
    assert!(!anonymous.to_string().contains("private"));
}
