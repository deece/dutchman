//! Anonymous progress that survives an unfinished conversion. No report messages or names
//! are written; paths enter only the anonymous report's salted id function.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

pub use dutchman_core::MemberEvent;
use dutchman_core::{MemberObserver, ProgressEvent, Stage};

mod location;
pub use location::{
    default_folder, finish_default_log, new_default_log, new_log_in, unfinished_default_log,
    unfinished_log_in,
};
use serde_json::{json, Value};

use crate::{anon, choices, ConvertOptions};

pub const SCHEMA: &str = "dutchman.diagnostics/1";
// Serialize whole lines across simultaneous conversions, even with separate file handles.
static WRITE: Mutex<()> = Mutex::new(());

struct State {
    file: File,
    stage: Stage,
    designs: Option<(u64, Option<u64>)>,
    member: Option<String>,
    error: Option<String>,
    member_decodes: BTreeMap<String, u64>,
    member_classifications: u64,
    synced: Instant,
}

/// One conversion's live diagnostic writer. Dropping it flushes and stops its timer, without
/// inventing an end record. A parent supervising a worker sets its pid for resource samples.
pub struct Log {
    sink: Sink,
    pid: Arc<AtomicU32>,
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Log {
    pub fn open(path: &Path, input: &Path, options: &ConvertOptions) -> Result<Self, String> {
        let interval = std::env::var("DUTCHMAN_DIAGNOSTICS_HEARTBEAT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .map_or(Duration::from_secs(30), Duration::from_millis);
        Self::open_every(path, input, options, interval)
    }

    fn open_every(
        path: &Path,
        input: &Path,
        options: &ConvertOptions,
        interval: Duration,
    ) -> Result<Self, String> {
        let salt = options
            .diagnostics_salt
            .clone()
            .map_or_else(anon::Salt::random, Ok)?;
        if path == input
            || std::fs::canonicalize(path)
                .ok()
                .zip(std::fs::canonicalize(input).ok())
                .is_some_and(|(a, b)| a == b)
        {
            return Err(
                "the diagnostic log would change the input; choose another --diagnostics path"
                    .into(),
            );
        }
        if let Some(notice) = version_notice(path) {
            eprintln!("{notice}");
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|err| format!("cannot open diagnostic log {}: {err}", path.display()))?;
        let state = Arc::new(Mutex::new(State {
            file,
            stage: Stage::Input,
            designs: None,
            member: None,
            error: None,
            member_decodes: BTreeMap::new(),
            member_classifications: 0,
            synced: Instant::now(),
        }));
        let pid = Arc::new(AtomicU32::new(std::process::id()));
        let started = Instant::now();
        let id = salt.file_id(input);
        let mut log = Self {
            sink: Sink {
                state,
                started,
                id,
                salt,
            },
            pid,
            stop: None,
            thread: None,
        };
        log.record(json!({
            "type": "start", "dutchman": crate::BUILD_VERSION,
            "cadmpeg": anon::CADMPEG_VERSION,
            "os": {"family": std::env::consts::FAMILY, "name": std::env::consts::OS,
                   "version": os_version()},
            "cpu_count": std::thread::available_parallelism().ok().map(usize::from),
            "total_memory_bytes": total_memory(), "available_memory_bytes": choices::available_memory(),
            "choices": crate::json::placement_choices(options, options.choices()),
            "threads": options.diagnostics_threads,
            "input": {"format": dutchman_core::Format::from_path(input).map(|f| f.id()),
                      "size_bytes": std::fs::metadata(input).ok().map(|m| m.len())},
            "members": members(input, &log.sink.salt),
        }));
        log.check()?;
        let (stop, receive) = mpsc::channel::<()>();
        let state = Arc::clone(&log.sink.state);
        let pid = Arc::clone(&log.pid);
        let id = log.sink.id.clone();
        log.thread = Some(std::thread::spawn(move || {
            let mut heartbeat = Instant::now();
            while matches!(
                receive.recv_timeout(interval.min(Duration::from_secs(5))),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                if let Ok(mut state) = state.lock() {
                    if heartbeat.elapsed() < interval {
                        sync(&mut state);
                        continue;
                    }
                    heartbeat = Instant::now();
                    let (rss, ticks) = resources(pid.load(Ordering::Relaxed));
                    let stage = state.stage.id();
                    let member = state.member.clone();
                    write_record(
                        &mut state,
                        &id,
                        started,
                        json!({
                            "type": "heartbeat", "stage": stage, "rss_bytes": rss,
                            "cpu_ticks": ticks, "cpu_ticks_per_second": clock_ticks(),
                            "cpu_ms": ticks.zip(clock_ticks()).and_then(|(t, hz)| t.checked_mul(1000)?.checked_div(hz)),
                            "member": member,
                        }),
                    );
                }
            }
        }));
        log.stop = Some(stop);
        Ok(log)
    }

    pub fn process(&self, pid: u32) {
        self.pid.store(pid, Ordering::Relaxed);
    }

    pub fn progress(&self, event: ProgressEvent) {
        if let Ok(mut state) = self.sink.state.lock() {
            if state.stage != event.stage || state.designs.is_none() {
                state.stage = event.stage;
                write_record(
                    &mut state,
                    &self.sink.id,
                    self.sink.started,
                    json!({"type": "stage", "stage": event.stage.id()}),
                );
            }
            let designs = (event.designs_read, event.designs_total);
            if state.designs != Some(designs) {
                state.designs = Some(designs);
                write_record(
                    &mut state,
                    &self.sink.id,
                    self.sink.started,
                    json!({"type": "design", "count": designs.0, "total": designs.1}),
                );
            }
        }
        // Tests hold a real conversion at its write stage to kill it there, on every platform;
        // release builds never do.
        #[cfg(debug_assertions)]
        if event.stage == dutchman_core::Stage::Write
            && std::env::var_os("DUTCHMAN_DIAGNOSTICS_TEST_HOLD").is_some()
        {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }

    /// A cloneable sink for the decoder or the isolated worker's progress stream.
    pub fn observer(&self) -> Arc<dyn MemberObserver> {
        Arc::new(self.sink.clone())
    }

    pub fn phase_event(&self, phase: &str, event: MemberEvent, ms: u64, bytes: u64) {
        if let Some(phase) = phase_code(phase) {
            self.sink.phase_event(phase, event, ms, bytes);
        }
    }

    pub fn member_classified(
        &self,
        member: &str,
        ms: u64,
        nested_scan_ms: u64,
        layers_ms: u64,
        metadata_ms: u64,
    ) {
        self.sink
            .member_classified(member, ms, nested_scan_ms, layers_ms, metadata_ms);
    }

    pub fn member_event(
        &self,
        member: &str,
        event: MemberEvent,
        ms: u64,
        code: Option<&'static str>,
    ) {
        self.sink.member_event(member, event, ms, code);
    }

    fn record(&self, record: Value) {
        if let Ok(mut state) = self.sink.state.lock() {
            write_record(&mut state, &self.sink.id, self.sink.started, record);
        }
    }

    fn check(&self) -> Result<(), String> {
        let state = self
            .sink
            .state
            .lock()
            .map_err(|_| "diagnostic log writer stopped".to_owned())?;
        match &state.error {
            Some(error) => Err(format!(
                "cannot write diagnostic log: {error}; check free space and permissions"
            )),
            None => Ok(()),
        }
    }

    pub fn end(&mut self, report: &Value) -> Result<(), String> {
        self.stop_timer();
        let stage = self.sink.state.lock().map_or(Stage::Input, |s| s.stage);
        let mut counts = BTreeMap::<String, u64>::new();
        let mut identical = BTreeMap::<String, u64>::new();
        for loss in report
            .get("losses")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let code = loss.get("code").and_then(Value::as_str).map_or_else(
                || {
                    format!(
                        "dutchman/{}",
                        anon::subject_kind(
                            loss.get("subject").and_then(Value::as_str).unwrap_or("")
                        )
                    )
                },
                |c| anon::generic(c, "cadmpeg/other"),
            );
            *counts.entry(code.clone()).or_default() += 1;
            // Messages distinguish identical losses in memory only. They are never written.
            let key = json!([
                code,
                loss.get("level"),
                loss.get("stage"),
                loss.get("subject"),
                loss.get("message")
            ])
            .to_string();
            *identical.entry(key).or_default() += 1;
        }
        let repeats: u64 = identical.values().map(|n| n.saturating_sub(1)).sum();
        let member_decodes = self
            .sink
            .state
            .lock()
            .map(|s| s.member_decodes.clone())
            .unwrap_or_default();
        let redecoded: u64 = member_decodes.values().map(|n| n.saturating_sub(1)).sum();
        self.record(
            json!({"type": "end", "ok": report.get("ok") == Some(&Value::Bool(true)),
            "stage": stage.id(), "losses": counts, "repeated_losses": repeats,
            "member_decodes": member_decodes, "redecoded": redecoded,
            "error": anonymous_failure(report)}),
        );
        self.check()
    }

    fn stop_timer(&mut self) {
        self.stop.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Log {
    fn drop(&mut self) {
        self.stop_timer();
        if let Ok(mut state) = self.sink.state.lock() {
            sync(&mut state);
        }
    }
}

#[derive(Clone)]
struct Sink {
    state: Arc<Mutex<State>>,
    started: Instant,
    id: String,
    salt: anon::Salt,
}

impl MemberObserver for Sink {
    fn phase_event(&self, phase: &'static str, event: MemberEvent, ms: u64, bytes: u64) {
        if let Some(phase) = phase_code(phase) {
            if let Ok(mut state) = self.state.lock() {
                write_record(
                    &mut state,
                    &self.id,
                    self.started,
                    json!({"type": "phase", "phase": phase,
                    "event": if event == MemberEvent::Start { "start" } else { "finish" },
                    "ms": ms, "bytes": bytes}),
                );
            }
        }
    }
    fn member_classified(
        &self,
        member: &str,
        ms: u64,
        nested_scan_ms: u64,
        layers_ms: u64,
        metadata_ms: u64,
    ) {
        if let Ok(mut state) = self.state.lock() {
            // Bound records independently of the decoder's archive member cap.
            if state.member_classifications >= 100_000 {
                return;
            }
            state.member_classifications += 1;
            write_record(
                &mut state,
                &self.id,
                self.started,
                json!({"type": "member_classified", "member": self.salt.member_id(member),
                    "ms": ms, "nested_scan_ms": nested_scan_ms,
                    "layers_ms": layers_ms, "metadata_ms": metadata_ms}),
            );
        }
    }

    fn member_event(&self, member: &str, event: MemberEvent, ms: u64, code: Option<&'static str>) {
        if let Ok(mut state) = self.state.lock() {
            let id = self.salt.member_id(member);
            if event == MemberEvent::Start {
                state.member = Some(id.clone());
                if state.member_decodes.len() < 100_000 || state.member_decodes.contains_key(&id) {
                    *state.member_decodes.entry(id.clone()).or_default() += 1;
                }
            }
            let code = if event == MemberEvent::Fail {
                code.map(member_code)
            } else {
                None
            };
            let event = match event {
                MemberEvent::Start => "start",
                MemberEvent::Finish => "finish",
                MemberEvent::Fail => "fail",
            };
            write_record(
                &mut state,
                &self.id,
                self.started,
                json!({
                    "type": "member", "member": id, "event": event, "ms": ms,
                    "code": code,
                }),
            );
        }
    }
}

/// Build from allowlisted categories even when the report came across a worker boundary.
pub(crate) fn anonymous_failure(report: &Value) -> Value {
    if report.get("ok") == Some(&Value::Bool(true)) {
        return Value::Null;
    }
    let error = &report["error"];
    let stage = error["stage"]
        .as_str()
        .and_then(|s| Stage::ALL.into_iter().find(|stage| stage.id() == s));
    let kind = error["kind"]
        .as_str()
        .filter(|kind| {
            matches!(
                *kind,
                "io" | "not_a_file"
                    | "too_large"
                    | "unknown_format"
                    | "not_supported_yet"
                    | "output_exists"
                    | "wrong_output_extension"
                    | "decode"
                    | "cadmpeg_checks"
                    | "memory_budget"
                    | "encode"
                    | "assembly"
                    | "no_geometry"
                    | "restate"
            )
        })
        .unwrap_or("other");
    let counts = |field: &str, accept: &dyn Fn(&str) -> bool| -> BTreeMap<String, u64> {
        error[field]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| accept(key).then_some((key.clone(), value.as_u64()?)))
            .collect()
    };
    let findings = counts("findings", &|key| {
        // Display strings are fixed check codes; never pass through a source message.
        dutchman_core::CheckFacts::recognizes_code(key)
    });
    let curve_kinds = counts("curve_kinds", &|key| {
        matches!(
            key,
            "procedural" | "nurbs" | "analytic" | "other" | "unavailable"
        )
    });
    json!({"stage": stage.map(|s| s.id()), "kind": kind, "findings": findings, "curve_kinds": curve_kinds})
}

/// The worker's stderr is private input, never a source of arbitrary log labels.
fn phase_code(phase: &str) -> Option<&'static str> {
    match phase {
        "codec_selection" => Some("codec_selection"),
        "container_prepare" => Some("container_prepare"),
        "archive_scan" => Some("archive_scan"),
        "archive_classify" => Some("archive_classify"),
        "archive_session_open" => Some("archive_session_open"),
        "container_compose" => Some("container_compose"),
        "model_validation" => Some("model_validation"),
        _ => None,
    }
}

/// Accept only decoder categories, including across the worker protocol.
pub(crate) fn member_code(code: &str) -> &'static str {
    match code {
        "wrong_format" => "wrong_format",
        "malformed" => "malformed",
        "invalid_input" => "invalid_input",
        "truncated" => "truncated",
        "resource_limit" => "resource_limit",
        "unsupported_dialect" => "unsupported_dialect",
        "unsupported_target" => "unsupported_target",
        "insufficient_depth" => "insufficient_depth",
        "not_implemented" => "not_implemented",
        "io" => "io",
        _ => "other",
    }
}

fn sync(state: &mut State) {
    if let Err(error) = state.file.sync_data() {
        state.error = Some(error.to_string());
    }
    state.synced = Instant::now();
}

fn write_record(state: &mut State, id: &str, started: Instant, mut record: Value) {
    if state.error.is_some() {
        return;
    }
    if let Some(fields) = record.as_object_mut() {
        fields.insert("schema".into(), json!(SCHEMA));
        fields.insert("file".into(), json!(id));
        fields.insert(
            "elapsed_ms".into(),
            json!(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        );
    }
    let durable = record
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "stage" | "start" | "end"))
        || state.synced.elapsed() >= Duration::from_secs(5);
    let result = (|| -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&record).map_err(|err| err.to_string())?;
        bytes.push(b'\n');
        let _write = WRITE
            .lock()
            .map_err(|_| "diagnostic log writer stopped".to_owned())?;
        state
            .file
            .write_all(&bytes)
            .and_then(|()| state.file.flush())
            .map_err(|err| err.to_string())
    })();
    if let Err(error) = result {
        state.error = Some(error);
    }
    if durable {
        sync(state);
    }
}

