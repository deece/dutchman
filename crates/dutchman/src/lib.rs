//! What the `dutchman` command and the `dutchman-mcp` server share: how a conversion is set
//! up, and the reports of converting, inspecting and listing formats, as the JSON documents
//! `docs/json.md` describes. Both front ends print or send these; neither builds its own.
//!
//! This is the front ends' shared code, not a stable library API.

/// The version of this build as the apps show it: the package version, or a nightly's own
/// version when the nightly workflow sets `DUTCHMAN_NIGHTLY_VERSION`.
pub const BUILD_VERSION: &str = match option_env!("DUTCHMAN_NIGHTLY_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

pub mod anon;
pub mod batch;
pub mod choices;
pub mod diagnose;
pub mod diagnostics;
pub mod doctor;
pub mod dxf;
pub mod formats;
pub mod issue;
pub mod json;
pub mod mcp;
pub mod present;
pub mod progress;
pub mod report_policy;
pub mod report_send;
pub mod repro;
pub mod saved_report;
pub mod stop;
pub mod watch;
pub mod worker;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use dutchman_core::PartSearch;
use dutchman_core::{Error, Format, Model, Report, Writer};
use dutchman_solidcraft::{SolidCraftVersion, SolidCraftWriter, Verification, DEFAULT_TIMEOUT};
use serde_json::{json, Value};

/// The format written by a conversion; independent of FreeCAD input support.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// A SolidCraft design (`.sc`).
    #[default]
    Solidcraft,
    /// A FreeCAD rebuild macro (`.FCMacro`). Experimental.
    Freecad,
    /// A native FreeCAD document (`.FCStd`). Experimental; requires FreeCAD.
    Fcstd,
    /// FreeCAD TechDraw pages as ASCII DXF (convert command).
    Dxf,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Solidcraft => SolidCraftVersion::default().extension(),
            Self::Freecad => "FCMacro",
            Self::Fcstd => "FCStd",
            Self::Dxf => "dxf",
        }
    }
}

/// The selected output adapter.
#[derive(Clone, Debug)]
pub enum OutputWriter {
    /// Retains options so the batch runner can prepare each input's memory budget.
    Configured {
        writer: Box<OutputWriter>,
        options: Box<ConvertOptions>,
    },
    SolidCraft(SolidCraftWriter),
    FreeCad(dutchman_freecad::run::FreeCadRunner),
    Dxf,
}

impl Writer for OutputWriter {
    fn extension(&self) -> &'static str {
        match self {
            Self::Configured { writer, .. } => writer.extension(),
            Self::SolidCraft(writer) => writer.extension(),
            Self::FreeCad(writer) => writer.extension(),
            Self::Dxf => "dxf",
        }
    }

    fn accepts_extension(&self, extension: &str) -> bool {
        match self {
            Self::Configured { writer, .. } => writer.accepts_extension(extension),
            Self::SolidCraft(writer) => writer.accepts_extension(extension),
            Self::FreeCad(writer) => writer.accepts_extension(extension),
            Self::Dxf => extension.eq_ignore_ascii_case(self.extension()),
        }
    }

    fn write(
        &self,
        model: &Model,
        out: &mut dyn std::io::Write,
        report: &mut Report,
    ) -> Result<(), Error> {
        // Tests make the writer fail on one named part, to see the rest carry on; release
        // builds have no such switch.
        #[cfg(debug_assertions)]
        if std::env::var("DUTCHMAN_TEST_FAIL_MODEL").is_ok_and(|name| name == model.name) {
            return Err(Error::Restate("injected failure".into()));
        }
        match self {
            Self::Configured { writer, .. } => writer.write(model, out, report),
            Self::SolidCraft(writer) => writer.write(model, out, report),
            Self::FreeCad(writer) => writer.write(model, out, report),
            Self::Dxf => Err(Error::Encode("DXF page output uses dutchman convert --to dxf; convert-all and the single-file Writer are not supported yet".into())),
        }
    }
}

