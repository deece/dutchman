//! Local, checkpointed folder checks using the shared project planner and isolated workers.
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Mutex};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::batch::{self, Event};
use crate::diagnose::{
    self,
    signature::{ErrorCode, Signature},
};

/// Anonymous report schema.
pub const SCHEMA: &str = "dutchman.doctor/1";

/// A folder's CPU and total converter memory budgets.
#[derive(Clone, Debug)]
pub struct Settings {
    /// Maximum concurrent designs; also capped by CPU availability.
    pub jobs: Option<NonZeroUsize>,
    /// Total memory shared by the converter workers, in bytes.
    pub memory: u64,
    /// Per-part limits, matching export and conversion choices.
    pub diagnose: diagnose::Settings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            jobs: None,
            memory: diagnose::default_memory_limit().unwrap_or(8 << 30),
            diagnose: diagnose::Settings::default(),
        }
    }
}

/// Stable sibling output, so invoking doctor again resumes the same folder.
pub fn default_output(input: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(input).unwrap_or_else(|_| input.to_path_buf());
    let name = if resolved.is_dir() {
        resolved.file_name()
    } else {
        resolved.file_stem()
    }
    .unwrap_or_default()
    .to_string_lossy();
    resolved.with_file_name(format!("{name} (Doctor)"))
}

