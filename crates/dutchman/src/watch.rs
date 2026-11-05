//! Polling inbox conversion with durable content identities and protected output ownership.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime};

use clap::Args;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::{batch, worker, ConvertOptions};

/// One line of watcher output.
pub const SCHEMA: &str = "dutchman.watch-event/1";
const STATE: &str = ".dutchman-watch.json";
const STATE_SCHEMA: &str = "dutchman.watch-state/1";

/// Command-line settings for the inbox service.
#[derive(Args)]
pub struct Command {
    /// Inbox to search recursively. Inputs are only read.
    pub inbox: PathBuf,
    /// Destination folder, outside the inbox.
    #[arg(long)]
    pub out_dir: PathBuf,
    /// Seconds between scans.
    #[arg(long, default_value = "5")]
    pub interval: std::num::NonZeroU64,
    /// Seconds size and modification time must stay unchanged.
    #[arg(long, default_value_t = 10)]
    pub settle: u64,
    /// Scan once, convert ready files, and exit.
    #[arg(long)]
    pub once: bool,
    /// Maximum simultaneous conversions (default: cores, at most 8).
    #[arg(long)]
    pub jobs: Option<std::num::NonZeroUsize>,
    /// Seconds allowed per conversion.
    #[arg(long, default_value_t = worker::DEFAULT_TIMEOUT.as_secs())]
    pub timeout: u64,
    /// Megabytes allowed per conversion (0: unlimited).
    #[arg(long, default_value_t = worker::DEFAULT_MAX_MEMORY_MB)]
    pub max_memory: u64,
    #[command(flatten)]
    pub options: ConvertOptions,
    /// Print one JSON event per line.
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: SystemTime,
}

fn stamp(path: &Path) -> Result<Stamp, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("input is no longer a regular file".into());
    }
    Ok(Stamp {
        size: meta.len(),
        modified: meta.modified().map_err(|e| e.to_string())?,
    })
}

fn hash(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if let Some(bytes) = buffer.get(..n) {
            digest.update(bytes);
        }
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn ignored(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        let lower = name.to_ascii_lowercase();
        name.starts_with('.')
            || name.starts_with("~$")
            || [".part", ".tmp", ".crdownload"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
    })
}

fn atomic_json(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path.parent().ok_or("destination has no parent folder")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut temporary, value).map_err(|e| e.to_string())?;
    temporary.flush().map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Runs until a signal requests shutdown, or until the single scan has finished.
pub fn run(command: Command) -> Result<bool, String> {
    run_with_progress(command, true)
}

/// Runs with conversion progress on stderr, unless `quiet` is set.
pub fn run_with_progress(mut command: Command, quiet: bool) -> Result<bool, String> {
    let owns_log = command.options.diagnostics_default && command.options.diagnostics.is_none();
    crate::diagnostics::prepare(&mut command.options, batch::workers(command.jobs))?;
    let path = owns_log
        .then(|| command.options.diagnostics.clone())
        .flatten();
    let mut stopped = false;
    let result = run_prepared(command, quiet, &mut stopped);
    if let Some(path) = path {
        crate::diagnostics::finish_default_log(
            &path,
            if stopped {
                "stopped"
            } else if result.as_ref().is_ok_and(|ok| *ok) {
                "converted"
            } else {
                "failed"
            },
        );
    }
    result
}

fn run_prepared(command: Command, quiet: bool, stopped: &mut bool) -> Result<bool, String> {
    let progress = Arc::new(crate::progress::StderrProgress::new(
        quiet,
        command.json,
        false,
    ));
    if command.options.to == crate::OutputFormat::Dxf {
        return Err(
            "DXF page output uses dutchman convert --to dxf; batch and watch are not supported yet"
                .into(),
        );
    }
    let stop = Arc::new(AtomicBool::new(false));
    let signal = Arc::clone(&stop);
    ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed))
        .map_err(|e| format!("cannot listen for shutdown: {e}"))?;
    let inbox = std::fs::canonicalize(&command.inbox).map_err(|e| e.to_string())?;
    if !inbox.is_dir() {
        return Err("choose an inbox folder".into());
    }
    std::fs::read_dir(&inbox).map_err(|e| format!("cannot scan inbox: {e}"))?;
    let out = destination_path(&command.out_dir)?;
    if out.starts_with(&inbox) || inbox.starts_with(&out) {
        return Err("choose separate inbox and output folders, neither inside the other".into());
    }
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let state_path = out.join(STATE);
    let mut state: Value = match std::fs::File::open(&state_path) {
        Ok(file) => serde_json::from_reader(file.take(64 << 20))
            .map_err(|e| format!("cannot read watch state (limit 64 MB): {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            json!({"schema": STATE_SCHEMA, "inbox": inbox.to_string_lossy(), "files": {}, "owned": {}})
        }
        Err(e) => return Err(format!("cannot read watch state: {e}")),
    };
    if state.get("schema") != Some(&json!(STATE_SCHEMA))
        || state.get("inbox") != Some(&json!(inbox.to_string_lossy()))
        || !state.get("files").is_some_and(Value::is_object)
        || !state.get("owned").is_some_and(Value::is_object)
    {
        return Err("watch state does not match this inbox; choose another output folder".into());
    }
    let result = run_loop(
        &command,
        &inbox,
        &out,
        &state_path,
        &mut state,
        Arc::clone(&stop),
        &progress,
    );
    result.map(|(ok, was_stopped)| {
        *stopped = was_stopped;
        ok
    })
}