/// How a design is converted.
#[derive(Clone, Debug, clap::Args)]
pub struct ConvertOptions {
    /// Part positions: smart, last-shown, or first-placed.
    #[arg(long, value_name = "MODE", default_value_t = dutchman_core::PartPositions::Smart)]
    pub part_positions: dutchman_core::PartPositions,
    /// Maximum assembly part workers (0: automatic, at most two; 1: serial), capped by free memory.
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub threads: usize,
    /// Append an anonymous live diagnostic log, including progress while a conversion runs.
    #[arg(long, value_name = "PATH")]
    pub diagnostics: Option<PathBuf>,
    /// Save a diagnostic log in the platform's default folder, keeping the newest 50.
    #[arg(long, conflicts_with = "diagnostics")]
    pub diagnostics_default: bool,
    /// Private salt shared with an anonymous report when one is requested.
    #[arg(skip)]
    pub diagnostics_salt: Option<anon::Salt>,
    /// Concurrent conversions selected by the front end.
    #[arg(skip = 1_usize)]
    pub diagnostics_threads: usize,
    /// Output format: SolidCraft (default), a FreeCAD macro/document (experimental),
    /// or partial TechDraw DXF pages (convert command only).
    #[arg(long, value_enum, default_value = "solidcraft")]
    pub to: OutputFormat,
    /// What to keep: fast (the shape only), standard (history, sketches and joints, with
    /// history left out when it would not fit in free memory) or full.
    #[arg(long, value_enum, default_value = "standard")]
    pub preset: choices::Preset,
    /// Leave feature history out; keep geometry and the preset's reference sketches.
    #[arg(long, visible_alias = "no-history")]
    pub import_only: bool,
    /// Leave sketches out.
    #[arg(long)]
    pub no_sketches: bool,
    /// Leave assembly joints out.
    #[arg(long)]
    pub no_joints: bool,
    /// Place parts where they were first placed, ignoring later moves
    #[arg(long, conflicts_with = "part_positions")]
    pub stored_placements: bool,
    /// Convert and check history even when their estimates exceed free memory.
    #[arg(long)]
    pub no_memory_check: bool,
    /// The SolidCraft CLI that checks a written history against the original geometry
    /// (default: `$SOLIDCRAFT_CLI`, else `solidcraft-cli` on `PATH`, beside this program, or inside an
    /// installed SolidCraft app).
    #[arg(long, value_name = "PATH", conflicts_with = "no_verify")]
    pub solidcraft: Option<PathBuf>,
    /// Made for the current SolidCraft (any release from the last two weeks); older
    /// versions get simplified features, or pass --solidcraft-version x.y.z (at least
    /// 0.18.2). Defaults to the found CLI's version, else the newest known at build time,
    /// even with --no-verify. Default output uses .sc from 0.25.1, else .solidcraft.
    #[arg(long, value_name = "x.y.z")]
    pub solidcraft_version: Option<SolidCraftVersion>,
    /// Write history without checking it in SolidCraft.
    #[arg(long)]
    pub no_verify: bool,
    /// Also write `<design>.replay.json` beside a SolidCraft design: the command script that
    /// rebuilds its history step by step, for `solidcraft-cli run`.
    #[arg(long)]
    pub write_replay: bool,
    /// Check FreeCAD macro output by running FreeCAD headlessly.
    #[arg(long, conflicts_with = "no_verify")]
    pub verify: bool,
    /// Seconds FreeCAD may run before it is killed.
    #[arg(long, value_name = "SECONDS", default_value_t = 300)]
    pub freecad_timeout: u64,
    /// Refuse to replace existing output files (explicit outputs otherwise replace).
    #[arg(long)]
    pub no_clobber: bool,
    /// Seconds each SolidCraft check may run before the history is written unverified.
    #[arg(long, value_name = "SECONDS", default_value_t = DEFAULT_TIMEOUT.as_secs())]
    pub verify_timeout: u64,
    /// The project folder an assembly's part files are searched in when they are not at their
    /// stored paths (default: the nearest folder above the assembly holding an Inventor .ipj,
    /// else two folders up; for convert-all, each folder given).
    #[arg(long, value_name = "DIR")]
    pub project: Option<PathBuf>,
    /// Another folder to search, with its subfolders, for part files the project folder does
    /// not hold; may be given more than once.
    #[arg(long = "search", value_name = "DIR")]
    pub search: Vec<PathBuf>,
    /// Convert only this part of the design (set by `convert --part`).
    #[arg(skip)]
    pub part: Option<dutchman_core::PartSelection>,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            threads: 0,
            diagnostics: None,
            diagnostics_default: false,
            diagnostics_salt: None,
            diagnostics_threads: 1,
            to: OutputFormat::default(),
            preset: choices::Preset::default(),
            import_only: false,
            no_sketches: false,
            no_joints: false,
            stored_placements: false,
            part_positions: dutchman_core::PartPositions::Smart,
            no_memory_check: false,
            solidcraft: None,
            solidcraft_version: None,
            no_verify: false,
            write_replay: false,
            verify: false,
            freecad_timeout: 300,
            no_clobber: false,
            verify_timeout: DEFAULT_TIMEOUT.as_secs(),
            project: None,
            search: Vec::new(),
            part: None,
        }
    }
}