/// Pick the same random salt source as report-anon, once for a front-end run.
pub fn prepare(options: &mut ConvertOptions, threads: usize) -> Result<(), String> {
    options.diagnostics_threads = threads;
    if options.diagnostics_default && options.diagnostics.is_none() {
        options.diagnostics = Some(new_default_log()?);
    }
    if options.diagnostics.is_some() && options.diagnostics_salt.is_none() {
        options.diagnostics_salt = Some(anon::Salt::random()?);
    }
    Ok(())
}

/// Wrap front-end work, including pre-read refusals, with a live log and final summary.
pub(crate) fn run(
    input: &Path,
    options: &ConvertOptions,
    progress: &(dyn Fn(ProgressEvent) + Send + Sync),
    work: impl FnOnce(&(dyn Fn(ProgressEvent) + Send + Sync), Option<&Log>) -> Value,
) -> Value {
    let owns_log = options.diagnostics_default && options.diagnostics.is_none();
    let mut prepared = options.clone();
    if let Err(message) = prepare(&mut prepared, options.diagnostics_threads) {
        return crate::json::failure(crate::json::CONVERT, crate::json::input(input), &message);
    }
    let options = &prepared;
    let mut log = match options
        .diagnostics
        .as_deref()
        .map(|path| Log::open(path, input, options))
        .transpose()
    {
        Ok(log) => log,
        Err(message) => {
            if owns_log {
                if let Some(path) = &options.diagnostics {
                    finish_default_log(path, "failed");
                }
            }
            return crate::json::failure(crate::json::CONVERT, crate::json::input(input), &message);
        }
    };
    let callback = |event: ProgressEvent| {
        if let Some(log) = &log {
            log.progress(event.clone());
        }
        progress(event);
    };
    let mut report = work(&callback, log.as_ref());
    if let Some(log) = &mut log {
        if let Err(message) = log.end(&report) {
            report =
                crate::json::failure(crate::json::CONVERT, crate::json::input(input), &message);
        }
    }
    drop(log); // Windows cannot rename an open log; observers have returned with work.
    if owns_log {
        if let Some(path) = &options.diagnostics {
            finish_default_log(
                path,
                if report.get("ok") == Some(&Value::Bool(true)) {
                    "converted"
                } else {
                    "failed"
                },
            );
        }
    }
    report
}