struct Pending {
    job: batch::Job,
    relative: PathBuf,
    digest: String,
    stamp: Stamp,
    started: Instant,
}

struct Finished {
    pending: Pending,
    stage: tempfile::TempDir,
    report: Value,
}

fn run_loop(
    command: &Command,
    inbox: &Path,
    out: &Path,
    state_path: &Path,
    state: &mut Value,
    stop: Arc<AtomicBool>,
    progress: &Arc<crate::progress::StderrProgress>,
) -> Result<(bool, bool), String> {
    let settings = batch::Settings {
        jobs: command.jobs,
        limits: worker::Limits {
            timeout: Duration::from_secs(command.timeout.max(1)),
            max_memory: (command.max_memory > 0)
                .then_some(command.max_memory.saturating_mul(1 << 20)),
        },
        ..batch::Settings::isolated()?
    };
    let program = settings
        .worker
        .as_ref()
        .ok_or("cannot find converter worker")?;
    let (sender, receiver) = mpsc::channel::<Finished>();
    let mut queued = std::collections::VecDeque::new();
    let mut running = 0usize;
    let mut children = Children {
        stop: Arc::clone(&stop),
        threads: Vec::new(),
    };
    let mut active = BTreeSet::new();
    let mut active_outputs = BTreeSet::new();
    let mut observed: BTreeMap<PathBuf, (Stamp, Instant)> = BTreeMap::new();
    let mut next_scan = Instant::now();
    let mut scanned = false;
    let mut failed = false;
    loop {
        while let Ok(finished) = receiver.try_recv() {
            let Finished {
                pending,
                stage,
                mut report,
            } = finished;
            active.remove(&pending.relative);
            active_outputs.remove(&pending.job.output);
            running = running.saturating_sub(1);
            // The next scan settles a changed upload again; never record a hash for other bytes.
            if stamp(&pending.job.input).ok().as_ref() != Some(&pending.stamp)
                || hash(&pending.job.input).ok().as_ref() != Some(&pending.digest)
            {
                observed.remove(&pending.relative);
                emit(
                    command,
                    "skipped",
                    &pending.relative,
                    pending.started,
                    "input changed during conversion",
                    None,
                )?;
                continue;
            }
            let output = &pending.job.output;
            let success = report.get("ok") == Some(&json!(true));
            let sidecar = output.with_extension(if success {
                "report.json"
            } else {
                "failed.json"
            });
            let destinations = if success {
                vec![output.clone(), sidecar.clone()]
            } else {
                vec![sidecar.clone()]
            };
            let blocked = destinations.iter().any(|path| {
                !owned_or_absent(state, out, path, &pending.relative)
                    || command.options.no_clobber && path.exists()
            });
            let outcome = if blocked {
                "skipped"
            } else {
                if success {
                    if let Some(object) = report.get_mut("output").and_then(Value::as_object_mut) {
                        object.insert("path".into(), json!(output.to_string_lossy()));
                    }
                    publish(
                        &stage
                            .path()
                            .join("result")
                            .with_extension(output.extension().unwrap_or_default()),
                        output,
                        state,
                        out,
                        &pending.relative,
                    )?;
                }
                let staged_report = stage.path().join("report.json");
                atomic_json(&staged_report, &report)?;
                publish(&staged_report, &sidecar, state, out, &pending.relative)?;
                // Remove an obsolete sidecar only when its bytes still belong to us.
                let obsolete = output.with_extension(if success {
                    "failed.json"
                } else {
                    "report.json"
                });
                if obsolete.exists() && owned_or_absent(state, out, &obsolete, &pending.relative) {
                    std::fs::remove_file(&obsolete).map_err(|e| e.to_string())?;
                    if let Some(owned) = state.get_mut("owned").and_then(Value::as_object_mut) {
                        owned.remove(&key(out, &obsolete)?);
                    }
                }
                if success {
                    "converted"
                } else {
                    failed = true;
                    "failed"
                }
            };
            let relative = pending.relative.to_string_lossy().into_owned();
            if let Some(files) = state.get_mut("files").and_then(Value::as_object_mut) {
                let entry = files.entry(relative).or_insert_with(|| json!({}));
                if let Some(hashes) = entry.as_object_mut() {
                    hashes.insert(pending.digest, json!({"outcome": outcome}));
                }
            }
            atomic_json(state_path, state)?;
            let message = if blocked {
                "destination already exists; choose another output folder"
            } else {
                report
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            };
            emit(
                command,
                outcome,
                &pending.relative,
                pending.started,
                message,
                Some(&report),
            )?;
        }
        let stopping = stop.load(Ordering::Relaxed);
        if (stopping || command.once && scanned) && active.is_empty() {
            atomic_json(state_path, state)?;
            return Ok((!failed, stopping));
        }
        if !stopping && (!command.once || !scanned) && Instant::now() >= next_scan {
            let mut plan = batch::plan_filtered(&[inbox.to_path_buf()], out, |path| !ignored(path));
            batch::prepare(&mut plan.jobs, &command.options);
            let mut seen = BTreeSet::new();
            for job in plan.jobs {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let relative = job
                    .input
                    .strip_prefix(inbox)
                    .map_err(|e| e.to_string())?
                    .to_path_buf();
                if ignored(&relative) {
                    continue;
                }
                seen.insert(relative.clone());
                if active.contains(&relative) || active_outputs.contains(&job.output) {
                    continue;
                }
                let Ok(current) = stamp(&job.input) else {
                    continue;
                };
                let observation = observed.entry(relative.clone()).or_insert_with(|| {
                    // A copy can keep its source's modification time (Finder, `cp -p`,
                    // `rsync -t`), so a watcher waits a whole settle period from when it first
                    // sees a file. A single `--once` pass has only the file's own age to go on.
                    let since = if command.once {
                        let age = SystemTime::now()
                            .duration_since(current.modified)
                            .unwrap_or_default();
                        Instant::now()
                            .checked_sub(age.min(Duration::from_secs(command.settle)))
                            .unwrap_or_else(Instant::now)
                    } else {
                        Instant::now()
                    };
                    (current.clone(), since)
                });
                if observation.0 != current {
                    *observation = (current.clone(), Instant::now());
                }
                if current.modified > SystemTime::now()
                    || observation.1.elapsed() < Duration::from_secs(command.settle)
                    || SystemTime::now()
                        .duration_since(current.modified)
                        .unwrap_or_default()
                        < Duration::from_secs(command.settle)
                {
                    continue;
                }
                let digest = match hash(&job.input) {
                    Ok(digest) => digest,
                    Err(error) => {
                        let report = crate::json::failure(
                            crate::json::CONVERT,
                            crate::json::input(&job.input),
                            &error,
                        );
                        emit(
                            command,
                            "failed",
                            &relative,
                            Instant::now(),
                            &error,
                            Some(&report),
                        )?;
                        failed = true;
                        continue;
                    }
                };
                if stamp(&job.input).ok().as_ref() != Some(&current) {
                    continue;
                }
                if state
                    .get("files")
                    .and_then(|v| v.get(relative.to_string_lossy().as_ref()))
                    .and_then(|v| v.get(&digest))
                    .is_some()
                {
                    continue;
                }
                active.insert(relative.clone());
                active_outputs.insert(job.output.clone());
                queued.push_back((job, relative, digest, current));
            }
            observed.retain(|path, _| seen.contains(path));
            scanned = true;
            next_scan = Instant::now()
                .checked_add(Duration::from_secs(command.interval.get()))
                .ok_or("polling interval is too large; choose fewer seconds")?;
        }
        if stopping {
            while let Some((job, relative, _, _)) = queued.pop_front() {
                active.remove(&relative);
                active_outputs.remove(&job.output);
            }
        }
        while !stop.load(Ordering::Relaxed) && running < batch::workers(settings.jobs) {
            let Some((mut job, relative, digest, current)) = queued.pop_front() else {
                break;
            };
            if stamp(&job.input).ok().as_ref() != Some(&current) {
                active.remove(&relative);
                active_outputs.remove(&job.output);
                observed.remove(&relative);
                continue;
            }
            let stage = tempfile::tempdir_in(out).map_err(|e| e.to_string())?;
            let staged = stage
                .path()
                .join("result")
                .with_extension(job.output.extension().unwrap_or_default());
            let pending = Pending {
                job: job.clone(),
                relative,
                digest,
                stamp: current,
                started: Instant::now(),
            };
            job.output = staged;
            job.replace = false;
            let program = program.clone();
            let options = command.options.clone();
            let limits = settings.limits.clone();
            let sender = sender.clone();
            let stop = Arc::clone(&stop);
            running += 1;
            children.threads.retain(|thread| !thread.is_finished());
            let progress = Arc::clone(progress);
            children.threads.push(std::thread::spawn(move || {
                let report = batch::convert_isolated_job_with_progress(
                    &program,
                    &job,
                    &options,
                    &limits,
                    Some(&stop),
                    &|event| progress.report(&job.input, event),
                );
                progress.finish();
                let _ = sender.send(Finished {
                    pending,
                    stage,
                    report,
                });
            }));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn key(out: &Path, path: &Path) -> Result<String, String> {
    path.strip_prefix(out)
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

fn owned_or_absent(state: &Value, out: &Path, path: &Path, input: &Path) -> bool {
    if !safe_parent(out, path) {
        return false;
    }
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Ok(meta) if meta.is_file() => key(out, path)
            .ok()
            .and_then(|key| {
                let owned = state.get("owned")?.get(key)?;
                if owned.get("input")?.as_str()? != owner(input) {
                    return None;
                }
                owned.get("sha256")?.as_str().map(str::to_owned)
            })
            .is_some_and(|expected| hash(path).ok().as_ref() == Some(&expected)),
        _ => false,
    }
}

fn publish(
    source: &Path,
    destination: &Path,
    state: &mut Value,
    out: &Path,
    input: &Path,
) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or("destination has no parent folder")?;
    // Reject symlinked output folders, including those pointing into the inbox.
    let mut folder = parent;
    while folder != out {
        if std::fs::symlink_metadata(folder).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err("choose an output folder without symbolic links".into());
        }
        folder = folder.parent().ok_or("destination outside output folder")?;
    }
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    if destination.try_exists().map_err(|e| e.to_string())? {
        if !owned_or_absent(state, out, destination, input) {
            return Err("destination changed during conversion".into());
        }
        std::fs::rename(source, destination).map_err(|e| e.to_string())?;
    } else if std::fs::hard_link(source, destination).is_err() {
        // Some mounted shares do not support hard links. Exclusive creation still
        // protects an unrelated file that appeared after the ownership check.
        let mut target = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|e| e.to_string())?;
        let copied = std::fs::File::open(source).and_then(|mut source| {
            std::io::copy(&mut source, &mut target)?;
            target.sync_all()
        });
        if let Err(error) = copied {
            drop(target);
            let _ = std::fs::remove_file(destination);
            return Err(error.to_string());
        }
    }
    let digest = hash(destination)?;
    if let Some(owned) = state.get_mut("owned").and_then(Value::as_object_mut) {
        owned.insert(
            key(out, destination)?,
            json!({"sha256": digest, "input": owner(input)}),
        );
    }
    Ok(())
}