impl ConvertOptions {
    /// How these options check written designs in SolidCraft.
    pub fn verification(&self) -> Verification {
        match (self.no_verify, &self.solidcraft) {
            (true, _) => Verification::Off,
            (false, Some(path)) => Verification::With(path.clone()),
            (false, None) => Verification::Auto,
        }
    }

    /// The CLI version for reports, avoiding discovery for an explicit unchecked target.
    pub(crate) fn reported_solidcraft_version(&self) -> Option<String> {
        let detection = self
            .solidcraft
            .as_ref()
            .map_or(Verification::Auto, |path| Verification::With(path.clone()));
        if self.no_verify && self.solidcraft_version.is_some() {
            dutchman_solidcraft::cached_solidcraft_version(&detection)
        } else {
            dutchman_solidcraft::solidcraft_version(&detection)
        }
    }

    /// Whether reports look for SolidCraft at all: not when verification is off and the target
    /// release is given, since then nothing may run solidcraft-cli.
    pub(crate) fn searches_for_solidcraft(&self) -> bool {
        !(self.no_verify && self.solidcraft_version.is_some())
    }

    /// How the SolidCraft CLI search went, for reports; `with_paths` adds the local paths.
    pub(crate) fn solidcraft_search(&self, with_paths: bool) -> Value {
        let detection = self
            .solidcraft
            .as_ref()
            .map_or(Verification::Auto, |path| Verification::With(path.clone()));
        dutchman_solidcraft::search_report(&detection, with_paths)
    }

    /// The default output suffix, respecting the writer's target release.
    pub fn output_extension(&self) -> &'static str {
        if self.to == OutputFormat::Solidcraft {
            self.output_writer().extension()
        } else {
            self.to.extension()
        }
    }

    /// What these options keep: their preset, narrowed by the per-choice switches.
    pub fn choices(&self) -> choices::Choices {
        let mut choices = choices::Choices::of(self.preset);
        choices.history &= !self.import_only;
        choices.sketches &= !self.no_sketches;
        choices.joints &= !self.no_joints;
        choices
    }

    /// The legacy switch is an alias of first-placed.
    pub fn effective_part_positions(&self) -> dutchman_core::PartPositions {
        if self.stored_placements {
            dutchman_core::PartPositions::FirstPlaced
        } else {
            self.part_positions
        }
    }

    /// Where these options say to look for an assembly's part files.
    pub fn part_search(&self) -> PartSearch {
        PartSearch {
            threads: self.threads,
            part_positions: self.effective_part_positions(),
            project: self.project.clone(),
            also: self.search.clone(),
        }
    }

    /// The writer these options describe, retaining options for per-input batch preparation.
    pub fn writer(&self) -> OutputWriter {
        OutputWriter::Configured {
            writer: Box::new(self.output_writer()),
            options: Box::new(self.clone()),
        }
    }

    fn output_writer(&self) -> OutputWriter {
        match self.to {
            OutputFormat::Dxf => OutputWriter::Dxf,
            OutputFormat::Solidcraft => OutputWriter::SolidCraft(
                SolidCraftWriter::new()
                    .threads(self.threads)
                    .import_only(!self.choices().history)
                    .sketches(self.choices().sketches)
                    .joints(self.choices().joints)
                    .verification(self.verification())
                    .target_version(self.solidcraft_version)
                    .timeout(Duration::from_secs(self.verify_timeout)),
            ),
            OutputFormat::Freecad | OutputFormat::Fcstd => {
                OutputWriter::FreeCad(dutchman_freecad::run::FreeCadRunner::new(
                    dutchman_freecad::FreeCadWriter::new().import_only(!self.choices().history),
                    self.to == OutputFormat::Fcstd,
                    self.verify,
                    Duration::from_secs(self.freecad_timeout),
                ))
            }
        }
    }
}

