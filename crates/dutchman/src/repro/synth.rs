//! A shareable synthetic reproduction: from a private SolidCraft command script that fails,
//! build a small script with the same feature structure on invented dimensions that fails the
//! same way. Reduce first, then re-dimension and re-run until the failure signature matches. The
//! privacy gate is code: a case that leaks a number, a name, or is long is refused.
//!
//! `dutchman investigate` runs it on the script `solidcraft-cli script` exports from the
//! design cut just after the feature SolidCraft refused, and writes `synthetic/` only when an
//! invented case fails the same way.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

pub const MAX_STEPS: usize = 12;
pub const TRIALS: usize = 64;
const MAX_REDUCE_RUNS: usize = 200;
const RUN_TIMEOUT: Duration = Duration::from_secs(180);
const STANDARD_ANGLES: [f64; 4] = [0.0, 90.0, 180.0, 118.0];
const COUNT_KEYS: [&str; 9] = [
    "count",
    "n",
    "segments",
    "sides",
    "index",
    "instances",
    "quantity",
    "copies",
    "turns",
];
const ENUM_KEYS: [&str; 16] = [
    "operation",
    "fit",
    "form",
    "kind",
    "type",
    "mode",
    "plane",
    "axis",
    "direction",
    "side",
    "extent",
    "style",
    "hand",
    "end",
    "unit",
    "units",
];

/// What one run of a script did.
#[derive(Clone, Debug, PartialEq)]
pub enum Run {
    Passed,
    /// A command failed: its 1-based step, its name, and the message.
    Failed {
        step: usize,
        command: String,
        message: String,
    },
    /// The run broke some other way (no signature).
    Other(String),
}

/// Runs scripts. The real one starts `solidcraft-cli`; tests use a fake.
pub trait Runner {
    fn version(&self) -> Result<String, String>;
    fn run(&self, script: &Value) -> Result<Run, String>;
}

/// The real runner: `solidcraft-cli run SCRIPT`.
pub struct CliRunner {
    cli: PathBuf,
}

impl CliRunner {
    /// The runner for the CLI at `cli`.
    pub fn at(cli: PathBuf) -> Self {
        Self { cli }
    }

    pub fn find(chosen: Option<&Path>) -> Result<Self, String> {
        let verification = match chosen {
            Some(path) => dutchman_solidcraft::Verification::With(path.to_path_buf()),
            None => dutchman_solidcraft::Verification::Auto,
        };
        Ok(Self {
            cli: dutchman_solidcraft::find_cli(&verification)?,
        })
    }
}

/// A CLI that refuses to run at all (rather than failing a step) says so on its first line.
fn refused(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .next()
        .filter(|line| line.contains("refus") && !line.contains("error: step "))
        .map(|line| format!("solidcraft-cli would not run: {}", line.trim()))
}

impl Runner for CliRunner {
    fn version(&self) -> Result<String, String> {
        let out = Command::new(&self.cli)
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("cannot start solidcraft-cli: {e}"))?;
        let err = String::from_utf8_lossy(&out.stderr);
        if let Some(why) = refused(&err) {
            return Err(why);
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .map(|l| l.trim().to_owned())
            .filter(|l| !l.is_empty())
            .ok_or_else(|| "solidcraft-cli did not report a version".to_owned())
    }

    fn run(&self, script: &Value) -> Result<Run, String> {
        let dir = tempfile::tempdir().map_err(|e| format!("cannot make a working folder: {e}"))?;
        let file = dir.path().join("script.json");
        std::fs::write(&file, script.to_string()).map_err(|e| format!("cannot write: {e}"))?;
        let mut child = Command::new(&self.cli)
            .arg("run")
            .arg(&file)
            .arg("--quiet")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start solidcraft-cli: {e}"))?;
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if started.elapsed() > RUN_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(format!("cannot wait for solidcraft-cli: {e}")),
            }
        };
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = (&mut pipe).take(65536).read_to_string(&mut stderr);
        }
        let Some(status) = status else {
            return Ok(Run::Other("timed out".to_owned()));
        };
        if status.success() {
            return Ok(Run::Passed);
        }
        if let Some(why) = refused(&stderr) {
            return Err(why);
        }
        Ok(parse_failure(&stderr))
    }
}

