//! Checks written designs in SolidCraft's own command-line tool, since Dutchman has no kernel
//! of its own: a written history against the original geometry (evaluating both in the same
//! SolidCraft makes any kernel difference cancel out), and geometry written as an import
//! against what the model holds.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

#[path = "version_cache.rs"]
mod version_cache;

/// How long CLI version discovery may run.
pub(crate) const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// The relative difference in volume at which a replay no longer matches the original, when
/// SolidCraft gives no area and box to bound its meshes' error by (see [`mesh_slack`]).
pub const VOLUME_TOLERANCE: f64 = 1e-4;

/// SolidCraft measures a body from a mesh whose chords keep within this fraction of the body's
/// size of its faces, and never closer than [`MESH_CHORD_FLOOR`].
const MESH_CHORD: f64 = 5e-5;

/// The closest, in millimetres, SolidCraft's measuring mesh keeps to a body's faces.
const MESH_CHORD_FLOOR: f64 = 1e-4;

/// The relative difference in area at which a replay no longer matches the original. A mesh
/// misses a curved face's area by about a third of its chord over the face's radius, and
/// SolidCraft keeps that chord within 3e-4 of the smallest radius: well inside this.
const AREA_TOLERANCE: f64 = 1e-3;

/// How long one SolidCraft evaluation may run before the check gives up.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// The environment variable that names the SolidCraft command-line tool.
pub const CLI_ENV: &str = "SOLIDCRAFT_CLI";

/// The SolidCraft command-line tool's file name.
const CLI_NAME: &str = "solidcraft-cli";

/// How often a running evaluation is polled.
const POLL: Duration = Duration::from_millis(20);

/// How a written history is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verification {
    /// With the SolidCraft CLI named by `SOLIDCRAFT_CLI`, else the newest found on `PATH`
    /// or inside an installed SolidCraft app (see [`find_cli`]).
    Auto,
    /// With this SolidCraft CLI.
    With(PathBuf),
    /// Not at all.
    Off,
}

/// What the check found.
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    /// The replayed bodies match the original's.
    Matches,
    /// The replay does not match; why.
    Differs(String),
    /// The check could not be made; why.
    Unverified(String),
}

/// The SolidCraft CLI `verification` checks with: for [`Verification::Auto`], the one
/// `SOLIDCRAFT_CLI` names (when that file exists), else the newest `solidcraft-cli` found on
/// `PATH`, beside this program, or inside an installed SolidCraft app: on macOS `SolidCraft*.app`
/// in `/Applications` or `~/Applications`, newest version first; on Windows
/// `SolidCraft\solidcraft-cli.exe` in Program Files or `%LOCALAPPDATA%`, else in the install
/// folder the installer recorded in the registry (`InstallDir`, or beside the registered
/// `solidcraft.exe`). The command line, the desktop app, the server and the reports all search
/// through this one function, so they find the same CLI (see [`discover`]).
pub fn find_cli(verification: &Verification) -> Result<PathBuf, String> {
    match verification {
        Verification::Off => Err("verification turned off".to_owned()),
        Verification::With(path) => Ok(path.clone()),
        Verification::Auto => discover().cli.ok_or_else(|| {
            format!("no {CLI_NAME} found (install SolidCraft, set {CLI_ENV}, or add it to PATH)")
        }),
    }
}

/// One place the search for the SolidCraft CLI looks, and what it found there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// A generic name for the place (`SOLIDCRAFT_CLI`, `PATH`, `Program Files`), never a path.
    pub label: &'static str,
    /// `unset`, `set_but_missing`, `not_found`, `found`, or `used` (the CLI that was chosen).
    pub outcome: &'static str,
    /// The CLI file found there (or the file `SOLIDCRAFT_CLI` names), for local reports only.
    pub path: Option<PathBuf>,
}

/// Where the search looked and the CLI it chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovery {
    /// The CLI chosen, if any.
    pub cli: Option<PathBuf>,
    /// The places tried, in search order.
    pub places: Vec<Place>,
}

/// Where the search looks, as plain values, so the order can be tested without a real
/// SolidCraft installed.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sources {
    /// The value of `SOLIDCRAFT_CLI`.
    pub env_cli: Option<std::ffi::OsString>,
    /// The value of `PATH`.
    pub path: Option<std::ffi::OsString>,
    /// The folder of the running program.
    pub program_folder: Option<PathBuf>,
    /// Windows folders that may hold a `SolidCraft` folder, labelled.
    pub windows_bases: Vec<(&'static str, PathBuf)>,
    /// Folders that hold `SolidCraft*.app` bundles (macOS), labelled.
    pub app_folders: Vec<(&'static str, PathBuf)>,
    /// Whether to read the Windows registry for the install folder.
    pub registry: bool,
}

impl Sources {
    /// The places this system installs SolidCraft.
    fn system() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        let mut sources = Sources {
            env_cli: var(CLI_ENV),
            path: std::env::var_os("PATH"),
            program_folder: std::env::current_exe()
                .ok()
                .and_then(|program| program.parent().map(Path::to_path_buf)),
            ..Sources::default()
        };
        if cfg!(target_os = "macos") {
            sources
                .app_folders
                .push(("Applications", PathBuf::from("/Applications")));
            if let Some(home) = var("HOME") {
                sources
                    .app_folders
                    .push(("user Applications", Path::new(&home).join("Applications")));
            }
        } else if cfg!(windows) {
            for (label, name) in [
                ("Program Files", "ProgramFiles"),
                ("Program Files", "ProgramW6432"),
                ("Program Files (x86)", "ProgramFiles(x86)"),
                ("per-user install folder", "LOCALAPPDATA"),
            ] {
                if let Some(base) = var(name) {
                    sources.windows_bases.push((label, PathBuf::from(base)));
                }
            }
            sources.registry = true;
        }
        sources
    }
}

/// Searches this system for the SolidCraft CLI: [`CLI_ENV`] first (when it names a file), then
/// the newest found on `PATH`, beside this program, or in the places this system installs
/// SolidCraft. Every caller that needs "the SolidCraft CLI" goes through here.
pub fn discover() -> Discovery {
    discover_in(&Sources::system(), &|cli| {
        version_of(cli)
            .ok()
            .flatten()
            .and_then(|line| parse_version(&line))
    })
}

/// [`discover`] over the given sources, ranking candidates by the version `version` reports.
pub(crate) fn discover_in(
    sources: &Sources,
    version: &impl Fn(&Path) -> Option<[u64; 3]>,
) -> Discovery {
    let mut places = Vec::new();
    let named = sources
        .env_cli
        .as_ref()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    match &named {
        None => places.push(Place {
            label: CLI_ENV,
            outcome: "unset",
            path: None,
        }),
        Some(path) if path.is_file() => places.push(Place {
            label: CLI_ENV,
            outcome: "found",
            path: Some(path.clone()),
        }),
        Some(path) => places.push(Place {
            label: CLI_ENV,
            outcome: "set_but_missing",
            path: Some(path.clone()),
        }),
    }
    if let Some(path) = named.filter(|path| path.is_file()) {
        if let Some(first) = places.first_mut() {
            first.outcome = "used";
        }
        return Discovery {
            cli: Some(path),
            places,
        };
    }
    let exe = format!("{CLI_NAME}{}", std::env::consts::EXE_SUFFIX);
    // (label, file) in search order; labels may repeat and are merged below.
    let mut candidates: Vec<(&'static str, PathBuf)> = Vec::new();
    let mut labels: Vec<&'static str> = vec!["PATH"];
    if let Some(path) = &sources.path {
        candidates.extend(
            std::env::split_paths(path)
                .map(|dir| ("PATH", dir.join(&exe)))
                .filter(|(_, file)| file.is_file()),
        );
    }
    labels.push("beside Dutchman");
    if let Some(folder) = &sources.program_folder {
        let file = folder.join(&exe);
        if file.is_file() {
            candidates.push(("beside Dutchman", file));
        }
    }
    for (label, base) in &sources.windows_bases {
        if !labels.contains(label) {
            labels.push(label);
        }
        for folder in [
            base.join("SolidCraft"),
            base.join("Programs").join("SolidCraft"),
        ] {
            let file = folder.join(format!("{CLI_NAME}.exe"));
            if file.is_file() {
                candidates.push((label, file));
            }
        }
    }
    for (label, folder) in &sources.app_folders {
        if !labels.contains(label) {
            labels.push(label);
        }
        if let Some(file) = in_app_bundles(std::slice::from_ref(folder)) {
            candidates.push((label, file));
        }
    }
    if sources.registry {
        labels.push("Windows registry");
        if let Some(file) = registered() {
            candidates.push(("Windows registry", file));
        }
    }
    let mut seen = Vec::new();
    candidates.retain(|(_, file)| {
        let key = std::fs::canonicalize(file).unwrap_or_else(|_| file.clone());
        !seen.contains(&key) && {
            seen.push(key);
            true
        }
    });
    let chosen = newest_cli(candidates.iter().map(|(_, file)| file.clone()), version);
    for label in labels {
        let here = candidates.iter().find(|(l, _)| *l == label);
        let used = here.is_some_and(|(_, file)| Some(file) == chosen.as_ref());
        places.push(Place {
            label,
            outcome: if used {
                "used"
            } else if here.is_some() {
                "found"
            } else {
                "not_found"
            },
            path: here.map(|(_, file)| file.clone()),
        });
    }
    Discovery {
        cli: chosen,
        places,
    }
}

/// How the SolidCraft CLI search went, for the reports: `outcome` is `found` (the CLI answered
/// with its version), `found_no_answer` (a CLI was found but did not report a version in time),
/// `not_found`, or `turned_off` / `chosen` when no search was made; `places` lists each place
/// tried with its outcome. A `path` appears on a place only when `with_paths` is set, so the
/// anonymous report carries generic labels only.
pub fn search_report(verification: &Verification, with_paths: bool) -> Value {
    let (found, places, outcome) = match verification {
        Verification::Off => (None, Vec::new(), "turned_off"),
        Verification::With(path) => (Some(path.clone()), Vec::new(), "chosen"),
        Verification::Auto => {
            let discovery = discover();
            let outcome = if discovery.cli.is_none() {
                "not_found"
            } else {
                "found"
            };
            (discovery.cli, discovery.places, outcome)
        }
    };
    let outcome = match (&found, outcome) {
        (Some(cli), "found" | "chosen") if version_of(cli).ok().flatten().is_none() => {
            "found_no_answer"
        }
        _ => outcome,
    };
    let places: Vec<Value> = places
        .iter()
        .map(|place| {
            let mut entry = serde_json::json!({"place": place.label, "outcome": place.outcome});
            if let (true, Some(path)) = (with_paths, &place.path) {
                if let Some(fields) = entry.as_object_mut() {
                    fields.insert(
                        "path".to_owned(),
                        serde_json::json!(path.display().to_string()),
                    );
                }
            }
            entry
        })
        .collect();
    let mut report = serde_json::json!({"outcome": outcome, "places": places});
    if let (true, Some(cli)) = (with_paths, &found) {
        if let Some(fields) = report.as_object_mut() {
            fields.insert(
                "cli".to_owned(),
                serde_json::json!(cli.display().to_string()),
            );
        }
    }
    report
}

/// The SolidCraft CLI in the same folder as `file`, if there is one.
fn beside(file: &Path) -> Option<PathBuf> {
    let cli = file
        .parent()?
        .join(format!("{CLI_NAME}{}", std::env::consts::EXE_SUFFIX));
    cli.is_file().then_some(cli)
}

/// Whether `file` is named as a SolidCraft app rather than its command-line tool, including the
/// single-file Windows download (`SolidCraft-0.30.1-windows-x64.exe`).
fn named_as_app(file: &Path) -> bool {
    file.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|stem| stem.starts_with("solidcraft") && !stem.starts_with(CLI_NAME))
}