/// A writer with the history decision fixed before decoding.
#[derive(Clone, Debug)]
pub struct PreparedWriter {
    writer: OutputWriter,
    /// Geometry only, for Standard when what the read really took leaves no room for history.
    without_history: Option<OutputWriter>,
    memory_note: Option<String>,
    solidcraft: Option<Value>,
    /// Whether to stop, once the design is read, when the machine cannot finish converting it.
    memory_stop: bool,
    check_history_memory: bool,
    history_check_bytes: Option<u64>,
}

impl Writer for PreparedWriter {
    fn extension(&self) -> &'static str {
        self.writer.extension()
    }

    fn accepts_extension(&self, extension: &str) -> bool {
        self.writer.accepts_extension(extension)
    }

    fn write(
        &self,
        model: &Model,
        out: &mut dyn std::io::Write,
        report: &mut Report,
    ) -> Result<(), Error> {
        dutchman_solidcraft::watch_check_memory(self.check_history_memory);
        let (held, available) = if self.memory_stop || self.check_history_memory {
            measured_memory()
        } else {
            (None, None)
        };
        if self.memory_stop {
            if let Some(message) = stop::check(model, held, available) {
                let Some(writer) = &self.without_history else {
                    return Err(Error::MemoryBudget(message));
                };
                report.push(
                    dutchman_core::Severity::Info,
                    "history",
                    format!("feature history left out: {message}; use the full preset to keep it"),
                );
                return writer.write(model, out, report);
            }
        }
        if let Some(note) = &self.memory_note {
            report.push(dutchman_core::Severity::Info, "memory", note.clone());
        }
        if self.check_history_memory {
            if let (OutputWriter::SolidCraft(writer), Some(needed)) =
                (&self.writer, self.history_check_bytes)
            {
                let writer = writer.clone().history_check_memory(needed, available);
                writer.write(model, out, report)?;
                if let Some((needed, available)) = writer.history_check_skipped() {
                    record_history_check_skip(report, needed, available);
                }
                return Ok(());
            }
        }
        self.writer.write(model, out, report)
    }
}

fn record_history_check_skip(report: &mut Report, needed: u64, available: u64) {
    report.skip_history_check(needed, available);
    let message = stop::history_check_message(needed, available);
    if let Some(outcome) = report.outcome_mut() {
        if outcome.check == dutchman_core::Check::Verified {
            outcome.check = dutchman_core::Check::Unverified(message.clone());
        }
    }
    if !report
        .losses()
        .iter()
        .any(|loss| loss.subject == "history" && loss.detail == message)
    {
        report.push(dutchman_core::Severity::Info, "history", message);
    }
}

/// What this process holds and what is still free, in bytes. Tests set both with
/// `DUTCHMAN_TEST_MEMORY=held,available`; release builds always measure.
fn measured_memory() -> (Option<u64>, Option<u64>) {
    #[cfg(debug_assertions)]
    if let Some((held, available)) = std::env::var("DUTCHMAN_TEST_MEMORY").ok().and_then(|text| {
        let (held, available) = text.split_once(',')?;
        Some((held.parse().ok()?, available.parse().ok()?))
    }) {
        return (Some(held), Some(available));
    }
    (
        worker::resident_bytes(std::process::id()),
        choices::available_memory(),
    )
}

impl ConvertOptions {
    /// Prepare the history choice from the pre-read estimate, without decoding.
    pub fn writer_for_input(&self, input: &Path) -> PreparedWriter {
        self.prepare_writer(input, None)
    }