/// Check a local folder or project ZIP. Only `report.json` is anonymous; all other output
/// stays private. No network operations occur. Reports are advisory until confirmed.
pub fn run(
    input: &Path,
    out: &Path,
    settings: &Settings,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Value, String> {
    if !(input.is_dir() || batch::extension_of(input) == "zip") {
        return Err("Choose a folder or project ZIP to check.".into());
    }
    if settings.memory < (256 << 20) {
        return Err("The memory budget must be at least 256 MiB.".into());
    }
    if settings.diagnose.target_part == Some(0) {
        return Err("Part numbers start at 1.".into());
    }
    if input.is_dir()
        && resolve_output(out)?
            .starts_with(std::fs::canonicalize(input).map_err(|e| e.to_string())?)
    {
        return Err("Choose an output folder outside the folder being checked.".into());
    }
    dutchman_serve::hardening::prepare_data_dir(out)
        .map_err(|e| format!("Cannot prepare the output folder: {e}"))?;
    let mut plan = batch::plan(&[input.to_path_buf()], &out.join("converted"));
    if settings.diagnose.step_path.is_some() && plan.jobs.len() != 1 {
        return Err("--step needs a project with one design; put same-name STEP exports beside other designs.".into());
    }
    batch::prepare(&mut plan.jobs, &settings.diagnose.options);
    let workers = concurrency(settings.jobs, settings.memory);
    let per_worker = settings.memory / workers as u64;
    let mut diagnose = settings.diagnose.clone();
    diagnose.memory_limit = Some(per_worker);
    // Avoid nested CPU oversubscription inside each child.
    diagnose.options.threads = 1;
    let program = diagnose
        .program
        .clone()
        .map_or_else(std::env::current_exe, Ok)
        .map_err(|e| format!("Cannot find the converter: {e}"))?;
    let checkpoints = out.join("private-checkpoints");
    dutchman_serve::hardening::prepare_data_dir(&checkpoints).map_err(|e| e.to_string())?;
    // Include the entire project inventory so a changed referenced part invalidates assembly
    // checkpoints too. ZIP identities use the archive, never temporary extraction paths.
    let inventory = inventory(input)?;
    let reports = Mutex::new(BTreeMap::new());
    batch::run_with(
        &plan.jobs,
        workers,
        &AtomicBool::new(false),
        |job, index| {
            let mut settings = diagnose.clone();
            settings.options.project = job.search.project.clone().or(settings.options.project);
            settings.options.search.extend(job.search.also.clone());
            settings.step_path = settings
                .step_path
                .or_else(|| job.fallback.clone())
                .or_else(|| diagnose::step::matching(&job.input));
            let mut digest = Sha256::new();
            digest.update(&inventory);
            match dependencies(job, &settings.options) {
                Some(dependencies) => digest.update(dependencies),
                // Never reuse an incomplete inventory. Keep the random private checkpoint
                // folder so an interrupted run still leaves useful diagnostic output.
                None => digest.update(format!("{:?}", std::time::SystemTime::now())),
            }
            digest.update(file_identity(&program));
            digest.update(crate::anon::CADMPEG_VERSION);
            if job.project_files.is_none() {
                digest.update(format!(
                    "{:?}",
                    crate::worker::search_args(&settings.options.part_search())
                ));
            }
            digest.update(format!(
                "{}:{index}:{:?}:{:?}:{:?}:{:?}",
                env!("CARGO_PKG_VERSION"),
                settings.options.to_args(),
                settings.time_limit,
                settings.memory_limit,
                settings.target_part
            ));
            if let Some(step) = &settings.step_path {
                if job.project_files.is_none() || diagnose.step_path.is_some() {
                    digest.update(file_identity(step));
                }
            }
            let hash: String = digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let folder = checkpoints.join(hash);
            let saved = folder.join("complete.json");
            if let Ok(bytes) = std::fs::read(&saved) {
                if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                    if value.pointer("/conversion/ok") != Some(&json!(true)) || job.output.is_file()
                    {
                        return value;
                    }
                }
            }
            let mut native = job.clone();
            // A fallback opening successfully must never hide the native failure being checked.
            native.fallback = None;
            if std::fs::create_dir_all(&folder).is_err() {
                return json!({"ok": false, "check_error": "checkpoint_write_failed"});
            }
            let conversion_file = folder.join("conversion.json");
            let cached = std::fs::read(&conversion_file)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .filter(|report| {
                    report.is_object()
                        && (report.get("ok") != Some(&json!(true)) || job.output.is_file())
                });
            let conversion = cached.unwrap_or_else(|| {
                batch::convert_isolated_job(
                    &program,
                    &native,
                    &settings.options,
                    &crate::worker::Limits {
                        timeout: settings.time_limit.saturating_mul(6),
                        max_memory: Some(per_worker),
                        design: Some(crate::worker::DEFAULT_DESIGN_LIMIT),
                    },
                    None,
                )
            });
            if serde_json::to_vec(&conversion)
                .ok()
                .is_none_or(|bytes| diagnose::write_file(&conversion_file, &bytes, true).is_err())
            {
                return json!({"ok": false, "check_error": "checkpoint_write_failed", "conversion": conversion});
            }
            if conversion.get("ok") == Some(&json!(true))
                && settings.options.to == crate::OutputFormat::Solidcraft
                && conversion.get("mode").and_then(Value::as_str) != Some("partial_history")
            {
                settings.converted_path = Some(job.output.clone());
            }
            let ranking = diagnose::run_resuming(&job.input, &folder, &settings, &|_| {});
            let result = match ranking {
                Ok(ranking) => {
                    json!({"ok": true, "conversion": conversion, "diagnose": ranking.get("report"), "bundle": ranking.get("bundle"), "checkpoint": folder})
                }
                Err(_) => {
                    json!({"ok": false, "conversion": conversion, "check_error": "checkpoint_write_failed"})
                }
            };
            if result.get("ok") == Some(&json!(true)) {
                if let Ok(bytes) = serde_json::to_vec(&result) {
                    if diagnose::write_file(&saved, &bytes, true).is_err() {
                        return json!({"ok": false, "check_error": "checkpoint_write_failed", "conversion": conversion});
                    }
                }
            }
            result
        },
        |event| {
            if let Event::Finished(index, report) = event {
                if let Ok(mut reports) = reports.lock() {
                    reports.insert(index, report);
                    progress(reports.len(), plan.jobs.len());
                }
            }
        },
    );
    let reports = reports
        .into_inner()
        .map_err(|_| "Folder checks stopped unexpectedly.".to_owned())?;
    let report = summarize(&reports, plan.problems.len(), workers, settings.memory);
    diagnose::write_file(
        &out.join("report.json"),
        &serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
        false,
    )?;
    let private = json!({"warning": "Keep this file private.", "input": input,
        "files": plan.jobs.iter().enumerate().map(|(i,job)| json!({"file": i+1, "path": job.input,
            "checkpoint": reports.get(&i).and_then(|r| r.get("checkpoint")),
            "matching_step": settings.diagnose.step_path.clone().or_else(|| job.fallback.clone())
                .or_else(|| diagnose::step::matching(&job.input))})).collect::<Vec<_>>(),
        "planning_problems": plan.problems});
    diagnose::write_file(
        &out.join("local-key.json"),
        &serde_json::to_vec_pretty(&private).map_err(|e| e.to_string())?,
        true,
    )?;
    Ok(report)
}

