//! Local progress wording, kept together for a future message catalog.
use crate::{ProgressEvent, RunningStep, StepKind};

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

pub fn running(step: &RunningStep) -> String {
    let name = step.name.as_deref().unwrap_or(match step.kind {
        StepKind::Opening => "input",
        StepKind::Reading => "design",
        StepKind::Preparing => "design",
        StepKind::Checking
        | StepKind::CheckingOriginal
        | StepKind::CheckingHistory
        | StepKind::CheckingBlends
        | StepKind::CheckingPrefix
        | StepKind::CheckingGeometry => "design",
        StepKind::Writing => "output",
        StepKind::Projecting => "decoded model",
    });
    if let (Some(limit), Some(consequence)) = (step.limit_ms, &step.consequence) {
        if step.elapsed_ms >= limit {
            return format!(
                "{name} passed the {} {} limit: {consequence}",
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
        StepKind::Opening => "Opening",
        StepKind::Reading => "Reading",
        StepKind::Preparing => "Preparing",
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
        StepKind::Projecting if step.name.is_some() => format!("decoded model for {name}"),
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
        "the check is stopped; imported geometry is kept with any retained history"
    } else if hidden {
        "the check is stopped; history is kept unverified and hidden beside the imported geometry"
    } else {
        "the check is stopped; history is kept unverified"
    }
}
pub fn prefix_limit() -> &'static str {
    "the check is stopped; retained history is unverified and imported geometry stays visible"
}
pub fn blend_limit() -> &'static str {
    "the check is stopped; the unproven feature is left out of the editable history and imported geometry is kept"
}
pub fn geometry_limit() -> &'static str {
    "the check is stopped; converted geometry is kept unchecked"
}
pub fn headless_limit(save: bool) -> &'static str {
    if save {
        "conversion is stopped; no output document is written"
    } else {
        "the check is stopped; the macro is kept unverified"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(running(&step), "Bracket passed the 120 s (2 min 0 s) check limit: the check is stopped; history is kept unverified and hidden beside the imported geometry");
    }
}