    fn prepare_writer(&self, input: &Path, memory_note: Option<String>) -> PreparedWriter {
        let mut options = self.clone();
        let solidcraft = json::solidcraft_info(self);
        if let Some(target) = solidcraft
            .as_ref()
            .and_then(|value| value.get("target"))
            .and_then(Value::as_str)
        {
            options.solidcraft_version = target.parse().ok();
        }
        let memory_stop = !self.no_memory_check
            && self.preset != choices::Preset::Fast
            && self.part.is_none()
            && self.diagnostics_threads <= 1
            && self.to == OutputFormat::Solidcraft;
        let without_history = (memory_stop
            && self.choices().history
            && self.choices().history_within_budget)
            .then(|| {
                ConvertOptions {
                    import_only: true,
                    ..options.clone()
                }
                .output_writer()
            });
        PreparedWriter {
            writer: options.output_writer(),
            without_history,
            memory_note,
            solidcraft,
            history_check_bytes: (!self.no_memory_check
                && !self.no_verify
                && self.to == OutputFormat::Solidcraft)
                .then(|| stop::history_check_bytes(input, &options))
                .flatten(),
            check_history_memory: !self.no_memory_check
                && !self.no_verify
                && self.to == OutputFormat::Solidcraft,
            memory_stop,
        }
    }

    fn preflight(&self, input: &Path, available: Option<u64>) -> PreparedWriter {
        let note = if self.part.is_some() {
            None
        } else {
            choices::memory_check_with(input, self, available).warning()
        };
        self.prepare_writer(input, note)
    }
}

/// Convert with the shared memory policy, prepared writer, and estimate/choice report fields.
pub fn convert_options_with_progress(
    input: &Path,
    output: &Path,
    options: &ConvertOptions,
    replace: bool,
    search: &PartSearch,
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
) -> Value {
    convert_options_with_observer(input, output, options, replace, search, on_progress, None)
}

pub(crate) fn convert_options_with_observer(
    input: &Path,
    output: &Path,
    options: &ConvertOptions,
    replace: bool,
    search: &PartSearch,
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
    observer: Option<std::sync::Arc<dyn dutchman_core::MemberObserver>>,
) -> Value {
    if options.diagnostics.as_ref().is_some_and(|path| {
        path == output
            || std::fs::canonicalize(path)
                .ok()
                .zip(std::fs::canonicalize(output).ok())
                .is_some_and(|(a, b)| a == b)
    }) {
        return json::failure(
            json::CONVERT,
            json::input(input),
            "the diagnostic log would be replaced by the output; choose another --diagnostics path",
        );
    }
    diagnostics::run(input, options, on_progress, |callback, log| {
        let available = if options.no_memory_check || options.preset == choices::Preset::Fast {
            None
        } else {
            choices::available_memory()
        };
        let prepared = options.preflight(input, available);
        let progress = dutchman_core::Progress::new(callback)
            .with_members(log.map(diagnostics::Log::observer).or(observer));
        convert_prepared(input, output, options, replace, search, &progress, prepared)
    })
}

fn convert_prepared(
    input: &Path,
    output: &Path,
    options: &ConvertOptions,
    replace: bool,
    search: &PartSearch,
    progress: &dutchman_core::Progress<'_>,
    writer: PreparedWriter,
) -> Value {
    let converted = match options.part {
        Some(part) => dutchman_core::convert_part_with_progress(
            input, output, &writer, replace, search, part, progress,
        ),
        None => {
            dutchman_core::convert_with_progress(input, output, &writer, replace, search, progress)
        }
    };
    let mut report = match converted {
        Ok(report) => convert_success(input, output, &report),
        Err(err) => json::error(json::CONVERT, json::input(input), &err),
    };
    if options.write_replay && report.get("ok").and_then(Value::as_bool) == Some(true) {
        write_replay_script(output, &mut report);
    }
    if let (Some(solidcraft), Some(fields)) = (writer.solidcraft, report.as_object_mut()) {
        fields.insert("solidcraft".to_owned(), solidcraft);
    }
    json::conversion_choices(&mut report, input, options);
    report
}