fn resolve_output(path: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(path).map_err(|e| e.to_string())?;
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        if component == std::path::Component::ParentDir {
            resolved.pop();
        } else if component != std::path::Component::CurDir {
            resolved.push(component.as_os_str());
        }
        if resolved.exists() {
            resolved = std::fs::canonicalize(&resolved).map_err(|e| e.to_string())?;
        }
    }
    Ok(resolved)
}

fn concurrency(jobs: Option<NonZeroUsize>, memory: u64) -> usize {
    let cpu = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    batch::workers(jobs)
        .min(cpu)
        .min(usize::try_from(memory / (256 << 20)).unwrap_or(usize::MAX))
        .max(1)
}

fn dependencies(job: &batch::Job, options: &crate::ConvertOptions) -> Option<Vec<u8>> {
    let mut pending = vec![job.input.clone()];
    let mut seen = std::collections::BTreeSet::new();
    let mut entries = Vec::new();
    while let Some(input) = pending.pop() {
        if !seen.insert(input.clone()) {
            continue;
        }
        if seen.len() > 10_000 {
            return None;
        }
        let references = dutchman_core::assembly_references(&input, &options.part_search()).ok()?;
        for path in references {
            // The archive identity already covers extracted members; temporary paths must
            // not invalidate ZIP checkpoints. External libraries still need fingerprints.
            if !job
                .project_files
                .as_ref()
                .is_some_and(|root| path.starts_with(root.path()))
            {
                entries.push(file_identity(&path));
            }
            pending.push(path);
            if pending.len() >= 10_000 {
                return None;
            }
        }
    }
    entries.sort();
    Some(entries.concat())
}

fn file_identity(path: &Path) -> Vec<u8> {
    let metadata = std::fs::metadata(path).ok();
    format!(
        "{}:{:?}:{:?}",
        path.display(),
        metadata.as_ref().map(|m| m.len()),
        metadata.and_then(|m| m.modified().ok())
    )
    .into_bytes()
}

fn inventory(input: &Path) -> Result<Vec<u8>, String> {
    let mut pending = vec![input.to_path_buf()];
    let mut entries = Vec::new();
    while let Some(path) = pending.pop() {
        if entries.len() >= dutchman_core::MAX_SEARCHED {
            return Err("Choose a smaller folder; too many entries to checkpoint.".into());
        }
        let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if meta.is_symlink() {
            continue;
        }
        entries.push(file_identity(&path));
        if meta.is_dir() {
            for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
                pending.push(entry.map_err(|e| e.to_string())?.path());
                if pending.len() >= dutchman_core::MAX_SEARCHED {
                    return Err("Choose a smaller folder; too many entries to checkpoint.".into());
                }
            }
        }
    }
    entries.sort();
    Ok(entries.concat())
}