/// The SolidCraft CLI to use for a file somebody chose: the file itself, or the CLI beside it when
/// they chose the SolidCraft app. An app is never started, as that would open its window. The
/// error says what to do instead.
pub fn chosen_cli(chosen: &Path) -> Result<PathBuf, String> {
    let cli = if named_as_app(chosen) {
        beside(chosen).ok_or_else(|| {
            "That is the SolidCraft app, and there is no solidcraft-cli beside it. Dutchman \
             checks designs with solidcraft-cli. On Windows, the single-file SolidCraft download \
             leaves it out; the portable ZIP and the MSI installer include it."
                .to_owned()
        })?
    } else {
        chosen.to_path_buf()
    };
    let says_solidcraft = version_of(&cli)?.filter(|line| {
        line.to_ascii_lowercase().contains("solidcraft") && parse_version(line).is_some()
    });
    match says_solidcraft {
        Some(_) => Ok(cli),
        None => Err(format!(
            "{} did not answer as solidcraft-cli. Choose the solidcraft-cli that comes with SolidCraft.",
            cli.display()
        )),
    }
}

/// The oldest accepted SolidCraft target, retained for simplified output.
pub const OLDEST_SOLIDCRAFT: [u64; 3] = [0, 18, 2];

/// The newest minor known at build time. Patch releases need no constant update.
pub const NEWEST_SOLIDCRAFT: [u64; 2] = [0, 30];

/// The oldest SolidCraft release published in the 14 days before the tag, never below
/// OLDEST_SOLIDCRAFT. Update alongside NEWEST_SOLIDCRAFT and the CI roundtrip pins at release time.
pub const WINDOW_OLDEST: [u64; 3] = [0, 18, 2];

/// Writer capabilities whose output depends on the target release.
#[derive(Clone, Copy)]
pub enum VersionGate {
    /// Explicit flat tips compensate for changed entry-recess angle handling.
    EntryRecessTip,
    /// Editable sketch-profile coils.
    SketchProfileCoil,
    /// Editable ordered mirror-pattern chains.
    MirrorChain,
}

impl VersionGate {
    fn minimum(self) -> SolidCraftVersion {
        match self {
            Self::EntryRecessTip => SolidCraftVersion::V0_22_1,
            Self::SketchProfileCoil | Self::MirrorChain => SolidCraftVersion::V0_25_1,
        }
    }

    /// Whether this target can write the gated representation.
    pub fn supported(self, target: SolidCraftVersion) -> bool {
        target >= self.minimum()
    }
}

/// Whether the writer's release gates can retain less history than the newest target.
pub fn target_loses_history(target: SolidCraftVersion) -> bool {
    [
        VersionGate::EntryRecessTip,
        VersionGate::SketchProfileCoil,
        VersionGate::MirrorChain,
    ]
    .into_iter()
    .any(|gate| {
        !matches!(gate, VersionGate::EntryRecessTip)
            && gate.supported(SolidCraftVersion::default())
            && !gate.supported(target)
    })
}

/// Installed-version status, independent of CLI discovery or network access.
pub fn solidcraft_status(target: SolidCraftVersion, found: Option<&str>) -> &'static str {
    let Some(found) = found.and_then(parse_version) else {
        return "none_found";
    };
    let [major, minor, _] = found;
    let found = [major, minor];
    let [old_major, old_minor, _] = OLDEST_SOLIDCRAFT;
    if found > NEWEST_SOLIDCRAFT {
        "newer"
    } else if found < [old_major, old_minor] {
        "too_old"
    } else if found < NEWEST_SOLIDCRAFT && target_loses_history(target) {
        "older_loses_history"
    } else {
        "current"
    }
}

/// A discovered version's minor, including releases below the writer's target floor.
pub fn solidcraft_minor(line: &str) -> Option<String> {
    let [major, minor, _] = parse_version(line)?;
    Some(format!("{major}.{minor}"))
}

/// An accepted SolidCraft target release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SolidCraftVersion([u64; 3]);

impl SolidCraftVersion {
    /// First release that applies a drill-point angle on counterbores and countersinks.
    pub const V0_22_1: Self = Self([0, 22, 1]);

    /// First release with sketch-profile coils and the `.sc` design extension.
    pub const V0_25_1: Self = Self([0, 25, 1]);

    /// First release with saved component History bar visibility.
    pub const V0_30_0: Self = Self([0, 30, 0]);

    /// The minor used in presentation; patch-sensitive writer gates keep the full version.
    pub fn minor(self) -> String {
        let Self([major, minor, _]) = self;
        format!("{major}.{minor}")
    }

    /// The default design extension this release recognises.
    pub fn extension(self) -> &'static str {
        if self >= Self::V0_25_1 {
            "sc"
        } else {
            "solidcraft"
        }
    }

    /// The found CLI's release capped at newest known (even with verification off), else newest.
    pub fn detected(verification: &Verification) -> Self {
        let detection = match verification {
            Verification::Off => &Verification::Auto,
            other => other,
        };
        solidcraft_version(detection)
            .and_then(|line| parse_version(&line))
            .filter(|version| *version >= OLDEST_SOLIDCRAFT)
            .map(|version| Self(version).min(Self::default()))
            .unwrap_or_default()
    }
}

impl Default for SolidCraftVersion {
    fn default() -> Self {
        let [major, minor] = NEWEST_SOLIDCRAFT;
        Self([major, minor, 0])
    }
}

impl std::fmt::Display for SolidCraftVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self([major, minor, patch]) = self;
        write!(f, "{major}.{minor}.{patch}")
    }
}

impl std::str::FromStr for SolidCraftVersion {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let malformed = || {
            "SolidCraft version must be three whole numbers: x.y.z (for example, 0.25.1)".to_owned()
        };
        let mut parts = value.split('.');
        let mut version = [0; 3];
        for number in &mut version {
            let part = parts.next().ok_or_else(malformed)?;
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(malformed());
            }
            *number = part.parse().map_err(|_| malformed())?;
        }
        if parts.next().is_some() {
            return Err(malformed());
        }
        if version < OLDEST_SOLIDCRAFT {
            return Err(
                "target needs SolidCraft 0.18 patch 2 or newer; choose a supported version"
                    .to_owned(),
            );
        }
        Ok(Self(version))
    }
}

/// Why no SolidCraft CLI checks a design.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoCli {
    /// None is found, or checking is turned off.
    NotFound(String),
    /// The one found is older than [`OLDEST_SOLIDCRAFT`].
    TooOld(String),
    /// macOS has not allowed the CLI to run yet.
    Blocked(String),
}

impl std::fmt::Display for NoCli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoCli::NotFound(why) | NoCli::TooOld(why) | NoCli::Blocked(why) => f.write_str(why),
        }
    }
}

/// [`find_cli`], refused with the reason when the CLI found says it is older than
/// [`OLDEST_SOLIDCRAFT`] or macOS blocks it. One that does not say its version is used as it is.
pub fn usable_cli(verification: &Verification) -> Result<PathBuf, NoCli> {
    let cli = find_cli(verification).map_err(NoCli::NotFound)?;
    let Some(line) = version_of(&cli).map_err(NoCli::Blocked)? else {
        return Ok(cli);
    };
    match parse_version(&line) {
        Some(version) if version < OLDEST_SOLIDCRAFT => {
            let [major, minor, patch] = OLDEST_SOLIDCRAFT;
            Err(NoCli::TooOld(format!(
                "{line} at {} is older than SolidCraft {major}.{minor} patch {patch}, the oldest that \
                 opens and checks Dutchman's designs; install a newer SolidCraft",
                cli.display()
            )))
        }
        _ => Ok(cli),
    }
}

/// The version a `--version` line gives (`solidcraft-cli 0.18.2` as `[0, 18, 2]`).
fn parse_version(line: &str) -> Option<[u64; 3]> {
    let word = line.split_whitespace().last()?.trim_start_matches('v');
    let mut parts = word.split('.').map(|part| {
        part.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse::<u64>()
            .ok()
    });
    Some([
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ])
}

/// Where the SolidCraft installer records its install folder: key, value name. `InstallDir`
/// holds the folder the person chose; `solidcraft-cli.exe` sits directly in it.
const REG_INSTALL_DIR: [(&str, &str); 2] = [
    (r"HKLM\Software\SolidCraft", "InstallDir"),
    (r"HKCU\Software\SolidCraft", "InstallDir"),
];

/// Where the installer records the path of `solidcraft.exe` (the CLI sits beside it): key,
/// value name. The `App Paths` default value is the bare path; the open command quotes it.
const REG_APP_EXE: [(&str, &str); 2] = [
    (
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\App Paths\solidcraft.exe",
        "(Default)",
    ),
    (
        r"HKLM\Software\Classes\Applications\solidcraft.exe\shell\open\command",
        "(Default)",
    ),
];

/// The CLI in the folder the Windows installer recorded in the registry, whatever folder the
/// person chose. Read through `reg.exe`, in the 64-bit and the 32-bit view.
fn registered() -> Option<PathBuf> {
    let read = |(key, name): (&str, &str)| {
        ["/reg:64", "/reg:32"]
            .into_iter()
            .filter_map(|view| reg_value(&reg_query(key, name, view)?, name))
            .collect::<Vec<_>>()
    };
    let folders = REG_INSTALL_DIR
        .into_iter()
        .flat_map(read)
        .map(PathBuf::from);
    let beside_exe = REG_APP_EXE
        .into_iter()
        .flat_map(read)
        .filter_map(|value| exe_path(&value))
        .filter_map(|exe| Path::new(&exe).parent().map(Path::to_path_buf));
    folders
        .chain(beside_exe)
        .map(|folder| folder.join(format!("{CLI_NAME}.exe")))
        .find(|cli| cli.is_file())
}

/// What `reg.exe query key /v name` (`/ve` for the default value) prints in one registry
/// view, when it succeeds.
#[cfg_attr(not(windows), allow(dead_code))]
fn reg_query(key: &str, name: &str, view: &str) -> Option<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up from the desktop app.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let reg = std::env::var_os("SystemRoot")
            .map(|root| Path::new(&root).join("System32").join("reg.exe"))
            .unwrap_or_else(|| PathBuf::from("reg"));
        let mut command = Command::new(reg);
        command.arg("query").arg(key);
        if name == "(Default)" {
            command.arg("/ve");
        } else {
            command.arg("/v").arg(name);
        }
        let output = command
            .arg(view)
            .stdin(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }
    #[cfg(not(windows))]
    {
        let _ = (key, name, view);
        None
    }
}