/// `error: step 3 (solid.fillet): not supported yet: the blend is ...`
fn parse_failure(stderr: &str) -> Run {
    for line in stderr.lines() {
        let Some(rest) = line.trim().strip_prefix("error: step ") else {
            continue;
        };
        let Some((step, rest)) = rest.split_once(' ') else {
            continue;
        };
        let Ok(step) = step.parse::<usize>() else {
            continue;
        };
        let Some(rest) = rest.strip_prefix('(') else {
            continue;
        };
        let Some((command, message)) = rest.split_once("):") else {
            continue;
        };
        return Run::Failed {
            step,
            command: command.to_owned(),
            message: message.trim().to_owned(),
        };
    }
    Run::Other(stderr.lines().next().unwrap_or("failed").to_owned())
}

/// The error message with every token holding a digit (numbers, ids, names with numbers)
/// replaced, so it names the failure class and not the geometry.
pub fn message_class(message: &str) -> String {
    message
        .split_whitespace()
        .map(|word| {
            if word.chars().any(|c| c.is_ascii_digit()) {
                "#"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How a failure is recognised.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub command: String,
    pub class: String,
    pub version: String,
}

fn signature(run: &Run, version: &str) -> Option<(usize, Signature)> {
    match run {
        Run::Failed {
            step,
            command,
            message,
        } => Some((
            *step,
            Signature {
                command: command.clone(),
                class: message_class(message),
                version: version.to_owned(),
            },
        )),
        _ => None,
    }
}

/// True when `script` fails at its last step with `want`.
fn fails_like(
    runner: &dyn Runner,
    script: &[Value],
    want: &Signature,
    version: &str,
) -> Result<bool, String> {
    let run = runner.run(&Value::Array(script.to_vec()))?;
    Ok(signature(&run, version).is_some_and(|(step, sig)| step == script.len() && &sig == want))
}

// ---------- reduction ----------

/// Drop every command that is not needed to keep the failure, never the failing (last) one.
/// Returns the reduced list and the runs used.
pub fn reduce(
    runner: &dyn Runner,
    commands: &[Value],
    want: &Signature,
    version: &str,
    until: Instant,
) -> Result<(Vec<Value>, usize), String> {
    let mut current = commands.to_vec();
    let mut runs = 0;
    let mut changed = true;
    while changed && runs < MAX_REDUCE_RUNS && Instant::now() < until {
        changed = false;
        let mut i = 0;
        while i + 1 < current.len() && runs < MAX_REDUCE_RUNS && Instant::now() < until {
            let mut candidate = current.clone();
            candidate.remove(i);
            runs += 1;
            if fails_like(runner, &candidate, want, version)? {
                current = candidate;
                changed = true;
            } else {
                i += 1;
            }
        }
    }
    Ok((current, runs))
}

// ---------- re-dimensioning ----------

/// A small seeded generator (splitmix64).
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Length,
    Angle,
    Count,
}

fn kind_of(key: &str) -> Kind {
    let key = key.to_ascii_lowercase();
    if COUNT_KEYS.contains(&key.as_str()) {
        Kind::Count
    } else if key.contains("angle") || key.contains("deg") {
        Kind::Angle
    } else {
        Kind::Length
    }
}

fn is_generic_name(s: &str) -> bool {
    ["Body", "Sketch", "Feature"].iter().any(|p| {
        s.strip_prefix(p)
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    })
}

fn same_number(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// Visit every number with its kind (the nearest enclosing object key) and every string
/// with its key.
fn walk(v: &Value, key: &str, num: &mut dyn FnMut(Kind, f64), text: &mut dyn FnMut(&str, &str)) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                num(kind_of(key), f);
            }
        }
        Value::String(s) => text(key, s),
        Value::Array(items) => items.iter().for_each(|i| walk(i, key, num, text)),
        Value::Object(map) => map.iter().for_each(|(k, i)| walk(i, k, num, text)),
        _ => {}
    }
}

fn rewrite(
    v: &mut Value,
    key: &str,
    num: &mut dyn FnMut(Kind, f64) -> f64,
    text: &mut dyn FnMut(&str, &str) -> String,
) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                let kind = kind_of(key);
                let out = num(kind, f);
                if let Some(new) = serde_json::Number::from_f64(out) {
                    *v = Value::Number(new);
                }
            }
        }
        Value::String(s) => {
            let new = text(key, s);
            *s = new;
        }
        Value::Array(items) => items.iter_mut().for_each(|i| rewrite(i, key, num, text)),
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for k in keys {
                if let Some(i) = map.get_mut(&k) {
                    rewrite(i, &k, num, text);
                }
            }
        }
        _ => {}
    }
}