/// Writes the command script that rebuilds the history of the SolidCraft design `output` to
/// `<output>.replay.json` (the file name with `.replay.json` in place of its suffix), and
/// names it in `report` as `replay_script`; a script that cannot be written is reported as
/// `replay_script_error`.
fn write_replay_script(output: &Path, report: &mut Value) {
    let result = (|| {
        let design =
            std::fs::read(output).map_err(|err| format!("cannot read the design: {err}"))?;
        let script = dutchman_solidcraft::replay_script(&design)?;
        let text = serde_json::to_string_pretty(&script).map_err(|err| err.to_string())?;
        let path = output.with_extension("replay.json");
        std::fs::write(&path, text)
            .map_err(|err| format!("cannot write {}: {err}", path.display()))?;
        Ok::<_, String>(path)
    })();
    if let Some(fields) = report.as_object_mut() {
        match result {
            Ok(path) => fields.insert(
                "replay_script".to_owned(),
                json!(path.display().to_string()),
            ),
            Err(why) => fields.insert("replay_script_error".to_owned(), json!(why)),
        };
    }
}

/// Converts `input` into `output`, writing nothing else, and returns the `dutchman.convert/1`
/// report, failed or not. An existing `output` is replaced only when `replace` is set. An
/// assembly's part files are looked for as `search` says (see [`dutchman_core::read_in`]).
pub fn convert(
    input: &Path,
    output: &Path,
    writer: &dyn Writer,
    replace: bool,
    search: &PartSearch,
) -> Value {
    match dutchman_core::convert_traced(input, output, writer, replace, search, &mut |_| {}) {
        Ok(report) => convert_success(input, output, &report),
        Err(err) => json::error(json::CONVERT, json::input(input), &err),
    }
}

/// Converts with ordered progress snapshots and a five-second heartbeat.
pub fn convert_with_progress(
    input: &Path,
    output: &Path,
    writer: &dyn Writer,
    replace: bool,
    search: &PartSearch,
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
) -> Value {
    let progress = dutchman_core::Progress::new(on_progress);
    match dutchman_core::convert_with_progress(input, output, writer, replace, search, &progress) {
        Ok(report) => convert_success(input, output, &report),
        Err(err) => json::error(json::CONVERT, json::input(input), &err),
    }
}

/// The `dutchman.convert/1` report of a conversion that succeeded.
pub fn convert_success(input: &Path, output: &Path, report: &Report) -> Value {
    let mut out = json::success(json::CONVERT, json::input(input), report);
    let format = if output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("FCMacro"))
    {
        dutchman_freecad::FORMAT
    } else if output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("FCStd"))
    {
        dutchman_freecad::run::DOCUMENT_FORMAT
    } else {
        dutchman_solidcraft::FORMAT
    };
    out.insert(
        "output".to_owned(),
        json!({"path": output.display().to_string(), "format": format}),
    );
    Value::Object(out)
}

/// What converting a file would carry across, found without writing anything.
pub struct Inspection {
    /// The losses, with the outcome converting would have.
    pub report: Report,
    /// The design's bodies and faces.
    pub bodies: usize,
    pub faces: usize,
    /// For an assembly: how many part files it reads, and how many placements it makes.
    pub assembly: Option<(usize, usize)>,
    /// Whether converting it would succeed: `Err` with why it would fail.
    pub conversion: Result<(), String>,
}

/// Reads `input`, looking for an assembly's part files as `search` says, and works out what
/// converting it would do, without writing anything.
pub fn inspect(input: &Path, search: &PartSearch) -> Result<Inspection, Error> {
    inspect_inner(input, search, None)
}

/// Inspects with read progress and a five-second heartbeat.
pub fn inspect_with_progress(
    input: &Path,
    search: &PartSearch,
    on_progress: &(dyn Fn(dutchman_core::ProgressEvent) + Send + Sync),
) -> Result<Inspection, Error> {
    let progress = dutchman_core::Progress::new(on_progress);
    progress.run(|| inspect_inner(input, search, Some(&progress)))
}

