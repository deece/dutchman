//! The synthetic case: a small SolidCraft command script, on invented dimensions and generic
//! names, that fails the way the design's first SolidCraft refusal does
//! ([`crate::repro::synth`]). It is made only when the invented case really fails the same
//! way; otherwise the summary says why there is none.
//!
//! The steps: find the first feature SolidCraft refused; find the part that holds it and its
//! place in that part's timeline; convert that part alone, cut just after the feature and not
//! checked, so the feature stays in; have `solidcraft-cli script` export that design as a
//! command script; then reduce and re-dimension it under the privacy gate. Every file but
//! `synthetic/` stays in the private folder.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use dutchman_core::{parts, Format, Model, Report};
use serde_json::{json, Value};

use crate::repro::synth;
use crate::ConvertOptions;

/// The longest the whole synthetic step may take, by default.
pub const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(20 * 60);

/// Where SolidCraft's export of a design with components is tracked.
pub const EXPORT_ISSUE: &str = "https://github.com/tangency-labs/solidcraft/issues/6562";

/// What the synthetic step came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Synthetic {
    /// The case's folder, when one was made.
    pub folder: Option<PathBuf>,
    /// How many steps the case has.
    pub steps: Option<usize>,
    /// Why none was made: fixed wording, safe to share.
    pub reason: Option<&'static str>,
    /// How long the step took, in whole seconds.
    pub seconds: u64,
}

impl Synthetic {
    /// Whether a case was made.
    pub fn made(&self) -> bool {
        self.folder.is_some()
    }

    fn not(reason: &'static str, started: Instant) -> Self {
        Synthetic {
            reason: Some(reason),
            seconds: started.elapsed().as_secs(),
            ..Synthetic::default()
        }
    }

    /// The local record of it, for the folder's state and the result.
    pub fn json(&self) -> Value {
        json!({
            "made": self.made(),
            "steps": self.steps,
            "reason": self.reason,
            "seconds": self.seconds,
        })
    }

    /// The record read back.
    pub fn from_json(value: &Value) -> Option<Self> {
        let reason = value.get("reason").and_then(Value::as_str);
        Some(Synthetic {
            folder: None,
            steps: value
                .get("steps")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok()),
            reason: reason.and_then(|r| REASONS.iter().copied().find(|known| *known == r)),
            seconds: value.get("seconds")?.as_u64()?,
        })
    }
}

const NOTHING_REFUSED: &str =
    "nothing SolidCraft refused, so there is no SolidCraft failure to reproduce";
const NO_SOLIDCRAFT: &str = "SolidCraft was not found to check a case in";
const NOT_READ: &str = "the design could not be read again to find the refused feature";
const NOT_FOUND: &str = "the refused feature could not be found in the design's parts";
const NOT_CONVERTED: &str = "the part with the refused feature did not convert on its own";
const COMPONENTS: &str = "SolidCraft can't yet export a design with components as a command script";
const NOT_EXPORTED: &str = "SolidCraft could not export the design as a command script";
const NO_FAILURE: &str = "the exported script does not fail in SolidCraft the same way";
const NO_MATCH: &str = "no invented case failed the same way within the limits";
const TOO_LONG: &str = "the failure needs more steps than a shareable case may have";
const TIMED_OUT: &str = "the time for the synthetic case ran out";
const BROKE: &str = "SolidCraft stopped while the case was being tried";

/// Every reason, so a saved one is only ever one of them.
const REASONS: [&str; 12] = [
    NOTHING_REFUSED,
    NO_SOLIDCRAFT,
    NOT_READ,
    NOT_FOUND,
    NOT_CONVERTED,
    COMPONENTS,
    NOT_EXPORTED,
    NO_FAILURE,
    NO_MATCH,
    TOO_LONG,
    TIMED_OUT,
    BROKE,
];

/// The first feature SolidCraft refused, in timeline order: its name and ordinal.
pub fn first_refused(report: &Value) -> Option<(String, u64)> {
    report
        .pointer("/history/features")
        .and_then(Value::as_array)?
        .iter()
        .filter(|f| f.get("source").and_then(Value::as_str) == Some("solidcraft"))
        .filter_map(|f| {
            Some((
                f.get("name")?.as_str()?.to_owned(),
                f.get("ordinal")?.as_u64()?,
            ))
        })
        .min_by_key(|(_, ordinal)| *ordinal)
}