/// Version notice for a selected existing log. Read only the bounded first row.
pub fn version_notice(path: &Path) -> Option<String> {
    use std::io::{BufRead, Read};
    let mut row = String::new();
    std::io::BufReader::new(File::open(path).ok()?)
        .take(4 << 20)
        .read_line(&mut row)
        .ok()?;
    let first: Value = serde_json::from_str(&row).ok()?;
    let version = first.get("dutchman")?.as_str()?;
    // A version is a label, never arbitrary file text in a UI or terminal.
    if version.is_empty()
        || version.len() > 100
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
    {
        return None;
    }
    (version != crate::BUILD_VERSION).then(|| {
        format!(
            "This log is from Dutchman {version}; this is {}.",
            crate::BUILD_VERSION
        )
    })
}

fn members(input: &Path, salt: &anon::Salt) -> Option<Vec<Value>> {
    if !input
        .extension()?
        .to_str()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("f3d") || ext.eq_ignore_ascii_case("f3z"))
    {
        return None;
    }
    let mut archive = choices::bounded_archive(File::open(input).ok()?)?;
    let mut budget = 64_u64 << 20;
    let mut members = Vec::new();
    for index in 0..archive.len() {
        let mut member = archive.by_index(index).ok()?;
        let name = member.name().ok()?.into_owned();
        let nested = if name.to_ascii_lowercase().ends_with(".f3d") && member.size() <= budget {
            let size = member.size();
            budget = budget.saturating_sub(size);
            choices::nested_content_size(&mut member, size)
        } else {
            None
        };
        members.push(
            json!({"id": salt.member_id(&name), "compressed_bytes": member.compressed_size(),
            "uncompressed_bytes": member.size(), "nested_content_bytes": nested}),
        );
    }
    Some(members)
}