fn snap(v: f64, step: f64) -> f64 {
    (v / step).round() * step
}

/// Only `command` and `params` are kept from a command.
fn strip(command: &Value) -> Value {
    let mut out = Map::new();
    for key in ["command", "params"] {
        if let Some(v) = command.get(key) {
            out.insert(key.to_owned(), v.clone());
        }
    }
    Value::Object(out)
}

/// One candidate: invented dimensions, generic names. Lengths map by value through one
/// strictly increasing function (one global scale, a jitter per value, snapped to 0.5), so
/// equal lengths stay equal and every order between lengths is kept, including a fillet
/// radius against the thickness beside it. Points follow because they map the same way.
pub fn redimension(commands: &[Value], rng: &mut Rng) -> Vec<Value> {
    let mut out: Vec<Value> = commands.iter().map(strip).collect();
    let mut lengths: BTreeSet<u64> = BTreeSet::new();
    let mut names: Vec<(String, String)> = Vec::new();
    for c in &out {
        walk(
            c.get("params").unwrap_or(&Value::Null),
            "",
            &mut |kind, f| {
                if kind == Kind::Length && f != 0.0 {
                    lengths.insert(f.abs().to_bits());
                }
            },
            &mut |key, s| {
                if !ENUM_KEYS.contains(&key.to_ascii_lowercase().as_str()) {
                    names.push((key.to_ascii_lowercase(), s.to_owned()));
                }
            },
        );
    }
    let scale = rng.between(0.5, 2.0);
    let mut map: BTreeMap<u64, f64> = BTreeMap::new();
    let mut floor = 0.0;
    for bits in lengths {
        let v = f64::from_bits(bits);
        let jitter = rng.between(0.85, 1.15);
        let mut m = snap(v * scale * jitter, 0.5).max(0.5);
        if m <= floor {
            m = floor + 0.5;
        }
        floor = m;
        map.insert(bits, m);
    }
    // Names: generic ones stay, the rest become Body/Sketch/Feature numbered fresh.
    let used: BTreeSet<String> = names
        .iter()
        .filter(|(_, s)| is_generic_name(s))
        .map(|(_, s)| s.clone())
        .collect();
    let mut renamed: BTreeMap<String, String> = BTreeMap::new();
    let mut counters: BTreeMap<&'static str, usize> = BTreeMap::new();
    for (key, s) in &names {
        if is_generic_name(s) || renamed.contains_key(s) {
            continue;
        }
        let prefix = if key.contains("sketch") {
            "Sketch"
        } else if key.contains("feature") {
            "Feature"
        } else {
            "Body"
        };
        let n = counters.entry(prefix).or_insert(0);
        let name = loop {
            *n += 1;
            let candidate = format!("{prefix}{n}");
            if !used.contains(&candidate) {
                break candidate;
            }
        };
        renamed.insert(s.clone(), name);
    }
    for c in &mut out {
        if let Some(params) = c.get_mut("params") {
            rewrite(
                params,
                "",
                &mut |kind, f| match kind {
                    Kind::Count => f,
                    Kind::Length => {
                        if f == 0.0 {
                            0.0
                        } else {
                            let m = map.get(&f.abs().to_bits()).copied().unwrap_or(f);
                            m.copysign(f)
                        }
                    }
                    Kind::Angle => {
                        if STANDARD_ANGLES.iter().any(|a| same_number(f.abs(), *a)) {
                            f
                        } else {
                            let r = snap(f + rng.between(-12.0, 12.0), 5.0);
                            if same_number(r, f) {
                                r + 5.0
                            } else {
                                r
                            }
                        }
                    }
                },
                &mut |key, s| {
                    if ENUM_KEYS.contains(&key.to_ascii_lowercase().as_str()) || is_generic_name(s)
                    {
                        s.to_owned()
                    } else {
                        renamed.get(s).cloned().unwrap_or_else(|| s.to_owned())
                    }
                },
            );
        }
    }
    out
}

// ---------- privacy gate ----------

/// What a case must not repeat from the private script.
pub struct Private {
    numbers: Vec<f64>,
    strings: Vec<String>,
}