fn inspect_inner(
    input: &Path,
    search: &PartSearch,
    progress: Option<&dutchman_core::Progress<'_>>,
) -> Result<Inspection, Error> {
    if let Some(progress) = progress {
        progress.stage(dutchman_core::Stage::Input);
    }
    let mut report = Report::default();
    let format = Format::from_path(input).ok_or_else(|| Error::UnknownFormat {
        path: input.to_path_buf(),
    })?;
    let model = dutchman_core::read_with_progress(format, input, search, &mut report, progress)?;
    if let Some(progress) = progress {
        progress.stage(dutchman_core::Stage::Map);
    }
    let writer = SolidCraftWriter::new();
    let conversion = writer.convertible(&model);
    let outcome = writer.plan(&model, &mut report);
    report.set_outcome(outcome);
    Ok(Inspection {
        conversion,
        report,
        bodies: model.ir.model.bodies.len(),
        faces: model.ir.model.faces.len(),
        assembly: model
            .assembly
            .as_ref()
            .map(|assembly| (assembly.parts.len(), assembly.placements.len())),
    })
}

/// The `dutchman.inspect/1` report of inspecting `input`, failed or not.
pub fn inspect_report(input: &Path, search: &PartSearch) -> Value {
    match inspect(input, search) {
        Ok(inspection) => inspection.json(input),
        Err(err) => json::error(json::INSPECT, json::input(input), &err),
    }
}

impl Inspection {
    /// The `dutchman.inspect/1` report.
    pub fn json(&self, input: &Path) -> Value {
        let mut out = json::success(json::INSPECT, json::input(input), &self.report);
        out.insert(
            "geometry".to_owned(),
            json!({"bodies": self.bodies, "faces": self.faces}),
        );
        out.insert(
            "conversion".to_owned(),
            match &self.conversion {
                Ok(()) => json!({"possible": true, "reason": null}),
                Err(reason) => json!({"possible": false, "reason": reason}),
            },
        );
        if let Some((parts, placements)) = self.assembly {
            out.insert(
                "assembly".to_owned(),
                json!({"parts": parts, "placements": placements}),
            );
        }
        Value::Object(out)
    }
}

#[cfg(test)]
mod option_tests {
    use super::*;

    #[test]
    fn a_skipped_final_assembly_check_cannot_claim_overall_verification() {
        use dutchman_core::{Check, Mode, Outcome};
        for (before, after) in [
            (
                Check::Verified,
                Check::Unverified(stop::history_check_message(9_000_000_000, 2_000_000_000)),
            ),
            (
                Check::Partial("retained prefix unverified".into()),
                Check::Partial("retained prefix unverified".into()),
            ),
        ] {
            let mut report = Report::default();
            report.set_outcome(Outcome {
                mode: Mode::PartialHistory,
                check: before,
                features: Vec::new(),
            });
            record_history_check_skip(&mut report, 9_000_000_000, 2_000_000_000);
            assert_eq!(report.outcome().unwrap().check, after);
            assert_eq!(
                report.history_check_skipped(),
                Some((9_000_000_000, 2_000_000_000))
            );
        }
    }

    #[test]
    fn memory_policy_never_enables_a_check_the_user_disabled() {
        let input = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../dutchman-core/tests/fixtures/generated_unit_cube.f3d");
        for (no_verify, no_memory_check, guarded) in [
            (false, false, true),
            (true, false, false),
            (false, true, false),
            (true, true, false),
        ] {
            let options = ConvertOptions {
                no_verify,
                no_memory_check,
                ..Default::default()
            };
            let prepared = options.prepare_writer(&input, None);
            assert_eq!(prepared.check_history_memory, guarded);
            assert_eq!(prepared.history_check_bytes.is_some(), guarded);
        }
    }

    #[test]
    fn standard_can_fall_back_to_geometry_and_the_others_cannot() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("synthetic.ipt");
        std::fs::write(&input, b"synthetic").unwrap();
        let writer = |preset| {
            ConvertOptions {
                preset,
                ..Default::default()
            }
            .preflight(&input, None)
        };
        let standard = writer(choices::Preset::Standard);
        assert!(standard.memory_stop && standard.without_history.is_some());
        let full = writer(choices::Preset::Full);
        assert!(full.memory_stop && full.without_history.is_none());
        let fast = writer(choices::Preset::Fast);
        assert!(!fast.memory_stop && fast.without_history.is_none());
    }
}