fn os_version() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .map(|s| s.trim().to_owned())
    }
    #[cfg(target_os = "macos")]
    {
        command_value("/usr/bin/sw_vers", &["-productVersion"])
    }
    #[cfg(windows)]
    {
        Some(std::env::consts::OS.to_owned())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    None
}

#[cfg(target_os = "macos")]
fn command_value(program: &str, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(program);
    command.args(args).stdin(std::process::Stdio::null());
    let output = command.output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn total_memory() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/meminfo")
            .ok()?
            .lines()
            .find_map(|line| {
                line.strip_prefix("MemTotal:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()?
                    .checked_mul(1024)
            })
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let mut system = sysinfo::System::new();
        system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());
        Some(system.total_memory())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    None
}

fn clock_ticks() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        static TICKS: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
        *TICKS.get_or_init(|| {
            let output = std::process::Command::new("getconf")
                .arg("CLK_TCK")
                .output()
                .ok()?;
            let hz = std::str::from_utf8(&output.stdout)
                .ok()?
                .trim()
                .parse()
                .ok()?;
            (hz > 0).then_some(hz)
        })
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        // sysinfo reports accumulated process CPU time in milliseconds.
        Some(1000)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    None
}

fn resources(pid: u32) -> (Option<u64>, Option<u64>) {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok();
        let rss = status.as_deref().and_then(|s| {
            s.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()?
                    .checked_mul(1024)
            })
        });
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok();
        let ticks = stat.as_deref().and_then(|s| {
            // comm (field 2) can contain spaces and parentheses. Fields after its last ')' start at 3.
            let mut fields = s.rsplit_once(')')?.1.split_whitespace();
            let user: u64 = fields.nth(11)?.parse().ok()?;
            let system: u64 = fields.next()?.parse().ok()?;
            user.checked_add(system)
        });
        (rss, ticks)
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let pid = sysinfo::Pid::from_u32(pid);
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing()
                .with_memory()
                .with_cpu(),
        );
        system.process(pid).map_or((None, None), |process| {
            (Some(process.memory()), Some(process.accumulated_cpu_time()))
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = pid;
        (None, None)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn lines(path: &Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn selected_log_version_notice_is_advisory_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("selected.jsonl");
        std::fs::write(&path, "{\"dutchman\":\"0.0.1\"}\nnot a row\n").unwrap();
        assert_eq!(
            version_notice(&path),
            Some(format!(
                "This log is from Dutchman 0.0.1; this is {}.",
                crate::BUILD_VERSION
            ))
        );
        std::fs::write(&path, json!({"dutchman": crate::BUILD_VERSION}).to_string()).unwrap();
        assert!(version_notice(&path).is_none());
        for row in ["", "not JSON", "{\"dutchman\":\"bad\\ntext\"}"] {
            std::fs::write(&path, row).unwrap();
            assert!(version_notice(&path).is_none());
        }
    }

    #[test]
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    fn unavailable_resources_are_null() {
        let (rss, ticks) = resources(std::process::id());
        assert_eq!(
            json!({"rss_bytes": rss, "cpu_ticks": ticks, "cpu_ms": ticks.zip(clock_ticks())}),
            json!({"rss_bytes": null, "cpu_ticks": null, "cpu_ms": null})
        );
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn resources_are_plausible() {
        let deadline = Instant::now() + Duration::from_millis(50);
        while Instant::now() < deadline {
            std::hint::black_box(42_u64.wrapping_mul(42));
        }
        let (rss, ticks) = resources(std::process::id());
        let rss = rss.unwrap();
        assert!(rss > 0 && rss < 1_000_000_000_000_000);
        let cpu_ms = ticks.unwrap() * 1000 / clock_ticks().unwrap();
        assert!(cpu_ms > 0 && cpu_ms < 1_000_000_000_000_000);
        let total = total_memory().unwrap();
        assert!(total > 0 && total < 1_000_000_000_000_000);
    }

    #[test]
    fn failed_end_and_phase_records_allow_only_anonymous_categories() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let mut log = Log::open(
            &path,
            Path::new("PrivateDesign.f3z"),
            &ConvertOptions::default(),
        )
        .unwrap();
        log.phase_event("container_prepare", MemberEvent::Start, 0, 396);
        log.phase_event("container_prepare", MemberEvent::Finish, 1945, 396);
        log.phase_event("PrivatePhase", MemberEvent::Finish, 1, 2);
        let error = dutchman_core::Error::Checks {
            message: "PrivateName /private/path 123.456".into(),
            facts: dutchman_core::CheckFacts::default(),
        };
        let mut report = crate::json::error(crate::json::CONVERT, json!({}), &error);
        report["error"]["findings"] = json!({"parameter_domain": 88, "PrivateName": 3});
        report["error"]["curve_kinds"] = json!({"nurbs": 88, "PrivateCurve": 2});
        log.end(&report).unwrap();
        let records = lines(&path);
        assert_eq!(records[1]["phase"], "container_prepare");
        assert_eq!(records[2]["ms"], 1945);
        assert_eq!(records[2]["bytes"], 396);
        let end = records.last().unwrap();
        assert_eq!(
            end["error"],
            json!({"stage": "project", "kind": "cadmpeg_checks",
            "findings": {"parameter_domain": 88}, "curve_kinds": {"nurbs": 88}})
        );
        let text = std::fs::read_to_string(path).unwrap();
        for private in ["Private", "/private/path", "123.456"] {
            assert!(!text.contains(private));
        }
        assert_eq!(anonymous_failure(&json!({"ok": true})), Value::Null);
        assert_eq!(
            anonymous_failure(
                &json!({"ok": false, "error": {"kind": "PrivateKind", "stage": "PrivateStage"}})
            )["kind"],
            "other"
        );
    }

    #[test]
    fn member_classification_is_anonymous_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let salt = anon::Salt::parse("00112233445566778899aabbccddeeff").unwrap();
        let options = ConvertOptions {
            diagnostics_salt: Some(salt.clone()),
            ..Default::default()
        };
        let log = Log::open(&path, Path::new("PrivateAssembly.f3z"), &options).unwrap();
        let member = "/private/SecretMember.f3d";
        log.observer().member_classified(member, 19, 11, 5, 3);
        log.member_event(member, MemberEvent::Start, 0, None);
        // Exercise the boundary without writing 100,000 records.
        log.sink.state.lock().unwrap().member_classifications = 99_999;
        log.member_classified(member, 20, 12, 5, 3);
        log.member_classified("/private/Other.f3d", 21, 13, 5, 3);
        drop(log);
        let records = lines(&path);
        let classified = &records[1];
        assert_eq!(classified["type"], "member_classified");
        assert_eq!(classified["member"], salt.member_id(member));
        assert_eq!(classified["member"], records[2]["member"]);
        assert_eq!(classified["ms"], 19);
        assert_eq!(classified["nested_scan_ms"], 11);
        assert_eq!(classified["layers_ms"], 5);
        assert_eq!(classified["metadata_ms"], 3);
        assert_eq!(records.len(), 4);
        assert_eq!(records[3]["ms"], 20);
        let text = std::fs::read_to_string(path).unwrap();
        for secret in ["PrivateAssembly", "/private", "SecretMember", "Other.f3d"] {
            assert!(!text.contains(secret));
        }
    }

    #[test]
    fn members_redecodes_failures_and_identical_losses_have_no_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let options = ConvertOptions {
            diagnostics_salt: Some(anon::Salt::parse("00112233445566778899aabbccddeeff").unwrap()),
            ..Default::default()
        };
        let mut log = Log::open(&path, Path::new("ZebraWidget.f3z"), &options).unwrap();
        for event in [
            MemberEvent::Start,
            MemberEvent::Fail,
            MemberEvent::Start,
            MemberEvent::Finish,
        ] {
            log.member_event("SecretMember.f3d", event, 7, Some("malformed"));
        }
        log.member_event(
            "SecretMember.f3d",
            MemberEvent::Fail,
            8,
            Some("SecretMessage"),
        );
        log.end(&json!({"ok": false, "losses": [
            {"code": "f3d/unknown-feature", "message": "ZebraWidget secret"},
            {"code": "f3d/unknown-feature", "message": "ZebraWidget secret"},
            {"code": "f3d/unknown-feature", "message": "other secret"}
        ]}))
        .unwrap();
        let records = lines(&path);
        let member = &records[1]["member"];
        assert_eq!(
            member,
            &json!(options
                .diagnostics_salt
                .unwrap()
                .member_id("SecretMember.f3d"))
        );
        let end = records.last().unwrap();
        assert_eq!(end["member_decodes"][member.as_str().unwrap()], 2);
        assert_eq!(end["redecoded"], 1);
        assert_eq!(end["repeated_losses"], 1);
        assert_eq!(end["losses"]["f3d/unknown-feature"], 3);
        assert_eq!(records[5]["code"], "other");
        let text = std::fs::read_to_string(path).unwrap();
        for secret in ["ZebraWidget", "SecretMember", "SecretMessage", "secret"] {
            assert!(!text.contains(secret));
        }
    }

    #[test]
    fn dropped_log_has_heartbeat_and_no_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let log = Log::open_every(
            &path,
            Path::new("fixture.f3d"),
            &ConvertOptions::default(),
            Duration::from_millis(5),
        )
        .unwrap();
        log.member_event("Private.f3d", MemberEvent::Start, 0, None);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if lines(&path).iter().any(|line| line["type"] == "heartbeat") {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(log);
        let records = lines(&path);
        assert_eq!(records[0]["type"], "start");
        let heartbeat = records
            .iter()
            .find(|line| line["type"] == "heartbeat")
            .unwrap();
        assert_eq!(heartbeat["member"], records[1]["member"]);
        assert!(records.iter().all(|line| line["type"] != "end"));
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        {
            assert!(heartbeat["rss_bytes"].as_u64().unwrap() > 0);
            assert!(heartbeat["cpu_ms"].as_u64().is_some());
        }
    }
}