/// The part (by number) holding the feature called `name` at `ordinal`, and that feature's
/// place in the part's history as `--stop-at-feature` counts it.
pub fn locate(model: &Model, name: &str, ordinal: u64) -> Option<(usize, usize)> {
    for part in parts::catalog(model) {
        if part.assembly && part.bodies == 0 {
            continue;
        }
        let Ok(alone) = parts::select(model, part.number, false) else {
            continue;
        };
        let length = parts::history_length(&alone);
        for number in 1..=length {
            let feature = parts::feature_position(&alone, number)
                .and_then(|at| alone.ir.model.features.get(at));
            if feature.is_some_and(|f| f.ordinal == ordinal && f.name.as_deref() == Some(name)) {
                return Some((part.number, number));
            }
        }
    }
    None
}

/// Makes the synthetic case for `input` into `out/synthetic`, or says why there is none.
pub fn make(
    input: &Path,
    out: &Path,
    report: &Value,
    options: &ConvertOptions,
    budget: Duration,
    on_progress: &(dyn Fn(&str) + Sync),
) -> Synthetic {
    let started = Instant::now();
    let Some((name, ordinal)) = first_refused(report) else {
        return Synthetic::not(NOTHING_REFUSED, started);
    };
    let verification = match &options.solidcraft {
        Some(path) => dutchman_solidcraft::Verification::With(path.clone()),
        None => dutchman_solidcraft::Verification::Auto,
    };
    let Ok(cli) = dutchman_solidcraft::find_cli(&verification) else {
        return Synthetic::not(NO_SOLIDCRAFT, started);
    };
    on_progress("making a synthetic case of the first SolidCraft refusal");
    let Some(format) = Format::from_path(input) else {
        return Synthetic::not(NOT_READ, started);
    };
    let located = match dutchman_core::read_in(
        format,
        input,
        &options.part_search(),
        &mut Report::default(),
    ) {
        Ok(model) => locate(&model, &name, ordinal),
        Err(_) => return Synthetic::not(NOT_READ, started),
    };
    let Some((part, stop)) = located else {
        return Synthetic::not(NOT_FOUND, started);
    };
    let work = out.join("synthetic-work");
    if std::fs::create_dir_all(&work).is_err() {
        return Synthetic::not(NOT_CONVERTED, started);
    }
    let trial = work.join("trial.sc");
    let _ = std::fs::remove_file(&trial);
    let cut = ConvertOptions {
        no_verify: true,
        part: Some(dutchman_core::PartSelection {
            number: part,
            children: false,
            stop_at_feature: Some(stop),
        }),
        ..options.clone()
    };
    let converted = crate::convert_options_with_progress(
        input,
        &trial,
        &cut,
        true,
        &cut.part_search(),
        &|_| {},
    );
    if converted.get("ok") != Some(&Value::Bool(true)) {
        return Synthetic::not(NOT_CONVERTED, started);
    }
    let script_path = work.join("private-script.json");
    let _ = std::fs::remove_file(&script_path);
    let exported = Command::new(&cli)
        .arg("script")
        .arg(&trial)
        .arg("--out")
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let script = match exported {
        Ok(result) if result.status.success() => std::fs::read(&script_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()),
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Synthetic::not(
                if stderr.contains("with components") {
                    COMPONENTS
                } else {
                    NOT_EXPORTED
                },
                started,
            );
        }
        Err(_) => None,
    };
    let Some(script) = script else {
        return Synthetic::not(NOT_EXPORTED, started);
    };
    let runner = synth::CliRunner::at(cli);
    let left = budget.saturating_sub(started.elapsed());
    synthesize(&runner, &script, out, left, started)
}

