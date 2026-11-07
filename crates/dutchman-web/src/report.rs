//! A batch's `dutchman.convert-all/1` report, made fit for the browser: server paths taken out,
//! download links added, and each file's result put in words as the desktop app puts it.

use std::path::Path;

use dutchman::anon::Salt;
use dutchman::present::{self, FileView, SendSteps, Surface};
use dutchman::report_policy::{MessageFacts, ReportPolicy};
use serde_json::{json, Value};

/// Takes the job's folders out of every string in `report`, so paths read as the uploaded
/// names (`project/part.ipt`) and messages do not show where the server keeps them, and adds
/// to each converted file's `output` a `download` link, relative to the page at `/`.
pub fn for_browser(report: &mut Value, job: &str, in_dir: &Path, out_dir: &Path) {
    let prefixes: Vec<String> = [in_dir, out_dir]
        .iter()
        .flat_map(|dir| {
            let shown = dir.display().to_string();
            [format!("{shown}/"), format!("{shown}\\"), shown]
        })
        .collect();
    scrub(report, &prefixes);
    if let Some(Value::Array(results)) = report.get_mut("results") {
        for result in results {
            result.as_object_mut().map(|r| r.remove("names"));
            if let Some(output) = result.get_mut("output").and_then(Value::as_object_mut) {
                if let Some(path) = output.get("path").and_then(Value::as_str) {
                    let path = path.replace('\\', "/");
                    output.insert(
                        "download".to_owned(),
                        json!(format!("jobs/{job}/files/{}", encode_path(&path))),
                    );
                    output.insert("path".to_owned(), json!(path));
                }
            }
            if let Some(input) = result.get_mut("input").and_then(Value::as_object_mut) {
                if let Some(path) = input.get("path").and_then(Value::as_str) {
                    let path = path.replace('\\', "/");
                    input.insert("path".to_owned(), json!(path));
                }
            }
        }
    }
}

/// Adds SolidCraft links to a browser-ready report, using the app's address and the prefix
/// before its relative download paths (the base path followed by /).
pub fn open_links(report: &mut Value, address: &str, prefix: &str) {
    let Some(Value::Array(results)) = report.get_mut("results") else {
        return;
    };
    for result in results {
        if result.get("ok") != Some(&json!(true)) {
            continue;
        }
        let Some(output) = result.get_mut("output").and_then(Value::as_object_mut) else {
            continue;
        };
        if !output
            .get("format")
            .and_then(Value::as_str)
            .is_some_and(|f| f.starts_with("solidcraft/"))
        {
            continue;
        }
        if let Some(download) = output.get("download").and_then(Value::as_str) {
            let design = encode_component(&format!("{prefix}{download}"));
            output.insert("open".to_owned(), json!(format!("{address}?open={design}")));
        }
    }
}

fn encode_component(value: &str) -> String {
    encode_path(value).replace('/', "%2F")
}

