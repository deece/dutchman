//! Local progress wording, kept together for a future message catalog.
use crate::{ProgressEvent, RunningStep, Stage, StepKind};

pub fn counts(read: u64, total: Option<u64>) -> String {
    match total {
        Some(total) => format!("{read} of {total} designs read"),
        None => format!("{read} designs read"),
    }
}

pub fn total_elapsed(ms: u64) -> String {
    format!("{} elapsed", crate::elapsed(ms / 1000))
}

pub fn summary(event: &ProgressEvent) -> String {
    let mut text = event.running_line();
    if event.designs_total.is_some() || event.designs_read > 0 {
        text.push_str(&format!(
            "; {}",
            counts(event.designs_read, event.designs_total)
        ));
    }
    text.push_str(&format!("; {}", total_elapsed(event.elapsed_ms)));
    text
}

/// A stage in plain words: reading, building history, checking or writing.
pub fn stage(stage: Stage) -> &'static str {
    match stage {
        Stage::Input | Stage::Decode => "reading",
        Stage::Project | Stage::SolidCraft => "checking",
        Stage::Map => "building history",
        Stage::Write => "writing",
    }
}

/// The oldest active file stays visible even when its card is offscreen.
pub fn running_file(name: &str, event: &ProgressEvent, others: usize) -> String {
    let mut text = format!("{name}: {}", stage(event.stage));
    if others > 0 {
        text.push_str(&format!(
            " ({others} other file{} running)",
            if others == 1 { "" } else { "s" }
        ));
    }
    text.push_str(&format!("\n{}", event.running_line()));
    text
}

pub fn running(step: &RunningStep) -> String {
    let name = step.name.as_deref().unwrap_or(match step.kind {
        StepKind::Opening => "the file",
        StepKind::Writing => "the output",
        _ => "the design",
    });
    if let (Some(limit), Some(consequence)) = (step.limit_ms, &step.consequence) {
        if step.elapsed_ms >= limit {
            // A part's own name stays as the file has it; a stand-in starts the sentence.
            let mut subject = name.to_owned();
            if step.name.is_none() {
                if let Some(first) = subject.get(..1) {
                    let upper = first.to_uppercase();
                    subject.replace_range(..1, &upper);
                }
            }
            return format!(
                "{subject} took longer than the {} {} limit: {consequence}.",
                crate::elapsed(limit / 1000),
                if step.kind.is_check() {
                    "check"
                } else {
                    step.kind.id()
                }
            );
        }
    }
    let verb = match step.kind {
        StepKind::Opening | StepKind::Reading => "Reading",
        StepKind::Preparing => "Building history for",
        StepKind::Checking
        | StepKind::CheckingOriginal
        | StepKind::CheckingHistory
        | StepKind::CheckingBlends
        | StepKind::CheckingPrefix
        | StepKind::CheckingGeometry => "Checking",
        StepKind::Writing => "Writing",
        StepKind::Projecting => "Checking",
    };
    let verb = if step.elapsed_ms > step.usual_ms {
        format!("Still {}", verb.to_lowercase())
    } else {
        verb.to_owned()
    };
    let target = match step.kind {
        StepKind::CheckingOriginal => format!("{name} (original geometry)"),
        StepKind::CheckingHistory => format!("{name} (rebuilt history)"),
        StepKind::CheckingBlends => format!("{name} (blend proof)"),
        StepKind::CheckingPrefix => format!("{name} (retained history)"),
        StepKind::CheckingGeometry => format!("{name} (converted geometry)"),
        _ => name.to_owned(),
    };
    format!(
        "{verb} {target}: {}",
        crate::elapsed(step.elapsed_ms / 1000)
    )
}