fn emit(
    command: &Command,
    outcome: &str,
    path: &Path,
    started: Instant,
    message: &str,
    report: Option<&Value>,
) -> Result<(), String> {
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let event = json!({"schema": SCHEMA, "event": outcome, "path": path.to_string_lossy(), "elapsed_ms": elapsed, "message": message, "report": report});
    let mut stdout = std::io::stdout().lock();
    if command.json {
        writeln!(stdout, "{event}")
    } else {
        writeln!(
            stdout,
            "{outcome} {} ({elapsed} ms) {message}",
            one_line(&path.to_string_lossy()),
            message = one_line(message)
        )
    }
    .and_then(|()| stdout.flush())
    .map_err(|e| e.to_string())
}

fn destination_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    let mut ancestor = absolute.as_path();
    let mut tail = Vec::new();
    while !ancestor.exists() {
        tail.push(
            ancestor
                .file_name()
                .ok_or("choose an output folder")?
                .to_os_string(),
        );
        ancestor = ancestor.parent().ok_or("choose an output folder")?;
    }
    let mut resolved = std::fs::canonicalize(ancestor).map_err(|e| e.to_string())?;
    for component in tail.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

struct Children {
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Drop for Children {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn safe_parent(out: &Path, path: &Path) -> bool {
    let Some(mut folder) = path.parent() else {
        return false;
    };
    while folder != out {
        if std::fs::symlink_metadata(folder).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return false;
        }
        let Some(parent) = folder.parent() else {
            return false;
        };
        folder = parent;
    }
    true
}

fn one_line(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

fn owner(input: &Path) -> String {
    dutchman_core::Format::part_revision(input)
        .map_or_else(|| input.to_path_buf(), |(part, _)| part)
        .to_string_lossy()
        .into_owned()
}