/// The value `name` in `reg query` output, with `%VAR%` expanded in a `REG_EXPAND_SZ` value;
/// `(Default)` names the key's default value. None when it is absent or empty.
#[cfg_attr(not(windows), allow(dead_code))]
fn reg_value(output: &str, name: &str) -> Option<String> {
    reg_value_with(output, name, &|var| std::env::var(var).ok())
}

/// [`reg_value`], expanding variables with `env`.
#[cfg_attr(not(windows), allow(dead_code))]
fn reg_value_with(
    output: &str,
    name: &str,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    // A value line is `    <name>    <REG_TYPE>    <data>`, separated by four spaces
    // (older systems use tabs); the name may hold single spaces and the data any text.
    let default = |name: &str| name.eq_ignore_ascii_case("(Default)") || name == "<NO NAME>";
    output.lines().find_map(|line| {
        let line = line.replace('\t', "    ");
        let (found, rest) = line.trim_start().split_once("    REG_")?;
        let found = found.trim_end();
        if !(found.eq_ignore_ascii_case(name) || default(found) && default(name)) {
            return None;
        }
        let rest = format!("REG_{rest}");
        let (kind, data) = rest.split_once("    ").unwrap_or((&rest, ""));
        let data = data.trim();
        if data.is_empty() {
            return None;
        }
        Some(if kind == "REG_EXPAND_SZ" {
            expand_vars(data, env)
        } else {
            data.to_owned()
        })
    })
}

/// `text` with each `%VAR%` that `env` knows replaced by its value; the rest stays as written.
#[cfg_attr(not(windows), allow(dead_code))]
fn expand_vars(text: &str, env: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('%') {
        out.push_str(before);
        match after.split_once('%') {
            Some((var, tail)) if !var.is_empty() && env(var).is_some() => {
                out.push_str(&env(var).unwrap_or_default());
                rest = tail;
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The path to `solidcraft.exe` in a registry value: the first quoted path of an open command
/// (`"C:\Apps\SolidCraft\solidcraft.exe" "%1"`), else an unquoted path, up to its `.exe`.
#[cfg_attr(not(windows), allow(dead_code))]
fn exe_path(command: &str) -> Option<String> {
    let command = command.trim();
    let path = if let Some(quoted) = command.strip_prefix('"') {
        quoted.split_once('"')?.0
    } else {
        let end = command.to_ascii_lowercase().find(".exe")? + ".exe".len();
        command.get(..end)?
    };
    (!path.is_empty()).then(|| path.to_owned())
}

/// The CLI in the newest `SolidCraft*.app` bundle in `folders` (`Contents/MacOS/solidcraft-cli`),
/// by the version its `Info.plist` gives, then by name.
fn in_app_bundles(folders: &[PathBuf]) -> Option<PathBuf> {
    let mut found: Vec<(Vec<u64>, PathBuf)> = folders
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|app| {
            app.file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .is_some_and(|name| name.starts_with("solidcraft") && name.ends_with(".app"))
        })
        .filter_map(|app| {
            let cli = app.join("Contents/MacOS").join(CLI_NAME);
            cli.is_file().then(|| (bundle_version(&app), cli))
        })
        .collect();
    found.sort();
    found.pop().map(|(_, cli)| cli)
}

/// The version an app bundle's `Info.plist` gives (`CFBundleShortVersionString`), as numbers
/// to compare; empty when it gives none.
fn bundle_version(app: &Path) -> Vec<u64> {
    let Ok(plist) = std::fs::read_to_string(app.join("Contents/Info.plist")) else {
        return Vec::new();
    };
    let Some(after) = plist
        .split_once("<key>CFBundleShortVersionString</key>")
        .map(|(_, after)| after)
    else {
        return Vec::new();
    };
    after
        .split_once("<string>")
        .and_then(|(_, rest)| rest.split_once("</string>"))
        .map(|(version, _)| {
            version
                .trim()
                .split(['.', '-'])
                .map_while(|part| part.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// The version the SolidCraft CLI `verification` finds prints (`solidcraft-cli 0.18.2`),
/// when there is one and it answers within a few seconds.
pub fn solidcraft_version(verification: &Verification) -> Option<String> {
    version_of(&find_cli(verification).ok()?).ok().flatten()
}

/// Return a previously discovered CLI version without starting the CLI.
/// Consults both the process cache and the executable's matching disk cache entry.
pub fn cached_solidcraft_version(verification: &Verification) -> Option<String> {
    version_cache::version(&find_cli(verification).ok()?, false)
        .ok()
        .flatten()
}

/// How many times to try running a CLI the system says is busy being written.
const BUSY_TRIES: u32 = 50;

/// The first line `cli --version` prints, when it answers within a few seconds.
fn version_of(cli: &Path) -> Result<Option<String>, String> {
    version_cache::version(cli, true)
}

fn probe_version(cli: &Path) -> Result<Option<(String, bool)>, String> {
    let mut child = match spawn(cli, |command| {
        command
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
    }) {
        Ok(child) => child,
        Err(_) => return blocked_failure(cli).map_or(Ok(None), Err),
    };
    let started = Instant::now();
    let successful = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    if let Some(why) = blocked_failure(cli) {
                        return Err(why);
                    }
                }
                break status.success();
            }
            Ok(None) if started.elapsed() > VERSION_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return blocked_failure(cli).map_or(Ok(None), Err);
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(_) => return blocked_failure(cli).map_or(Ok(None), Err),
        }
    };
    let line = (|| {
        let mut text = String::new();
        std::io::Read::read_to_string(
            &mut std::io::Read::take(child.stdout.take()?, 4096),
            &mut text,
        )
        .ok()?;
        let line = text.lines().next()?.trim();
        (!line.is_empty()).then(|| line.to_owned())
    })();
    Ok(line.map(|line| (line, successful)))
}

#[cfg(any(target_os = "macos", test))]
const MACOS_BLOCKED: &str = "macOS has not yet allowed SolidCraft to run: open SolidCraft once \
    from Applications, click Done when macOS stops it, then click Open Anyway in System \
    Settings > Privacy & Security, and try again.";

#[cfg(any(target_os = "macos", test))]
fn enclosing_app(cli: &Path) -> Option<&Path> {
    cli.parent()?.ancestors().find(|path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    })
}

#[cfg(any(target_os = "macos", test))]
fn quarantine_blocks(value: &str) -> bool {
    let Some((flags, _)) = value.trim().split_once(';') else {
        return false;
    };
    flags.len() == 4
        && flags.bytes().all(|byte| byte.is_ascii_hexdigit())
        && u16::from_str_radix(flags, 16).is_ok_and(|flags| flags & 0x0040 == 0)
}

fn blocked_failure(cli: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let resolved = std::fs::canonicalize(cli).ok();
        let app = enclosing_app(resolved.as_deref().unwrap_or(cli))?;
        let output = Command::new("/usr/bin/xattr")
            .arg("-p")
            .arg("com.apple.quarantine")
            .arg(app)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        (output.status.success()
            && std::str::from_utf8(&output.stdout).is_ok_and(quarantine_blocks))
        .then(|| MACOS_BLOCKED.to_owned())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cli;
        None
    }
}

fn launch_failure(cli: &Path, err: std::io::Error) -> String {
    blocked_failure(cli).unwrap_or_else(|| format!("cannot run {}: {err}", cli.display()))
}

fn newest_cli(
    candidates: impl IntoIterator<Item = PathBuf>,
    version: &impl Fn(&Path) -> Option<[u64; 3]>,
) -> Option<PathBuf> {
    candidates.into_iter().max_by_key(|cli| version(cli))
}

/// Compares `history` (the design as written) with `original` (the same design with the
/// history suppressed and the original geometry live).
/// Replays `history` and compares it with `original` (the same design with its original
/// geometry live), whose model has `solids` solid bodies. When SolidCraft opens the original
/// unsoundly (solids without a volume, bodies inside out), there is nothing sound to compare
/// with, and a replay that runs is kept unverified, saying why.
pub(crate) fn compare(
    cli: &Path,
    timeout: Duration,
    history: &[u8],
    original: &[u8],
    bodies: Expected,
) -> Outcome {
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => return Outcome::Unverified(format!("no scratch folder: {err}")),
    };
    let expected = match evaluate(cli, timeout, dir.path(), "original", original) {
        Ok(Evaluation::Done(eval)) => eval,
        Ok(Evaluation::Failed(why)) | Err(why) => {
            return Outcome::Unverified(format!("cannot evaluate the original geometry: {why}"))
        }
    };
    let replayed = match evaluate(cli, timeout, dir.path(), "history", history) {
        Ok(Evaluation::Done(eval)) => eval,
        Ok(Evaluation::Failed(why)) => {
            return Outcome::Differs(format!("SolidCraft cannot replay it: {why}"))
        }
        Err(why) => return Outcome::Unverified(why),
    };
    if let Some(error) = replayed.errors.first() {
        return Outcome::Differs(error.clone());
    }
    let unsound = volume_problems(&expected.volumes(), bodies);
    if !unsound.is_empty() {
        return Outcome::Unverified(format!(
            "the original geometry cannot be compared with: {}",
            unsound.join("; ")
        ));
    }
    match_bodies(&replayed.bodies, &expected.bodies)
}

/// Checks that retained features replay, without comparing an unfinished body to the final one.
pub(crate) fn replay(cli: &Path, timeout: Duration, design: &[u8]) -> Outcome {
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => return Outcome::Unverified(format!("no scratch folder: {err}")),
    };
    match evaluate(cli, timeout, dir.path(), "prefix", design) {
        Ok(Evaluation::Done(eval)) => match eval.errors.first() {
            Some(why) => Outcome::Differs(why.clone()),
            None => Outcome::Matches,
        },
        Ok(Evaluation::Failed(why)) => Outcome::Differs(why),
        Err(why) => Outcome::Unverified(why),
    }
}

/// What is wrong with a design's body `volumes` in SolidCraft, for a design with
/// `bodies.solids` solid bodies (0: not counted): bodies inside out, and solids without a volume.
///
/// A sheet body bounds no volume, but SolidCraft gives it one, of either sign, and does not
/// say which of its bodies are sheets. So as many bodies inside out as the design has sheets
/// are taken for them, and only the rest are reported.
fn volume_problems(volumes: &[f64], bodies: Expected) -> Vec<String> {
    let solids = bodies.solids;
    let mut problems = Vec::new();
    let inside_out = volumes
        .iter()
        .filter(|v| **v < -NO_VOLUME)
        .count()
        .saturating_sub(bodies.sheets);
    if inside_out > 0 {
        problems.push(format!(
            "SolidCraft reads {inside_out} bod{} inside out, with a negative volume",
            if inside_out == 1 { "y" } else { "ies" }
        ));
    }
    let with_volume = volumes.iter().filter(|v| **v > NO_VOLUME).count();
    if with_volume < solids {
        problems.push(format!(
            "SolidCraft gives {with_volume} of its {solids} solids a volume; the others open as \
             empty shells"
        ));
    }
    problems
}

