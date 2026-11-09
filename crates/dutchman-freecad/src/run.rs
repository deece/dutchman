//! Execute only Dutchman's generated macros through FreeCAD's public scripting API.
use std::ffi::{OsStr, OsString};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use dutchman_core::{Check, Error, Mode, Model, Report, Severity, Writer};
use serde_json::Value;

use crate::{generate, FreeCadWriter};

/// The report format name of a native FreeCAD document.
pub const DOCUMENT_FORMAT: &str = "freecad-document/1";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_RESULT: u64 = 16 << 20;
const MARKER: &str = "DUTCHMAN_FREECAD_RESULT=";

#[derive(Clone, Debug)]
pub(crate) struct FreeCadCommand {
    program: PathBuf,
    args: Vec<OsString>,
    /// The Flatpak application id, when FreeCAD runs inside Flatpak's sandbox. The macro's
    /// temporary folder is then shared with the sandbox, and the id goes after that option.
    flatpak: Option<OsString>,
}

impl FreeCadCommand {
    /// The process to run, before its `-c` script. Arguments are passed as a vector; no shell
    /// sees them. A Flatpak sandbox is given `share` (the macro's folder) and nothing else new.
    pub(crate) fn process(&self, share: &Path) -> Command {
        let mut process = Command::new(&self.program);
        dutchman_solidcraft::hide_console(&mut process);
        process.args(&self.args);
        if let Some(app) = &self.flatpak {
            let mut option = OsString::from("--filesystem=");
            option.push(share);
            process.arg(option).arg(app);
        }
        process
    }
}

const FLATPAK_APP: &str = "org.freecad.FreeCAD";

/// `FREECAD_CMD="flatpak run --command=FreeCADCmd org.freecad.FreeCAD"`: split on whitespace
/// (no shell is involved) so the sandbox can be given the macro's folder.
fn flatpak_form(value: &OsStr, path: &OsStr) -> Option<FreeCadCommand> {
    let text = value.to_str()?;
    if Path::new(text).is_file() {
        return None;
    }
    let words = text.split_whitespace().collect::<Vec<_>>();
    let (first, rest) = words.split_first()?;
    let (app, args) = rest.split_last()?;
    if Path::new(first).file_name()? != "flatpak" || !app.starts_with("org.freecad.") {
        return None;
    }
    let program = if first.contains('/') {
        PathBuf::from(first)
    } else {
        on_path(first, path)?
    };
    Some(FreeCadCommand {
        program,
        args: args.iter().map(OsString::from).collect(),
        flatpak: Some(OsString::from(app)),
    })
}