/// The rest of [`make`], from the private script on, with any runner.
pub fn synthesize(
    runner: &dyn synth::Runner,
    script: &Value,
    out: &Path,
    budget: Duration,
    started: Instant,
) -> Synthetic {
    let fail_step = match runner.run(script) {
        Ok(synth::Run::Failed { step, .. }) => step,
        Ok(_) => return Synthetic::not(NO_FAILURE, started),
        Err(_) => return Synthetic::not(BROKE, started),
    };
    let mut seed = [0u8; 8];
    let _ = getrandom::fill(&mut seed);
    let options = synth::Options {
        fail_step,
        trials: synth::TRIALS,
        max_steps: synth::MAX_STEPS,
        seed: u64::from_le_bytes(seed),
        budget,
    };
    match synth::synthesize(runner, script, &options) {
        Ok(Ok(case)) => match synth::write(out, &case) {
            Ok(folder) => Synthetic {
                folder: Some(folder),
                steps: Some(case.steps()),
                reason: None,
                seconds: started.elapsed().as_secs(),
            },
            Err(_) => Synthetic::not(BROKE, started),
        },
        Ok(Err(synth::Outcome::NoFailure(_))) => Synthetic::not(NO_FAILURE, started),
        Ok(Err(synth::Outcome::NoMatch(why))) => Synthetic::not(
            if why.contains("time") {
                TIMED_OUT
            } else if why.contains("limit of") {
                TOO_LONG
            } else {
                NO_MATCH
            },
            started,
        ),
        Ok(Err(synth::Outcome::Written(_))) | Err(_) => Synthetic::not(BROKE, started),
    }
}

/// The files of a made case, as the sent bundle holds them: (role, bytes).
pub fn files(folder: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    ["case.json", "recipe.md", "manifest.json"]
        .iter()
        .map(|name| {
            std::fs::read(folder.join(name))
                .map(|bytes| (format!("synthetic/{name}"), bytes))
                .map_err(|e| format!("cannot read the synthetic case: {e}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fails at a fillet whose radius is larger than the box it rounds.
    struct Fake;
    impl synth::Runner for Fake {
        fn version(&self) -> Result<String, String> {
            Ok("solidcraft-cli 9.9.9".to_owned())
        }
        fn run(&self, script: &Value) -> Result<synth::Run, String> {
            let list = script.as_array().unwrap();
            let mut size = None;
            for (i, c) in list.iter().enumerate() {
                match c["command"].as_str().unwrap() {
                    "solid.box" => size = c["params"]["length"].as_f64(),
                    "solid.fillet" => {
                        let r = c["params"]["radius"].as_f64().unwrap();
                        if size.is_some_and(|s| r > s) {
                            return Ok(synth::Run::Failed {
                                step: i + 1,
                                command: "solid.fillet".to_owned(),
                                message: format!("the blend {r} is too large"),
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(synth::Run::Passed)
        }
    }

    #[test]
    fn the_first_refusal_is_the_earliest_in_the_timeline() {
        let report = json!({"history": {"features": [
            {"name": "Fillet 4", "ordinal": 9, "source": "solidcraft"},
            {"name": "Extrude 1", "ordinal": 2},
            {"name": "Chamfer 2", "ordinal": 5, "source": "solidcraft"},
        ]}});
        assert_eq!(first_refused(&report), Some(("Chamfer 2".to_owned(), 5)));
        assert_eq!(first_refused(&json!({})), None);
    }

    #[test]
    fn a_case_is_made_only_when_it_fails_the_same_way() {
        let dir = tempfile::tempdir().unwrap();
        let script = json!([
            {"command": "solid.box", "params": {"length": 13.7, "name": "Secret Block"}},
            {"command": "solid.fillet", "params": {"radius": 21.3, "body": "Secret Block"}},
        ]);
        let made = synthesize(
            &Fake,
            &script,
            dir.path(),
            Duration::from_secs(60),
            Instant::now(),
        );
        assert!(made.made(), "{made:?}");
        assert_eq!(made.steps, Some(2));
        let case = std::fs::read_to_string(dir.path().join("synthetic").join("case.json")).unwrap();
        assert!(!case.contains("Secret") && !case.contains("13.7") && !case.contains("21.3"));
        assert_eq!(files(&dir.path().join("synthetic")).unwrap().len(), 3);

        let passes = json!([{"command": "solid.box", "params": {"length": 5.5}}]);
        let none = synthesize(
            &Fake,
            &passes,
            dir.path(),
            Duration::from_secs(60),
            Instant::now(),
        );
        assert_eq!(none.reason, Some(NO_FAILURE));
        assert!(!none.made());
    }

    #[test]
    fn a_saved_record_reads_back_with_its_fixed_reason_only() {
        let record = Synthetic {
            reason: Some(COMPONENTS),
            seconds: 4,
            ..Synthetic::default()
        };
        assert_eq!(Synthetic::from_json(&record.json()), Some(record));
        let tampered = json!({"made": false, "reason": "a name", "seconds": 1});
        assert_eq!(Synthetic::from_json(&tampered).unwrap().reason, None);
    }
}