/// How SolidCraft says a body it imports has a shell that is not closed.
const OPEN_BODY: &str = "imported as an open body";

/// What SolidCraft shows of geometry written as an import.
#[derive(Debug, PartialEq)]
pub(crate) enum Shown {
    /// Geometry with no measured defect or meshing warning.
    Whole,
    /// A measured defect, evaluation failure, or meshing warning; what.
    Short(String),
    /// The check could not be made; why.
    Unchecked(String),
}

/// What a design written as geometry holds, to compare with what SolidCraft shows of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Expected {
    /// Its solid bodies, each of which should have a volume; 0 when not counted.
    pub solids: usize,
    /// Its sheet bodies: open by design, so neither a volume nor an open shell is a fault.
    pub sheets: usize,
}

/// A body volume, in cubic millimetres, below which SolidCraft gives the body none.
const NO_VOLUME: f64 = 1e-6;

pub(crate) fn check_import(
    cli: &Path,
    timeout: Duration,
    design: &[u8],
    expected: Expected,
) -> Shown {
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => return Shown::Unchecked(format!("no scratch folder: {err}")),
    };
    let eval = match evaluate(cli, timeout, dir.path(), "import", design) {
        Ok(Evaluation::Done(eval)) => eval,
        Ok(Evaluation::Failed(why)) => {
            return Shown::Short(format!("SolidCraft cannot open it: {why}"))
        }
        Err(why) => return Shown::Unchecked(why),
    };
    shown(&eval, expected)
}

fn shown(eval: &Eval, expected: Expected) -> Shown {
    let mut problems: Vec<String> = eval.errors.clone();
    problems.extend(volume_problems(&eval.volumes(), expected));
    // SolidCraft merges faces that share a surface, so the face count is no measure. An import
    // has no original area or volume to compare, so an open shell is still a fault, except for
    // the design's sheets, which are open by design. SolidCraft may join several warnings.
    let mut sheets = expected.sheets;
    problems.extend(
        eval.warnings
            .iter()
            .flat_map(|warning| warning.split("; "))
            .filter_map(|part| {
                if sheets > 0 && part.contains(OPEN_BODY) && !part.contains("could not be meshed") {
                    sheets -= 1;
                    None
                } else {
                    Some(part.to_owned())
                }
            }),
    );
    if problems.is_empty() {
        Shown::Whole
    } else {
        Shown::Short(problems.join("; "))
    }
}

/// Whether two sets of bodies match, regardless of body order: in volume, within what
/// SolidCraft's measuring meshes can miss, and in area.
fn match_bodies(replayed: &[Measured], expected: &[Measured]) -> Outcome {
    if replayed.len() != expected.len() {
        return Outcome::Differs(format!(
            "{} bodies replayed, {} expected",
            replayed.len(),
            expected.len()
        ));
    }
    let mut replayed = replayed.to_vec();
    let mut expected = expected.to_vec();
    replayed.sort_by(|a, b| a.volume.total_cmp(&b.volume));
    expected.sort_by(|a, b| a.volume.total_cmp(&b.volume));
    for (got, want) in replayed.iter().zip(&expected) {
        let (v, w) = (got.volume, want.volume);
        let allowed = (VOLUME_TOLERANCE * v.abs().max(w.abs())).max(mesh_slack(got, want));
        if !within((v - w).abs(), allowed) {
            return Outcome::Differs(format!(
                "a body's volume is {v:.6} mm³, the original's {w:.6} mm³"
            ));
        }
        if let (Some(a), Some(b)) = (got.area, want.area) {
            if !within((a - b).abs(), AREA_TOLERANCE * a.abs().max(b.abs())) {
                return Outcome::Differs(format!(
                    "a body's area is {a:.6} mm², the original's {b:.6} mm²"
                ));
            }
        }
    }
    Outcome::Matches
}

/// Whether `difference` is no more than `limit`; not when either is not a number.
fn within(difference: f64, limit: f64) -> bool {
    difference <= limit
}

/// The most two SolidCraft measurements of one shape can differ in volume: each mesh's chords
/// sag at most [`MESH_CHORD`] of the body's size from its faces, so it misses at most its area
/// times that. Zero when SolidCraft gives no area or box.
fn mesh_slack(a: &Measured, b: &Measured) -> f64 {
    match (a.area, a.size, b.area, b.size) {
        (Some(area_a), Some(size_a), Some(area_b), Some(size_b)) => {
            let chord = (MESH_CHORD * size_a.max(size_b)).max(MESH_CHORD_FLOOR);
            (area_a.abs() + area_b.abs()) * chord
        }
        _ => 0.0,
    }
}

/// One body as SolidCraft measured it, in millimetres.
#[derive(Clone, Debug, PartialEq)]
struct Measured {
    volume: f64,
    /// Its area, when SolidCraft gives it.
    area: Option<f64>,
    /// The diagonal of its box, when SolidCraft gives it.
    size: Option<f64>,
}

/// One design as SolidCraft evaluated it.
struct Eval {
    bodies: Vec<Measured>,
    errors: Vec<String>,
    /// Warnings on timeline features, such as faces that could not be meshed.
    warnings: Vec<String>,
}

impl Eval {
    fn volumes(&self) -> Vec<f64> {
        self.bodies.iter().map(|body| body.volume).collect()
    }
}

enum Evaluation {
    Done(Eval),
    /// SolidCraft ran and refused the design.
    Failed(String),
}

/// Evaluates `design` with the CLI. `Err` when the CLI could not give an answer at all.
fn evaluate(
    cli: &Path,
    timeout: Duration,
    dir: &Path,
    name: &str,
    design: &[u8],
) -> Result<Evaluation, String> {
    let value = evaluate_json(cli, timeout, dir, name, design)?;
    if let Some(why) = value.get("evaluation_failed").and_then(Value::as_str) {
        return Ok(Evaluation::Failed(why.to_owned()));
    }
    parse(&value).map(Evaluation::Done)
}

pub(crate) fn evaluate_json(
    cli: &Path,
    timeout: Duration,
    dir: &Path,
    name: &str,
    design: &[u8],
) -> Result<Value, String> {
    let input = dir.join(format!("{name}.solidcraft"));
    let output = dir.join(format!("{name}.json"));
    std::fs::write(&input, design).map_err(|err| format!("cannot stage the design: {err}"))?;
    // Like every other run of the CLI, retry while the system says it is busy being written.
    let mut child = spawn(cli, |command| {
        command
            .arg("eval")
            .arg(&input)
            .arg("--json-out")
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    })
    .map_err(|err| launch_failure(cli, err))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(blocked_failure(cli).unwrap_or_else(|| {
                    format!("{} timed out after {} s", cli.display(), timeout.as_secs())
                }));
            }
            Ok(None) if check_must_stop() => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CHECK_STOPPED_FOR_MEMORY.to_owned());
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(err) => {
                return Err(blocked_failure(cli)
                    .unwrap_or_else(|| format!("cannot wait for {}: {err}", cli.display())))
            }
        }
    };
    if !status.success() {
        if let Some(why) = blocked_failure(cli) {
            return Err(why);
        }
        return Ok(
            serde_json::json!({"evaluation_failed": format!("{CLI_NAME} exited with {status}")}),
        );
    }
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(&output)
        .map_err(|err| format!("{CLI_NAME} wrote no result: {err}"))?
        .take((64 << 20) + 1)
        .read_to_string(&mut text)
        .map_err(|err| format!("{CLI_NAME} result could not be read: {err}"))?;
    if text.len() > (64 << 20) {
        return Err(format!("{CLI_NAME} result exceeds 64 MiB"));
    }

    let value: Value =
        serde_json::from_str(&text).map_err(|err| format!("{CLI_NAME} result: {err}"))?;
    Ok(value)
}

/// One edge of a body SolidCraft replayed, as `model.edges` gives it: its end vertices, the
/// point halfway along it and its length, in millimetres. It gives no carrier or tangent.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReplayedEdge {
    pub start: [f64; 3],
    pub end: [f64; 3],
    pub mid: [f64; 3],
    pub length: f64,
    /// SolidCraft's name for it: the names of the faces it joins, separated by `|`.
    pub name: String,
}

/// One face of a body SolidCraft replayed, as `model.faces` gives it. `normal` is the
/// outward unit normal of a planar face, `None` for another surface.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReplayedFace {
    pub name: String,
    pub area: f64,
    pub normal: Option<[f64; 3]>,
    pub point: [f64; 3],
}

/// What SolidCraft rebuilt at one point of a replayed timeline: the bodies with their edges,
/// and when faces were asked for, each body's faces (in the order of the bodies).
#[derive(Clone, Debug, Default)]
pub(crate) struct Replay {
    pub bodies: ReplayedBodies,
    pub faces: Vec<Vec<ReplayedFace>>,
}

/// The bodies at one point of a replayed timeline, each by name with its edges.
pub(crate) type ReplayedBodies = Vec<(String, Vec<ReplayedEdge>)>;

/// How many bodies, and edges in all, one replayed step may report before it is refused.
const REPLAY_BODIES: usize = 1_000;
const REPLAY_EDGES: usize = 200_000;

/// Replays `design` in SolidCraft and reports the bodies and their edges with the timeline
/// rolled to just after each feature id in `after` (in that order): the bodies each later
/// feature starts from. Two runs of the CLI in all, however many steps, within `timeout`
/// together: one finds the bodies at every step, the next their edges. Each step is `Err`
/// when a feature before it fails there; the whole is `Err` when SolidCraft gives no answer.
pub(crate) fn replay_edges(
    cli: &Path,
    timeout: Duration,
    design: &[u8],
    after: &[u64],
    faces: bool,
) -> Result<Vec<Result<Replay, String>>, String> {
    let deadline = Instant::now() + timeout;
    let dir = tempfile::tempdir().map_err(|err| format!("no scratch folder: {err}"))?;
    let input = dir.path().join("history.solidcraft");
    std::fs::write(&input, design).map_err(|err| format!("cannot stage the design: {err}"))?;
    let roll = |id: u64| crate::replay::roll_commands(id)[0].clone();
    let mut script = Vec::new();
    for id in after {
        script.extend(crate::replay::roll_commands(*id));
    }
    let results = run_script(cli, deadline, dir.path(), "bodies", &input, &script)?;
    let mut steps = Vec::new();
    for (k, _) in after.iter().enumerate() {
        let inspect = results
            .get(k.saturating_mul(2).saturating_add(1))
            .ok_or_else(|| format!("{CLI_NAME} result is missing a step"))?;
        steps.push(step_bodies(inspect).map(|bodies| Replay {
            bodies,
            faces: Vec::new(),
        }));
    }
    let mut script = Vec::new();
    for (id, step) in after.iter().zip(&steps) {
        script.push(roll(*id));
        if let Ok(replay) = step {
            for (name, _) in &replay.bodies {
                script
                    .push(serde_json::json!({"command": "model.edges", "params": {"body": name}}));
                if faces {
                    script.push(
                        serde_json::json!({"command": "model.faces", "params": {"body": name}}),
                    );
                }
            }
        }
    }
    let results = run_script(cli, deadline, dir.path(), "edges", &input, &script)?;
    let mut at = 0usize;
    for step in &mut steps {
        at = at.saturating_add(1);
        let Ok(replay) = step else { continue };
        let mut total = 0usize;
        for (name, edges) in replay.bodies.iter_mut() {
            let result = results
                .get(at)
                .ok_or_else(|| format!("{CLI_NAME} result is missing a step"))?;
            at = at.saturating_add(1);
            if result.get("body").and_then(Value::as_str) != Some(name.as_str()) {
                return Err(format!(
                    "{CLI_NAME} reported the edges of another body than {name:?}"
                ));
            }
            let rows = result
                .get("edges")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{CLI_NAME} result has no edges for {name:?}"))?;
            total = total.saturating_add(rows.len());
            if total > REPLAY_EDGES {
                return Err(format!(
                    "the replayed bodies have more than {REPLAY_EDGES} edges"
                ));
            }
            *edges = rows.iter().map(replayed_edge).collect::<Result<_, _>>()?;
            if faces {
                let result = results
                    .get(at)
                    .ok_or_else(|| format!("{CLI_NAME} result is missing a step"))?;
                at = at.saturating_add(1);
                let rows = result
                    .get("faces")
                    .and_then(Value::as_array)
                    .ok_or_else(|| format!("{CLI_NAME} result has no faces for {name:?}"))?;
                if rows.len() > REPLAY_EDGES {
                    return Err(format!(
                        "the replayed bodies have more than {REPLAY_EDGES} faces"
                    ));
                }
                replay
                    .faces
                    .push(rows.iter().map(replayed_face).collect::<Result<_, _>>()?);
            }
        }
    }
    Ok(steps)
}