fn on_path(name: &str, path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path).find_map(|dir| {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if cfg!(windows) {
            let candidate = dir.join(format!("{name}.exe"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        None
    })
}

fn discover(explicit: Option<OsString>, path: &OsStr, apps: &[PathBuf]) -> Option<FreeCadCommand> {
    if let Some(program) = explicit.filter(|value| !value.is_empty()) {
        if let Some(command) = flatpak_form(&program, path) {
            return Some(command);
        }
        return Some(FreeCadCommand {
            program: program.into(),
            args: Vec::new(),
            flatpak: None,
        });
    }
    for name in ["freecadcmd", "FreeCADCmd"] {
        if let Some(program) = on_path(name, path) {
            return Some(FreeCadCommand {
                program,
                args: Vec::new(),
                flatpak: None,
            });
        }
    }
    for app in apps {
        for name in ["freecadcmd", "FreeCADCmd"] {
            let program = app.join("Contents/Resources/bin").join(name);
            if program.is_file() {
                return Some(FreeCadCommand {
                    program,
                    args: Vec::new(),
                    flatpak: None,
                });
            }
        }
    }
    // Flatpak installations expose this metadata without starting a process.
    let installed = Path::new("/var/lib/flatpak/app").join(FLATPAK_APP).is_dir()
        || std::env::var_os("HOME").is_some_and(|home| {
            PathBuf::from(home)
                .join(".local/share/flatpak/app")
                .join(FLATPAK_APP)
                .is_dir()
        });
    if installed {
        return on_path("flatpak", path).map(|program| FreeCadCommand {
            program,
            args: ["run", "--command=FreeCADCmd"].map(OsString::from).to_vec(),
            flatpak: Some(OsString::from(FLATPAK_APP)),
        });
    }
    None
}

/// A macro writer that optionally executes it, or writes a native FreeCAD document.
#[derive(Clone, Debug)]
pub struct FreeCadRunner {
    writer: FreeCadWriter,
    save: bool,
    verify: bool,
    timeout: Duration,
}

impl FreeCadRunner {
    pub fn new(writer: FreeCadWriter, save: bool, verify: bool, timeout: Duration) -> Self {
        Self {
            writer,
            save,
            verify,
            timeout,
        }
    }
}

impl Writer for FreeCadRunner {
    fn extension(&self) -> &'static str {
        if self.save {
            "FCStd"
        } else {
            "FCMacro"
        }
    }

    fn write(&self, model: &Model, out: &mut dyn Write, report: &mut Report) -> Result<(), Error> {
        self.write_with_progress(model, out, report, None)
    }

    fn write_with_progress(
        &self,
        model: &Model,
        out: &mut dyn Write,
        report: &mut Report,
        progress: Option<&dutchman_core::Progress<'_>>,
    ) -> Result<(), Error> {
        let mut script = Vec::new();
        let preparing =
            progress.map(|p| p.stage_step(dutchman_core::StepKind::Preparing, Some(&model.name)));
        self.writer.write(model, &mut script, report)?;
        drop(preparing);
        if self.save || self.verify {
            let checking = progress.map(|p| {
                p.step(
                    dutchman_core::StepKind::Checking,
                    Some(&model.name),
                    Some(self.timeout),
                    Some(dutchman_core::progress_words::headless_limit(self.save)),
                )
            });
            let result = self.execute(&script);
            drop(checking);
            match result {
                Ok((check, notes, saved)) => {
                    for note in notes {
                        report.push(Severity::Degraded, "FreeCAD rebuild", note);
                    }
                    // Geometry-only output has no rebuild to verify; keep its existing status.
                    if let Some(mut outcome) = report.outcome().cloned() {
                        if outcome.mode == Mode::History {
                            outcome.check = check;
                            report.set_outcome(outcome);
                        }
                    }
                    if self.save {
                        out.write_all(&saved).map_err(encode)?;
                        return Ok(());
                    }
                }
                Err(reason) if self.save => {
                    return Err(encode(format!(
                        "{reason}; set FREECAD_CMD to a working FreeCAD headless executable"
                    )));
                }
                Err(reason) => {
                    if let Some(mut outcome) = report.outcome().cloned() {
                        if outcome.mode == Mode::History {
                            outcome.check = Check::Unverified(format!("not checked: {reason}"));
                            report.set_outcome(outcome);
                        }
                    }
                }
            }
        }
        out.write_all(&script).map_err(encode)
    }
}

fn encode(error: impl std::fmt::Display) -> Error {
    Error::Encode(format!("FreeCAD output: {error}"))
}

/// FreeCAD's headless command on this machine: `FREECAD_CMD`, else the usual install names.
pub(crate) fn installed() -> Option<FreeCadCommand> {
    let mut apps = vec![PathBuf::from("/Applications/FreeCAD.app")];
    if let Some(home) = std::env::var_os("HOME") {
        apps.push(PathBuf::from(home).join("Applications/FreeCAD.app"));
    }
    discover(
        std::env::var_os("FREECAD_CMD"),
        &std::env::var_os("PATH").unwrap_or_default(),
        &apps,
    )
}

impl FreeCadRunner {
    fn execute(&self, script: &[u8]) -> Result<(Check, Vec<String>, Vec<u8>), String> {
        let command = installed().ok_or_else(|| "FreeCAD not found".to_owned())?;
        self.execute_command(script, &command)
    }

    fn execute_command(
        &self,
        script: &[u8],
        command: &FreeCadCommand,
    ) -> Result<(Check, Vec<String>, Vec<u8>), String> {
        let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
        let input = dir.path().join("rebuild.FCMacro");
        let output = dir.path().join("out.FCStd");
        let save = if self.save {
            generate::quoted(&output.to_string_lossy()).map_err(|error| error.to_string())?
        } else {
            "None".to_owned()
        };
        let mut code = format!("dutchman_save_path = {save}\n").into_bytes();
        code.extend_from_slice(script);
        code.extend_from_slice(FINISH.as_bytes());
        std::fs::write(&input, code).map_err(|error| error.to_string())?;
        let log = tempfile::tempfile().map_err(|error| error.to_string())?;
        let mut process = command.process(dir.path());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            process.process_group(0);
        }
        let mut child = process
            .arg("-c")
            .arg(format!(
                "exec(open({}, encoding='utf-8').read())",
                generate::quoted(&input.to_string_lossy()).map_err(|error| error.to_string())?
            ))
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|error| error.to_string())?)
            .stderr(log.try_clone().map_err(|error| error.to_string())?)
            .spawn()
            .map_err(|error| format!("cannot run {}: {error}", command.program.display()))?;
        let started = Instant::now();
        loop {
            if log
                .metadata()
                .map_or(true, |metadata| metadata.len() > MAX_RESULT)
            {
                stop(&mut child);
                return Err("FreeCAD measurement output exceeds 16 MiB".to_owned());
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        return Err(format!("FreeCAD exited with {status}{}", last_line(&log)));
                    }
                    break;
                }
                Ok(None) if started.elapsed() >= self.timeout => {
                    stop(&mut child);
                    return Err(format!(
                        "FreeCAD timed out after {} s",
                        self.timeout.as_secs()
                    ));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(error) => {
                    stop(&mut child);
                    return Err(format!("cannot wait for FreeCAD: {error}"));
                }
            }
        }
        let mut log = log;
        log.seek(SeekFrom::Start(0))
            .map_err(|error| error.to_string())?;
        if log.metadata().map_err(|error| error.to_string())?.len() > MAX_RESULT {
            return Err("FreeCAD measurement output exceeds 16 MiB".to_owned());
        }
        let mut text = String::new();
        log.take(MAX_RESULT)
            .read_to_string(&mut text)
            .map_err(|error| error.to_string())?;
        let (check, notes) = parse(&text).map_err(|reason| {
            match text.lines().rev().find(|line| !line.trim().is_empty()) {
                Some(line) if !text.contains(MARKER) => format!("{reason}: {}", clip(line)),
                _ => reason,
            }
        })?;
        let saved = if self.save {
            let size = std::fs::metadata(&output)
                .map_err(|error| format!("FreeCAD did not save the document: {error}"))?
                .len();
            if size == 0 || size > 1 << 30 {
                return Err("FreeCAD saved an empty document or one larger than 1 GiB".to_owned());
            }
            std::fs::read(output).map_err(|error| error.to_string())?
        } else {
            Vec::new()
        };
        Ok((check, notes, saved))
    }
}

