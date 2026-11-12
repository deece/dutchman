//! Live diagnostics: privacy, member events, default placement, and a killed real converter.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures/generated_unit_cube.f3d")
}

fn records(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Assemble repository-authored geometry using cadmpeg's public synthetic F3Z recipe.
fn archive(path: &Path) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("ZebraWidget.f3d", stored).unwrap();
    zip.write_all(&std::fs::read(fixture()).unwrap()).unwrap();
    // An unreachable member is still classified during archive preparation.
    zip.start_file("PrivateUnreachable.f3d", stored).unwrap();
    zip.write_all(&std::fs::read(fixture()).unwrap()).unwrap();
    zip.start_file("Manifest.json", stored).unwrap();
    zip.write_all(br#"{"root":"ZebraWidget.f3d"}"#).unwrap();
    zip.start_file("DesignDescription.json", stored).unwrap();
    zip.write_all(br#"{"name":"Autodesk Design Description","version":"0.1","designDescription":{"id":"0","designGraphs":[]}}"#).unwrap();
    zip.finish().unwrap();
}

#[test]
fn conversion_scrubs_report_names_and_paths_and_finishes_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("ZebraWidgetSecret.f3d");
    std::fs::copy(fixture(), &input).unwrap();
    let path = dir.path().join("diagnostic.jsonl");
    let options = dutchman::ConvertOptions {
        diagnostics: Some(path.clone()),
        no_verify: true,
        ..Default::default()
    };
    let local_names = std::sync::Mutex::new(Vec::new());
    let report = dutchman::convert_options_with_progress(
        &input,
        &dir.path().join("out.solidcraft"),
        &options,
        true,
        &Default::default(),
        &|event| {
            if let Some(name) = event.running.and_then(|s| s.name) {
                local_names.lock().unwrap().push(name);
            }
        },
    );
    assert_eq!(report["ok"], true, "{report}");
    let names = report["names"].as_array().unwrap();
    assert!(
        names
            .iter()
            .any(|name| name.as_str().is_some_and(|s| s.contains("unit cube"))),
        "{names:?}"
    );
    let text = std::fs::read_to_string(&path).unwrap();
    for name in names.iter().filter_map(Value::as_str) {
        assert!(!text.contains(name), "name {name:?} leaked into {text}");
    }
    let local_names = local_names.into_inner().unwrap();
    assert!(local_names
        .iter()
        .any(|name| name.contains("ZebraWidgetSecret")));
    assert!(local_names.iter().any(|name| name == "ZebraWidgetSecret"));
    for name in local_names {
        assert!(!text.contains(&name), "running name leaked: {name}");
    }
    assert!(!text.contains("running"));
    assert!(!text.contains("ZebraWidgetSecret"));
    assert!(!text.contains(dir.path().to_str().unwrap()));
    let lines = records(&path);
    assert!(lines
        .iter()
        .all(|line| line["schema"] == dutchman::diagnostics::SCHEMA));
    assert_eq!(lines.first().unwrap()["type"], "start");
    assert!(lines
        .iter()
        .any(|line| line["type"] == "stage" && line["stage"] == "decode"));
    assert!(lines
        .iter()
        .any(|line| line["type"] == "design" && line["count"] == 1));
    assert_eq!(lines.last().unwrap()["type"], "end");
    assert_eq!(lines.last().unwrap()["ok"], true);
}

#[test]
fn fusion_member_events_reach_both_direct_and_isolated_logs() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("PrivateAssembly.f3z");
    archive(&input);
    for isolated in [false, true] {
        let path = dir.path().join(format!("diagnostic-{isolated}.jsonl"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_dutchman"));
        if isolated {
            command
                .args(["convert-all", "--jobs", "1", "--out-dir"])
                .arg(dir.path().join("out"));
        } else {
            command
                .arg("convert")
                .arg(&input)
                .arg(dir.path().join("direct.solidcraft"));
        }
        command.args(["--no-verify", "--diagnostics"]).arg(&path);
        if isolated {
            command.arg(&input);
        }
        let out = command.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let lines = records(&path);
        let start = lines
            .iter()
            .find(|v| v["type"] == "member" && v["event"] == "start")
            .unwrap();
        let id = start["member"].as_str().unwrap();
        let members = lines[0]["members"].as_array().unwrap();
        let listed = members.iter().find(|v| v["id"] == id).unwrap();
        assert!(listed["nested_content_bytes"].as_u64().unwrap() > 0);
        assert!(lines
            .iter()
            .any(|v| v["type"] == "member" && v["event"] == "finish" && v["member"] == id));
        for phase in [
            "codec_selection",
            "container_prepare",
            "archive_scan",
            "archive_classify",
            "archive_session_open",
            "container_compose",
            "model_validation",
        ] {
            let timed: Vec<_> = lines
                .iter()
                .filter(|v| v["type"] == "phase" && v["phase"] == phase)
                .collect();
            assert_eq!(timed.len(), 2, "{phase}: {lines:?}");
            assert_eq!(timed[0]["event"], "start");
            assert_eq!(timed[1]["event"], "finish");
            assert_eq!(timed[1]["bytes"], std::fs::metadata(&input).unwrap().len());
        }
        let prepared = lines
            .iter()
            .position(|v| {
                v["type"] == "phase" && v["phase"] == "container_prepare" && v["event"] == "finish"
            })
            .unwrap();
        let first_member = lines.iter().position(|v| v["type"] == "member").unwrap();
        assert!(prepared < first_member);
        let classifications: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(_, row)| row["type"] == "member_classified")
            .collect();
        assert_eq!(classifications.len(), 2, "{lines:?}");
        let ids: std::collections::BTreeSet<_> = classifications
            .iter()
            .map(|(index, row)| {
                assert!(*index < first_member);
                for field in ["ms", "nested_scan_ms", "layers_ms", "metadata_ms"] {
                    assert!(row[field].as_u64().is_some(), "{row}");
                }
                let id = row["member"].as_str().unwrap();
                assert!(members.iter().any(|member| member["id"] == id));
                id
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(id));

        let end = lines.last().unwrap();
        assert_eq!(end["type"], "end");
        assert_eq!(end["member_decodes"][id], 1);
        assert_eq!(end["redecoded"], 0);
        let text = std::fs::read_to_string(&path).unwrap();
        for secret in [
            "ZebraWidget",
            "PrivateAssembly",
            "PrivateUnreachable",
            "Manifest.json",
            "unit cube",
        ] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }
}

#[test]
fn default_location_and_explicit_override_are_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_dutchman"));
    // Change only the child's environment; parallel tests keep their actual home folders.
    command
        .env("HOME", dir.path())
        .env("LOCALAPPDATA", dir.path())
        .env("XDG_STATE_HOME", dir.path());
    let out = command
        .args(["convert", "--diagnostics-default", "--no-verify"])
        .arg(fixture())
        .arg(dir.path().join("out.solidcraft"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let folder = if cfg!(windows) {
        dir.path().join("Dutchman/diagnostics")
    } else if cfg!(target_os = "macos") {
        dir.path().join("Library/Logs/Dutchman")
    } else {
        dir.path().join("dutchman/diagnostics")
    };
    let paths: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(paths.len(), 1);
    assert!(!paths[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("cube"));
    assert!(paths[0]
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with(&format!(
            "_dutchman-{}_converted.jsonl",
            dutchman::BUILD_VERSION
        )));
    assert_eq!(records(&paths[0]).last().unwrap()["type"], "end");
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args([
            "convert",
            "--diagnostics-default",
            "--diagnostics",
            "explicit.jsonl",
        ])
        .arg(fixture())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn a_killed_conversion_leaves_start_and_heartbeat_as_valid_complete_lines() {
    // The debug build holds the real conversion at its write stage (a test-only switch), so
    // the kill lands there deterministically on every platform: SIGKILL on Unix,
    // TerminateProcess on Windows.
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("held.solidcraft");
    let log = dir.path().join("killed.jsonl");
    let child = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args(["convert", "--no-verify", "--diagnostics"])
        .arg(&log)
        .arg(fixture())
        .arg(&output)
        .env("DUTCHMAN_DIAGNOSTICS_HEARTBEAT_MS", "10")
        .env("DUTCHMAN_DIAGNOSTICS_TEST_HOLD", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    struct KillOnDrop(std::process::Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = KillOnDrop(child);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let ready = std::fs::read_to_string(&log).ok().is_some_and(|text| {
            text.lines().any(|line| {
                serde_json::from_str::<Value>(line)
                    .ok()
                    .is_some_and(|v| v["type"] == "heartbeat" && v["stage"] == "write")
            })
        });
        if ready {
            break;
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "conversion ended before kill"
        );
        assert!(
            Instant::now() < deadline,
            "conversion never reached the held write"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    child.0.kill().unwrap();
    let status = child.0.wait().unwrap();
    assert!(!status.success());
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9));
    }
    let text = std::fs::read_to_string(&log).unwrap();
    // SIGKILL may interrupt one write: ignore only an incomplete trailing line.
    let complete = text.rsplit_once('\n').map(|(lines, _)| lines).unwrap_or("");
    let lines: Vec<Value> = complete
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["type"], "start");
    assert!(lines.iter().any(|line| line["type"] == "heartbeat"));
    assert!(lines.iter().all(|line| line["type"] != "end"));
}

#[test]
fn batch_diagnostics_share_the_anonymous_reports_salt_and_file_ids() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("log.jsonl");
    let key = dir.path().join("key.json");
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args(["convert-all", "--no-verify", "--jobs", "1", "--out-dir"])
        .arg(dir.path().join("out"))
        .arg("--diagnostics")
        .arg(&log)
        .arg("--report-anon")
        .arg(dir.path().join("anon.json"))
        .arg("--report-key")
        .arg(&key)
        .args(["--report-salt", "00112233445566778899aabbccddeeff"])
        .arg(fixture())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let key: Value = serde_json::from_slice(&std::fs::read(key).unwrap()).unwrap();
    let lines = records(&log);
    let id = lines[0]["file"].as_str().unwrap();
    assert!(key["files"].get(id).is_some(), "{key}");
    assert!(lines.iter().all(|line| line["file"] == id));
    assert!(!std::fs::read_to_string(log)
        .unwrap()
        .contains("00112233445566778899aabbccddeeff"));
}

#[test]
fn watch_writes_diagnostics_while_keeping_event_output_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = dir.path().join("inbox");
    std::fs::create_dir(&inbox).unwrap();
    std::fs::copy(fixture(), inbox.join("ZebraWatch.f3d")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .env("HOME", dir.path())
        .env("LOCALAPPDATA", dir.path())
        .env("XDG_STATE_HOME", dir.path())
        .arg("watch")
        .arg(inbox)
        .arg("--out-dir")
        .arg(dir.path().join("out"))
        .args([
            "--once",
            "--settle",
            "0",
            "--no-verify",
            "--json",
            "--jobs",
            "1",
            "--diagnostics-default",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let folder = if cfg!(windows) {
        dir.path().join("Dutchman").join("diagnostics")
    } else if cfg!(target_os = "macos") {
        dir.path().join("Library/Logs/Dutchman")
    } else {
        dir.path().join("dutchman/diagnostics")
    };
    let log = std::fs::read_dir(folder)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(log
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("_converted.jsonl"));
    let event: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(event["schema"], "dutchman.watch-event/1");
    assert_eq!(event["event"], "converted");
    assert_eq!(records(&log).last().unwrap()["type"], "end");
    assert!(!std::fs::read_to_string(log).unwrap().contains("ZebraWatch"));
}

#[cfg(unix)]
#[test]
fn supervised_timeout_finishes_the_log_at_the_last_stage() {
    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("out");
    std::fs::create_dir(&out_dir).unwrap();
    let output = out_dir.join("generated_unit_cube.sc");
    assert!(Command::new("mkfifo")
        .arg(output)
        .status()
        .unwrap()
        .success());
    let log = dir.path().join("timeout.jsonl");
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args([
            "convert-all",
            "--no-verify",
            "--jobs",
            "1",
            "--timeout",
            "3",
            "--out-dir",
        ])
        .arg(&out_dir)
        .arg("--diagnostics")
        .arg(&log)
        .arg(fixture())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines = records(&log);
    let end = lines.last().unwrap();
    assert_eq!(end["type"], "end");
    assert_eq!(end["ok"], false);
    assert_eq!(end["stage"], "write");
}

#[cfg(feature = "freecad")]
#[test]
fn drawing_conversion_also_writes_the_requested_log() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("drawing.jsonl");
    let input = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures/techdraw.FCStd");
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args(["convert", "--to", "dxf", "--diagnostics"])
        .arg(&log)
        .arg(input)
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines = records(&log);
    assert_eq!(lines[0]["type"], "start");
    assert_eq!(lines.last().unwrap()["type"], "end");
    assert_eq!(lines.last().unwrap()["ok"], true);
    assert!(!std::fs::read_to_string(log).unwrap().contains("techdraw"));
}

#[test]
fn default_batch_log_finishes_after_isolated_workers_and_names_failure() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("synthetic.f3d");
    std::fs::write(&bad, "synthetic refusal").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .env("HOME", dir.path())
        .env("LOCALAPPDATA", dir.path())
        .env("XDG_STATE_HOME", dir.path())
        .args([
            "convert-all",
            "--diagnostics-default",
            "--no-verify",
            "--jobs",
            "2",
            "--out-dir",
        ])
        .arg(dir.path().join("out"))
        .arg(fixture())
        .arg(bad)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let folder = if cfg!(windows) {
        dir.path().join("Dutchman").join("diagnostics")
    } else if cfg!(target_os = "macos") {
        dir.path().join("Library/Logs/Dutchman")
    } else {
        dir.path().join("dutchman/diagnostics")
    };
    let logs: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(logs.len(), 1);
    let path = logs.first().unwrap();
    assert!(path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("_failed.jsonl"));
    let rows = records(path);
    assert_eq!(rows.iter().filter(|r| r["type"] == "start").count(), 2);
    assert_eq!(rows.iter().filter(|r| r["type"] == "end").count(), 2);
}

#[test]
fn explicit_log_from_another_version_warns_and_still_converts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("selected.jsonl");
    std::fs::write(&path, "{\"dutchman\":\"0.0.1\"}\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args(["convert", "--no-verify", "--diagnostics"])
        .arg(&path)
        .arg(fixture())
        .arg(dir.path().join("out.sc"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!(
        "This log is from Dutchman 0.0.1; this is {}.",
        dutchman::BUILD_VERSION
    )));
    assert!(path.is_file());
    assert_eq!(records(&path).last().unwrap()["type"], "end");
}

#[test]
fn a_stuck_design_stops_at_the_design_limit_and_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("Stuck.f3z");
    archive(&input);
    let started = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_dutchman"))
        .args(["convert-all", "--jobs", "1", "--no-verify", "--out-dir"])
        .arg(dir.path().join("out"))
        .arg(&input)
        // Test builds only: the root design never finishes, and a design may read 10 s (a
        // debug build reads a small design in a few seconds).
        .env("DUTCHMAN_TEST_STUCK_DESIGN", "ZebraWidget.f3d")
        .env("DUTCHMAN_TEST_DESIGN_LIMIT_MS", "10000")
        .output()
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(120),
        "{:?}",
        started.elapsed()
    );
    let text = String::from_utf8_lossy(&out.stdout);
    // Left out and tried again without it; with the root gone the archive cannot convert, so
    // the failure names the design that was stuck.
    assert!(text.contains("ZebraWidget.f3d"), "{text}");
    assert!(text.contains("left out"), "{text}");
    assert!(text.contains("limit for one design"), "{text}");
    assert!(text.contains("DUTCHMAN_DESIGN_TIME_LIMIT"), "{text}");
}