/// The bodies a `document.inspect` result holds, or the first feature still active that
/// failed.
fn step_bodies(inspect: &Value) -> Result<ReplayedBodies, String> {
    let timeline = inspect
        .get("timeline")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for entry in timeline {
        let active = entry.get("rolled_back").and_then(Value::as_bool) != Some(true)
            && entry.get("suppressed").and_then(Value::as_bool) != Some(true);
        if let (true, Some(error)) = (active, entry.get("error").and_then(Value::as_str)) {
            let name = entry.get("name").and_then(Value::as_str).unwrap_or("?");
            return Err(format!("feature \"{name}\" failed there: {error}"));
        }
    }
    let bodies = inspect
        .get("bodies")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{CLI_NAME} result has no bodies"))?;
    if bodies.len() > REPLAY_BODIES {
        return Err(format!("more than {REPLAY_BODIES} bodies"));
    }
    bodies
        .iter()
        .map(|body| {
            body.get("name")
                .and_then(Value::as_str)
                .map(|name| (name.to_owned(), Vec::new()))
                .ok_or_else(|| format!("{CLI_NAME} result has a body without a name"))
        })
        .collect()
}

fn replayed_edge(row: &Value) -> Result<ReplayedEdge, String> {
    edge_fields(row).ok_or_else(|| {
        format!("{CLI_NAME} result has an edge without its ends, midpoint or length")
    })
}

fn replayed_face(row: &Value) -> Result<ReplayedFace, String> {
    let point = |key: &str| -> Option<[f64; 3]> {
        let values = row.get(key)?.as_array()?;
        if values.len() != 3 {
            return None;
        }
        let mut out = [0.0; 3];
        for (slot, value) in out.iter_mut().zip(values) {
            *slot = value.as_f64().filter(|v| v.is_finite())?;
        }
        Some(out)
    };
    (|| {
        Some(ReplayedFace {
            name: row.get("name")?.as_str()?.to_owned(),
            area: row
                .get("area")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v >= 0.0)?,
            normal: point("plane_normal"),
            point: point("point")?,
        })
    })()
    .ok_or_else(|| format!("{CLI_NAME} result has a face without its name, area or point"))
}

fn edge_fields(row: &Value) -> Option<ReplayedEdge> {
    let point = |key: &str| -> Option<[f64; 3]> {
        let values = row.get(key)?.as_array()?;
        if values.len() != 3 {
            return None;
        }
        let mut out = [0.0; 3];
        for (slot, value) in out.iter_mut().zip(values) {
            *slot = value.as_f64().filter(|v| v.is_finite())?;
        }
        Some(out)
    };
    Some(ReplayedEdge {
        start: point("start")?,
        end: point("end")?,
        mid: point("mid")?,
        length: row
            .get("length")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && *v >= 0.0)?,
        name: row
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

/// Runs `script` on `design` with the CLI and returns its per-command results.
fn run_script(
    cli: &Path,
    deadline: Instant,
    dir: &Path,
    name: &str,
    design: &Path,
    script: &[Value],
) -> Result<Vec<Value>, String> {
    let path = dir.join(format!("{name}.json"));
    let output = dir.join(format!("{name}-out.json"));
    let text =
        serde_json::to_string(script).map_err(|err| format!("cannot write the script: {err}"))?;
    std::fs::write(&path, text).map_err(|err| format!("cannot stage the script: {err}"))?;
    let mut child = spawn(cli, |command| {
        command
            .arg("run")
            .arg(&path)
            .arg("--in")
            .arg(design)
            .arg("--results")
            .arg("--allow-errors")
            .arg("--json-out")
            .arg(&output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    })
    .map_err(|err| launch_failure(cli, err))?;
    let status = wait(&mut child, cli, deadline)?;
    if !status.success() {
        return Err(
            blocked_failure(cli).unwrap_or_else(|| format!("{CLI_NAME} exited with {status}"))
        );
    }
    let text = std::fs::read_to_string(&output)
        .map_err(|err| format!("{CLI_NAME} wrote no result: {err}"))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|err| format!("{CLI_NAME} result: {err}"))?;
    match value {
        Value::Array(results) => Ok(results),
        _ => Err(format!("{CLI_NAME} result is not a list of results")),
    }
}

/// Starts the CLI, retrying while the system says it is busy being written.
/// Keeps a console program this one starts from opening a window of its own, as it does when
/// this one has no console: the desktop app, and the children it converts in. Attached to a
/// terminal it shares that one, where Ctrl+C reaches it.
pub fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::io::IsTerminal;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if !std::io::stderr().is_terminal() {
            command.creation_flags(CREATE_NO_WINDOW);
        }
    }
    #[cfg(not(windows))]
    let _ = command;
}

pub(crate) fn spawn(
    cli: &Path,
    configure: impl Fn(&mut Command),
) -> std::io::Result<std::process::Child> {
    let mut tries = 0;
    loop {
        let mut command = Command::new(cli);
        hide_console(&mut command);
        configure(&mut command);
        match command.spawn() {
            Err(err)
                if err.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < BUSY_TRIES =>
            {
                tries += 1;
                std::thread::sleep(POLL);
            }
            spawned => return spawned,
        }
    }
}

/// Free memory below which a SolidCraft check stops and the design is kept unverified. Above
/// the floor where a conversion itself stops, so a built design is never lost to its check.
const CHECK_MEMORY_FLOOR: u64 = 3 << 29;

/// Why a check stopped for memory.
const CHECK_STOPPED_FOR_MEMORY: &str =
    "History check stopped: the computer was running low on memory";

/// Set once a check stops for memory: the checks after it would only run out again.
static CHECKS_STOPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether checks watch memory at all: on unless memory checks are turned off for the run.
static CHECKS_WATCH_MEMORY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Turns the memory watch on running checks on or off, for this run.
pub fn watch_check_memory(on: bool) {
    CHECKS_WATCH_MEMORY.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether this much free memory is too little for a check to carry on.
fn below_check_floor(available: Option<u64>) -> bool {
    available.is_some_and(|free| free < CHECK_MEMORY_FLOOR)
}

/// Whether a running check should stop for memory, measuring it now.
fn check_must_stop() -> bool {
    use std::sync::atomic::Ordering;
    #[cfg(debug_assertions)]
    {
        if std::env::var_os("DUTCHMAN_TEST_CHECK_LOW_MEMORY").is_some() {
            return CHECKS_WATCH_MEMORY.load(Ordering::Relaxed);
        }
        // A test's made-up free memory is for the decisions before a check, not during one.
        if std::env::var_os("DUTCHMAN_TEST_MEMORY").is_some() {
            return false;
        }
    }
    if !CHECKS_WATCH_MEMORY.load(Ordering::Relaxed) {
        return false;
    }
    if below_check_floor(dutchman_core::available_memory()) {
        CHECKS_STOPPED.store(true, Ordering::Relaxed);
    }
    CHECKS_STOPPED.load(Ordering::Relaxed)
}

/// Waits for `child` until `deadline`, killing it then.
fn wait(
    child: &mut std::process::Child,
    cli: &Path,
    deadline: Instant,
) -> Result<std::process::ExitStatus, String> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(
                    blocked_failure(cli).unwrap_or_else(|| format!("{} timed out", cli.display()))
                );
            }
            Ok(None) if check_must_stop() => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CHECK_STOPPED_FOR_MEMORY.to_owned());
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(err) => {
                return Err(blocked_failure(cli)
                    .unwrap_or_else(|| format!("cannot wait for {}: {err}", cli.display())))
            }
        }
    }
}