/// The last line FreeCAD printed, to explain a failure.
fn last_line(log: &std::fs::File) -> String {
    let mut log = log;
    let mut text = String::new();
    if log.seek(SeekFrom::Start(0)).is_err()
        || log.take(MAX_RESULT).read_to_string(&mut text).is_err()
    {
        return String::new();
    }
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| format!(": {}", clip(line)))
        .unwrap_or_default()
}

fn clip(line: &str) -> String {
    let line = line.trim();
    match line.char_indices().nth(300) {
        Some((end, _)) => format!("{}...", &line[..end]),
        None => line.to_owned(),
    }
}

fn stop(child: &mut std::process::Child) {
    // Include descendants such as the FreeCAD process started by Flatpak or a wrapper.
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("taskkill")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["/F", "/T", "/PID", &child.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn measurement(value: &Value) -> Result<(f64, Vec<f64>), String> {
    let volume = value
        .get("volume")
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0)
        .ok_or("FreeCAD result has no positive finite solid volume")?;
    let bounds = value
        .get("bounds")
        .and_then(Value::as_array)
        .filter(|b| b.len() == 6)
        .ok_or("FreeCAD result has no six-coordinate bounding box")?;
    let bounds = bounds
        .iter()
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| "FreeCAD bounding box is not finite".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if bounds
        .iter()
        .take(3)
        .zip(bounds.iter().skip(3))
        .any(|(a, b)| a > b || !(b - a).is_finite())
    {
        return Err("FreeCAD bounding box has reversed or unbounded limits".to_owned());
    }
    Ok((volume, bounds))
}

fn parse(text: &str) -> Result<(Check, Vec<String>), String> {
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix(MARKER))
        .ok_or("FreeCAD wrote no measurement JSON")?;
    let value: Value =
        serde_json::from_str(line).map_err(|error| format!("FreeCAD measurement JSON: {error}"))?;
    let parts = value
        .get("parts")
        .and_then(Value::as_array)
        .filter(|p| !p.is_empty() && p.len() <= 100_000)
        .ok_or("FreeCAD result has no parts or exceeds the part limit")?;
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    let mut unchecked = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let name = part
            .get("name")
            .and_then(Value::as_str)
            .ok_or("FreeCAD part has no name")?;
        let label = format!("part {} ({name:?})", index + 1);
        if let Some(note) = part.get("fallback").and_then(Value::as_str) {
            notes.push(format!("{label}: original STEP restored: {note}"));
            problems.push(format!("{label}: rebuild differs: {note}"));
        }
        let original = part
            .get("original")
            .and_then(Value::as_array)
            .ok_or("FreeCAD part has no original solids")?;
        let actual = part
            .get("actual")
            .and_then(Value::as_array)
            .ok_or("FreeCAD part has no resulting solids")?;
        if original.len() > 100_000 || actual.len() > 100_000 {
            return Err(format!("{label}: solid count exceeds 100000"));
        }
        if original.is_empty() {
            // Sheets and wires have no volume to compare.
            unchecked.push(format!("{label} has no solids"));
            continue;
        }
        let mut original = original
            .iter()
            .map(measurement)
            .collect::<Result<Vec<_>, _>>()?;
        let mut actual = actual
            .iter()
            .map(measurement)
            .collect::<Result<Vec<_>, _>>()?;
        if original.len() != actual.len() {
            problems.push(format!(
                "{label}: {} solids, expected {}",
                actual.len(),
                original.len()
            ));
            continue;
        }
        let sort = |a: &(f64, Vec<f64>), b: &(f64, Vec<f64>)| {
            a.0.total_cmp(&b.0).then_with(|| {
                a.1.iter()
                    .zip(&b.1)
                    .map(|(x, y)| x.total_cmp(y))
                    .find(|o| !o.is_eq())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        };
        original.sort_by(sort);
        actual.sort_by(sort);
        for ((want, bounds), (got, shown)) in original.iter().zip(&actual) {
            let tolerance = dutchman_solidcraft::VOLUME_TOLERANCE;
            if (want - got).abs() > want.abs().max(got.abs()) * tolerance {
                problems.push(format!(
                    "{label}: volume {got:.6} mm³, expected {want:.6} mm³"
                ));
            }
            // SolidCraft compares volume at 1e-4 relative tolerance. Use that same
            // relative tolerance for bounds, scaled by the part's span, not its origin.
            let span = bounds
                .iter()
                .take(3)
                .zip(bounds.iter().skip(3))
                .map(|(a, b)| (b - a).abs())
                .fold(1.0_f64, f64::max);
            if bounds
                .iter()
                .zip(shown)
                .any(|(a, b)| (a - b).abs() > span * tolerance)
            {
                problems.push(format!("{label}: bounding box differs"));
            }
        }
    }
    let check = if !problems.is_empty() {
        Check::Differs(problems.join("; "))
    } else if !unchecked.is_empty() {
        Check::Unverified(format!("not checked: {}", unchecked.join("; ")))
    } else {
        Check::Verified
    };
    Ok((check, notes))
}

/// Hooked into each generated part; local coordinates avoid mixing assembly placements.
pub(crate) fn record(name: &str) -> Result<String, Error> {
    Ok(format!(
        "dutchman_parts.append(({}, original, selected, fallback))\n",
        generate::quoted(name)?
    ))
}
pub(crate) const INIT: &str = "dutchman_parts = []\n";
const FINISH: &str = r#"
def dutchman_measure(shape):
    rows = []
    for solid in shape.Solids:
        box = solid.BoundBox
        rows.append({'volume': solid.Volume, 'bounds': [box.XMin, box.YMin, box.ZMin, box.XMax, box.YMax, box.ZMax]})
    return rows

doc.recompute()
result = {'parts': []}
for name, original, selected, fallback in dutchman_parts:
    result['parts'].append({'name': name, 'original': dutchman_measure(original.Shape), 'actual': dutchman_measure(selected.Shape), 'fallback': fallback})
if dutchman_save_path is not None:
    doc.saveAs(dutchman_save_path)
print('DUTCHMAN_FREECAD_RESULT=' + json.dumps(result, allow_nan=False))
"#;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_command_wins_and_paths_with_spaces_are_one_argument() {
        let command = discover(
            Some("/opt/FreeCAD app/FreeCADCmd".into()),
            OsStr::new(""),
            &[],
        )
        .unwrap();
        assert_eq!(
            command.program,
            PathBuf::from("/opt/FreeCAD app/FreeCADCmd")
        );
        assert!(command.args.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn the_flatpak_form_of_freecad_cmd_shares_the_macro_folder_with_the_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("flatpak"), b"").unwrap();
        let command = discover(
            Some("flatpak run --command=FreeCADCmd org.freecad.FreeCAD".into()),
            dir.path().as_os_str(),
            &[],
        )
        .unwrap();
        assert_eq!(command.program, dir.path().join("flatpak"));
        assert_eq!(command.args, ["run", "--command=FreeCADCmd"]);
        assert_eq!(command.flatpak, Some(OsString::from("org.freecad.FreeCAD")));
        // Any other command with spaces stays one program path.
        let other = discover(Some("my freecad".into()), dir.path().as_os_str(), &[]).unwrap();
        assert_eq!(other.program, PathBuf::from("my freecad"));
        assert!(other.flatpak.is_none());
    }

    #[test]
    fn discovers_path_names_and_macos_bundles() {
        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join(if cfg!(windows) {
            "FreeCADCmd.exe"
        } else {
            "FreeCADCmd"
        });
        std::fs::write(&executable, b"").unwrap();
        // Case-insensitive file systems (macOS) find it under either name.
        let found = discover(None, dir.path().as_os_str(), &[]).unwrap().program;
        assert_eq!(found.parent(), executable.parent());
        assert!(found
            .file_name()
            .unwrap()
            .to_string_lossy()
            .eq_ignore_ascii_case(&executable.file_name().unwrap().to_string_lossy()));
        let app = dir.path().join("FreeCAD.app");
        let bin = app.join("Contents/Resources/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("freecadcmd"), b"").unwrap();
        assert_eq!(
            discover(None, OsStr::new(""), &[app]).unwrap().program,
            bin.join("freecadcmd")
        );
        let missing = dir.path().join("missing");
        assert!(on_path("freecadcmd", missing.as_os_str()).is_none());
        assert!(discover(None, missing.as_os_str(), &[]).is_none());
    }

    fn result(parts: Value) -> String {
        format!(
            "FreeCAD startup noise\n{MARKER}{}\n",
            json!({"parts": parts})
        )
    }

    fn part() -> Value {
        let shape = json!({"volume": 1.0, "bounds": [0, 0, 0, 1, 1, 1]});
        json!({"name": "Synthetic", "fallback": null, "original": [shape.clone()], "actual": [shape]})
    }

    #[test]
    fn parses_matching_solids_and_tolerance() {
        let mut row = part();
        row["actual"][0]["volume"] = json!(1.00001);
        row["actual"][0]["bounds"][0] = json!(0.00001);
        assert_eq!(
            parse(&result(json!([row]))).unwrap(),
            (Check::Verified, Vec::<String>::new())
        );
    }

    #[test]
    fn reports_part_for_volume_bounds_counts_and_fallback() {
        for (field, value) in [
            ("volume", json!(1.1)),
            ("bounds", json!([0, 0, 0, 2, 1, 1])),
        ] {
            let mut row = part();
            row["actual"][0][field] = value;
            assert!(
                matches!(parse(&result(json!([row]))).unwrap().0, Check::Differs(reason) if reason.contains("part 1") && reason.contains("Synthetic"))
            );
        }
        let mut row = part();
        row["actual"] = json!([]);
        assert!(matches!(
            parse(&result(json!([row]))).unwrap().0,
            Check::Differs(_)
        ));
        let mut row = part();
        row["fallback"] = json!("Rebuilt geometry differs from original STEP");
        let (check, notes) = parse(&result(json!([row]))).unwrap();
        assert!(matches!(check, Check::Differs(_)));
        assert!(notes[0].contains("original STEP restored"));
    }

    #[test]
    fn refuses_missing_malformed_and_unmeasurable_results() {
        for text in [
            "",
            "DUTCHMAN_FREECAD_RESULT={",
            "DUTCHMAN_FREECAD_RESULT={\"parts\":[]}",
        ] {
            assert!(parse(text).is_err());
        }
        for bad in [json!(-1), json!(0), json!("NaN"), Value::Null] {
            let mut row = part();
            row["actual"][0]["volume"] = bad;
            assert!(parse(&result(json!([row]))).is_err());
        }
        let mut row = part();
        row["original"] = json!([]);
        assert!(matches!(parse(&result(json!([row]))).unwrap().0,
            Check::Unverified(reason) if reason.contains("has no solids")));
        let mut row = part();
        row["actual"][0]["bounds"] = json!([0, 1]);
        assert!(parse(&result(json!([row]))).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn child_output_is_parsed_and_slow_processes_are_stopped() {
        let command = |script: String| FreeCadCommand {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            flatpak: None,
        };
        let runner = FreeCadRunner::new(FreeCadWriter::new(), false, true, Duration::from_secs(5));
        let output = result(json!([part()]));
        // The fake only prints synthetic measurements; it never executes the macro.
        let print = format!("printf '%s' '{}'", output.replace('\'', "'\\''"));
        assert_eq!(
            runner.execute_command(b"", &command(print)).unwrap().0,
            Check::Verified
        );
        let runner =
            FreeCadRunner::new(FreeCadWriter::new(), false, true, Duration::from_millis(50));
        let started = Instant::now();
        assert!(runner
            .execute_command(b"", &command("exec sleep 10".to_owned()))
            .unwrap_err()
            .contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