fn summarize(
    reports: &BTreeMap<usize, Value>,
    planning_failed: usize,
    workers: usize,
    memory: u64,
) -> Value {
    let mut groups: BTreeMap<String, Value> = BTreeMap::new();
    let mut files = Vec::new();
    for (index, result) in reports {
        let mut add = |part: Option<u64>, error: ErrorCode| {
            let signature = Signature::new(error, None);
            let group = groups
                .entry(signature.hash.clone())
                .or_insert_with(|| json!({"signature": signature, "count": 0, "occurrences": []}));
            let count = group.get("count").and_then(Value::as_u64).unwrap_or(0);
            if let Some(value) = group.get_mut("count") {
                *value = json!(count + 1);
            }
            if let Some(rows) = group.get_mut("occurrences").and_then(Value::as_array_mut) {
                rows.push(json!({"file": index+1, "part": part}));
            }
        };
        let conversion = result.get("conversion").unwrap_or(&Value::Null);
        if conversion.get("ok") != Some(&json!(true)) {
            let error = conversion.get("error").unwrap_or(&Value::Null);
            add(
                None,
                ErrorCode {
                    outcome: "failed".into(),
                    stage: error
                        .get("stage")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    kind: error.get("kind").and_then(Value::as_str).map(str::to_owned),
                    codes: vec![],
                },
            );
        }
        // Only counts enter the anonymous report; left-out reasons and file names stay private.
        let completeness = conversion.get("completeness").map(|counts| {
            let n = |key: &str| counts.get(key).and_then(Value::as_u64).unwrap_or(0);
            let left_out = counts
                .get("left_out")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .fold(0u64, |total, row| {
                    total.saturating_add(row.get("count").and_then(Value::as_u64).unwrap_or(0))
                });
            json!({"placed": n("placed"), "placed_empty": n("placed_empty"),
                "placements": n("placements"), "left_out": left_out})
        });
        if completeness
            .as_ref()
            .and_then(|c| c.get("placed_empty"))
            .and_then(Value::as_u64)
            .is_some_and(|n| n > 0)
        {
            add(
                None,
                ErrorCode {
                    outcome: "degraded".into(),
                    stage: Some("map".into()),
                    kind: Some("no_geometry".into()),
                    codes: vec!["empty_placements".into()],
                },
            );
        }
        if conversion
            .get("losses")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|loss| {
                loss.get("code").and_then(Value::as_str)
                    == Some("step/topology.region-solid-reduced-to-sheet")
            })
        {
            add(
                None,
                ErrorCode {
                    outcome: "degraded".into(),
                    stage: Some("write".into()),
                    kind: Some("geometry".into()),
                    codes: vec!["solid_reduced_to_sheet".into()],
                },
            );
        }
        let diagnose = result.get("diagnose").unwrap_or(&Value::Null);
        if conversion
            .pointer("/verification/status")
            .and_then(Value::as_str)
            == Some("differs")
        {
            add(
                None,
                ErrorCode {
                    outcome: "failed".into(),
                    stage: Some("check".into()),
                    kind: Some("looks_wrong".into()),
                    codes: vec!["history_replay".into()],
                },
            );
        }
        for row in diagnose
            .pointer("/placement_checks/findings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            add(
                row.get("part").and_then(Value::as_u64),
                ErrorCode {
                    outcome: "failed".into(),
                    stage: Some("check".into()),
                    kind: Some("looks_wrong".into()),
                    codes: row
                        .get("code")
                        .and_then(Value::as_str)
                        .map(|c| vec![c.to_owned()])
                        .unwrap_or_default(),
                },
            );
        }

        for row in diagnose
            .get("parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let outcome = row
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("not_run");
            if matches!(outcome, "failed" | "timed_out" | "out_of_memory") {
                add(
                    row.get("part").and_then(Value::as_u64),
                    ErrorCode {
                        outcome: outcome.into(),
                        stage: row.get("stage").and_then(Value::as_str).map(str::to_owned),
                        kind: row
                            .get("error_kind")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        codes: row
                            .get("findings")
                            .and_then(Value::as_object)
                            .map(|m| m.keys().cloned().collect())
                            .unwrap_or_default(),
                    },
                );
            }
        }
        for row in ["/step_comparison/parts", "/geometry_checks/parts"]
            .into_iter()
            .flat_map(|pointer| {
                diagnose
                    .pointer(pointer)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
        {
            if row.get("looks_wrong") == Some(&json!(true)) {
                add(
                    None,
                    ErrorCode {
                        outcome: "failed".into(),
                        stage: Some("check".into()),
                        kind: Some("looks_wrong".into()),
                        codes: row
                            .get("findings")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect(),
                    },
                );
            }
        }
        files.push(json!({"file": index+1, "converted": conversion.get("ok") == Some(&json!(true)),
            "completeness": completeness,
            "check_status": result.get("check_error").cloned().unwrap_or_else(|| diagnose.get("status").cloned().unwrap_or(json!("unavailable"))),
            "step_comparison": diagnose.get("step_comparison"), "target_part": diagnose.get("target_part"),
            "placement_checks": diagnose.get("placement_checks"), "geometry_checks": diagnose.get("geometry_checks"), "history_replay": conversion.pointer("/verification/status")}));
    }
    json!({"schema": SCHEMA, "ok": planning_failed == 0 && reports.values().all(|r| r.get("ok") == Some(&json!(true))),
        "advisory": true, "files": files, "planning_failed": planning_failed,
        "budget": {"jobs": workers, "memory_bytes": memory, "per_worker_bytes": memory / workers as u64},
        "findings": groups.into_values().collect::<Vec<_>>(),
        "summary": "Folder checks finished. Review findings before making a report. Only report.json is anonymous; keep all other output private. Nothing was sent."})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_failures_are_one_anonymous_finding() {
        let result = json!({"ok": true, "conversion": {"ok": false, "error": {"kind": "decode", "stage": "decode", "message": "private name"}}});
        let reports = (0..100).map(|i| (i, result.clone())).collect();
        let report = summarize(&reports, 0, 2, 1 << 30);
        assert_eq!(report["findings"].as_array().unwrap().len(), 1);
        assert_eq!(report["findings"][0]["count"], 100);
        assert!(!report.to_string().contains("private name"));
    }
    #[test]
    fn successful_conversion_keeps_empty_counts_and_sheet_losses_in_doctor() {
        let result = json!({"ok": true, "conversion": {"ok": true, "completeness": {
            "placed": 67, "placed_empty": 13, "placements": 88,
            "left_out": [{"file": "SyntheticPrivateName.ipt", "reason": "SyntheticPrivateReason", "count": 8}]
        }, "losses": [{"code": "step/topology.region-solid-reduced-to-sheet"}]}});
        let report = summarize(&BTreeMap::from([(0, result)]), 0, 1, 1 << 30);
        assert_eq!(
            report["files"][0]["completeness"],
            json!({"placed": 67, "placed_empty": 13, "placements": 88, "left_out": 8})
        );
        assert_eq!(report["findings"].as_array().unwrap().len(), 2);
        assert_eq!(
            report["findings"][0]["signature"]["error"]["outcome"],
            "degraded"
        );
        assert!(!report.to_string().contains("SyntheticPrivate"));
    }

    #[test]
    fn budget_never_oversubscribes_memory() {
        assert_eq!(concurrency(NonZeroUsize::new(100), 256 << 20), 1);
    }
    #[test]
    fn changed_project_inventory_invalidates_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("part.f3d");
        std::fs::write(&part, b"synthetic").unwrap();
        let before = inventory(dir.path()).unwrap();
        std::fs::write(part, b"synthetic changed").unwrap();
        assert_ne!(before, inventory(dir.path()).unwrap());
    }
}