/// The bodies and timeline errors and warnings in a SolidCraft `eval` result.
fn parse(value: &Value) -> Result<Eval, String> {
    let bodies = value
        .get("bodies")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{CLI_NAME} result has no bodies"))?;
    let bodies = bodies
        .iter()
        .map(|body| {
            let volume = body
                .get("volume_mm3")
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("{CLI_NAME} result has a body without a volume"))?;
            let corner = |end: &str| -> Option<Vec<f64>> {
                let corner = body.pointer(&format!("/bbox/{end}"))?.as_array()?;
                let corner: Option<Vec<f64>> = corner.iter().map(Value::as_f64).collect();
                corner.filter(|corner| corner.len() == 3)
            };
            let size = corner("min").zip(corner("max")).map(|(min, max)| {
                min.iter()
                    .zip(&max)
                    .map(|(lo, hi)| (hi - lo) * (hi - lo))
                    .sum::<f64>()
                    .sqrt()
            });
            Ok(Measured {
                volume,
                area: body.get("area_mm2").and_then(Value::as_f64),
                size: size.filter(|size| size.is_finite()),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let timeline = value
        .get("timeline")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let errors = timeline
        .iter()
        .filter_map(|entry| {
            let error = entry.get("error").and_then(Value::as_str)?;
            let name = entry.get("name").and_then(Value::as_str).unwrap_or("?");
            Some(format!("feature \"{name}\" failed: {error}"))
        })
        .collect();
    let warnings = timeline
        .iter()
        .filter_map(|entry| entry.get("warning").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    Ok(Eval {
        bodies,
        errors,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(folder: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(folder).unwrap();
        let file = folder.join(name);
        std::fs::write(&file, "x").unwrap();
        file
    }

    fn outcomes(found: &Discovery) -> Vec<(&'static str, &'static str)> {
        found.places.iter().map(|p| (p.label, p.outcome)).collect()
    }

    #[test]
    fn the_environment_variable_wins_over_every_other_place() {
        let dir = tempfile::tempdir().unwrap();
        let named = touch(&dir.path().join("named"), "tool");
        let on_path = touch(
            &dir.path().join("bin"),
            &format!("{CLI_NAME}{}", std::env::consts::EXE_SUFFIX),
        );
        let sources = Sources {
            env_cli: Some(named.clone().into_os_string()),
            path: Some(dir.path().join("bin").into_os_string()),
            ..Sources::default()
        };
        let found = discover_in(&sources, &|cli| (cli == on_path).then_some([9, 9, 9]));
        assert_eq!(found.cli, Some(named));
        assert_eq!(outcomes(&found), vec![(CLI_ENV, "used")]);
    }

    #[test]
    fn a_missing_environment_file_falls_through_to_the_other_places() {
        let dir = tempfile::tempdir().unwrap();
        let on_path = touch(
            &dir.path().join("bin"),
            &format!("{CLI_NAME}{}", std::env::consts::EXE_SUFFIX),
        );
        let sources = Sources {
            env_cli: Some(dir.path().join("gone").into_os_string()),
            path: Some(dir.path().join("bin").into_os_string()),
            ..Sources::default()
        };
        let found = discover_in(&sources, &|_| None);
        assert_eq!(found.cli, Some(on_path));
        assert_eq!(found.places[0].outcome, "set_but_missing");
        assert_eq!(
            found.places[1],
            Place {
                label: "PATH",
                outcome: "used",
                path: found.cli.clone()
            }
        );
    }

    #[test]
    fn install_folders_are_searched_and_the_newest_release_wins() {
        let dir = tempfile::tempdir().unwrap();
        let old = touch(&dir.path().join("pf/SolidCraft"), "solidcraft-cli.exe");
        let new = touch(
            &dir.path().join("local/Programs/SolidCraft"),
            "solidcraft-cli.exe",
        );
        let sources = Sources {
            windows_bases: vec![
                ("Program Files", dir.path().join("pf")),
                ("per-user install folder", dir.path().join("local")),
            ],
            ..Sources::default()
        };
        let found = discover_in(&sources, &|cli| {
            Some(if cli == new { [0, 30, 1] } else { [0, 26, 0] })
        });
        assert_eq!(found.cli, Some(new));
        assert_eq!(
            outcomes(&found),
            vec![
                (CLI_ENV, "unset"),
                ("PATH", "not_found"),
                ("beside Dutchman", "not_found"),
                ("Program Files", "found"),
                ("per-user install folder", "used"),
            ]
        );
        // Only Program Files holds one: it is chosen.
        let found = discover_in(
            &Sources {
                windows_bases: vec![("Program Files", dir.path().join("pf"))],
                ..Sources::default()
            },
            &|_| None,
        );
        assert_eq!(found.cli, Some(old));
    }

    #[test]
    fn nothing_found_lists_every_place_tried() {
        let dir = tempfile::tempdir().unwrap();
        let sources = Sources {
            path: Some(dir.path().as_os_str().to_owned()),
            windows_bases: vec![("Program Files", dir.path().join("pf"))],
            app_folders: vec![("Applications", dir.path().join("apps"))],
            ..Sources::default()
        };
        let found = discover_in(&sources, &|_| None);
        assert_eq!(found.cli, None);
        assert_eq!(
            outcomes(&found),
            vec![
                (CLI_ENV, "unset"),
                ("PATH", "not_found"),
                ("beside Dutchman", "not_found"),
                ("Program Files", "not_found"),
                ("Applications", "not_found"),
            ]
        );
    }

    #[test]
    fn the_desktop_and_command_line_search_through_the_same_function() {
        assert_eq!(find_cli(&Verification::Auto).ok(), discover().cli);
    }

    #[test]
    fn the_search_report_carries_paths_only_when_asked() {
        let local = search_report(&Verification::Auto, true);
        let anonymous = search_report(&Verification::Auto, false);
        assert!(anonymous["places"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p.get("path").is_none()));
        assert!(anonymous.get("cli").is_none());
        assert_eq!(local["places"][0]["place"], CLI_ENV);
    }

    const REG_INSTALL: &str = "\r\nHKEY_LOCAL_MACHINE\\Software\\SolidCraft\r\n    InstallDir    REG_SZ    C:\\Apps\\Solid Craft\\\r\n\r\n";

    fn fake_env(var: &str) -> Option<String> {
        var.eq_ignore_ascii_case("ProgramFiles")
            .then(|| r"C:\Program Files".to_owned())
    }

    #[test]
    fn a_registry_string_value_is_read() {
        assert_eq!(
            reg_value(REG_INSTALL, "InstallDir").as_deref(),
            Some("C:\\Apps\\Solid Craft\\")
        );
        assert_eq!(reg_value(REG_INSTALL, "Other"), None);
    }

    #[test]
    fn registry_variables_are_expanded_in_expand_sz_only() {
        let expand = "HKEY_LOCAL_MACHINE\\Software\\SolidCraft\n    InstallDir    REG_EXPAND_SZ    %ProgramFiles%\\SolidCraft\n";
        assert_eq!(
            reg_value_with(expand, "InstallDir", &fake_env).as_deref(),
            Some(r"C:\Program Files\SolidCraft")
        );
        let plain = expand.replace("REG_EXPAND_SZ", "REG_SZ");
        assert_eq!(
            reg_value_with(&plain, "InstallDir", &fake_env).as_deref(),
            Some(r"%ProgramFiles%\SolidCraft")
        );
        assert_eq!(expand_vars("%NOPE%\\x 100%", &fake_env), r"%NOPE%\x 100%");
    }

    #[test]
    fn a_default_registry_value_is_read() {
        let output = "HKEY_LOCAL_MACHINE\\Software\\Classes\\Applications\\solidcraft.exe\\shell\\open\\command\n    (Default)    REG_SZ    \"C:\\Program Files\\SolidCraft\\solidcraft.exe\" \"%1\"\n";
        let value = reg_value(output, "(Default)");
        assert_eq!(
            value.as_deref(),
            Some(r#""C:\Program Files\SolidCraft\solidcraft.exe" "%1""#)
        );
        let old = "HKLM\\Software\\X\n\t<NO NAME>\tREG_SZ\tC:\\a b\\solidcraft.exe\n";
        assert_eq!(
            reg_value(old, "(Default)").as_deref(),
            Some(r"C:\a b\solidcraft.exe")
        );
    }

    #[test]
    fn an_unset_registry_value_is_none() {
        let output = "HKEY_LOCAL_MACHINE\\Software\\X\n    (Default)    REG_SZ    \n";
        assert_eq!(reg_value(output, "(Default)"), None);
        assert_eq!(reg_value("", "InstallDir"), None);
        assert_eq!(
            reg_value(
                "ERROR: The system was unable to find the specified registry key or value.",
                "InstallDir"
            ),
            None
        );
    }

    #[test]
    fn the_exe_in_an_open_command_is_found() {
        assert_eq!(
            exe_path(r#""C:\Program Files\SolidCraft\solidcraft.exe" "%1""#).as_deref(),
            Some(r"C:\Program Files\SolidCraft\solidcraft.exe")
        );
        assert_eq!(
            exe_path(r"D:\My Apps\SolidCraft\solidcraft.EXE").as_deref(),
            Some(r"D:\My Apps\SolidCraft\solidcraft.EXE")
        );
        assert_eq!(
            exe_path(r"D:\My Apps\solidcraft.exe %1").as_deref(),
            Some(r"D:\My Apps\solidcraft.exe")
        );
        assert_eq!(exe_path(r#""unterminated"#), None);
        assert_eq!(exe_path(""), None);
    }

    #[test]
    fn the_nearest_enclosing_app_is_found() {
        for bundle in ["SolidCraft.app", "SolidCraft 0.20.APP", "Solid Craft.app"] {
            let app = Path::new("Applications").join(bundle);
            let cli = app.join("Contents/MacOS/solidcraft-cli");
            assert_eq!(enclosing_app(&cli), Some(app.as_path()));
        }
        assert_eq!(enclosing_app(Path::new("bin/solidcraft-cli")), None);
        assert_eq!(enclosing_app(Path::new("solidcraft-cli")), None);
        assert_eq!(
            enclosing_app(Path::new("Apps.app/Inner.app/bin/cli")),
            Some(Path::new("Apps.app/Inner.app"))
        );
        assert_eq!(
            enclosing_app(Path::new("SolidCraft.app.backup/bin/cli")),
            None
        );
    }

    #[test]
    fn only_quarantine_without_user_approval_blocks() {
        for value in ["0081;1234;Safari;uuid", "0083;1234;Safari;uuid"] {
            assert!(quarantine_blocks(value), "{value}");
        }
        for value in [
            "00c1;1234;Safari;uuid",
            "00C3;1234;Safari;uuid\n",
            "",
            "garbage",
            "zzzz;1234;Safari;uuid",
            "081;1234;Safari;uuid",
            "00081;1234;Safari;uuid",
            "+081;1234;Safari;uuid",
        ] {
            assert!(!quarantine_blocks(value), "{value}");
        }
        assert!(MACOS_BLOCKED.contains("Open Anyway in System Settings > Privacy & Security"));
        // One sentence, so the CLI's first-sentence notes keep the steps.
        assert!(!MACOS_BLOCKED.trim_end_matches('.').contains(". "));
    }

    pub(super) fn volumes_of(volumes: &[f64]) -> Vec<Measured> {
        volumes
            .iter()
            .map(|&volume| Measured {
                volume,
                area: None,
                size: None,
            })
            .collect()
    }

    fn body(volume: f64, area: f64, size: f64) -> Vec<Measured> {
        vec![Measured {
            volume,
            area: Some(area),
            size: Some(size),
        }]
    }

    #[test]
    fn volumes_match_in_any_order_within_tolerance() {
        assert_eq!(
            match_bodies(&volumes_of(&[2.0, 1.0]), &volumes_of(&[1.0, 2.00001])),
            Outcome::Matches
        );
    }

    #[test]
    fn a_different_volume_or_body_count_differs() {
        assert!(matches!(
            match_bodies(&volumes_of(&[1.0]), &volumes_of(&[1.1])),
            Outcome::Differs(_)
        ));
        assert!(matches!(
            match_bodies(&volumes_of(&[1.0]), &volumes_of(&[1.0, 1.0])),
            Outcome::Differs(_)
        ));
    }

    #[test]
    fn a_curved_body_matches_within_what_the_meshes_miss() {
        // A 60 mm ring 40 mm tall around a 40 mm bore (exactly 20000π mm³), revolved in
        // SolidCraft and read from the original's surfaces: the two meshes differ by 1.6e-4.
        assert_eq!(
            match_bodies(
                &body(62835.566016, 15707.98, 93.81),
                &body(62825.683952, 15707.33, 93.81)
            ),
            Outcome::Matches
        );
        // A replay 5 % larger differs far past that.
        assert!(matches!(
            match_bodies(
                &body(13995.7, 7012.0, 85.0),
                &body(13288.7, 6823.4, 85.0)
            ),
            Outcome::Differs(why) if why.contains("volume")
        ));
    }

    #[test]
    fn a_thin_plate_missing_a_hole_differs_in_area() {
        // 100 × 100 × 1 mm: the mesh bound allows about 285 mm³, more than an 18 mm hole's
        // 254 mm³, but the hole changes the area by 2.2 %.
        let hole = std::f64::consts::PI * 9.0 * 9.0;
        let plate = 2.0 * 100.0 * 100.0 + 4.0 * 100.0;
        assert!(matches!(
            match_bodies(
                &body(10_000.0, plate, 141.4),
                &body(10_000.0 - hole, plate - 2.0 * hole + std::f64::consts::PI * 18.0, 141.4)
            ),
            Outcome::Differs(why) if why.contains("area")
        ));
    }

    #[test]
    fn parses_an_eval_result() {
        let value = serde_json::json!({
            "bodies": [
                {"name": "Body1", "volume_mm3": 1000.0},
                {
                    "name": "Body2", "volume_mm3": 24.0, "area_mm2": 52.0,
                    "bbox": {"min": [0.0, 0.0, 0.0], "max": [2.0, 3.0, 6.0]}
                }
            ],
            "timeline": [
                {"name": "Sketch1", "error": null},
                {"name": "Extrude1", "error": "profile not closed"}
            ]
        });
        let eval = parse(&value).unwrap();
        assert_eq!(eval.volumes(), [1000.0, 24.0]);
        assert_eq!(eval.bodies[0].area, None);
        assert_eq!(eval.bodies[1].area, Some(52.0));
        assert_eq!(eval.bodies[1].size, Some(7.0));
        assert_eq!(
            eval.errors,
            ["feature \"Extrude1\" failed: profile not closed"]
        );
    }

    #[test]
    fn an_import_shows_whole_unless_measures_differ_or_faces_cannot_mesh() {
        let eval = |faces: u64, warning: Option<&str>| {
            parse(&serde_json::json!({
                "bodies": [{"volume_mm3": 1000.0, "area_mm2": 600.0}],
                "timeline": [{"name": "cube", "error": null, "warning": warning}],
                "total": {"faces": faces},
            }))
            .unwrap()
        };
        let six = Expected {
            solids: 1,
            sheets: 0,
        };
        assert_eq!(shown(&eval(6, None), six), Shown::Whole);
        let original = eval(6, None);
        for faces in [3, 6, 10] {
            let merged = eval(faces, None);
            assert_eq!(
                match_bodies(&merged.bodies, &original.bodies),
                Outcome::Matches
            );
            assert_eq!(shown(&merged, six), Shown::Whole);
        }
        let open = "solid 'cube': imported as an open body (the shell is not closed)";
        let merged = eval(3, Some(open));
        assert_eq!(
            match_bodies(&merged.bodies, &original.bodies),
            Outcome::Matches
        );
        // An import has nothing to compare the measures with, so an open solid is a fault.
        assert_eq!(shown(&merged, six), Shown::Short(open.to_owned()));
        // Except for as many open shells as the design has sheets.
        let one_sheet = Expected {
            solids: 1,
            sheets: 1,
        };
        assert_eq!(shown(&merged, one_sheet), Shown::Whole);
        for (volume, area, measure) in [(900.0, 600.0, "volume"), (1000.0, 500.0, "area")] {
            let mut missing = eval(3, Some(open));
            missing.bodies = body(volume, area, 17.32);
            assert!(matches!(
                match_bodies(&missing.bodies, &original.bodies),
                Outcome::Differs(why) if why.contains(measure)
            ));
        }
        assert_eq!(
            shown(
                &eval(3, Some("solid 'cube': 3 face(s) could not be meshed")),
                six
            ),
            Shown::Short("solid 'cube': 3 face(s) could not be meshed".to_owned())
        );
        let combined = format!("{open}; solid 'cube': 3 face(s) could not be meshed");
        assert_eq!(
            shown(&eval(3, Some(&combined)), one_sheet),
            Shown::Short("solid 'cube': 3 face(s) could not be meshed".to_owned())
        );
        assert_eq!(
            shown(&eval(3, Some(&combined)), six),
            Shown::Short(combined.clone())
        );
        let combined = format!("{open}, 3 face(s) could not be meshed");
        assert_eq!(
            shown(&eval(3, Some(&combined)), six),
            Shown::Short(combined)
        );
        assert!(matches!(shown(&eval(6, Some("odd")), six), Shown::Short(_)));
    }

    #[test]
    fn solids_without_a_volume_or_inside_out_are_reported() {
        let eval = |volumes: &[f64]| Eval {
            bodies: volumes_of(volumes),
            errors: Vec::new(),
            warnings: Vec::new(),
        };
        let two = Expected {
            solids: 2,
            sheets: 0,
        };
        assert_eq!(shown(&eval(&[1000.0, 8.0]), two), Shown::Whole);
        assert_eq!(
            shown(&eval(&[1000.0, 0.0, 0.0]), two),
            Shown::Short(
                "SolidCraft gives 1 of its 2 solids a volume; the others open as empty shells"
                    .to_owned()
            )
        );
        assert_eq!(
            shown(&eval(&[1000.0, -65.4]), Expected::default()),
            Shown::Short("SolidCraft reads 1 body inside out, with a negative volume".to_owned())
        );
    }

    #[test]
    fn verification_off_finds_no_cli() {
        assert!(find_cli(&Verification::Off).is_err());
        let path = PathBuf::from("/opt/sc");
        assert_eq!(find_cli(&Verification::With(path.clone())), Ok(path));
    }
}

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)]
mod cli_tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn an_unblocked_probe_keeps_its_version_even_on_failure() {
        let _turn = crate::testing::scripts();
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("solidcraft-cli");
        std::fs::write(&cli, "#!/bin/sh\necho 'solidcraft-cli 0.1.0'\nexit 3\n").unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            version_of(&cli),
            Ok(Some("solidcraft-cli 0.1.0".to_owned()))
        );
        assert!(matches!(
            usable_cli(&Verification::With(cli)),
            Err(NoCli::TooOld(_))
        ));
    }

    /// A stand-in SolidCraft CLI: `script` runs with `$design` (the design path) and must
    /// print the eval result, which is written to the `--json-out` file.
    #[test]
    fn the_newest_solidcraft_app_carries_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        for (app, version) in [
            ("SolidCraft.app", Some("0.18.2")),
            ("SolidCraft 2.app", Some("0.18.10")),
            ("SolidCraft old.app", None),
            ("Other.app", Some("9.0")),
        ] {
            let macos = dir.path().join(app).join("Contents/MacOS");
            std::fs::create_dir_all(&macos).unwrap();
            std::fs::write(macos.join(CLI_NAME), b"").unwrap();
            if let Some(version) = version {
                std::fs::write(
                    dir.path().join(app).join("Contents/Info.plist"),
                    format!(
                        "<dict><key>CFBundleShortVersionString</key>\n<string>{version}</string></dict>"
                    ),
                )
                .unwrap();
            }
        }
        let cli = in_app_bundles(&[dir.path().to_path_buf(), dir.path().join("missing")]).unwrap();
        assert!(
            cli.ends_with("SolidCraft 2.app/Contents/MacOS/solidcraft-cli"),
            "{cli:?}"
        );
        assert_eq!(in_app_bundles(&[dir.path().join("missing")]), None);
    }

    #[test]
    fn automatic_cli_selection_compares_versions_across_locations() {
        let path = PathBuf::from("path/solidcraft-cli");
        let app = PathBuf::from("applications/solidcraft-cli");
        let found = newest_cli([path.clone(), app.clone()], &|candidate| {
            Some(if candidate == path.as_path() {
                [0, 27, 0]
            } else {
                [0, 29, 0]
            })
        });
        assert_eq!(found, Some(app));
        assert_eq!(newest_cli([path.clone()], &|_| None), Some(path));
    }

    fn version_script(dir: &Path, name: &str, line: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\necho '{line}'\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn finds_the_cli_beside_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("dutchman");
        assert_eq!(beside(&program), None);
        let cli = version_script(dir.path(), "solidcraft-cli", "solidcraft-cli 0.30.0");
        assert_eq!(beside(&program), Some(cli));
    }

    #[test]
    fn a_chosen_cli_must_answer_as_solidcraft() {
        let dir = tempfile::tempdir().unwrap();
        let cli = version_script(dir.path(), "solidcraft-cli", "solidcraft-cli 0.30.0");
        assert_eq!(chosen_cli(&cli), Ok(cli));
        let other = version_script(dir.path(), "other-tool", "other-tool 1.2.3");
        assert!(chosen_cli(&other)
            .unwrap_err()
            .contains("did not answer as solidcraft-cli"));
    }

    #[test]
    fn a_chosen_app_uses_the_cli_beside_it_and_is_never_started() {
        let dir = tempfile::tempdir().unwrap();
        let started = dir.path().join("started");
        let app = dir.path().join("SolidCraft-0.30.1-windows-x64");
        std::fs::write(
            &app,
            format!(
                "#!/bin/sh\ntouch '{}'\necho 'solidcraft 0.30.1'\n",
                started.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(chosen_cli(&app).unwrap_err().contains("portable ZIP"));
        let cli = version_script(dir.path(), "solidcraft-cli", "solidcraft-cli 0.30.1");
        assert_eq!(chosen_cli(&app), Ok(cli));
        assert!(!started.exists());
    }

    fn fake_cli(dir: &Path, script: &str) -> PathBuf {
        let path = dir.join("solidcraft-cli");
        let body = format!("#!/bin/sh\ndesign=\"$2\"\nout=\"$4\"\n{{\n{script}\n}} > \"$out\"\n");
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn eval(volume: f64, error: &str) -> String {
        format!(
            "echo '{{\"bodies\":[{{\"volume_mm3\":{volume}}}],\"timeline\":[{{\"name\":\"Extrude1\",\"error\":{error}}}]}}'"
        )
    }

    fn by_design(original: &str, history: &str) -> String {
        format!("case \"$design\" in *original*) {original};; *) {history};; esac")
    }

    fn run(script: &str, timeout: Duration) -> Outcome {
        let _turn = crate::testing::scripts();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), script);
        compare(
            &cli,
            timeout,
            b"history",
            b"original",
            Expected {
                solids: 1,
                ..Expected::default()
            },
        )
    }

    #[test]
    fn a_sigkill_without_unapproved_quarantine_keeps_its_failure_message() {
        assert!(matches!(
            run("kill -KILL $$", DEFAULT_TIMEOUT),
            Outcome::Unverified(why) if why.contains("signal: 9")
        ));
    }

    #[test]
    fn an_original_that_opens_unsoundly_leaves_a_replayed_history_unverified() {
        // The original opens as an empty shell; the history replays to a solid.
        let shell = by_design(&eval(0.0, "null"), &eval(1000.0, "null"));
        assert_eq!(
            run(&shell, DEFAULT_TIMEOUT),
            Outcome::Unverified(
                "the original geometry cannot be compared with: SolidCraft gives 0 of its 1 \
                 solids a volume; the others open as empty shells"
                    .to_owned()
            )
        );
        let inside_out = by_design(&eval(-65.4, "null"), &eval(1000.0, "null"));
        assert!(matches!(
            run(&inside_out, DEFAULT_TIMEOUT),
            Outcome::Unverified(_)
        ));
        // A history that does not replay still differs.
        let failing = by_design(&eval(0.0, "null"), &eval(0.0, "\"profile not closed\""));
        assert!(matches!(
            run(&failing, DEFAULT_TIMEOUT),
            Outcome::Differs(_)
        ));
    }

    #[test]
    fn a_matching_replay_matches() {
        let script = by_design(&eval(1000.0, "null"), &eval(1000.0, "null"));
        assert_eq!(run(&script, DEFAULT_TIMEOUT), Outcome::Matches);
    }

    #[test]
    fn merged_faces_and_open_shells_do_not_hide_a_measured_difference() {
        let measured = |faces, volume, area, warning| {
            let value = serde_json::json!({
                "bodies": [{
                    "volume_mm3": volume, "area_mm2": area,
                    "bbox": {"min": [0, 0, 0], "max": [10, 10, 10]}
                }],
                "timeline": [{"warning": warning}],
                "total": {"faces": faces}
            });
            format!("echo '{value}'")
        };
        let original = measured(14, 1000.0, 600.0, "");
        let open = "solid #1: imported as an open body (the shell is not closed)";
        // The small measurement differences are within the existing mesh tolerances.
        let merged = measured(10, 1000.01, 600.01, open);
        assert_eq!(
            run(&by_design(&original, &merged), DEFAULT_TIMEOUT),
            Outcome::Matches
        );
        for (volume, area, measure) in [(900.0, 600.0, "volume"), (1000.0, 500.0, "area")] {
            let missing = measured(10, volume, area, open);
            assert!(matches!(
                run(&by_design(&original, &missing), DEFAULT_TIMEOUT),
                Outcome::Differs(why) if why.contains(measure)
            ));
        }
    }

    #[test]
    fn a_different_volume_differs() {
        let script = by_design(&eval(1000.0, "null"), &eval(900.0, "null"));
        assert!(matches!(run(&script, DEFAULT_TIMEOUT), Outcome::Differs(_)));
    }

    #[test]
    fn a_failing_feature_differs() {
        let script = by_design(&eval(1000.0, "null"), &eval(1000.0, "\"no profile\""));
        assert_eq!(
            run(&script, DEFAULT_TIMEOUT),
            Outcome::Differs("feature \"Extrude1\" failed: no profile".to_owned())
        );
    }

    #[test]
    fn a_history_solidcraft_refuses_differs() {
        let script = by_design(&eval(1000.0, "null"), "exit 3");
        assert!(matches!(run(&script, DEFAULT_TIMEOUT), Outcome::Differs(_)));
    }

    #[test]
    fn an_original_solidcraft_cannot_read_leaves_it_unverified() {
        let script = by_design("exit 3", &eval(1000.0, "null"));
        assert!(matches!(
            run(&script, DEFAULT_TIMEOUT),
            Outcome::Unverified(_)
        ));
    }

    #[test]
    fn a_check_stops_above_the_floor_where_a_conversion_stops() {
        assert!(below_check_floor(Some(CHECK_MEMORY_FLOOR - 1)));
        assert!(!below_check_floor(Some(CHECK_MEMORY_FLOOR)));
        assert!(!below_check_floor(None));
        const { assert!(CHECK_MEMORY_FLOOR > 1 << 30) };
    }

    #[test]
    fn a_slow_solidcraft_leaves_it_unverified() {
        let outcome = run("sleep 5", Duration::from_millis(200));
        assert!(
            matches!(&outcome, Outcome::Unverified(why) if why.contains("timed out")),
            "{outcome:?}"
        );
    }

    fn import_check(script: &str) -> Shown {
        let _turn = crate::testing::scripts();
        let dir = tempfile::tempdir().unwrap();
        let cli = fake_cli(dir.path(), script);
        check_import(
            &cli,
            DEFAULT_TIMEOUT,
            b"design",
            Expected {
                solids: 0,
                sheets: 0,
            },
        )
    }

    #[test]
    fn an_import_solidcraft_shows_short_is_reported() {
        let whole =
            "echo '{\"bodies\":[{\"volume_mm3\":1000}],\"timeline\":[],\"total\":{\"faces\":6}}'";
        assert_eq!(import_check(whole), Shown::Whole);
        let short = "echo '{\"bodies\":[{\"volume_mm3\":3.3}],\"timeline\":[{\"name\":\"cube\",\"error\":null,\"warning\":\"3 face(s) could not be meshed\"}],\"total\":{\"faces\":3}}'";
        assert_eq!(
            import_check(short),
            Shown::Short("3 face(s) could not be meshed".to_owned())
        );
        assert!(matches!(import_check("exit 3"), Shown::Short(_)));
        assert!(matches!(import_check("echo nonsense"), Shown::Unchecked(_)));
    }

    #[test]
    fn a_missing_cli_leaves_it_unverified() {
        let outcome = compare(
            Path::new("/nonexistent/solidcraft-cli"),
            DEFAULT_TIMEOUT,
            b"h",
            b"o",
            Expected {
                solids: 1,
                ..Expected::default()
            },
        );
        assert!(matches!(outcome, Outcome::Unverified(_)));
    }
    #[test]
    fn versions_are_read_from_the_cli_line() {
        assert_eq!(parse_version("solidcraft-cli 0.19.0"), Some([0, 19, 0]));
        assert_eq!(parse_version("solidcraft-cli v0.18"), Some([0, 18, 0]));
        assert_eq!(parse_version("solidcraft-cli 1.2.3-beta"), Some([1, 2, 3]));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn target_window_bounds_and_default_are_consistent() {
        assert!(OLDEST_SOLIDCRAFT <= WINDOW_OLDEST);
        assert!(SolidCraftVersion(WINDOW_OLDEST) <= SolidCraftVersion::default());
        assert!(SolidCraftVersion(OLDEST_SOLIDCRAFT) <= SolidCraftVersion::default());
        let [major, minor] = NEWEST_SOLIDCRAFT;
        assert_eq!(
            SolidCraftVersion::default(),
            SolidCraftVersion([major, minor, 0])
        );
    }

    #[cfg(unix)]
    #[test]
    fn target_defaults_to_the_found_cli_or_the_built_in_newest() {
        let _turn = crate::testing::scripts();
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for version in [
            "0.18.2", "0.20.0", "0.25.1", "0.27.1", "0.30.0", "0.30.9", "0.31.0", "nonsense",
            "0.17.0",
        ] {
            let cli = dir.path().join(version);
            std::fs::write(
                &cli,
                format!("#!/bin/sh\necho 'solidcraft-cli {version}'\n"),
            )
            .unwrap();
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
            let verification = Verification::With(cli);
            let expected = version
                .parse::<SolidCraftVersion>()
                .unwrap_or_default()
                .min(SolidCraftVersion::default());
            assert_eq!(SolidCraftVersion::detected(&verification), expected);
            assert_eq!(
                crate::SolidCraftWriter::new()
                    .verification(verification)
                    .resolved_version(),
                expected
            );
        }
        assert_eq!(
            crate::SolidCraftWriter::new()
                .verification(Verification::Off)
                .resolved_version(),
            SolidCraftVersion::detected(&Verification::Auto)
        );
        assert_eq!(
            crate::SolidCraftWriter::new()
                .verification(Verification::With(dir.path().join("missing")))
                .resolved_version(),
            SolidCraftVersion::default()
        );
        assert_eq!(
            crate::SolidCraftWriter::new()
                .verification(Verification::Off)
                .target_version(Some(SolidCraftVersion::V0_25_1))
                .resolved_version(),
            SolidCraftVersion::V0_25_1
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_solidcraft_too_old_is_refused_with_the_reason() {
        let _turn = crate::testing::scripts();
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let cli = |name: &str, version: &str| {
            let path = dir.path().join(name);
            std::fs::write(
                &path,
                format!("#!/bin/sh\necho 'solidcraft-cli {version}'\n"),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        let old = cli("old", "0.17.0");
        match usable_cli(&Verification::With(old)) {
            Err(NoCli::TooOld(why)) => assert!(
                why.starts_with("solidcraft-cli 0.17.0 at ")
                    && why.ends_with(
                        "is older than SolidCraft 0.18 patch 2, the oldest that opens \
                                      and checks Dutchman's designs; install a newer SolidCraft"
                    ),
                "{why}"
            ),
            other => panic!("{other:?}"),
        }
        let new = cli("new", "0.19.0");
        assert_eq!(usable_cli(&Verification::With(new.clone())), Ok(new));
        assert!(matches!(
            usable_cli(&Verification::Off),
            Err(NoCli::NotFound(_))
        ));
    }

    #[test]
    fn sheets_are_not_reported_as_inside_out_or_open() {
        let eval = |volumes: &[f64], warnings: &[&str]| Eval {
            bodies: super::tests::volumes_of(volumes),
            errors: Vec::new(),
            warnings: warnings.iter().map(|w| (*w).to_owned()).collect(),
        };
        let open = "solid #6704: imported as an open body (the shell is not closed)";
        // A solid and a sheet disk, which SolidCraft gives a negative volume and calls open.
        let one_sheet = Expected {
            solids: 1,
            sheets: 1,
        };
        assert_eq!(
            shown(&eval(&[1000.0, -2629.0], &[open]), one_sheet),
            Shown::Whole
        );
        // A solid inside out, or a second open shell, is still reported.
        assert_eq!(
            shown(&eval(&[-1000.0, -2629.0], &[open, open]), one_sheet),
            Shown::Short(format!(
                "SolidCraft reads 1 body inside out, with a negative volume; SolidCraft gives 0 \
                 of its 1 solids a volume; the others open as empty shells; {open}"
            ))
        );
        // Without sheets, both are faults.
        assert!(matches!(
            shown(&eval(&[1000.0, -2629.0], &[open]), Expected::default()),
            Shown::Short(_)
        ));
    }
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn minor_status_preserves_patch_sensitive_writer_rules() {
        for (found, status) in [
            ("0.27.0", "current"),
            ("0.27.9", "current"),
            ("0.30.0", "current"),
            ("0.30.9", "current"),
            ("0.31.0", "newer"),
            ("0.17.99", "too_old"),
        ] {
            assert_eq!(
                solidcraft_status(SolidCraftVersion::default(), Some(found)),
                status
            );
        }
        let before: SolidCraftVersion = "0.25.0".parse().unwrap();
        let after: SolidCraftVersion = "0.25.1".parse().unwrap();
        assert_eq!(before.minor(), after.minor());
        assert!(target_loses_history(before));
        assert!(!target_loses_history(after));
        assert!(!VersionGate::EntryRecessTip.supported("0.22.0".parse().unwrap()));
        assert!(VersionGate::EntryRecessTip.supported(SolidCraftVersion::V0_22_1));
        assert!("0.18.1".parse::<SolidCraftVersion>().is_err());
        assert!("0.18.2".parse::<SolidCraftVersion>().is_ok());
        assert_ne!(
            solidcraft_status(SolidCraftVersion::default(), Some("0.18.1")),
            "too_old"
        );
    }
}