impl Private {
    pub fn of(commands: &[Value]) -> Self {
        let mut numbers = Vec::new();
        let mut strings = Vec::new();
        for c in commands {
            walk(
                c.get("params").unwrap_or(&Value::Null),
                "",
                &mut |kind, f| {
                    if kind != Kind::Count {
                        numbers.push(f);
                    }
                },
                &mut |key, s| {
                    if !ENUM_KEYS.contains(&key.to_ascii_lowercase().as_str())
                        && !is_generic_name(s)
                    {
                        strings.push(s.to_owned());
                    }
                },
            );
            // Any other string in the command object (e.g. a label) is private too.
            if let Some(map) = c.as_object() {
                for (k, v) in map {
                    if k != "command" && k != "params" {
                        walk(v, k, &mut |_, f| numbers.push(f), &mut |_, s| {
                            strings.push(s.to_owned())
                        });
                    }
                }
            }
        }
        Self { numbers, strings }
    }
}

/// Why a case may not be shared; `None` when it may.
pub fn leak(private: &Private, case: &[Value], max_steps: usize) -> Option<String> {
    if case.len() > max_steps {
        return Some(format!(
            "{} steps remain, more than the limit of {max_steps}",
            case.len()
        ));
    }
    let why: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
    let flag = |m: &str| {
        let mut w = why.borrow_mut();
        if w.is_none() {
            *w = Some(m.to_owned());
        }
    };
    for c in case {
        walk(
            c.get("params").unwrap_or(&Value::Null),
            "",
            &mut |kind, f| {
                let allowed = f == 0.0
                    || kind == Kind::Count
                    || (kind == Kind::Angle
                        && STANDARD_ANGLES.iter().any(|a| same_number(f.abs(), *a)));
                if !allowed && private.numbers.iter().any(|n| same_number(*n, f)) {
                    flag("a number from the private script survives");
                }
            },
            &mut |_, s| {
                if private.strings.iter().any(|p| p == s) {
                    flag("a name from the private script survives");
                }
            },
        );
        if c.as_object()
            .is_some_and(|m| m.keys().any(|k| k != "command" && k != "params"))
        {
            flag("an unexpected field survives");
        }
    }
    why.into_inner()
}

/// Whether free text (a recipe or manifest) repeats a private name of four letters or more.
fn text_leak(private: &Private, text: &str) -> bool {
    let lower = text.to_lowercase();
    private
        .strings
        .iter()
        .any(|s| s.chars().count() >= 4 && lower.contains(&s.to_lowercase()))
}

// ---------- the whole stage ----------

pub struct Options {
    pub fail_step: usize,
    pub trials: usize,
    pub max_steps: usize,
    pub seed: u64,
    /// The longest the whole stage may take; past it, it stops and writes nothing.
    pub budget: Duration,
}

/// A case the gate cleared; only [`synthesize`] makes one.
pub struct Shareable {
    case: Vec<Value>,
    manifest: Value,
    recipe: String,
}

