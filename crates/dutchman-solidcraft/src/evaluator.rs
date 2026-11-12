//! Host boundary for evaluating designs. Parsing and proof policy stay in the writer.
use std::time::Duration;

use serde_json::Value;

/// Evaluates isolated SolidCraft design archives without changing the user's open design.
///
/// Results use SolidCraft's existing eval and engine-command JSON schemas, unchanged.
/// `Err` means no answer (for example a timeout); an evaluated refusal is returned as
/// `{"evaluation_failed": "reason"}`, and timeline errors remain in the result JSON.
/// Implementations must respect `timeout`, bound output sizes, and isolate concurrent calls.
/// `label` identifies the request for diagnostics; it is not a path supplied by the user.
/// Hosts that serialize engine access may use a lock. On wasm calls run serially.
pub trait SolidCraftEvaluator: std::fmt::Debug + Send + Sync {
    /// Open and evaluate the archive, returning the complete eval JSON: bodies with
    /// volume, area and bounding box, and timeline errors and warnings. This combines
    /// inspect.measure with document.inspect, just as CLI eval does.
    fn evaluate(&self, design: &[u8], timeout: Duration, label: &str) -> Result<Value, String>;

    /// Open the archive and execute ordered engine commands, returning one raw result
    /// per command, including failures. The writer requests timeline.roll_to,
    /// document.inspect, model.edges and model.faces. State persists within this call
    /// only; each call starts from the supplied archive.
    fn run(
        &self,
        design: &[u8],
        commands: &[Value],
        timeout: Duration,
        label: &str,
    ) -> Result<Vec<Value>, String>;
}

/// The native SolidCraft CLI transport, retaining launch retries, deadlines and memory watch.
#[cfg(not(target_family = "wasm"))]
#[derive(Clone, Debug)]
pub struct CliEvaluator {
    path: std::path::PathBuf,
}

#[cfg(not(target_family = "wasm"))]
impl CliEvaluator {
    /// Use an already selected CLI. Discovery/version checks are owned by the writer.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[cfg(not(target_family = "wasm"))]
impl SolidCraftEvaluator for CliEvaluator {
    fn evaluate(&self, design: &[u8], timeout: Duration, label: &str) -> Result<Value, String> {
        let dir = tempfile::tempdir().map_err(|err| format!("no scratch folder: {err}"))?;
        crate::verify::evaluate_json(&self.path, timeout, dir.path(), label, design)
    }

    fn run(
        &self,
        design: &[u8],
        commands: &[Value],
        timeout: Duration,
        label: &str,
    ) -> Result<Vec<Value>, String> {
        let deadline = std::time::Instant::now() + timeout;
        let dir = tempfile::tempdir().map_err(|err| format!("no scratch folder: {err}"))?;
        let input = dir.path().join("history.solidcraft");
        std::fs::write(&input, design).map_err(|err| format!("cannot stage the design: {err}"))?;
        crate::verify::run_script(&self.path, deadline, dir.path(), label, &input, commands)
    }
}

// Keep the existing CLI-focused tests calling the shared proof functions.
#[cfg(all(test, not(target_family = "wasm")))]
impl SolidCraftEvaluator for std::path::Path {
    fn evaluate(&self, design: &[u8], timeout: Duration, label: &str) -> Result<Value, String> {
        CliEvaluator::new(self).evaluate(design, timeout, label)
    }

    fn run(
        &self,
        design: &[u8],
        commands: &[Value],
        timeout: Duration,
        label: &str,
    ) -> Result<Vec<Value>, String> {
        CliEvaluator::new(self).run(design, commands, timeout, label)
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
impl SolidCraftEvaluator for std::path::PathBuf {
    fn evaluate(&self, design: &[u8], timeout: Duration, label: &str) -> Result<Value, String> {
        self.as_path().evaluate(design, timeout, label)
    }
    fn run(
        &self,
        design: &[u8],
        commands: &[Value],
        timeout: Duration,
        label: &str,
    ) -> Result<Vec<Value>, String> {
        self.as_path().run(design, commands, timeout, label)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug)]
    struct Host;
    impl SolidCraftEvaluator for Host {
        fn evaluate(&self, _: &[u8], _: Duration, _: &str) -> Result<Value, String> {
            Ok(json!({"evaluation_failed": "host refused the design"}))
        }
        fn run(
            &self,
            _: &[u8],
            commands: &[Value],
            _: Duration,
            _: &str,
        ) -> Result<Vec<Value>, String> {
            commands
                .iter()
                .map(|command| {
                    Ok(match command["command"].as_str().unwrap() {
                        "timeline.roll_to" => json!({}),
                        "document.inspect" => json!({"bodies": [{"name": "Cube"}], "timeline": []}),
                        "model.edges" => json!({"body": "Cube", "edges": [{"name": "a|b",
                    "start": [0, 0, 0], "end": [0, 0, 10], "mid": [0, 0, 5], "length": 10}]}),
                        "model.faces" => json!({"faces": [{"name": "a", "area": 100,
                    "plane_normal": [1, 0, 0], "point": [0, 5, 5]}]}),
                        _ => return Err("unknown command".to_owned()),
                    })
                })
                .collect()
        }
    }

    #[test]
    fn host_command_results_use_the_shared_edge_and_face_parsers() {
        let host: &dyn SolidCraftEvaluator = &Host;
        let steps =
            crate::verify::replay_edges(host, DEFAULT_TIMEOUT, b"design", &[2, 4], true).unwrap();
        assert_eq!(steps.len(), 2);
        for step in steps {
            let step = step.unwrap();
            assert_eq!(step.bodies[0].0, "Cube");
            assert_eq!(step.bodies[0].1[0].length, 10.0);
            assert_eq!(step.faces[0][0].normal, Some([1.0, 0.0, 0.0]));
            assert_eq!(step.faces[0][0].area, 100.0);
        }
        assert!(
            matches!(crate::verify::check_import(host, DEFAULT_TIMEOUT, b"design",
            crate::verify::Expected::default()), crate::verify::Shown::Short(why) if why.contains("host refused"))
        );
    }

    use crate::DEFAULT_TIMEOUT;
}