/// Consequences follow the writer's history retention policy.
pub fn history_limit(pattern: bool, hidden: bool) -> &'static str {
    if pattern {
        "the check stopped, and the imported geometry is kept with any retained history"
    } else if hidden {
        "the check stopped, and the history is kept unverified and hidden beside the imported geometry"
    } else {
        "the check stopped, and the history is kept unverified"
    }
}
pub fn prefix_limit() -> &'static str {
    "the check stopped, and the retained history is kept unverified while the imported geometry stays visible"
}
pub fn blend_limit() -> &'static str {
    "the check stopped, and the unproven feature was left out of the editable history while the imported geometry is kept"
}
pub fn geometry_limit() -> &'static str {
    "the check stopped, and the converted geometry is kept unchecked"
}
pub fn headless_limit(save: bool) -> &'static str {
    if save {
        "conversion stopped, and no output document was written"
    } else {
        "the check stopped, and the macro is kept unverified"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_check_limits_state_the_actual_outcome_and_end_in_a_full_stop() {
        for (consequence, expected) in [
            (history_limit(false, false), "the check stopped, and the history is kept unverified"),
            (history_limit(false, true), "the check stopped, and the history is kept unverified and hidden beside the imported geometry"),
            (history_limit(true, false), "the check stopped, and the imported geometry is kept with any retained history"),
            (prefix_limit(), "the check stopped, and the retained history is kept unverified while the imported geometry stays visible"),
            (blend_limit(), "the check stopped, and the unproven feature was left out of the editable history while the imported geometry is kept"),
            (geometry_limit(), "the check stopped, and the converted geometry is kept unchecked"),
            (headless_limit(false), "the check stopped, and the macro is kept unverified"),
            (headless_limit(true), "conversion stopped, and no output document was written"),
        ] {
            let step = RunningStep {
                kind: StepKind::Checking,
                name: Some("Bracket".into()),
                elapsed_ms: 121_000,
                usual_ms: 60_000,
                limit_ms: Some(120_000),
                consequence: Some(consequence.into()),
            };
            assert_eq!(running(&step), format!("Bracket took longer than the 2 min check limit: {expected}."));
        }
    }

    #[test]
    fn cli_summary_uses_the_same_single_duration_format_as_the_step() {
        let mut event = ProgressEvent {
            stage: Stage::Map,
            elapsed_ms: 61_000,
            designs_read: 261,
            designs_total: Some(264),
            heartbeat: true,
            running: Some(RunningStep {
                kind: StepKind::Checking,
                name: Some("Bracket".into()),
                elapsed_ms: 61_000,
                usual_ms: 60_000,
                limit_ms: None,
                consequence: None,
            }),
        };
        assert_eq!(
            event.to_string(),
            "Still checking Bracket: 1 min 1 s; 261 of 264 designs read; 1 min 1 s elapsed"
        );
        event.elapsed_ms = 120_000;
        assert_eq!(total_elapsed(event.elapsed_ms), "2 min elapsed");
        event.elapsed_ms = 3_900_000;
        assert_eq!(total_elapsed(event.elapsed_ms), "1 h 5 min elapsed");
    }

    #[test]
    fn usual_notice_is_strict_and_limit_notice_matches_timeout_boundary() {
        let mut step = RunningStep {
            kind: StepKind::Checking,
            name: Some("Bracket".into()),
            elapsed_ms: 12_000,
            usual_ms: 60_000,
            limit_ms: Some(120_000),
            consequence: Some(history_limit(false, true).into()),
        };
        assert_eq!(running(&step), "Checking Bracket: 12 s");
        step.elapsed_ms = 60_000;
        assert!(!running(&step).starts_with("Still"));
        step.elapsed_ms = 60_001;
        assert!(running(&step).starts_with("Still checking Bracket:"));
        step.elapsed_ms = 119_999;
        assert!(running(&step).starts_with("Still checking"));
        step.elapsed_ms = 120_000;
        assert_eq!(running(&step), "Bracket took longer than the 2 min check limit: the check stopped, and the history is kept unverified and hidden beside the imported geometry.");
    }
}