impl Shareable {
    /// How many steps the case has.
    pub fn steps(&self) -> usize {
        self.case.len()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The script does not fail (or not at the named step).
    NoFailure(String),
    /// No shareable case; why.
    NoMatch(String),
    Written(PathBuf),
}

/// Build a shareable case, or say why there is none.
pub fn synthesize(
    runner: &dyn Runner,
    script: &Value,
    options: &Options,
) -> Result<Result<Shareable, Outcome>, String> {
    let commands = script
        .as_array()
        .ok_or("the script must be a JSON array of commands")?;
    if options.fail_step == 0 || options.fail_step > commands.len() {
        return Err(format!(
            "--fail-step must be between 1 and {}",
            commands.len()
        ));
    }
    let until = Instant::now() + options.budget;
    let version = runner.version()?;
    let first = runner.run(script)?;
    let Some((step, want)) = signature(&first, &version) else {
        return Ok(Err(Outcome::NoFailure(
            "the script does not fail, so there is nothing to reproduce".to_owned(),
        )));
    };
    if step != options.fail_step {
        return Ok(Err(Outcome::NoFailure(format!(
            "the script fails at step {step}, not step {}",
            options.fail_step
        ))));
    }
    let private = Private::of(commands);
    let head: Vec<Value> = commands.iter().take(step).cloned().collect();
    let (reduced, reduce_runs) = reduce(runner, &head, &want, &version, until)?;
    if reduced.len() > options.max_steps {
        return Ok(Err(Outcome::NoMatch(format!(
            "the failure needs {} steps, more than the limit of {}; \
             a case that long is too close to the original design",
            reduced.len(),
            options.max_steps
        ))));
    }
    let mut rng = Rng::new(options.seed);
    for trial in 1..=options.trials {
        if Instant::now() >= until {
            return Ok(Err(Outcome::NoMatch(
                "the time for the synthetic case ran out; nothing shareable was written".to_owned(),
            )));
        }
        let candidate = redimension(&reduced, &mut rng);
        if leak(&private, &candidate, options.max_steps).is_some() {
            continue;
        }
        if !fails_like(runner, &candidate, &want, &version)? {
            continue;
        }
        let recipe = recipe(&candidate, &want);
        let manifest = json!({
            "schema": "dutchman.repro.synthetic/1",
            "signature": {
                "step": candidate.len(),
                "command": want.command,
                "message_class": want.class,
                "solidcraft": want.version,
            },
            "dutchman": env!("CARGO_PKG_VERSION"),
            "steps_before": commands.len(),
            "steps_after_reduction": reduced.len(),
            "reduction_runs": reduce_runs,
            "trials_used": trial,
            "trial_budget": options.trials,
            "seed": options.seed,
            "privacy_gate": {"passed": true, "step_limit": options.max_steps},
        });
        if text_leak(&private, &recipe) || text_leak(&private, &manifest.to_string()) {
            continue;
        }
        return Ok(Ok(Shareable {
            case: candidate,
            manifest,
            recipe,
        }));
    }
    Ok(Err(Outcome::NoMatch(format!(
        "no invented set of dimensions kept the failure within {} tries; nothing shareable was written",
        options.trials
    ))))
}

/// Write `synthetic/` under `out`.
pub fn write(out: &Path, case: &Shareable) -> Result<PathBuf, String> {
    let dir = out.join("synthetic");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let put = |name: &str, text: String| {
        std::fs::write(dir.join(name), text).map_err(|e| format!("cannot write {name}: {e}"))
    };
    put(
        "case.json",
        serde_json::to_string_pretty(&case.case).map_err(|e| e.to_string())? + "\n",
    )?;
    put(
        "manifest.json",
        serde_json::to_string_pretty(&case.manifest).map_err(|e| e.to_string())? + "\n",
    )?;
    put("recipe.md", case.recipe.clone())?;
    Ok(dir)
}

fn words(command: &str) -> &str {
    match command {
        "solid.box" => "Box",
        "solid.cylinder" => "Cylinder",
        "solid.fillet" => "Fillet",
        "solid.chamfer" => "Chamfer",
        "solid.hole" => "Hole",
        other => other,
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::Number(n) => n.as_f64().map_or(n.to_string(), |f| format!("{f}")),
        Value::Array(items) => format!(
            "({})",
            items.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn recipe(case: &[Value], want: &Signature) -> String {
    let mut text = String::from(
        "# Synthetic case (invented dimensions, unverified in other CAD)\n\n\
         Build these steps in the source CAD, in order. Lengths are millimetres, angles degrees.\n\n",
    );
    for (i, c) in case.iter().enumerate() {
        let command = c.get("command").and_then(Value::as_str).unwrap_or("?");
        let mut parts = Vec::new();
        if let Some(Value::Object(params)) = c.get("params") {
            for (k, v) in params {
                parts.push(format!("{k} {}", show(v)));
            }
        }
        text.push_str(&format!(
            "{}. {}: {}\n",
            i + 1,
            words(command),
            parts.join("; ")
        ));
    }
    text.push_str(&format!(
        "\nIn SolidCraft {} the last step fails with: {}\n",
        want.version, want.class
    ));
    text
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use std::cell::Cell;

    /// Fails like a fillet wider than the thin joined box: the fillet at the end needs two
    /// boxes before it and a radius above the second box's length.
    struct Fake {
        runs: Cell<usize>,
    }
    impl Fake {
        fn new() -> Self {
            Self { runs: Cell::new(0) }
        }
    }
    impl Runner for Fake {
        fn version(&self) -> Result<String, String> {
            Ok("solidcraft-cli 9.9.9".to_owned())
        }
        fn run(&self, script: &Value) -> Result<Run, String> {
            self.runs.set(self.runs.get() + 1);
            let list = script.as_array().unwrap();
            let mut boxes: Vec<f64> = Vec::new();
            for (i, c) in list.iter().enumerate() {
                let name = c["command"].as_str().unwrap();
                let p = &c["params"];
                match name {
                    "solid.box" => boxes.push(p["length"].as_f64().unwrap()),
                    "solid.fillet" => {
                        let flange = boxes.get(1).copied();
                        let r = p["radius"].as_f64().unwrap();
                        return Ok(match flange {
                            None => Run::Failed {
                                step: i + 1,
                                command: name.to_owned(),
                                message: "no such body".to_owned(),
                            },
                            Some(f) if r > f => Run::Failed {
                                step: i + 1,
                                command: name.to_owned(),
                                message: format!(
                                    "the blend is larger than the neighbouring faces ({r})"
                                ),
                            },
                            Some(_) => Run::Passed,
                        });
                    }
                    _ => {}
                }
            }
            Ok(Run::Passed)
        }
    }

    fn private_script() -> Value {
        json!([
            {"command":"solid.cylinder","params":{"radius":7.77,"height":13.31,"name":"Secret Shaft"}},
            {"command":"solid.box","params":{"length":23.7,"width":21.3,"height":19.9,"corner":[0,0,0],"name":"Bracket Main"}},
            {"command":"solid.cylinder","params":{"radius":2.21,"height":9.13,"name":"Secret Pin"}},
            {"command":"solid.box","params":{"length":3.13,"width":21.3,"height":9.7,"corner":[23.7,0,0],"operation":"join","name":"Bracket Main"}},
            {"command":"solid.fillet","params":{"edges":[[23.7,10.65,9.7]],"radius":4.37,"body":"Body1"}}
        ])
    }

    fn opts() -> Options {
        Options {
            fail_step: 5,
            trials: 64,
            max_steps: MAX_STEPS,
            seed: 7,
            budget: Duration::from_secs(60),
        }
    }

    fn numbers_of(v: &Value, out: &mut Vec<f64>) {
        walk(v, "", &mut |_, f| out.push(f), &mut |_, _| {});
    }

    #[test]
    fn reduction_keeps_the_signature() {
        let fake = Fake::new();
        let script = private_script();
        let want = Signature {
            command: "solid.fillet".into(),
            class: message_class("the blend is larger than the neighbouring faces (4.37)"),
            version: "solidcraft-cli 9.9.9".into(),
        };
        let list = script.as_array().unwrap().clone();
        let until = Instant::now() + Duration::from_secs(60);
        let (reduced, _) = reduce(&fake, &list, &want, "solidcraft-cli 9.9.9", until).unwrap();
        assert_eq!(reduced.len(), 3);
        assert!(fails_like(&fake, &reduced, &want, "solidcraft-cli 9.9.9").unwrap());
        assert!(reduced.iter().all(|c| c["command"] != "solid.cylinder"));
    }

    #[test]
    fn the_whole_stage_makes_a_clean_case() {
        let fake = Fake::new();
        let case = synthesize(&fake, &private_script(), &opts())
            .unwrap()
            .ok()
            .unwrap();
        assert_eq!(case.case.len(), 3);
        let mut mine = Vec::new();
        numbers_of(&Value::Array(case.case.clone()), &mut mine);
        let mut theirs = Vec::new();
        numbers_of(&private_script(), &mut theirs);
        for n in mine {
            assert!(
                n == 0.0 || !theirs.iter().any(|t| same_number(*t, n)),
                "{n}"
            );
        }
        let text = Value::Array(case.case.clone()).to_string();
        assert!(!text.contains("Bracket") && !text.contains("Secret"));
        assert!(case.recipe.contains("Fillet"));
        assert_eq!(case.manifest["signature"]["step"], 3);
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &case).unwrap();
        for f in ["case.json", "manifest.json", "recipe.md"] {
            assert!(dir.path().join("synthetic").join(f).is_file());
        }
    }

    #[test]
    fn the_gate_refuses_leaks_and_long_cases() {
        let private = Private::of(private_script().as_array().unwrap());
        let leaked_number = vec![json!({"command":"solid.box","params":{"length":23.7}})];
        assert!(leak(&private, &leaked_number, 12)
            .unwrap()
            .contains("number"));
        let leaked_name = vec![json!({"command":"solid.box","params":{"name":"Bracket Main"}})];
        assert!(leak(&private, &leaked_name, 12).unwrap().contains("name"));
        let extra = vec![json!({"command":"solid.box","params":{},"label":"x"})];
        assert!(leak(&private, &extra, 12).is_some());
        let long: Vec<Value> = (0..13)
            .map(|_| json!({"command":"solid.box","params":{"length":1.25}}))
            .collect();
        assert!(leak(&private, &long, 12).unwrap().contains("steps"));
        let fine =
            vec![json!({"command":"solid.box","params":{"length":1.25,"angle":90,"name":"Body1"}})];
        assert_eq!(leak(&private, &fine, 12), None);
    }

    #[test]
    fn a_long_failure_is_refused() {
        let fake = Fake::new();
        let o = Options {
            max_steps: 2,
            ..opts()
        };
        let out = synthesize(&fake, &private_script(), &o).unwrap();
        assert!(matches!(out, Err(Outcome::NoMatch(m)) if m.contains("limit")));
    }

    #[test]
    fn redimension_keeps_every_order_between_lengths() {
        let reduced = reduced_case();
        for seed in 0..50 {
            let out = redimension(&reduced, &mut Rng::new(seed));
            let flange = out[1]["params"]["length"].as_f64().unwrap();
            let radius = out[2]["params"]["radius"].as_f64().unwrap();
            assert!(radius > flange, "seed {seed}: {radius} vs {flange}");
            let base = out[0]["params"]["length"].as_f64().unwrap();
            assert!(base > radius);
            // the edge pick still sits on the joined face: same value, same mapping
            assert_eq!(out[2]["params"]["edges"][0][0].as_f64().unwrap(), base);
            assert_eq!(out[1]["params"]["corner"][0].as_f64().unwrap(), base);
            assert_eq!(out[2]["params"]["body"], "Body1");
            for n in [base, flange, radius] {
                assert!((n * 2.0).fract() == 0.0, "snapped to 0.5: {n}");
            }
        }
    }

    fn reduced_case() -> Vec<Value> {
        let all = private_script();
        let l = all.as_array().unwrap();
        vec![l[1].clone(), l[3].clone(), l[4].clone()]
    }

    #[test]
    fn a_failure_that_does_not_survive_writes_nothing() {
        // The fake only fails for radius > flange; this runner fails only for the exact
        // private radius, which the gate forbids reusing.
        struct OnlyOriginal;
        impl Runner for OnlyOriginal {
            fn version(&self) -> Result<String, String> {
                Ok("v".into())
            }
            fn run(&self, script: &Value) -> Result<Run, String> {
                let hit = script.to_string().contains("4.37");
                Ok(if hit {
                    Run::Failed {
                        step: script.as_array().unwrap().len(),
                        command: "solid.fillet".into(),
                        message: "boom".into(),
                    }
                } else {
                    Run::Passed
                })
            }
        }
        let out = synthesize(&OnlyOriginal, &private_script(), &opts()).unwrap();
        assert!(matches!(out, Err(Outcome::NoMatch(_))));
    }

    #[test]
    fn a_passing_script_produces_nothing() {
        let fake = Fake::new();
        let script = json!([{"command":"solid.box","params":{"length":5.5}}]);
        let o = Options {
            fail_step: 1,
            ..opts()
        };
        let out = synthesize(&fake, &script, &o).unwrap();
        assert!(matches!(out, Err(Outcome::NoFailure(_))));
        let wrong = synthesize(
            &fake,
            &private_script(),
            &Options {
                fail_step: 2,
                ..opts()
            },
        )
        .unwrap();
        assert!(matches!(wrong, Err(Outcome::NoFailure(_))));
    }

    #[test]
    fn the_cli_failure_line_is_read() {
        let run = parse_failure("error: step 3 (solid.fillet): not supported yet: the blend is larger than the neighbouring faces\n");
        assert_eq!(
            run,
            Run::Failed {
                step: 3,
                command: "solid.fillet".into(),
                message: "not supported yet: the blend is larger than the neighbouring faces"
                    .into()
            }
        );
        assert_eq!(
            message_class("radius 4.37 at e12 too big"),
            "radius # at # too big"
        );
    }
}