fn scrub(value: &mut Value, prefixes: &[String]) {
    match value {
        Value::String(text) => {
            for prefix in prefixes {
                if text.contains(prefix.as_str()) {
                    *text = text.replace(prefix.as_str(), "");
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| scrub(item, prefixes)),
        Value::Object(fields) => fields.values_mut().for_each(|item| scrub(item, prefixes)),
        _ => {}
    }
}

/// Adds to each failed file of `report` a `private_report`: its id under the job's `salt` and
/// its size, for the steps to send it privately. The files must still be in place.
pub fn mark_failures(report: &mut Value, salt: &Salt) {
    let Some(Value::Array(results)) = report.get_mut("results") else {
        return;
    };
    for result in results {
        if result.get("ok") == Some(&json!(true)) {
            continue;
        }
        let Some(input) = result.pointer("/input/path").and_then(Value::as_str) else {
            continue;
        };
        let input = Path::new(input);
        let mark = json!({
            "id": salt.file_id(input),
            "size_bytes": std::fs::metadata(input).ok().map(|file| file.len()),
        });
        if let Some(fields) = result.as_object_mut() {
            fields.insert("private_report".to_owned(), mark);
        }
    }
}

/// `path` with each segment percent-encoded for a URL, keeping the `/` between them.
pub fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The steps to send a failed file privately, as the results page shows them.
#[derive(Debug, PartialEq, Eq)]
pub struct Send {
    pub invite_url: String,
    pub dm_url: String,
    pub dm_text: String,
    pub step_card: bool,
    /// The one-line message the DM button copies.
    pub message: String,
}

/// One file's result, as the results page shows it.
#[derive(Debug, PartialEq, Eq)]
pub struct Card {
    /// The uploaded file, as uploaded (`project/part.ipt`).
    pub name: String,
    pub ok: bool,
    /// Whether placements were left out or placed empty.
    pub incomplete: bool,
    /// Shared assembly placement summary and causes.
    pub assembly_lines: Vec<String>,
    /// What was written, in a sentence; or, for a failure, why it failed.
    pub outcome: String,
    /// For a failure: the stage it stopped in.
    pub stage: Option<String>,
    /// Features that did not map: name and reason.
    pub blocked: Vec<(String, String)>,
    /// Losses, most severe first: level, subject and message.
    pub losses: Vec<(String, String, String)>,
    /// How many losses of each level, as "2 Dropped, 1 Info".
    pub loss_counts: String,
    /// The written design: its name and download link.
    pub download: Option<(String, String)>,
    /// The link to open the design in SolidCraft.
    pub open: Option<String>,
    /// Where the design was saved in the server's output folder.
    pub saved: Option<String>,
    /// Why the design could not be saved in the server's output folder.
    pub save_error: Option<String>,
    /// For a failure worth reporting: how to send the file privately.
    pub send: Option<Send>,
}

/// The cards of a browser-ready report (see [`for_browser`]), in the report's order.
pub fn cards(report: &Value) -> Vec<Card> {
    report
        .get("results")
        .and_then(Value::as_array)
        .map(|results| results.iter().map(card).collect())
        .unwrap_or_default()
}

/// The steps for the failed file `result`, marked by [`mark_failures`].
fn send(result: &Value) -> Option<Send> {
    let mark = result.get("private_report")?;
    if !FileView::from_report(result).can_report {
        return None;
    }
    let policy = ReportPolicy::bundled();
    let size_bytes = mark.get("size_bytes").and_then(Value::as_u64);
    let steps = SendSteps::new(policy, size_bytes, result, Surface::Web);
    Some(Send {
        invite_url: policy.invite_url.clone(),
        dm_url: policy.dm_url(),
        dm_text: steps.dm_text,
        step_card: steps.step_card,
        message: policy.message(&MessageFacts {
            code: present::error_code(result),
            report_id: mark.get("id").and_then(Value::as_str)?,
            size_bytes,
            step: present::step_known(result),
        }),
    })
}

fn card(result: &Value) -> Card {
    let text = |pointer: &str| {
        result
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let ok = result.get("ok") == Some(&json!(true));
    let mut losses: Vec<(String, String, String)> = result
        .get("losses")
        .and_then(Value::as_array)
        .map(|losses| {
            losses
                .iter()
                .map(|loss| {
                    let field = |key: &str| {
                        loss.get(key)
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    (field("level"), field("subject"), field("message"))
                })
                .collect()
        })
        .unwrap_or_default();
    losses.sort_by_key(|(level, _, _)| severity(level));
    let loss_counts = ["Dropped", "Degraded", "Info"]
        .into_iter()
        .map(|level| (level, losses.iter().filter(|l| l.0 == level).count()))
        .filter(|(_, n)| *n > 0)
        .map(|(level, n)| format!("{n} {level}"))
        .collect::<Vec<_>>()
        .join(", ");
    let blocked = result
        .pointer("/history/features")
        .and_then(Value::as_array)
        .map(|features| {
            features
                .iter()
                .filter(|f| f.get("status").and_then(Value::as_str) == Some("blocked"))
                .map(|f| {
                    let field = |key: &str| {
                        f.get(key)
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    (field("name"), field("reason"))
                })
                .collect()
        })
        .unwrap_or_default();
    let download = match (text("/output/path"), text("/output/download")) {
        (Some(path), Some(link)) if ok => Some((path, link)),
        _ => None,
    };
    Card {
        name: text("/input/path").unwrap_or_else(|| "file".to_owned()),
        ok,
        incomplete: present::incomplete(result),
        assembly_lines: dutchman::json::completeness_lines(result),
        outcome: if ok {
            outcome(result)
        } else {
            text("/error/message").unwrap_or_else(|| "It could not be converted.".to_owned())
        },
        stage: if ok { None } else { text("/error/stage") },
        blocked,
        losses,
        loss_counts,
        download,
        open: text("/output/open").filter(|_| ok),
        saved: text("/output/saved").filter(|_| ok),
        save_error: text("/output/save_error").filter(|_| ok),
        send: send(result).filter(|_| !ok),
    }
}

fn severity(level: &str) -> u8 {
    match level {
        "Dropped" => 0,
        "Degraded" => 1,
        _ => 2,
    }
}

/// What was written, in a sentence, as the desktop app says it.
pub fn outcome(report: &Value) -> String {
    dutchman::present::outcome(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_history_card_shows_a_memory_skip() {
        let reason = "History check skipped: needs about 9.4 GB, 2.0 GB free";
        let report = serde_json::json!({"ok": true, "mode": "partial_history",
            "history": {"mapped": 2, "total": 3},
            "verification": {"status": "partial", "reason": "prefix unverified"},
            "history_check_skipped": {"reason": reason}});
        assert!(outcome(&report).contains(reason));
        assert!(outcome(&report).contains("unverified"));
    }

    use std::path::PathBuf;

    #[test]
    fn empty_placements_are_shown_on_the_browser_card() {
        let report = json!({"results": [{"ok": true, "completeness": {
            "placed": 67, "placed_empty": 13, "placements": 88,
            "left_out": [{"count": 8, "file": "pin.ipt", "reason": "not found"}]
        }}]});
        let cards = cards(&report);
        assert!(cards[0].incomplete);
        assert_eq!(
            cards[0].assembly_lines[0],
            "assembly: 67 of 88 placements placed; 13 placed empty; 8 left out:"
        );
    }

    #[test]
    fn server_paths_are_taken_out_and_links_added() {
        let in_dir = PathBuf::from("/data/jobs/abc/in");
        let out_dir = PathBuf::from("/data/jobs/abc/out");
        let mut report = json!({"results": [
            {"ok": true, "input": {"path": "/data/jobs/abc/in/proj/a b.ipt"},
             "output": {"path": "/data/jobs/abc/out/proj/a b.solidcraft"},
             "names": ["/data/jobs/abc/in/proj"]},
            {"ok": false, "input": {"path": "/data/jobs/abc/in/x.sldasm"},
             "error": {"message": "cannot read /data/jobs/abc/in/x.sldasm"}}
        ]});
        for_browser(&mut report, "abc", &in_dir, &out_dir);
        assert_eq!(report["results"][0]["input"]["path"], "proj/a b.ipt");
        assert_eq!(
            report["results"][0]["output"]["path"],
            "proj/a b.solidcraft"
        );
        assert_eq!(
            report["results"][0]["output"]["download"],
            "jobs/abc/files/proj/a%20b.solidcraft"
        );
        assert!(report["results"][0].get("names").is_none());
        assert_eq!(
            report["results"][1]["error"]["message"],
            "cannot read x.sldasm"
        );
    }

    #[test]
    fn open_query_encodes_the_download_path_again() {
        let mut report = json!({"results": [{"ok": true, "output": {
            "format": "solidcraft/23", "download": format!("jobs/j/files/{}", encode_path("a &?#%é.solidcraft"))
        }}]});
        open_links(&mut report, "/", "/dutchman/");
        assert_eq!(report["results"][0]["output"]["open"],
            "/?open=%2Fdutchman%2Fjobs%2Fj%2Ffiles%2Fa%2520%2526%253F%2523%2525%25C3%25A9.solidcraft");
    }

    #[test]
    fn cards_put_results_in_words() {
        let report = json!({"results": [
            {"ok": true, "input": {"path": "a.f3d"},
             "output": {"path": "a.solidcraft", "download": "jobs/j/files/a.solidcraft"},
             "mode": "import",
             "verification": {"status": "flattened", "reason": "feature \"Fillet\": not mapped yet"},
             "history": {"mapped": 1, "total": 2, "features": [
                {"name": "Sketch", "kind": "sketch", "status": "mapped", "reason": null},
                {"name": "Fillet", "kind": "fillet", "status": "blocked", "reason": "not mapped yet"}]},
             "losses": [
                {"level": "Info", "subject": "s", "message": "fine"},
                {"level": "Dropped", "subject": "t", "message": "gone"}]},
            {"ok": false, "input": {"path": "b.sldasm"},
             "error": {"message": "not supported yet", "stage": "decode"}}
        ]});
        let cards = cards(&report);
        assert_eq!(
            cards[0].outcome,
            "Geometry without history (flattened: feature \"Fillet\": not mapped yet)."
        );
        assert_eq!(
            cards[0].blocked,
            vec![("Fillet".to_owned(), "not mapped yet".to_owned())]
        );
        assert_eq!(cards[0].losses[0].0, "Dropped");
        assert_eq!(cards[0].loss_counts, "1 Dropped, 1 Info");
        assert_eq!(
            cards[0].download,
            Some((
                "a.solidcraft".to_owned(),
                "jobs/j/files/a.solidcraft".to_owned()
            ))
        );
        assert!(!cards[1].ok);
        assert_eq!(cards[1].outcome, "not supported yet");
        assert_eq!(cards[1].stage.as_deref(), Some("decode"));
        assert_eq!(cards[1].download, None);
        assert_eq!(cards[0].saved, None);
    }

    #[test]
    fn a_failed_card_offers_the_private_path_and_a_converted_one_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("Secret Bracket.f3d");
        std::fs::write(&small, vec![0; 3 << 19]).unwrap();
        let large = dir.path().join("big.step");
        std::fs::File::create(&large)
            .unwrap()
            .set_len(11 << 20)
            .unwrap();
        let mut report = json!({"results": [
            {"ok": false, "input": {"path": small, "format": "fusion_archive"},
             "error": {"message": "failed", "kind": "no_geometry"}},
            {"ok": false, "input": {"path": large, "format": "step"},
             "error": {"message": "two words: here"}},
            {"ok": true, "input": {"path": small}, "output": {"path": "a.sc"}}
        ]});
        let salt = Salt::random().unwrap();
        mark_failures(&mut report, &salt);
        let cards = cards(&report);

        let send = cards[0].send.as_ref().unwrap();
        assert_eq!(send.dm_text, "Drag the file in (the file you picked).");
        assert!(send.step_card);
        assert_eq!(send.dm_url, ReportPolicy::bundled().dm_url());
        assert_eq!(
            send.message,
            format!(
                "Dutchman {} couldn't convert my file. Error: no_geometry. Report id: {}. \
                 Size: 1.5 MB. STEP: unknown.",
                env!("CARGO_PKG_VERSION"),
                salt.file_id(&small)
            )
        );
        assert!(!send.message.contains("Secret"));

        let send = cards[1].send.as_ref().unwrap();
        assert!(send.dm_text.contains("private upload link"));
        assert!(!send.step_card);
        assert!(send.message.contains("Error: unknown."));
        assert!(send.message.ends_with("Size: 11 MB. STEP: yes."));
        assert_eq!(cards[2].send, None);
    }

    #[test]
    fn a_verified_history_says_so() {
        let report = json!({"mode": "history", "history": {"mapped": 3, "total": 3},
            "verification": {"status": "verified"}, "output": {"format": "solidcraft/23"}});
        assert_eq!(
            outcome(&report),
            "Editable history, 3 of 3 features rebuilt; verified in SolidCraft."
        );
    }
}
