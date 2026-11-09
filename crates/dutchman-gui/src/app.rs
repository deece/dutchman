//! The window: designs in, an output folder, Convert, and what became of each file.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use dutchman::batch::{Event, Job};
use dutchman::choices::{estimate_text, Choices, Estimate, Preset};
use dutchman::present::FileView;
use dutchman::report_policy::{MessageFacts, ReportPolicy};
use dutchman::{json, present, ConvertOptions};
use eframe::egui::{
    self, Align, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Stroke, TextStyle,
    ThemePreference,
};
use serde_json::Value;

use crate::platform;
use crate::theme::{self, Palette};
use crate::worker::{self, Message, Pick, Run};

pub use dutchman::present::Progress as Status;

/// One file of a run.
pub struct Row {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub status: Status,
    pub progress: Option<dutchman_core::ProgressEvent>,
    /// Its `dutchman.convert/1` report, once it has one.
    pub report: Option<Value>,
    /// The job that converts it; none for an input that could not be planned.
    pub job: Option<Job>,
    /// The file's size once it has finished, for the steps to send it privately.
    pub size: Option<u64>,
}

/// The "last shown" Part positions choice, naming the program when every queued design comes from one.
fn last_shown_label(inputs: &[PathBuf]) -> String {
    let mut families = inputs
        .iter()
        .map(|input| dutchman_core::Format::from_path(input).map(|format| format.family()));
    if let Some(Some(first @ ("Fusion" | "Inventor" | "SolidWorks" | "FreeCAD"))) = families.next()
    {
        if families.all(|family| family == Some(first)) {
            return format!("As {first} last showed them");
        }
    }
    "As last shown in the original program".to_owned()
}

impl Row {
    fn view(&self) -> FileView {
        let mut view = FileView::new(
            &self.input,
            self.output.as_deref(),
            self.status,
            self.report.as_ref(),
        );
        if view.capability.is_none() {
            view.capability = dutchman_core::Format::from_path(&self.input)
                .map(|format| format.status_for(&self.input));
        }
        view
    }
}

/// Dutchman Convert's window.
pub struct App {
    consent: crate::report_consent::Consent,
    private_send: Option<(usize, Option<PathBuf>)>,
    pub updater: crate::update::Updater,
    diagnosis: Option<crate::diagnose::Wizard>,
    inputs: Vec<PathBuf>,
    doctor: Option<crate::doctor::Page>,
    project_summary: String,
    has_inventor: bool,
    custom: bool,
    estimate_key: Option<EstimateKey>,
    size_inputs: Option<Vec<PathBuf>>,
    sizes: Vec<(PathBuf, Option<dutchman::choices::Sizes>)>,
    estimate_generation: u64,
    estimates: Vec<(PathBuf, Option<Estimate>)>,
    available: Option<u64>,
    pending: Option<PendingRun>,
    out_dir: Option<PathBuf>,
    options: ConvertOptions,
    diagnostic_path: Option<PathBuf>,
    diagnostic_stopped: bool,
    diagnostic_automatic: bool,
    row_logs: std::collections::BTreeMap<usize, PathBuf>,
    /// The AI prompts shown on result cards, by row.
    ai_prompts: std::collections::BTreeMap<usize, AiPrompt>,
    /// Where run logs go instead of the platform's diagnostics folder (tests only).
    diagnostics_folder: Option<PathBuf>,
    unfinished_diagnostic: Option<PathBuf>,
    /// The log of the unfinished run last dismissed, so its notice does not come back.
    dismissed_unfinished: Option<PathBuf>,
    rows: Vec<Row>,
    /// Rows before the run's first job: inputs that could not be planned.
    offset: usize,
    run: Option<Run>,
    messages: Receiver<Message>,
    sender: Sender<Message>,
    /// A one-line notice, such as an app that could not be opened.
    notice: Option<String>,
    has_app: bool,
    /// The SolidCraft CLI that checks histories when nothing else is chosen, if one is found.
    checker: Result<PathBuf, dutchman_solidcraft::NoCli>,
    solidcraft_open: bool,
    /// Where the search for `solidcraft-cli` looked, when it found none.
    search_note: String,
    /// Where the header ends, so its windows open below its controls.
    header_bottom: f32,
    /// Why the file last chosen as the `solidcraft-cli` cannot be used.
    solidcraft_cli_error: Option<String>,
    found_version: Option<String>,
    detected_version: dutchman_solidcraft::SolidCraftVersion,
    /// What to scroll into view on the next frame, so the results are seen without looking
    /// for them.
    reveal: Option<Reveal>,
    /// Where a failed file can be sent privately.
    report_policy: crate::report_policy::Refresher,
    /// The salt of the results shown: their diagnostic log and anonymous reports share it,
    /// so every report of a file gives it the one id.
    salt: Option<dutchman::anon::Salt>,
    /// Shows a file in the system's file manager.
    file_manager: fn(&Path) -> Result<(), String>,
    /// Frames left to size and centre the window once the monitor is known; none once done or
    /// when a saved window was restored.
    fit_frames: Option<u8>,
    /// Whether the main screen's page needed to scroll in the last frame.
    page_scrolls: bool,
}

#[derive(PartialEq)]
struct EstimateKey {
    inputs: Vec<PathBuf>,
    choices: Choices,
    check: bool,
    no_memory_check: bool,
    to: dutchman::OutputFormat,
    threads: usize,
    available: Option<u64>,
}

struct PendingRun {
    options: ConvertOptions,
    kind: RunKind,
}

enum RunKind {
    Batch {
        inputs: Vec<PathBuf>,
        out_dir: PathBuf,
    },
    Retry {
        job: Box<Job>,
        index: usize,
    },
}

const SETTINGS_KEY: &str = "settings";

fn preset_label(preset: Preset) -> &'static str {
    match preset {
        Preset::Fast => "Fast",
        Preset::Standard => "Standard",
        Preset::Full => "Full",
    }
}

/// Selecting a preset clears the narrowing switches from the previous selection.
fn select_preset(options: &mut ConvertOptions, preset: Preset) {
    options.preset = preset;
    options.import_only = false;
    options.no_sketches = false;
    options.no_joints = false;
}

fn set_choices(options: &mut ConvertOptions, history: bool, sketches: bool, joints: bool) {
    options.import_only = !history;
    options.no_sketches = !sketches;
    options.no_joints = !joints;
}

/// Older settings only held import_only; keep its meaning under Standard.
fn restore_choices(options: &mut ConvertOptions, settings: &Value) -> bool {
    let preset = match settings.get("preset").and_then(Value::as_str) {
        Some("fast") => Preset::Fast,
        Some("full") => Preset::Full,
        _ => Preset::Standard,
    };
    select_preset(options, preset);
    // Every run keeps its log; an older saved "save_diagnostics" choice is ignored.
    options.diagnostics_default = true;
    options.import_only = settings
        .get("import_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    options.no_sketches = settings
        .get("no_sketches")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    options.no_verify = settings
        .get("no_verify")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    options.part_positions = settings
        .get("part_positions")
        .and_then(Value::as_str)
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| {
            if settings.get("stored_placements").and_then(Value::as_bool) == Some(true) {
                dutchman_core::PartPositions::FirstPlaced
            } else {
                dutchman_core::PartPositions::Smart
            }
        });
    options.stored_placements = false;
    options.no_joints = settings
        .get("no_joints")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    settings
        .get("custom")
        .and_then(Value::as_bool)
        .unwrap_or(options.import_only || options.no_sketches || options.no_joints)
}

/// Where the settings keep the `solidcraft-cli` a person chose.
const SOLIDCRAFT_CLI_KEY: &str = "solidcraft_cli";

/// How to find the CLI: the one a person chose, else the automatic search.
fn checking_with(chosen: Option<&PathBuf>) -> dutchman_solidcraft::Verification {
    chosen.map_or(dutchman_solidcraft::Verification::Auto, |cli| {
        dutchman_solidcraft::Verification::With(cli.clone())
    })
}

/// The release `checking` finds, and the target it means, capped at the newest known.
fn found_solidcraft(
    checking: &dutchman_solidcraft::Verification,
) -> (Option<String>, dutchman_solidcraft::SolidCraftVersion) {
    let found = dutchman_solidcraft::solidcraft_version(checking)
        .and_then(|line| dutchman::json::solidcraft_release(&line));
    let detected = found
        .as_deref()
        .and_then(|version| {
            version
                .parse::<dutchman_solidcraft::SolidCraftVersion>()
                .ok()
        })
        .unwrap_or_default()
        .min(Default::default());
    (found, detected)
}

/// Where the search for `solidcraft-cli` looked and what it found, in plain words, when no
/// release was found; empty otherwise.
fn search_note(checking: &dutchman_solidcraft::Verification, found: Option<&str>) -> String {
    if found.is_some() || *checking != dutchman_solidcraft::Verification::Auto {
        return String::new();
    }
    places_note(&dutchman_solidcraft::search_report(checking, false))
}

/// The "Looked in" text for a search report.
fn places_note(report: &Value) -> String {
    let places = report["places"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|place| {
            let outcome = match place["outcome"].as_str() {
                Some("unset") => "not set",
                Some("set_but_missing") => "set, but the file is missing",
                Some("not_found") => "not found",
                _ => "found",
            };
            format!(
                "• {}: {outcome}",
                place["place"].as_str().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    // One place per line, so the list reads at a glance.
    format!(
        "Looked in:\n{places}\nTo use another copy, choose it below or set the {} environment \
         variable to its full path.",
        dutchman_solidcraft::CLI_ENV
    )
}

fn saved_choices(options: &ConvertOptions, custom: bool) -> Value {
    serde_json::json!({
        "preset": options.preset.name(), "custom": custom,
        "import_only": options.import_only,
        "no_sketches": options.no_sketches, "no_joints": options.no_joints,
        "no_verify": options.no_verify,
        "part_positions": options.effective_part_positions().name(),
    })
}

fn gigabytes(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000_000.0
}

/// How many frames to wait for the monitor's size before leaving the window as it opened.
const FIT_FRAMES: u8 = 30;

/// The most of the Designs list shown before it scrolls, in a wide window.
const INPUT_LIST_HEIGHT: f32 = 140.0;

/// What to scroll into view.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Reveal {
    /// The results' heading, as a run starts.
    Results,
    /// The first file that needs attention, as a run ends.
    FirstIssue,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let (sender, messages) = channel();
        let diagnostics_folder = test_diagnostics_folder();
        let stored = cc
            .storage
            .and_then(|storage| storage.get_string(SETTINGS_KEY))
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let dismissed_unfinished = stored
            .as_ref()
            .and_then(|settings| settings.get(DISMISSED_UNFINISHED_KEY))
            .and_then(Value::as_str)
            .map(PathBuf::from);
        // A chosen CLI that has since moved falls back to the automatic search.
        let chosen_cli = stored
            .as_ref()
            .and_then(|settings| settings.get(SOLIDCRAFT_CLI_KEY))
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|cli| cli.is_file());
        // A remembered file that no longer answers as a SolidCraft CLI (an uninstalled or
        // replaced copy) must not hide the one the automatic search finds.
        let chosen_cli = chosen_cli.filter(|cli| {
            dutchman_solidcraft::solidcraft_version(&checking_with(Some(cli))).is_some()
        });
        let checking_with = checking_with(chosen_cli.as_ref());
        let (found_version, detected_version) = found_solidcraft(&checking_with);
        let search_note = search_note(&checking_with, found_version.as_deref());
        let mut options = ConvertOptions {
            solidcraft: chosen_cli,
            solidcraft_version: Some(detected_version),
            ..ConvertOptions::default()
        };
        let custom = stored
            .as_ref()
            .is_some_and(|settings| restore_choices(&mut options, settings));
        App {
            updater: crate::update::Updater::new(stored.as_ref().and_then(|v| v.get("update"))),
            inputs: Vec::new(),
            doctor: None,
            project_summary: String::new(),
            has_inventor: false,
            custom,
            estimate_key: None,
            size_inputs: None,
            sizes: Vec::new(),
            estimate_generation: 0,
            estimates: Vec::new(),
            available: None,
            pending: None,
            out_dir: None,
            options,
            diagnosis: None,
            diagnostic_path: None,
            diagnostic_stopped: false,
            diagnostic_automatic: false,
            row_logs: Default::default(),
            ai_prompts: Default::default(),
            diagnostics_folder: diagnostics_folder.clone(),
            unfinished_diagnostic: undismissed(
                unfinished_diagnostic(diagnostics_folder.as_deref()),
                dismissed_unfinished.as_deref(),
            ),
            dismissed_unfinished,
            rows: Vec::new(),
            offset: 0,
            run: None,
            messages,
            sender,
            notice: None,
            has_app: platform::has_solidcraft_app(),
            checker: dutchman_solidcraft::usable_cli(&checking_with),
            solidcraft_open: false,
            search_note,
            header_bottom: 72.0,
            solidcraft_cli_error: None,
            found_version,
            detected_version,
            reveal: None,
            report_policy: crate::report_policy::Refresher::new(
                stored.as_ref().and_then(|v| v.get("report_policy")),
            ),
            private_send: None,
            consent: crate::report_consent::Consent {
                ids: stored
                    .as_ref()
                    .and_then(|v| v.get("report_ids"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                card: None,
                #[cfg(feature = "test-support")]
                test_transport: None,
            },
            salt: None,
            file_manager: platform::reveal,
            fit_frames: cc
                .storage
                .is_some_and(|storage| storage.get_string("window").is_none())
                .then_some(FIT_FRAMES),
            page_scrolls: false,
        }
    }

    /// Replaces the system's file manager, so a test sees what would be shown.
    pub fn set_file_manager(&mut self, reveal: fn(&Path) -> Result<(), String>) {
        self.file_manager = reveal;
    }

    /// Writes the diagnostic log of the next runs to `path`.
    pub fn set_diagnostic_log(&mut self, path: PathBuf) {
        self.options.diagnostics = Some(path);
    }

    /// Supplies a result row's size without a file of that size, for UI previews.
    pub fn preview_size(&mut self, index: usize, size: u64) {
        if let Some(row) = self.rows.get_mut(index) {
            row.size = Some(size);
        }
    }

    /// Supplies an installed release without requiring SolidCraft for UI previews.
    pub fn preview_solidcraft(&mut self, found: Option<&str>) {
        self.found_version = found.map(str::to_owned);
        self.detected_version = found.and_then(|v| v.parse().ok()).unwrap_or_default();
        self.detected_version = self.detected_version.min(Default::default());
        self.options.solidcraft_version = Some(self.detected_version);
    }

    /// Supplies a search that found nothing, with these `(place, outcome)` pairs (outcomes as in
    /// the search report), for UI previews.
    #[cfg(feature = "test-support")]
    pub fn preview_search_for_test(&mut self, places: &[(&str, &str)]) {
        let places: Vec<Value> = places
            .iter()
            .map(|(place, outcome)| serde_json::json!({"place": place, "outcome": outcome}))
            .collect();
        self.checker = Err(dutchman_solidcraft::NoCli::NotFound("Not found".into()));
        self.found_version = None;
        self.search_note = places_note(&serde_json::json!({"places": places}));
    }

    /// Shows the notice for a run that didn't finish, with its log at `path`, for UI previews.
    pub fn preview_unfinished_run(&mut self, path: PathBuf) {
        self.unfinished_diagnostic = Some(path);
    }

    /// Supplies a SolidCraft app without its CLI, whatever this machine has, for UI previews.
    pub fn preview_solidcraft_app(&mut self, found: bool) {
        self.has_app = found;
    }

    /// Checks histories with the `solidcraft-cli` a person chose, or searches for one again.
    /// A target left on Automatic follows the release this finds.
    pub fn set_solidcraft_cli(&mut self, cli: Option<PathBuf>) {
        let automatic = self.options.solidcraft_version == Some(self.detected_version);
        let checking = checking_with(cli.as_ref());
        self.options.solidcraft = cli;
        self.checker = dutchman_solidcraft::usable_cli(&checking);
        (self.found_version, self.detected_version) = found_solidcraft(&checking);
        if automatic {
            self.options.solidcraft_version = Some(self.detected_version);
        }
        self.solidcraft_cli_error = None;
        self.search_note = search_note(&checking, self.found_version.as_deref());
    }

    /// Takes the file a person chose as the `solidcraft-cli`, or says why it cannot be used.
    pub fn choose_solidcraft_cli(&mut self, chosen: &Path) {
        match dutchman_solidcraft::chosen_cli(chosen) {
            Ok(cli) => self.set_solidcraft_cli(Some(cli)),
            Err(why) => self.solidcraft_cli_error = Some(why),
        }
    }

    /// Adds design files and folders to convert, each once.
    pub fn add_inputs(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for path in paths {
            if self.out_dir.is_none() {
                self.out_dir = Some(dutchman::batch::default_output(&path));
            }
            if !self.inputs.contains(&path) {
                self.inputs.push(path);
            }
        }
    }

    /// Sets the folder designs are written to.
    pub fn set_out_dir(&mut self, dir: PathBuf) {
        self.out_dir = Some(dir);
    }

    /// Whether written histories are checked in SolidCraft.
    pub fn set_verify(&mut self, verify: bool) {
        self.options.no_verify = !verify;
    }

    /// Injects report transport and an optional synthetic signature for UI tests only.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn set_report_transport_for_test(&mut self, endpoint: String, signature: Option<String>) {
        self.consent.test_transport = Some((endpoint, signature));
    }

    /// Supplies a design's read sizes and the free memory, so that its estimate is shown
    /// without allocating a huge design.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn set_sizes_for_test(
        &mut self,
        input: PathBuf,
        sizes: dutchman::choices::Sizes,
        available: u64,
    ) {
        self.sizes.retain(|(path, _)| *path != input);
        self.sizes.push((input, Some(sizes)));
        self.size_inputs = Some(self.inputs.clone());
        self.available = Some(available);
        self.estimate_key = None;
    }

    #[cfg(feature = "test-support")]
    pub fn set_checker_for_test(&mut self, found: bool) {
        self.checker = if found {
            Ok(PathBuf::from("solidcraft-cli"))
        } else {
            Err(dutchman_solidcraft::NoCli::NotFound("Not found".into()))
        };
    }

    /// Supplies a synthetic result without decoding a design for UI review.
    #[cfg(feature = "test-support")]
    pub fn preview_result_for_test(&mut self, row: Row) {
        self.rows = vec![row];
    }

    /// The folder this app keeps its run logs in under test.
    #[cfg(feature = "test-support")]
    pub fn diagnostics_folder_for_test(&self) -> Option<&Path> {
        self.diagnostics_folder.as_deref()
    }

    /// The rows of the current or last run.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Sets the project folder assemblies' part files are searched in; `None` finds it.
    pub fn set_project(&mut self, project: Option<PathBuf>) {
        self.options.project = project;
    }

    /// Converts the row at `index` again, searching `folder` too for its assemblies' part
    /// files.
    pub fn search_for_row(&mut self, index: usize, folder: PathBuf, ctx: &egui::Context) {
        if self.running() || self.updater.blocks_conversions() {
            return;
        }
        let Some(job_index) = index.checked_sub(self.offset) else {
            return;
        };
        let Some(row) = self.rows.get_mut(index) else {
            return;
        };
        let Some(job) = row.job.as_mut() else {
            return;
        };
        if !job.search.also.contains(&folder) {
            job.search.also.push(folder);
        }
        let job = job.clone();
        self.pending = Some(PendingRun {
            options: self.options.clone(),
            kind: RunKind::Retry {
                job: Box::new(job),
                index: job_index,
            },
        });
        self.start_checked(ctx);
    }

    /// The anonymous report of the file of the row at `index`, its id in it, and its convert
    /// report.
    fn anonymous(&self, index: usize) -> Result<Option<(Value, String, &Value)>, String> {
        let Some(report) = self.rows.get(index).and_then(|row| row.report.as_ref()) else {
            return Ok(None);
        };
        let salt = match &self.salt {
            Some(salt) => salt.clone(),
            None => dutchman::anon::Salt::random()?,
        };
        let out_dir = self.out_dir.clone().unwrap_or_default();
        let run = dutchman::anon::Run {
            verify: !self.options.no_verify,
            import_only: self.options.import_only,
            jobs: 1,
            ..dutchman::anon::Run::default()
        };
        let (anonymous, id) = present::anonymous_report(report, &out_dir, &salt, &run);
        Ok(Some((anonymous, id, report)))
    }

    fn open_consent(&mut self, index: usize) -> Result<(), String> {
        if self.salt.is_none() {
            self.salt = Some(dutchman::anon::Salt::random()?);
        }
        let Some((anonymous, id, report)) = self.anonymous(index)? else {
            return Ok(());
        };
        let converted = report.get("ok").and_then(Value::as_bool) == Some(true);
        let error = anonymous.pointer("/errors/0");
        let field = |key| {
            error
                .and_then(|e| e.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let signature = dutchman::diagnose::signature::Signature::new(
            dutchman::diagnose::signature::ErrorCode {
                outcome: "failed".into(),
                stage: field("stage"),
                kind: field("code"),
                codes: Vec::new(),
            },
            None,
        );
        let format = report
            .pointer("/input/path")
            .and_then(Value::as_str)
            .and_then(|p| Path::new(p).extension())
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        let mut folder = dutchman::report_send::local_folder(
            &self.out_dir.clone().unwrap_or_else(std::env::temp_dir),
            &id,
        );
        let signature = if converted {
            dutchman::report_send::CONVERTED_PROBLEM_SIGNATURE.to_owned()
        } else {
            signature.hash
        };
        // A later successful conversion must not reuse a previous failure's bundle.
        if converted {
            folder = folder.join("converted");
        }
        #[cfg(feature = "test-support")]
        let signature = self
            .consent
            .test_transport
            .as_ref()
            .and_then(|(_, signature)| signature.clone())
            .unwrap_or(signature);
        if let Some(row) = self.rows.get(index).filter(|_| !converted) {
            let name = row.input.file_name().unwrap_or_default().to_string_lossy();
            let saved = row
                .input
                .with_file_name(format!("{name}.diagnosis"))
                .join("diagnose")
                .join("repro");
            if saved.join("prepared.json").exists() {
                folder = saved;
            }
        }
        let prepared = if folder.join("prepared.json").exists() {
            dutchman::report_send::Prepared::load(&folder)?
        } else {
            dutchman::report_send::Prepared::new(&id, &format, &signature, None, &folder)?
        };
        let mut card = crate::report_consent::Card::new(index, prepared, folder);
        card.log = self.row_logs.get(&index).cloned();
        card.log_ready = !self.running();
        self.consent.card = Some(card);
        Ok(())
    }

    /// Opens the GitHub issue form about the file of the row at `index`, filled in from its
    /// report. When its anonymous report is too long for the link, it is copied for the
    /// reporter to paste, and the notice says so.
    pub fn report_on_github(&mut self, index: usize, ctx: &egui::Context) -> Result<(), String> {
        let Some((anonymous, _, report)) = self.anonymous(index)? else {
            return Ok(());
        };
        let link = present::report_link(report, &anonymous);
        let issue = link.issue;
        ctx.copy_text(issue.anonymous_report.clone());
        self.notice = Some(link.copy_notice.to_owned());
        ctx.open_url(egui::OpenUrl::new_tab(issue.url));
        Ok(())
    }

    /// Copies the AI prompt for the row at `index` and keeps it shown on its card.
    fn ask_ai(&mut self, index: usize, ctx: &egui::Context) -> Result<(), String> {
        let Some(row) = self.rows.get(index) else {
            return Ok(());
        };
        let Some(output) = row.output.as_deref() else {
            return Ok(());
        };
        let converted = row.status == Status::Converted;
        let text = ai_prompt(
            &row.input,
            output,
            converted,
            self.row_logs.get(&index).map(PathBuf::as_path),
        )?;
        ctx.copy_text(text.clone());
        self.ai_prompts.insert(
            index,
            AiPrompt {
                text,
                copied: std::time::Instant::now(),
            },
        );
        Ok(())
    }

    /// Copies the message about the file of the row at `index` and opens Dutchy's
    /// DM. A file small enough to drag in has its folder shown too.
    pub fn dm_dutchy(&mut self, index: usize, ctx: &egui::Context) -> Result<(), String> {
        self.dm_private(index, None, ctx)
    }

    fn dm_private(
        &mut self,
        index: usize,
        file: Option<&Path>,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        let Some((_, id, report)) = self.anonymous(index)? else {
            return Ok(());
        };
        let Some(row) = self.rows.get(index) else {
            return Ok(());
        };
        let policy = self.report_policy.policy();
        let size = file
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .or(row.size);
        let steps = present::SendSteps::new(policy, size, report, present::Surface::Desktop);
        let facts = MessageFacts {
            code: present::error_code(report),
            report_id: &id,
            size_bytes: size,
            step: present::step_known(report),
        };
        let mut message = if report.get("ok").and_then(Value::as_bool) == Some(true) {
            policy.converted_message(&facts)
        } else {
            policy.message(&facts)
        };
        if let Some(log) = self.row_logs.get(&index) {
            message.push_str(&format!(
                " Run log: {}. It contains no names or contents from the design.",
                log.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
        ctx.copy_text(message);
        ctx.open_url(egui::OpenUrl::new_tab(policy.dm_url()));
        let shown = if steps.drag {
            (self.file_manager)(file.unwrap_or(&row.input))
        } else {
            Ok(())
        };
        self.notice = Some(present::SEND_COPIED.to_owned());
        shown
    }

    fn private_send_window(&mut self, ctx: &egui::Context) {
        let Some((index, file)) = self.private_send.clone() else {
            return;
        };
        let mut open = true;
        let mut action = None;
        let log_ready = !self.running();
        egui::Window::new("Send privately")
            .open(&mut open)
            .show(ctx, |ui| {
                let Some(row) = self.rows.get(index) else {
                    return;
                };
                let Some(report) = row.report.as_ref() else {
                    return;
                };
                if let Some(log) = self.row_logs.get(&index) {
                    ui.label("This run's log contains no names or contents from the design. Its name is included in the copied DM; attach it if you choose.");
                    if let Err(message) = platform::log_controls(ui, log, log_ready) { self.notice = Some(message); }
                }
                if let Some(path) = &file {
                    ui.label(format!("Saved bundle: {}", path.display()));
                }
                let size = file
                    .as_ref()
                    .and_then(|p| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .or(row.size);
                let steps = present::SendSteps::new(
                    self.report_policy.policy(),
                    size,
                    report,
                    present::Surface::Desktop,
                );
                let palette = theme::palette(ui);
                action = send_ui(ui, palette, index, &steps, false);
            });
        if action.is_some() {
            self.notice = None;
        }
        let result = match action {
            Some(Action::Dm(index)) => self.dm_private(index, file.as_deref(), ctx),
            Some(Action::Join) => {
                ctx.open_url(egui::OpenUrl::new_tab(
                    self.report_policy.policy().invite_url.clone(),
                ));
                Ok(())
            }
            Some(Action::Report(index)) => self.report_on_github(index, ctx),
            Some(Action::Consent(index)) => {
                open = false;
                self.open_consent(index)
            }
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.notice = Some(error);
        }
        if !open {
            self.private_send = None;
        }
    }

    /// Whether a run is in progress.
    pub fn running(&self) -> bool {
        self.run.is_some() || self.pending.is_some()
    }

    fn refresh_blocked_checker(&mut self) {
        if matches!(self.checker, Err(dutchman_solidcraft::NoCli::Blocked(_))) {
            self.checker =
                dutchman_solidcraft::usable_cli(&checking_with(self.options.solidcraft.as_ref()));
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        if self.updater.blocks_conversions() {
            return;
        }
        self.refresh_blocked_checker();
        let Some(out_dir) = self.out_dir.clone() else {
            return;
        };
        self.pending = Some(PendingRun {
            options: self.options.clone(),
            kind: RunKind::Batch {
                inputs: self.inputs.clone(),
                out_dir,
            },
        });
        self.start_checked(ctx);
    }

    fn start_checked(&mut self, ctx: &egui::Context) {
        let Some(mut pending) = self.pending.take() else {
            return;
        };
        self.refresh_blocked_checker();
        self.notice = None;
        self.diagnostic_stopped = false;
        self.diagnostic_path = None;
        // A retry keeps its run's salt: the other rows' ids stay as they were given.
        let salt = match (&pending.kind, &self.salt) {
            (RunKind::Retry { .. }, Some(salt)) => salt.clone(),
            _ => match dutchman::anon::Salt::random() {
                Ok(salt) => salt,
                Err(message) => {
                    self.notice = Some(message);
                    return;
                }
            },
        };
        pending.options.diagnostics_salt = Some(salt.clone());
        self.salt = Some(salt);
        // Every run keeps a local log in the diagnostics folder, so that the AI prompt and a
        // report can name it.
        pending.options.diagnostics_default = true;
        self.diagnostic_automatic = pending.options.diagnostics.is_none();
        if let (Some(folder), None) = (&self.diagnostics_folder, &pending.options.diagnostics) {
            match dutchman::diagnostics::new_log_in(folder) {
                Ok(path) => pending.options.diagnostics = Some(path),
                Err(message) => {
                    self.notice = Some(message);
                    return;
                }
            }
        }
        if pending.options.diagnostics_default || pending.options.diagnostics.is_some() {
            let threads = match &pending.kind {
                RunKind::Batch { .. } => dutchman::batch::workers(None),
                RunKind::Retry { .. } => 1,
            };
            if let Err(message) = dutchman::diagnostics::prepare(&mut pending.options, threads) {
                self.notice = Some(message);
                return;
            }
            self.diagnostic_path = pending.options.diagnostics.clone();
            self.notice = self
                .diagnostic_path
                .as_deref()
                .and_then(dutchman::diagnostics::version_notice);
        }
        self.run = Some(match pending.kind {
            RunKind::Batch { inputs, out_dir } => {
                self.row_logs.clear();
                self.ai_prompts.clear();
                self.rows.clear();
                self.offset = 0;
                Run::start(
                    inputs,
                    out_dir,
                    &pending.options,
                    self.sender.clone(),
                    ctx.clone(),
                )
            }
            RunKind::Retry { job, index } => {
                self.row_logs.remove(&(self.offset + index));
                self.ai_prompts.remove(&(self.offset + index));
                if let Some(path) = &self.diagnostic_path {
                    self.row_logs.insert(self.offset + index, path.clone());
                }
                if let Some(row) = self.rows.get_mut(self.offset + index) {
                    row.status = Status::Waiting;
                    row.report = None;
                }
                Run::retry(
                    *job,
                    index,
                    &pending.options,
                    self.sender.clone(),
                    ctx.clone(),
                )
            }
        });
    }

    fn estimate_available(&self) -> Option<u64> {
        self.available
    }

    fn current_estimate_key(&self) -> EstimateKey {
        EstimateKey {
            inputs: self.inputs.clone(),
            choices: self.options.choices(),
            check: !self.options.no_verify && self.checker.is_ok(),
            no_memory_check: self.options.no_memory_check,
            to: self.options.to,
            threads: self.options.threads,
            available: self.estimate_available(),
        }
    }

    fn refresh_estimates(&mut self, ctx: &egui::Context) {
        let key = self.current_estimate_key();
        if self.estimate_key.as_ref() == Some(&key) {
            return;
        }
        if self.size_inputs.as_ref() != Some(&self.inputs) {
            self.estimate_generation = self.estimate_generation.wrapping_add(1);
            self.estimates.clear();
            self.sizes.clear();
            self.size_inputs = Some(self.inputs.clone());
            worker::estimates(
                key.inputs.clone(),
                self.options.clone(),
                self.checker.is_ok(),
                self.estimate_generation,
                self.sender.clone(),
                ctx.clone(),
            );
        } else if !self.sizes.is_empty() {
            self.estimates = self
                .sizes
                .iter()
                .map(|(path, sizes)| {
                    (
                        path.clone(),
                        sizes.and_then(|sizes| self.desktop_parts(sizes).map(|parts| parts.total)),
                    )
                })
                .collect();
        }
        self.estimate_key = Some(key);
    }

    fn desktop_parts(
        &self,
        sizes: dutchman::choices::Sizes,
    ) -> Option<dutchman::choices::DesktopEstimate> {
        dutchman::choices::desktop_estimate_parts(sizes, &self.options, self.checker.is_ok())
    }

    fn file_estimate_text(&self, path: &Path, estimate: Estimate) -> String {
        self.sizes
            .iter()
            .find(|(input, _)| input == path)
            .and_then(|(_, sizes)| *sizes)
            .and_then(|sizes| self.desktop_parts(sizes))
            .map_or_else(
                || estimate_text(estimate),
                dutchman::choices::desktop_estimate_text,
            )
    }

    fn picked(&mut self, pick: Pick, paths: Vec<PathBuf>, ctx: &egui::Context) {
        let Some(first) = paths.first().cloned() else {
            return;
        };
        match pick {
            Pick::Files | Pick::Folder => self.add_inputs(paths),
            Pick::Output => self.out_dir = Some(first),
            Pick::Project => self.options.project = Some(first),
            Pick::SolidCraftCli => self.choose_solidcraft_cli(&first),
            Pick::SearchFolder(row) => self.search_for_row(row, first, ctx),
            Pick::SearchFile(row) => {
                if let Some(folder) = first.parent() {
                    self.search_for_row(row, folder.to_path_buf(), ctx);
                }
            }
            Pick::SaveLog(row) => {
                if let Some(report) = self.rows.get(row).and_then(|row| row.report.as_ref()) {
                    let text = dutchman::json::full_log(report, &dutchman::batch::line(report));
                    if let Err(err) = std::fs::write(&first, text) {
                        self.notice = Some(format!("Cannot save {}: {err}", display(&first)));
                    }
                }
            }
        }
    }

    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.messages.try_recv() {
            match message {
                Message::ProjectSummary {
                    generation,
                    text,
                    inventor,
                } => {
                    if generation == self.estimate_generation {
                        self.project_summary = text;
                        self.has_inventor = inventor;
                    }
                }
                Message::Estimated {
                    generation,
                    files,
                    available,
                    sizes,
                } => {
                    if generation == self.estimate_generation {
                        self.estimates = files;
                        self.available = available;
                        if !sizes.is_empty() {
                            self.sizes = sizes;
                            self.estimate_key = None;
                        }
                    }
                }
                Message::Picked { pick, paths } => self.picked(pick, paths, ctx),
                Message::Planned { problems, jobs } => {
                    self.reveal = Some(Reveal::Results);
                    self.offset = problems.len();
                    self.rows = problems
                        .into_iter()
                        .map(|report| Row {
                            input: report
                                .pointer("/input/path")
                                .and_then(Value::as_str)
                                .map(PathBuf::from)
                                .unwrap_or_default(),
                            output: None,
                            status: Status::Failed,
                            report: Some(report),
                            progress: None,
                            job: None,
                            size: None,
                        })
                        .chain(jobs.into_iter().map(|job| Row {
                            input: job.input.clone(),
                            output: Some(job.output.clone()),
                            status: Status::Waiting,
                            progress: None,
                            report: None,
                            job: Some(job),
                            size: None,
                        }))
                        .collect();
                    if let Some(path) = &self.diagnostic_path {
                        self.row_logs = (0..self.rows.len())
                            .map(|index| (index, path.clone()))
                            .collect();
                    }
                }
                Message::Sized { input, size } => {
                    if let Some(row) = self.rows.iter_mut().find(|row| row.input == input) {
                        row.size = size;
                    }
                }
                Message::ReportSaveFailed(message) => self.notice = Some(message),
                Message::Progress(Event::Started(index)) => {
                    if let Some(row) = self.rows.get_mut(self.offset + index) {
                        row.status = Status::Converting;
                        row.progress = None;
                    }
                }
                Message::Progress(Event::Progress(index, event)) => {
                    if let Some(row) = self.rows.get_mut(self.offset + index) {
                        row.progress = Some(event);
                    }
                }
                Message::Progress(Event::Finished(index, report)) => {
                    self.ai_prompts.remove(&(self.offset + index));
                    if let Some(row) = self.rows.get_mut(self.offset + index) {
                        row.status = if report.get("ok") == Some(&Value::Bool(true)) {
                            Status::Converted
                        } else {
                            Status::Failed
                        };
                        row.report = Some(report);
                    }
                }
                Message::Done => {
                    self.run = None;
                    if let Some(path) = self.diagnostic_path.clone() {
                        let failed = self.row_logs.iter().any(|(index, log)| {
                            log == &path
                                && self
                                    .rows
                                    .get(*index)
                                    .is_some_and(|r| r.status != Status::Converted)
                        });
                        let result = if self.diagnostic_stopped {
                            "stopped"
                        } else if failed {
                            "failed"
                        } else {
                            "converted"
                        };
                        let final_path = if self.diagnostic_automatic {
                            dutchman::diagnostics::finish_default_log(&path, result)
                        } else {
                            path.clone()
                        };
                        for log in self.row_logs.values_mut().filter(|log| **log == path) {
                            *log = final_path.clone();
                        }
                        if let Some(card) = &mut self.consent.card {
                            if card.log.as_ref() == Some(&path) {
                                card.log = Some(final_path.clone());
                                card.log_ready = true;
                            }
                        }
                        self.diagnostic_path = Some(final_path);
                    }
                    self.reveal = Some(Reveal::FirstIssue);
                    for row in &mut self.rows {
                        if matches!(row.status, Status::Waiting | Status::Converting) {
                            row.status = Status::NotStarted;
                        }
                    }
                }
            }
        }
    }

    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if !dropped.is_empty() && !self.running() {
            self.add_inputs(dropped);
        }
    }

    /// Whether the main screen needs to scroll as it is: false when everything is in view.
    /// Results that do not fit scroll inside their own column and do not count.
    pub fn page_scrolls(&self) -> bool {
        self.page_scrolls
    }

    /// On the first frames of a window with no saved size, sizes it to fit the main screen and
    /// centres it on its monitor, once the monitor's size is known.
    fn fit_window(&mut self, ctx: &egui::Context) {
        let Some(left) = self.fit_frames else {
            return;
        };
        let (monitor, maximized) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.monitor_size,
                viewport.maximized.unwrap_or(false) || viewport.fullscreen.unwrap_or(false),
            )
        });
        match monitor {
            Some(monitor) if monitor.x > 0.0 && monitor.y > 0.0 => {
                self.fit_frames = None;
                if !maximized {
                    let monitor = [monitor.x, monitor.y];
                    let size = crate::fit_size(monitor);
                    let at = crate::centred_position(monitor, size);
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(at.into()));
                }
            }
            _ if left > 0 => {
                self.fit_frames = Some(left - 1);
                ctx.request_repaint();
            }
            _ => self.fit_frames = None,
        }
    }

    /// Draws the window, for eframe and for tests.
    pub fn ui(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| {
            input.pointer.any_pressed()
                || input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key { pressed: true, .. }
                            | egui::Event::AccessKitActionRequest(_)
                    )
                })
        }) {
            self.notice = None;
        }
        if ctx.input(|input| {
            input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(true)))
        }) {
            self.refresh_blocked_checker();
        }
        if self.updater.stopping_for_install() {
            self.pending = None;
            if let Some(run) = &self.run {
                run.stop();
                self.diagnostic_stopped = true;
            }
        }
        self.updater.tick(ctx);
        self.report_policy.tick(ctx, self.updater.automatic());
        self.receive(ctx);
        if self.diagnosis.as_mut().is_some_and(|wizard| wizard.ui(ctx)) {
            self.diagnosis = None;
        }
        self.take_dropped_files(ctx);
        self.refresh_estimates(ctx);
        let view = self.view();

        let header = egui::TopBottomPanel::top("title")
            .frame(
                Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(Margin::symmetric(
                        theme::SPACE_8 as i8,
                        theme::BAR_PADDING as i8,
                    )),
            )
            .show_separator_line(false)
            .show(ctx, |ui| self.title(ui));
        self.header_bottom = header.response.rect.bottom();

        if let Some(page) = &mut self.doctor {
            if page.ui(ctx) {
                self.doctor = None;
            }
            return;
        }

        egui::TopBottomPanel::bottom("conversion")
            .frame(
                Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(Margin::symmetric(
                        theme::SPACE_8 as i8,
                        theme::BAR_PADDING as i8,
                    )),
            )
            .show(ctx, |ui| self.actions_ui(ui, ctx, view.summary.as_deref()));

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(Margin {
                        left: theme::SPACE_8 as i8,
                        right: theme::SPACE_8 as i8,
                        top: 0,
                        bottom: theme::SPACE_2 as i8,
                    }),
            )
            .show(ctx, |ui| {
                ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
                if ui.available_width() + 2.0 * theme::SPACE_8 >= crate::WIDE_BREAKPOINT {
                    self.wide_page(ui, ctx, &view);
                } else {
                    self.narrow_page(ui, ctx, &view);
                }
            });
        if let Some(action) = self.consent.window(ctx) {
            self.notice = None;
            let result = match action {
                crate::report_consent::Action::Private(index, file) => {
                    self.private_send = Some((index, file));
                    Ok(())
                }
                crate::report_consent::Action::Github(index) => self.report_on_github(index, ctx),
            };
            if let Err(error) = result {
                self.notice = Some(error);
            }
        }
        self.private_send_window(ctx);
        self.updater.report_ids = self.consent.ids.clone();
        self.solidcraft_window(ctx);
        let can_restart =
            !self.running() && (!self.updater.windows_package() || self.diagnosis.is_none());
        self.updater.windows(ctx, can_restart, self.header_bottom);
        self.refresh_estimates(ctx);
    }

    /// The notice about a run that did not finish, above the main screen.
    /// After a run that didn't finish: one compact line, with the log's place on hover, so it
    /// never pushes the main screen into scrolling. Wide windows show it above the results.
    fn unfinished_notice(&mut self, ui: &mut egui::Ui) {
        if let Some(path) = self.unfinished_diagnostic.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.label("The last run didn't finish.")
                    .on_hover_text(platform::display_path(&path));
                if ui
                    .button("Open its log folder")
                    .on_hover_text(platform::display_path(&path))
                    .clicked()
                {
                    if let Err(message) = platform::reveal(&path) {
                        self.notice = Some(message);
                    }
                }
                if ui.button("Dismiss").clicked() {
                    self.dismissed_unfinished = self.unfinished_diagnostic.take();
                }
            });
            ui.add_space(theme::SPACE_4);
        }
    }

    /// A narrow window: one column, Designs, Output folder, then Results, scrolling as one page.
    fn narrow_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, view: &present::RunView) {
        let area = ScrollArea::vertical()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.unfinished_notice(ui);
                self.inputs_panel(ui);
                ui.add_space(theme::SPACE_4);
                self.output_panel(ui, ctx);
                if !self.rows.is_empty() {
                    ui.add_space(theme::SPACE_8);
                    self.results(ui, view, false);
                }
            });
        self.page_scrolls = area.content_size.y > area.inner_rect.height() + 1.0;
    }

    /// A wide window: Designs and Output folder on the left, Results on the right. The page
    /// scrolls only if the window is too short for the left column; results scroll in their own
    /// column.
    fn wide_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, view: &present::RunView) {
        let rect = ui.available_rect_before_wrap();
        let gap = theme::SPACE_8;
        let left_width = ((rect.width() - gap) * 0.54).round();
        let left = egui::Rect::from_min_size(rect.min, egui::vec2(left_width, rect.height()));
        let right = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + left_width + gap, rect.min.y),
            rect.max,
        );
        let mut scrolls = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
            let area = ScrollArea::vertical()
                .id_salt("main-left")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.inputs_panel(ui);
                    ui.add_space(theme::SPACE_4);
                    self.output_panel(ui, ctx);
                });
            scrolls = area.content_size.y > area.inner_rect.height() + 1.0;
        });
        ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
            self.unfinished_notice(ui);
            self.results(ui, view, true);
        });
        ui.advance_cursor_after_rect(rect);
        self.page_scrolls = scrolls;
    }

    fn title(&mut self, ui: &mut egui::Ui) {
        let narrow = ui.available_width() < 850.0;
        let compact = ui.available_width() < 1200.0;
        let two_rows = ui.available_width() < 1000.0;
        // Give the controls their own row before the title and toolbar compete for space.
        // The wide title includes a subtitle, so it needs more room than the compact title.
        if two_rows {
            ui.horizontal(|ui| self.title_brand(ui, compact));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                self.title_controls(ui, narrow);
            });
        } else {
            ui.horizontal(|ui| {
                self.title_brand(ui, compact);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.title_controls(ui, narrow);
                });
            });
        }
    }

    fn title_brand(&self, ui: &mut egui::Ui, compact: bool) {
        let p = theme::palette(ui);
        let mark = if ui.visuals().dark_mode {
            egui::include_image!("../assets/dutchman-mark-night.svg")
        } else {
            egui::include_image!("../assets/dutchman-mark.svg")
        };
        ui.add(egui::Image::new(mark).fit_to_exact_size(egui::vec2(40.0, 40.0)));
        ui.add_space(theme::SPACE_2);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.with_layout(Layout::left_to_right(Align::Max), |ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_2;
                if compact {
                    ui.label(RichText::new(crate::APP_NAME).heading());
                } else {
                    ui.label(RichText::new(crate::APP_NAME).text_style(theme::title()));
                }
                ui.label(RichText::new(crate::version_label()).small().color(p.tide));
            });
            if !compact {
                ui.label(
                    RichText::new("Converts native CAD designs into SolidCraft designs.")
                        .small()
                        .color(p.tide),
                );
            }
        });
    }

    fn title_controls(&mut self, ui: &mut egui::Ui, narrow: bool) {
        ui.hyperlink_to("Report a problem", crate::REPORT_PROBLEM_URL);
        ui.add_space(theme::SPACE_2);
        let current = ui.ctx().options(|o| o.theme_preference);
        let themes = [
            (ThemePreference::Dark, "Night"),
            (ThemePreference::Light, "Day"),
            (ThemePreference::System, "System"),
        ];
        if narrow {
            let label = match current {
                ThemePreference::Dark => "Night",
                ThemePreference::Light => "Day",
                ThemePreference::System => "System",
            };
            ui.menu_button(label, |ui| {
                for (preference, name) in themes {
                    if ui.selectable_label(current == preference, name).clicked() {
                        ui.ctx().set_theme(preference);
                        ui.close();
                    }
                }
            });
        } else {
            for (preference, name) in themes {
                if ui.selectable_label(current == preference, name).clicked() {
                    ui.ctx().set_theme(preference);
                }
            }
        }
        self.solidcraft_pill(ui, narrow);
        self.updater.header(ui, narrow);
    }

    fn solidcraft_report(&self) -> Value {
        serde_json::json!({"solidcraft": dutchman::json::solidcraft_values(
            self.options.solidcraft_version, self.found_version.clone())})
    }

    fn solidcraft_pill(&mut self, ui: &mut egui::Ui, narrow: bool) {
        let report = self.solidcraft_report();
        let status = report
            .pointer("/solidcraft/status")
            .and_then(Value::as_str)
            .unwrap_or("none_found");
        let p = theme::palette(ui);
        let color = match status {
            "too_old" | "none_found" => p.lantern_text,
            "newer" => p.spectre,
            _ => p.tide,
        };
        let label = match (&self.found_version, narrow) {
            (Some(version), true) => {
                dutchman_solidcraft::solidcraft_minor(version).unwrap_or_default()
            }
            (Some(version), false) => format!(
                "SolidCraft {}",
                dutchman_solidcraft::solidcraft_minor(version).unwrap_or_default()
            ),
            (None, true) => "not found".into(),
            (None, false) => "SolidCraft not found".into(),
        };
        let notice = dutchman::json::solidcraft_notice(&report);
        if ui
            .add(
                egui::Button::new(RichText::new(label).color(color))
                    .corner_radius(CornerRadius::same(16)),
            )
            .on_hover_text(
                notice.unwrap_or_else(|| "SolidCraft target and installed version".into()),
            )
            .clicked()
        {
            self.solidcraft_open = !self.solidcraft_open;
            if self.solidcraft_open && self.found_version.is_none() {
                // Search again: SolidCraft may have been installed, or been slow to start.
                self.set_solidcraft_cli(self.options.solidcraft.clone());
            }
        }
    }

    fn solidcraft_window(&mut self, ctx: &egui::Context) {
        let mut open = self.solidcraft_open;
        egui::Window::new("SolidCraft version")
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-24.0, self.header_bottom + 8.0))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                let report = self.solidcraft_report();
                if let Some(notice) = dutchman::json::solidcraft_notice(&report) {
                    ui.label(notice);
                } else if self.found_version.is_none() && self.solidcraft_cli_error.is_some() {
                    // The error below says what is missing and which downloads have it.
                } else if self.found_version.is_none() && self.has_app {
                    ui.label("Found the SolidCraft app, but not solidcraft-cli, which checks feature history. On Windows, the single-file SolidCraft download leaves it out; the portable ZIP and the MSI installer include it.");
                } else if self.found_version.is_none() {
                    ui.label("SolidCraft not found. Install SolidCraft to check feature history and open designs.");
                } else if let Some(found) = &self.found_version {
                    if let Some(minor) = dutchman_solidcraft::solidcraft_minor(found) {
                        ui.label(format!("SolidCraft {minor} is installed."));
                    }
                }
                if !self.search_note.is_empty() {
                    ui.label(&self.search_note);
                }
                self.solidcraft_picker(ui);
                self.solidcraft_cli_chooser(ui, ctx);
                if report.pointer("/solidcraft/status").and_then(Value::as_str) == Some("newer")
                    && self.updater.newer_known()
                    && ui.button("Update Dutchman").clicked()
                {
                    self.updater.open = true;
                }
                if matches!(report.pointer("/solidcraft/status").and_then(Value::as_str), Some("too_old" | "none_found")) {
                    ui.hyperlink_to("Download SolidCraft", dutchman::json::SOLIDCRAFT_DOWNLOAD);
                }
            });
        self.solidcraft_open = open;
    }

    fn solidcraft_cli_chooser(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if ui.button("Choose solidcraft-cli…").clicked() {
                worker::pick(Pick::SolidCraftCli, self.sender.clone(), ctx.clone());
            }
            if self.options.solidcraft.is_some() && ui.button("Find automatically").clicked() {
                self.set_solidcraft_cli(None);
            }
        });
        if let Some(why) = &self.solidcraft_cli_error {
            let danger = theme::palette(ui).danger;
            ui.label(RichText::new(why).color(danger));
        }
    }

    fn solidcraft_picker(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_id_salt("solidcraft_version")
            .selected_text(format!(
                "Target SolidCraft {}",
                self.options
                    .solidcraft_version
                    .unwrap_or(self.detected_version)
                    .minor()
            ))
            .show_ui(ui, |ui| {
                for (version, label) in [
                    (
                        self.detected_version,
                        format!("Automatic ({})", self.detected_version.minor()),
                    ),
                    (
                        dutchman_solidcraft::SolidCraftVersion::default(),
                        format!(
                            "Newest known ({})",
                            dutchman_solidcraft::SolidCraftVersion::default().minor()
                        ),
                    ),
                    (
                        dutchman_solidcraft::SolidCraftVersion::V0_25_1,
                        "0.25 and newer".to_owned(),
                    ),
                ] {
                    ui.selectable_value(&mut self.options.solidcraft_version, Some(version), label);
                }
            });
    }

    fn inputs_panel(&mut self, ui: &mut egui::Ui) {
        let p = *theme::palette(ui);
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        section_label(ui, &p, "Designs");
        panel(ui, &p, hovering, |ui| {
            if self.inputs.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(theme::SPACE_4);
                    ui.label(RichText::new("Drop design files, folders or ZIPs here").heading());
                    ui.label(
                        RichText::new("Fusion, Inventor, SolidWorks, FreeCAD, CATIA, Creo, NX, Creo View, Rhino, ACIS, STEP and IGES")
                        .small()
                        .color(p.tide),
                    );
                    ui.add_space(theme::SPACE_4);
                });
            } else {
                let mut remove = None;
                let running = self.running();
                // A long list scrolls inside the card when the columns stand side by side.
                let height = if is_wide(ui) {
                    INPUT_LIST_HEIGHT
                } else {
                    f32::INFINITY
                };
                ScrollArea::vertical()
                    .id_salt("main-designs")
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for (index, input) in self.inputs.iter().enumerate() {
                            egui::Sides::new().shrink_left().truncate().show(
                                ui,
                                |ui| {
                                    // Keep the elided text clear of the Remove button.
                                    let reserve = ui.spacing().item_spacing.x;
                                    ui.set_max_width((ui.max_rect().width() - reserve).max(0.0));
                                    // A narrow row stacks its text, so no piece is squeezed to a stub.
                                    let stacked = ui.max_rect().width() < 450.0;
                                    let layout = if stacked {
                                        egui::Layout::top_down(Align::Min)
                                    } else {
                                        egui::Layout::left_to_right(Align::Center)
                                    };
                                    ui.with_layout(layout, |ui| {
                                        path_label(ui, &p, input);
                                        for (path, estimate) in &self.estimates {
                                            if path == input || path.starts_with(input) {
                                                if let Some(estimate) = estimate {
                                                    let text = if path == input {
                                                        self.file_estimate_text(path, *estimate)
                                                    } else {
                                                        format!(
                                                            "{}: {}",
                                                            file_name(path),
                                                            self.file_estimate_text(
                                                                path, *estimate
                                                            )
                                                        )
                                                    };
                                                    // Room for less than a stub: leave it out rather than draw "…" under the button.
                                                    if ui.available_width() > 48.0 {
                                                        ui.label(
                                                            RichText::new(text)
                                                                .small()
                                                                .color(p.tide),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        if let Some(format) =
                                            dutchman_core::Format::from_path(input)
                                        {
                                            let level = format
                                                .headline_level()
                                                .map(|level| format!("  {level}"))
                                                .unwrap_or_default();
                                            if ui.available_width() > 48.0 {
                                                ui.label(format!(
                                                    "{}{level}",
                                                    format.status_for(input)
                                                ));
                                            }
                                        }
                                    });
                                },
                                |ui| {
                                    let button = egui::Button::new(RichText::new("Remove").small());
                                    if ui.add_enabled(!running, button).clicked() {
                                        remove = Some(index);
                                    }
                                },
                            );
                        }
                    });
                if let Some(index) = remove {
                    self.inputs.remove(index);
                }
            }
            if !self.inputs.is_empty() && !self.project_summary.is_empty() {
                ui.label(RichText::new(&self.project_summary).small().color(p.tide));
            }
            if let Some(available) = self.estimate_available().filter(|bytes| *bytes > 0) {
                ui.label(
                    RichText::new(format!("{:.1} GB of memory free", gigabytes(available)))
                        .small()
                        .color(p.tide),
                );
            }
            ui.add_space(theme::SPACE_2);
            ui.horizontal(|ui| {
                ui.add_enabled_ui(
                    !self.running() && !self.updater.blocks_conversions(),
                    |ui| {
                        if ui.button("Add files…").clicked() {
                            worker::pick(Pick::Files, self.sender.clone(), ui.ctx().clone());
                        }
                        if ui.button("Check a folder").clicked() {
                            self.notice = None;
                            self.doctor = Some(crate::doctor::Page::new(None));
                        }
                        if ui.button("Add folder…").clicked() {
                            worker::pick(Pick::Folder, self.sender.clone(), ui.ctx().clone());
                        }
                        if !self.inputs.is_empty() && ui.button("Clear").clicked() {
                            self.inputs.clear();
                        }
                    },
                );
            });
        });
    }

    fn preset_ui(&mut self, ui: &mut egui::Ui) {
        let label = if self.custom {
            format!("Custom (based on {})", preset_label(self.options.preset))
        } else {
            preset_label(self.options.preset).to_owned()
        };
        let response = egui::ComboBox::from_label("Preset")
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.set_width((ui.ctx().content_rect().width() - 80.0).clamp(260.0, 480.0));
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                for preset in [Preset::Fast, Preset::Standard, Preset::Full] {
                    let description = preset_description(preset);
                    let response = ui.selectable_label(
                        !self.custom && self.options.preset == preset,
                        preset_label(preset),
                    );
                    preset_help(&response, description);
                    if response.clicked() {
                        select_preset(&mut self.options, preset);
                        self.custom = false;
                        ui.close();
                    }
                }
            })
            .response;
        preset_help(&response, preset_description(self.options.preset));
    }

    fn choices_ui(&mut self, ui: &mut egui::Ui) {
        let wide = is_wide(ui);
        self.preset_ui(ui);
        egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
            ui.horizontal(|ui| {
                let last_shown = last_shown_label(&self.inputs);
                let label = |mode| match mode {
                    dutchman_core::PartPositions::Smart => "Smart".to_owned(),
                    dutchman_core::PartPositions::LastShown => last_shown.clone(),
                    dutchman_core::PartPositions::FirstPlaced => "As first placed".to_owned(),
                };
                egui::ComboBox::from_label("Part positions")
                    .selected_text(label(self.options.part_positions))
                    .show_ui(ui, |ui| {
                        for mode in [
                            dutchman_core::PartPositions::Smart,
                            dutchman_core::PartPositions::LastShown,
                            dutchman_core::PartPositions::FirstPlaced,
                        ] {
                            ui.selectable_value(
                                &mut self.options.part_positions,
                                mode,
                                label(mode),
                            );
                        }
                    });
            });
            let choices = self.options.choices();
            let (mut history, mut sketches, mut joints) =
                (choices.history, choices.sketches, choices.joints);
            // Switches narrow the selected base; Fast has nothing further to leave out.
            ui.add_enabled_ui(self.options.preset != Preset::Fast, |ui| {
                let changed = side_by_side(ui, wide, |ui| {
                    ui.checkbox(&mut history, "Feature history").changed()
                        | ui.checkbox(&mut sketches, "Sketches").changed()
                        | ui.checkbox(&mut joints, "Joints").changed()
                });
                if changed {
                    set_choices(&mut self.options, history, sketches, joints);
                    self.custom = true;
                }
            })
            .response
            .on_disabled_hover_text(
                "Choose Standard or Full to keep history, sketches, or joints.",
            );
        });
    }

    fn output_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let p = *theme::palette(ui);
        let wide = is_wide(ui);
        section_label(ui, &p, "Output folder");
        panel(ui, &p, false, |ui| {
            let running = self.running();
            let sender = self.sender.clone();
            egui::Sides::new().shrink_left().truncate().show(
                ui,
                |ui| match &self.out_dir {
                    Some(dir) => {
                        ui.add(
                            egui::Label::new(RichText::new(display(dir)).monospace()).truncate(),
                        )
                        .on_hover_text(display(dir));
                    }
                    None => {
                        ui.label(RichText::new("Not chosen yet").color(p.tide));
                    }
                },
                |ui| {
                    if ui
                        .add_enabled(!running, egui::Button::new("Choose…"))
                        .clicked()
                    {
                        worker::pick(Pick::Output, sender, ctx.clone());
                    }
                },
            );
            ui.add_space(theme::SPACE_2);
            ui.add_enabled_ui(!self.running(), |ui| {
                self.choices_ui(ui);
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_label("Output format")
                        .selected_text(match self.options.to {
                            dutchman::OutputFormat::Solidcraft => "SolidCraft",
                            dutchman::OutputFormat::Freecad => "FreeCAD macro (experimental)",
                            dutchman::OutputFormat::Fcstd => "FreeCAD document (experimental)",
                            dutchman::OutputFormat::Dxf => "DXF (convert command only)",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.options.to,
                                dutchman::OutputFormat::Solidcraft,
                                "SolidCraft",
                            );
                            ui.selectable_value(
                                &mut self.options.to,
                                dutchman::OutputFormat::Freecad,
                                "FreeCAD macro (experimental)",
                            );
                            ui.selectable_value(
                                &mut self.options.to,
                                dutchman::OutputFormat::Fcstd,
                                "FreeCAD document (experimental)",
                            );
                        });
                });
                side_by_side(ui, wide, |ui| {
                    let mut replace = !self.options.no_clobber;
                    if ui
                        .checkbox(&mut replace, "Replace designs already in the output folder")
                        .changed()
                    {
                        self.options.no_clobber = !replace;
                    }
                    if ui.small_button("SolidCraft version…").clicked() {
                        self.solidcraft_open = true;
                    }
                });
                side_by_side(ui, wide, |ui| {
                    let mut verify = !self.options.no_verify;
                    if ui
                        .checkbox(&mut verify, "Check feature history in SolidCraft")
                        .changed()
                    {
                        self.options.no_verify = !verify;
                    }
                    ui.checkbox(
                        &mut self.options.no_memory_check,
                        "Convert and check history even if memory is low",
                    ).on_hover_text("Force the conversion and history check despite the memory estimate. This can run out of memory. Checking history off stays off.");
                });
                let checker = match &self.checker {
                    Ok(cli) => format!("Checks with {}.", display(cli)),
                    Err(dutchman_solidcraft::NoCli::Blocked(why)) => why.clone(),
                    Err(dutchman_solidcraft::NoCli::TooOld(why)) => {
                        format!("History is written unchecked: {why}.")
                    }
                    Err(dutchman_solidcraft::NoCli::NotFound(_)) => format!(
                        "No solidcraft-cli found, so history is written unchecked: install SolidCraft, \
                         choose its solidcraft-cli under SolidCraft version…, or set {} to its full path.",
                        dutchman_solidcraft::CLI_ENV
                    ),
                };
                let label = egui::Label::new(RichText::new(&checker).small().color(p.tide));
                if matches!(self.checker, Err(dutchman_solidcraft::NoCli::Blocked(_))) {
                    ui.add(label.wrap());
                } else {
                    // Keep paths on one line; the whole path is on hover.
                    ui.add(label.truncate()).on_hover_text(checker);
                }
                ui.add_space(theme::SPACE_2);
                if !self.inputs.is_empty() && self.has_inventor {
                    let chosen = self.options.project.clone();
                    let mut find = false;
                    egui::Sides::new().shrink_left().truncate().show(
                        ui,
                        |ui| {
                            let project = match &chosen {
                                Some(dir) => display(dir),
                                None => "found from each assembly".to_owned(),
                            };
                            ui.add(
                            egui::Label::new(
                                RichText::new(format!("Inventor project folder: {project}"))
                                    .small()
                                    .color(p.tide),
                            )
                            .truncate(),
                        )
                        .on_hover_text(
                            "Where an assembly's part files are searched for when they are not \
                             at the paths it stores. Found from each assembly: the nearest folder \
                             above it holding an .ipj, else two folders up; a folder you add is \
                             its files' project.",
                        );
                        },
                        |ui| {
                            if chosen.is_some()
                                && ui.button(RichText::new("Automatic").small()).clicked()
                            {
                                find = true;
                            }
                            if ui.button(RichText::new("Choose…").small()).clicked() {
                                worker::pick(Pick::Project, self.sender.clone(), ctx.clone());
                            }
                        },
                    );
                    if find {
                        self.options.project = None;
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Open diagnostics folder").clicked() {
                    let result = dutchman::diagnostics::default_folder().and_then(|folder| {
                        std::fs::create_dir_all(&folder).map_err(|err| err.to_string())?;
                        platform::open(&folder)
                    });
                    if let Err(message) = result {
                        self.notice = Some(message);
                    }
                }
            });
        });
    }

    fn actions_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, summary: Option<&str>) {
        let p = *theme::palette(ui);
        ui.horizontal(|ui| {
            if self.running() {
                if ui
                    .add_enabled(self.run.is_some(), egui::Button::new("Stop"))
                    .clicked()
                {
                    if let Some(run) = &self.run {
                        run.stop();
                        self.diagnostic_stopped = true;
                    }
                }
            } else {
                let ready = !self.updater.blocks_conversions()
                    && !self.inputs.is_empty()
                    && self.out_dir.is_some();
                let convert = egui::Button::new(
                    RichText::new("Convert")
                        .color(if ready { p.on_spectre } else { p.tide })
                        .text_style(TextStyle::Button),
                )
                .fill(if ready { p.spectre } else { p.surface_2 })
                .stroke(Stroke::NONE)
                .min_size(egui::vec2(120.0, 34.0));
                if ui.add_enabled(ready, convert).clicked() {
                    self.start(ctx);
                }
                if !ready {
                    let why = if self.updater.blocks_conversions() {
                        "Finish updating before starting another conversion."
                    } else if self.inputs.is_empty() {
                        "Add designs to convert."
                    } else {
                        "Choose an output folder."
                    };
                    ui.label(RichText::new(why).small().color(p.tide));
                }
            }
            if let Some(summary) = summary.filter(|_| !self.rows.is_empty()) {
                ui.label(RichText::new(summary).color(p.tide));
            }
            ui.menu_button("Formats", |ui| {
                for format in dutchman_core::Format::ALL {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format.name());
                        ui.label(format.words());
                        if let Some(level) = format.headline_level() {
                            ui.label(level);
                        }
                    });
                }
            });
        });
        if let Some(notice) = &self.notice {
            let colour = if notice == present::REPORT_COPIED || notice == present::SEND_COPIED {
                p.spectre
            } else {
                p.danger
            };
            ui.label(RichText::new(notice).color(colour));
        }
    }

    fn view(&self) -> present::RunView {
        present::RunView::from_files(
            self.rows.iter().map(Row::view).collect(),
            self.rows.iter().filter_map(|row| row.report.as_ref()),
            self.running(),
        )
    }

    /// The results. With `own_scroll` they fill the rest of the column and scroll inside it,
    /// and a placeholder shows until there are any.
    fn results(&mut self, ui: &mut egui::Ui, view: &present::RunView, own_scroll: bool) {
        let p = *theme::palette(ui);
        let heading = section_label(ui, &p, "Results");
        let reveal = self.reveal.take();
        if reveal == Some(Reveal::Results) {
            heading.scroll_to_me(Some(Align::Min));
        }
        let first = (reveal == Some(Reveal::FirstIssue)).then_some(view.first_to_show);
        let mut action = None;
        let log_ready = !self.running();
        let running = !log_ready || self.diagnosis.is_some() || self.updater.blocks_conversions();
        if own_scroll && self.rows.is_empty() {
            panel(ui, &p, false, |ui| {
                ui.set_min_height(theme::SPACE_8 * 3.0);
                ui.label(RichText::new("Converted designs appear here.").color(p.tide));
            });
        }
        let mut cards = |ui: &mut egui::Ui| {
            let mut files: Vec<_> = view.files.iter().collect();
            files.sort_by_key(|(index, _)| *index);
            for (index, file) in files {
                let index = *index;
                let Some(row) = self.rows.get(index) else {
                    continue;
                };
                let shown = card(ui, &p, |ui| {
                    let policy = self.report_policy.policy();
                    let log = self
                        .row_logs
                        .get(&index)
                        .map(|path| (path.as_path(), log_ready));
                    let chosen = row_ui(
                        ui,
                        &p,
                        index,
                        row,
                        file,
                        policy,
                        self.has_app,
                        running,
                        log,
                        self.ai_prompts.get(&index),
                    );
                    if let Some(chosen) = chosen {
                        action = Some(chosen);
                    }
                });
                if first == Some(index) {
                    shown.scroll_to_me(Some(Align::Min));
                }
                ui.add_space(theme::SPACE_2);
            }
        };
        if own_scroll && !self.rows.is_empty() {
            let mut area = ScrollArea::vertical()
                .id_salt("main-results")
                .auto_shrink([false, false]);
            if reveal == Some(Reveal::Results) {
                area = area.vertical_scroll_offset(0.0);
            }
            area.show(ui, |ui| cards(ui));
        } else {
            cards(ui);
        }
        if action.is_some() {
            self.notice = None;
        }
        let result = match action {
            Some(Action::Open(path)) => platform::open_in_solidcraft(&path),
            Some(Action::Reveal(path)) => platform::reveal(&path),
            Some(Action::Check(index)) => {
                if let Some(row) = self.rows.get(index) {
                    self.doctor = Some(crate::doctor::Page::new(Some(row.input.clone())));
                }
                Ok(())
            }
            Some(Action::Notice(message)) => {
                self.notice = Some(message);
                Ok(())
            }
            Some(Action::Pick(pick)) => {
                worker::pick(pick, self.sender.clone(), ui.ctx().clone());
                Ok(())
            }
            Some(Action::Diagnose(index)) => {
                if self.diagnosis.is_some() {
                    return;
                }
                if let Some(row) = self.rows.get(index) {
                    let mut wizard =
                        crate::diagnose::Wizard::new(row.input.clone(), self.options.clone());
                    wizard.start(ui.ctx());
                    self.diagnosis = Some(wizard);
                }
                Ok(())
            }

            Some(Action::Consent(index)) => self.open_consent(index),
            Some(Action::Report(index)) => self.report_on_github(index, ui.ctx()),
            Some(Action::Join) => {
                let invite = self.report_policy.policy().invite_url.clone();
                ui.ctx().open_url(egui::OpenUrl::new_tab(invite));
                Ok(())
            }
            Some(Action::Dm(index)) => self.dm_dutchy(index, ui.ctx()),
            Some(Action::AskAi(index)) => self.ask_ai(index, ui.ctx()),
            Some(Action::OpenReport(index)) => {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(egui::Id::new(("open-report", index)), true));
                ui.ctx().request_repaint();
                Ok(())
            }
            Some(Action::CloseAi(index)) => {
                self.ai_prompts.remove(&index);
                Ok(())
            }
            None => Ok(()),
        };
        if let Err(err) = result {
            self.notice = Some(err);
        }
        if !running {
            if let Some(pitch) = &view.invitation {
                ui.horizontal_wrapped(|ui| {
                    for (index, part) in pitch.split(json::FIXTURES_URL).enumerate() {
                        if index > 0 {
                            ui.hyperlink_to(json::FIXTURES_URL, json::FIXTURES_URL);
                        }
                        for (index, text) in part.split(json::DISCORD_URL).enumerate() {
                            if index > 0 {
                                ui.hyperlink_to(json::DISCORD_URL, json::DISCORD_URL);
                            }
                            if !text.is_empty() {
                                ui.label(text);
                            }
                        }
                    }
                });
            }
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let mut settings = saved_choices(&self.options, self.custom);
        if let Some(object) = settings.as_object_mut() {
            object.insert("report_ids".into(), serde_json::json!(self.consent.ids));
            object.insert("update".into(), self.updater.saved());
            object.insert("report_policy".into(), self.report_policy.saved());
            if let Some(log) = &self.dismissed_unfinished {
                object.insert(
                    DISMISSED_UNFINISHED_KEY.into(),
                    serde_json::json!(log.display().to_string()),
                );
            }
            if let Some(cli) = &self.options.solidcraft {
                object.insert(
                    SOLIDCRAFT_CLI_KEY.into(),
                    serde_json::json!(cli.display().to_string()),
                );
            }
        }
        storage.set_string(SETTINGS_KEY, settings.to_string());
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.fit_window(ctx);
        self.ui(ctx);
    }
}

enum Action {
    /// Show this message.
    Notice(String),
    Diagnose(usize),
    Check(usize),
    Open(PathBuf),
    Reveal(PathBuf),
    Pick(Pick),
    Consent(usize),
    /// Open the GitHub issue form about the row's file, filled in.
    Report(usize),
    /// Open the Discord invite.
    Join,
    /// Copy the message about the row's file and open Dutchy's DM.
    Dm(usize),
    /// Copy the AI prompt for the row and show it on its card.
    AskAi(usize),
    /// Close the row's AI prompt.
    CloseAi(usize),
    /// Open the row's "Something wrong? Report this file" section.
    OpenReport(usize),
}

/// One file's result: its status, what was written, and its losses.
#[allow(clippy::too_many_arguments)]
fn row_ui(
    ui: &mut egui::Ui,
    p: &Palette,
    index: usize,
    row: &Row,
    view: &FileView,
    policy: &ReportPolicy,
    has_app: bool,
    running: bool,
    log: Option<(&Path, bool)>,
    ai: Option<&AiPrompt>,
) -> Option<Action> {
    let mut action = None;
    let colour = match view.status {
        present::Status::Incomplete => p.lantern_text,
        present::Status::Converted
        | present::Status::ConvertedWithLosses
        | present::Status::Flattened => p.spectre,
        present::Status::Failed => p.danger,
        _ => p.tide,
    };
    let word = view.status_word;
    let output = row
        .output
        .as_ref()
        .filter(|_| row.status == Status::Converted);
    let is_macro = output.is_some_and(|output| {
        output
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("FCMacro"))
    });
    // The title has its own full-width line and wraps rather than elides.
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(word).heading().color(colour));
        ui.add(egui::Label::new(RichText::new(&view.name).heading()).wrap())
            .on_hover_text(display(&row.input));
    });
    let step_button = !running && row.report.is_some();
    if output.is_some() || step_button {
        // One row of buttons under it.
        ui.horizontal_wrapped(|ui| {
            if let Some(output) = output {
                if is_macro {
                    if ui.button("Show in folder").clicked() {
                        action = Some(Action::Reveal(output.clone()));
                    }
                    ui.label("Run macro in FreeCAD")
                        .on_hover_text("In FreeCAD, choose Macro > Macros… > Execute.");
                } else {
                    let open = ui.button("Open in SolidCraft");
                    let open = if has_app {
                        open
                    } else {
                        open.on_hover_text(format!(
                            "Opens with the system's app for this design. Set {} to choose SolidCraft.",
                            platform::APP_ENV
                        ))
                    };
                    if open.clicked() {
                        action = Some(Action::Open(output.clone()));
                    }
                    if ui.button("Show in folder").clicked() {
                        action = Some(Action::Reveal(output.clone()));
                    }
                }
            }
            if step_button && ui.button("Add matching STEP").clicked() {
                action = Some(Action::Check(index));
            }
        });
    }
    if row.status == Status::Converting {
        if let Some(event) = row.progress {
            ui.label(event.to_string());
            if let Some(total) = event.designs_total.filter(|total| *total > 0) {
                ui.add(
                    egui::ProgressBar::new(
                        (event.designs_read as f32 / total as f32).clamp(0.0, 1.0),
                    )
                    .desired_width(200.0),
                );
            } else {
                ui.add(egui::Spinner::new().size(14.0).color(p.lantern));
            }
        } else {
            ui.add(egui::Spinner::new().size(14.0).color(p.lantern));
        }
    }
    if let Some(status) = &view.capability {
        ui.label(status);
    }
    if let Some(text) = &view.missing_summary {
        missing_ui(ui, p, text);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!running && row.job.is_some(), |ui| {
                if ui.button("Search a folder…").clicked() {
                    action = Some(Action::Pick(Pick::SearchFolder(index)));
                }
                if ui.button("Find a file…").clicked() {
                    action = Some(Action::Pick(Pick::SearchFile(index)));
                }
            });
        });
    }
    if let Some(message) = &view.error {
        ui.label(RichText::new(message).color(p.danger));
        let mut diagnose = |ui: &mut egui::Ui| {
            if ui
                .add_enabled(!running, egui::Button::new("Diagnose this file"))
                .clicked()
            {
                action = Some(Action::Diagnose(index));
            }
        };
        match log {
            Some((path, ready)) => {
                if let Err(message) = platform::log_controls_with(ui, path, ready, diagnose) {
                    action = Some(Action::Notice(message));
                }
            }
            None => {
                ui.horizontal_wrapped(|ui| diagnose(ui));
            }
        }
        if row.output.is_some() && row.report.is_some() {
            if let Some(chosen) = ask_ai_ui(ui, index, ai) {
                action = Some(chosen);
            }
        }
        if view.can_report {
            if let Some(report) = &row.report {
                let steps =
                    present::SendSteps::new(policy, row.size, report, present::Surface::Desktop);
                // Folded, so the whole failed card fits the window at once.
                if let Some(chosen) = send_ui(ui, p, index, &steps, true) {
                    action = Some(chosen);
                }
            }
        }
        return action;
    }
    if let Some((path, ready)) = log {
        if let Err(message) = platform::log_controls(ui, path, ready) {
            action = Some(Action::Notice(message));
        }
    }
    if row.status == Status::Converted && row.report.is_some() {
        let output = row.output.as_deref().map(display).unwrap_or_default();
        ui.add(
            egui::Label::new(
                RichText::new(format!("→ {output}"))
                    .monospace()
                    .color(p.tide),
            )
            .truncate(),
        )
        .on_hover_text(output);
        let (outcome, history) = outcome_and_history(&view.outcome, row.report.as_ref());
        ui.label(RichText::new(outcome).color(p.ink));
        for loss in history {
            ui.label(RichText::new(loss).color(p.tide));
        }
        if let Some(summary) = &view.assembly {
            let colour = if view.incomplete { p.danger } else { p.tide };
            ui.label(RichText::new(summary).color(colour));
            for cause in &view.assembly_causes {
                ui.label(RichText::new(cause).small().color(p.ink));
            }
        }
        if !view.components.is_empty() {
            let components = &view.components;
            egui::CollapsingHeader::new(RichText::new(&view.loss_summary).color(p.tide))
                .id_salt(("losses", index))
                .show(ui, |ui| {
                    // One component: its losses. Several: each in its own section, most that
                    // matters first.
                    if let [only] = components.as_slice() {
                        for loss in &only.lines {
                            loss_line(ui, p, loss);
                        }
                        return;
                    }
                    for (n, component) in components.iter().enumerate() {
                        let colour = match component.severity_rank {
                            0 => p.danger,
                            1 => p.lantern_text,
                            _ => p.tide,
                        };
                        egui::CollapsingHeader::new(
                            RichText::new(&component.heading).color(colour),
                        )
                        .id_salt(("component", index, n))
                        .show(ui, |ui| {
                            for loss in &component.lines {
                                loss_line(ui, p, loss);
                            }
                        });
                    }
                });
        }
        let has_losses = row
            .report
            .as_ref()
            .and_then(|r| r.get("losses"))
            .and_then(Value::as_array)
            .is_some_and(|losses| !losses.is_empty());
        let limited = has_losses
            || view.status != present::Status::Converted
            || row
                .report
                .as_ref()
                .and_then(|r| r.get("mode"))
                .and_then(Value::as_str)
                == Some("partial_history");
        let ask = has_losses && row.output.is_some();
        let report_button = view.can_report && limited;
        if !view.components.is_empty() || ask || report_button {
            ui.horizontal_wrapped(|ui| {
                if !view.components.is_empty() && ui.button("Save the full log…").clicked() {
                    action = Some(Action::Pick(Pick::SaveLog(index)));
                }
                if ask && ui.button("Ask your AI about this file").clicked() {
                    action = Some(Action::AskAi(index));
                }
                if report_button && ui.button("Report this file…").clicked() {
                    action = Some(Action::OpenReport(index));
                }
            });
        }
        if ask {
            if let Some(chosen) = ai_prompt_ui(ui, index, ai) {
                action = Some(chosen);
            }
        }
    }
    if view.can_report && view.report_case == Some(present::ReportCase::Converted) {
        if let Some(report) = &row.report {
            // "Report this file…" opens the section and brings it into view.
            let open_id = egui::Id::new(("open-report", index));
            let force = ui
                .ctx()
                .data_mut(|data| data.remove_temp::<bool>(open_id))
                .is_some();
            let section = egui::CollapsingHeader::new(present::REPORT_CONVERTED)
                .id_salt(("report-converted", index, &row.input))
                .default_open(false)
                .open(force.then_some(true))
                .show(ui, |ui| {
                    let steps = present::SendSteps::new(
                        policy,
                        row.size,
                        report,
                        present::Surface::Desktop,
                    );
                    if let Some(chosen) = send_ui(ui, p, index, &steps, false) {
                        action = Some(chosen);
                    }
                });
            if force {
                section.header_response.scroll_to_me(Some(Align::Min));
            }
        }
    }
    action
}

/// The short, fill-in-the-blanks prompt for the user's AI assistant. Everything else it needs is
/// in the saved report, the run's log and the guidance bundled beside the app.
fn ai_prompt(
    input: &Path,
    output: &Path,
    converted: bool,
    log: Option<&Path>,
) -> Result<String, String> {
    let input = std::path::absolute(input).map_err(|err| err.to_string())?;
    let output = std::path::absolute(output).map_err(|err| err.to_string())?;
    let report = dutchman::saved_report::path(&output, converted);
    let executable = std::env::current_exe().map_err(|err| {
        format!(
            "Could not locate dutchman beside the app: {err}. Check where Dutchman is installed."
        )
    })?;
    let app_folder = executable
        .parent()
        .ok_or("Could not locate the app's folder. Check where Dutchman is installed.")?;
    // A macOS app bundle keeps its guidance in Resources.
    let agents = [
        app_folder.join("AGENTS.md"),
        app_folder.join("..").join("Resources").join("AGENTS.md"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .map_or_else(
        || agents_url(dutchman::BUILD_VERSION),
        |path| path.display().to_string(),
    );
    let mut text = String::from("Help me with a Dutchman conversion.");
    text.push_str(&format!(" Original file: {}.", input.display()));
    if converted {
        text.push_str(&format!(" Converted design: {}.", output.display()));
    }
    text.push_str(&format!(
        " Dutchman saved the full conversion report at {}",
        report.display()
    ));
    match log {
        Some(log) => text.push_str(&format!(
            " and its log at {}.",
            std::path::absolute(log)
                .unwrap_or_else(|_| log.to_path_buf())
                .display()
        )),
        None => text.push('.'),
    }
    text.push_str(&format!(
        " Start by running `dutchman investigate` on the original file, as {agents} explains, then explain its summary.md to me. Don't send any files anywhere without asking me."
    ));
    Ok(text)
}

/// Where the installed guide is read when it is not beside the app, as in a source build: the
/// copy at this build's own version, so the instructions match the app. A release links its
/// tag (`v0.2.7`); a nightly (`0.2.8-nightly.abc1234`) links the commit it was built from.
pub fn agents_url(version: &str) -> String {
    let rev = match version.split_once("-nightly.") {
        Some((_, commit)) if !commit.is_empty() => commit.to_owned(),
        _ => format!("v{version}"),
    };
    format!("https://github.com/dutchman-cad/dutchman/blob/{rev}/assets/install/AGENTS.md")
}

/// How long "Copied" shows before the button looks as it did.
const COPIED_FOR: std::time::Duration = std::time::Duration::from_secs(2);

/// The prompt shown on a result card after **Ask your AI about this file**.
struct AiPrompt {
    text: String,
    /// When it was last copied.
    copied: std::time::Instant,
}

/// **Ask your AI about this file**: always copies, and the prompt it copied stays open under the
/// button, with a Copy button, until closed or the card is replaced.
fn ask_ai_ui(ui: &mut egui::Ui, index: usize, ai: Option<&AiPrompt>) -> Option<Action> {
    let mut action = None;
    if ui.button("Ask your AI about this file").clicked() {
        action = Some(Action::AskAi(index));
    }
    ai_prompt_ui(ui, index, ai).or(action)
}

/// Lines of the prompt box shown at once; the rest scrolls inside it.
const AI_PROMPT_LINES: f32 = 5.0;

/// The copied prompt under the button, with "Copied" for a moment after a copy.
fn ai_prompt_ui(ui: &mut egui::Ui, index: usize, ai: Option<&AiPrompt>) -> Option<Action> {
    let mut action = None;
    let Some(ai) = ai else {
        return action;
    };
    if let Some(left) = COPIED_FOR.checked_sub(ai.copied.elapsed()) {
        ui.label("Copied. Paste it into your AI assistant.");
        ui.ctx().request_repaint_after(left);
    }
    let mut text = ai.text.as_str();
    // At most about five lines tall, scrolling inside, so the card still fits the window.
    let line = ui.text_style_height(&egui::TextStyle::Monospace);
    ScrollArea::vertical()
        .id_salt(("ask-ai-scroll", index))
        .max_height(line * AI_PROMPT_LINES + 8.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .id_salt(("ask-ai-text", index)),
            );
        });
    ui.horizontal(|ui| {
        if ui.button("Copy").clicked() {
            action = Some(Action::AskAi(index));
        }
        if ui.button("Close").clicked() {
            action = Some(Action::CloseAi(index));
        }
    });
    action
}

/// A folder that replaces the platform diagnostics folder for every app created after it is
/// set, for tests (which must neither fill, prune nor read the real folder) and embedding.
static DIAGNOSTICS_FOLDER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Keeps run logs under `folder` instead of the platform diagnostics folder, for every app
/// created afterwards, and reads unfinished runs from there. Only the first call counts.
pub fn set_diagnostics_folder(folder: PathBuf) {
    let _ = DIAGNOSTICS_FOLDER.set(folder);
}

/// The unfinished run log the launch notice offers, from this process's diagnostics folder.
/// Where the log of the unfinished run last dismissed is remembered in the settings.
const DISMISSED_UNFINISHED_KEY: &str = "dismissed_unfinished";

/// The unfinished run's log, unless that is the one already dismissed: a later unfinished run
/// has its own log, and shows its notice.
fn undismissed(found: Option<PathBuf>, dismissed: Option<&Path>) -> Option<PathBuf> {
    found.filter(|log| Some(log.as_path()) != dismissed)
}

fn unfinished_diagnostic(folder: Option<&Path>) -> Option<PathBuf> {
    match folder {
        Some(folder) => dutchman::diagnostics::unfinished_log_in(folder),
        None => dutchman::diagnostics::unfinished_default_log(),
    }
}

/// Tests keep their run logs in a folder of their own, so that they neither fill nor prune
/// the real diagnostics folder.
fn test_diagnostics_folder() -> Option<PathBuf> {
    if let Some(folder) = DIAGNOSTICS_FOLDER.get() {
        static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Some(folder.join(format!("app-{n}")));
    }
    #[cfg(any(test, feature = "test-support"))]
    {
        static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Some(std::env::temp_dir().join(format!(
            "dutchman-gui-test-diagnostics-{}-{n}",
            std::process::id()
        )));
    }
    #[allow(unreachable_code)]
    None
}

/// The narrowest a step card gets before the cards wrap onto another line.
const MIN_STEP_WIDTH: f32 = 190.0;

/// The way to send a file privately, and the anonymous report beside it.
/// "Review and send report", then the other ways to send the file: in a collapsed
/// "Help us fix it" section when `fold`, else shown in full.
fn send_ui(
    ui: &mut egui::Ui,
    p: &Palette,
    index: usize,
    steps: &present::SendSteps,
    fold: bool,
) -> Option<Action> {
    let mut action = None;
    if ui.button("Review and send report").clicked() {
        action = Some(Action::Consent(index));
    }
    let other = if fold {
        egui::CollapsingHeader::new(present::SEND_FOLD)
            .id_salt(("send-fold", index))
            .default_open(false)
            .show(ui, |ui| send_ways_ui(ui, p, index, steps))
            .body_returned
            .flatten()
    } else {
        send_ways_ui(ui, p, index, steps)
    };
    other.or(action)
}

/// Discord, the DM with Dutchy, a STEP export and the anonymous GitHub report.
fn send_ways_ui(
    ui: &mut egui::Ui,
    p: &Palette,
    index: usize,
    steps: &present::SendSteps,
) -> Option<Action> {
    let mut action = None;
    ui.add_space(theme::SPACE_2);
    ui.label(RichText::new(steps.intro).color(p.ink));
    ui.add_space(theme::SPACE_2);
    let cards: &[usize] = if steps.step_card { &[0, 1, 2] } else { &[0, 1] };
    let gap = ui.spacing().item_spacing.x;
    let across = ((ui.available_width() + gap) / (MIN_STEP_WIDTH + gap)).floor() as usize;
    let across = across.clamp(1, cards.len());
    for (line, chunk) in cards.chunks(across).enumerate() {
        // Cards on a line share the height of last frame's tallest.
        let id = ui.id().with(("send-line", index, line, across));
        let height = ui.data(|data| data.get_temp::<f32>(id)).unwrap_or(0.0);
        let mut tallest = 0.0_f32;
        ui.columns(across, |columns| {
            for (ui, &card) in columns.iter_mut().zip(chunk) {
                let fill = if card == 2 {
                    egui::Color32::TRANSPARENT
                } else {
                    p.surface_2
                };
                framed(ui, p, fill, false, |ui| {
                    ui.set_min_height(height);
                    let shown = ui.scope(|ui| match card {
                        0 => {
                            step_title(ui, p, present::SEND_JOIN_TITLE);
                            if ui.button(present::SEND_JOIN_BUTTON).clicked() {
                                action = Some(Action::Join);
                            }
                            step_text(ui, p, present::SEND_JOIN_NOTE);
                        }
                        1 => {
                            step_title(ui, p, present::SEND_DM_TITLE);
                            if ui.button(present::SEND_DM_BUTTON).clicked() {
                                action = Some(Action::Dm(index));
                            }
                            step_text(ui, p, &steps.dm_text);
                        }
                        _ => {
                            ui.label(RichText::new(present::SEND_STEP_TITLE).color(p.tide));
                            step_text(ui, p, present::SEND_STEP_TEXT);
                        }
                    });
                    tallest = tallest.max(shown.response.rect.height());
                });
            }
        });
        ui.data_mut(|data| data.insert_temp(id, tallest));
        ui.add_space(theme::SPACE_2);
    }
    ui.label(
        RichText::new(present::SEND_SMALL_PRINT)
            .small()
            .color(p.tide),
    );
    let report = ui
        .button(present::REPORT_BUTTON)
        .on_hover_text(present::REPORT_HELP);
    if report.clicked() {
        action = Some(Action::Report(index));
    }
    action
}

fn step_title(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).strong().color(p.ink));
}

fn step_text(ui: &mut egui::Ui, p: &Palette, text: &str) {
    // Left-aligned: a justified column would stretch the gaps between words.
    ui.with_layout(Layout::top_down(Align::Min), |ui| {
        ui.label(RichText::new(text).small().color(p.tide));
    });
}

/// History decisions must be visible even while the full loss log is collapsed.
/// Whether the window is wide enough for the main screen's two columns.
fn is_wide(ui: &egui::Ui) -> bool {
    ui.ctx().content_rect().width() >= crate::WIDE_BREAKPOINT
}

/// Runs `body` with its controls in one row that wraps when `wide`, else one under another.
fn side_by_side<R>(ui: &mut egui::Ui, wide: bool, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if wide {
        ui.horizontal_wrapped(body).inner
    } else {
        body(ui)
    }
}

/// The outcome line and the history notes under it, saying each thing once: a design
/// without history names why in the outcome line, not again in a note under it.
fn outcome_and_history<'a>(outcome: &str, report: Option<&'a Value>) -> (String, Vec<&'a str>) {
    let mut notes = history_info(report);
    let Some(rest) = outcome.strip_prefix("Geometry without history") else {
        return (outcome.to_owned(), notes);
    };
    let reason = notes
        .iter()
        .position(|note| note.starts_with("flattened: "))
        .map(|at| notes.remove(at)["flattened: ".len()..].to_owned());
    notes.retain(|note| !note.starts_with("flattened: "));
    let outcome = match (rest.strip_prefix('.'), reason) {
        (Some(after), Some(reason)) => format!("Geometry without history: {reason}.{after}"),
        _ => outcome.to_owned(),
    };
    (outcome, notes)
}

fn history_info(report: Option<&Value>) -> Vec<&str> {
    report
        .and_then(|report| report.get("losses"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|loss| {
            loss.get("level").and_then(Value::as_str) == Some("Info")
                && loss.get("subject").and_then(Value::as_str) == Some("history")
        })
        .filter_map(|loss| loss.get("message").and_then(Value::as_str))
        .collect()
}

/// The missing-part summary and recovery instructions.
fn missing_ui(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).color(p.danger));
    ui.label(RichText::new(present::MISSING_HELP).small().color(p.tide));
}

/// One loss: its level word, coloured, then its subject and message.
fn loss_line(ui: &mut egui::Ui, p: &Palette, loss: &present::LossLine) {
    let level = loss.level.as_str();
    let colour = match level {
        "Degraded" => p.lantern_text,
        "Dropped" => p.danger,
        _ => p.tide,
    };
    // One paragraph keeps the label within the card. egui prefers word boundaries and
    // falls back to splitting oversized ids; break_anywhere would also split normal words.
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let mut job = egui::text::LayoutJob::default();
    let format = |color| egui::TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    job.append(level, 0.0, format(colour));
    job.append(&loss.text, theme::SPACE_1, format(p.ink));
    job.wrap.max_width = ui.available_width();
    ui.add(egui::Label::new(job).wrap());
}

fn section_label(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    ui.label(
        RichText::new(text.to_uppercase())
            .text_style(theme::label())
            .color(p.tide),
    )
}

/// A raised panel on `surface-2` with a hairline border (spectre while files hover over it).
/// It holds ink and tide text only: by day, spectre and danger text need `surface`.
fn panel(ui: &mut egui::Ui, p: &Palette, highlight: bool, body: impl FnOnce(&mut egui::Ui)) {
    framed(ui, p, p.surface_2, highlight, body);
}

/// A result card: `surface` with a hairline border, so status and loss colours read on it.
fn card(ui: &mut egui::Ui, p: &Palette, body: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    framed(ui, p, p.surface, false, body)
}

fn framed(
    ui: &mut egui::Ui,
    p: &Palette,
    fill: egui::Color32,
    highlight: bool,
    body: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let stroke = if highlight {
        Stroke::new(1.5_f32, p.spectre)
    } else {
        Stroke::new(1.0_f32, p.hairline)
    };
    Frame::new()
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(Margin::same(theme::CARD_PADDING as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::compact_rows(ui);
            body(ui);
        })
        .response
}

/// A file's name, then its folder in `tide`, cut short to fit; the whole path on hover.
fn path_label(ui: &mut egui::Ui, p: &Palette, path: &Path) {
    let folder = path.parent().map(display).unwrap_or_default();
    let mut job = egui::text::LayoutJob::default();
    RichText::new(file_name(path)).append_to(
        &mut job,
        ui.style(),
        egui::FontSelection::Default,
        Align::Center,
    );
    RichText::new(format!("   {folder}"))
        .small()
        .color(p.tide)
        .append_to(
            &mut job,
            ui.style(),
            egui::FontSelection::Default,
            Align::Center,
        );
    ui.add(egui::Label::new(job).truncate())
        .on_hover_text(display(path));
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| display(path))
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn preset_description(preset: Preset) -> &'static str {
    match preset {
        Preset::Fast => "Shape only; no feature history, sketches, parameters or joints. Least memory.",
        Preset::Standard => "Feature history, sketches, parameters and joints that map; leaves history out if it would not fit in free memory.",
        Preset::Full => "Feature history, sketches, parameters and joints that map. Most memory; keeps history regardless of free memory.",
    }
}

fn preset_help(response: &egui::Response, description: &str) {
    if response.has_focus() || response.hovered() {
        egui::Tooltip::always_open(
            response.ctx.clone(),
            response.layer_id,
            response.id.with("preset_help"),
            response.rect,
        )
        .show(|ui| {
            ui.label(description);
        });
    }
    response.ctx.accesskit_node_builder(response.id, |node| {
        node.set_description(description);
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn results_scroll_to_the_last_private_send_option() {
        use egui_kittest::kittest::Queryable;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(760.0, 680.0))
            .build_eframe(|cc| {
                let mut app = App::new(cc);
                for index in 0..6 {
                    app.rows.push(Row {
                        input: PathBuf::from(format!("synthetic-{index}.f3d")),
                        output: None, status: Status::Failed, progress: None, job: None,
                        size: Some(100),
                        report: Some(json!({"ok": false, "error": {"message": "Conversion stopped.", "stage": "decode", "kind": "decode"}})),
                    });
                }
                app
            });
        harness.run_steps(8);
        // The other ways to send a failed file are folded under "Help us fix it".
        harness
            .query_all_by_label(present::SEND_FOLD)
            .last()
            .unwrap()
            .click_accesskit();
        harness.run_steps(8);
        harness
            .query_all_by_label(present::SEND_DM_BUTTON)
            .last()
            .unwrap()
            .scroll_to_me();
        harness.run_steps(12);
        let button = harness
            .query_all_by_label(present::SEND_DM_BUTTON)
            .last()
            .unwrap()
            .rect();
        assert!(button.top() > 80.0);
        assert!(button.bottom() < 620.0);
    }

    #[test]
    fn failed_card_offers_diagnosis_after_a_click() {
        use egui_kittest::kittest::Queryable;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(900.0, 1000.0))
            .build_state(
                |ctx, chosen| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let row = Row {
                            input: PathBuf::from("synthetic.f3d"),
                            output: None,
                            status: Status::Failed,
                            progress: None,
                            job: None,
                            size: None,
                            report: Some(
                                json!({"ok": false, "error": {"message": "Conversion stopped."}}),
                            ),
                        };
                        let view = row.view();
                        if let Some(Action::Diagnose(0)) = row_ui(
                            ui,
                            theme::palette(ui),
                            0,
                            &row,
                            &view,
                            ReportPolicy::bundled(),
                            false,
                            false,
                            None,
                            None,
                        ) {
                            *chosen = true;
                        }
                    });
                },
                false,
            );
        harness.run_steps(2);
        assert!(!harness.state());
        harness.get_by_label("Diagnose this file").click();
        harness.run_steps(2);
        assert!(*harness.state());
    }

    #[test]
    fn converted_card_reports_only_after_expanding_and_reviews_exact_metadata() {
        use egui_kittest::kittest::Queryable;
        for width in [1100.0, 520.0] {
            let dir = tempfile::tempdir().unwrap();
            // Seed an earlier failed report: a converted card must never reuse its bundle.
            let input = dir.path().join("synthetic.f3d");
            let salt = dutchman::anon::Salt::random().unwrap();
            let id = salt.file_id(&input);
            let old_folder = dutchman::report_send::local_folder(dir.path(), &id);
            let mut old =
                dutchman::report_send::Prepared::new(&id, "f3d", "old-failure", None, &old_folder)
                    .unwrap();
            old.bundle_sha256 = Some("old-bundle".into());
            std::fs::write(
                old_folder.join("prepared.json"),
                serde_json::to_vec(&old).unwrap(),
            )
            .unwrap();
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(width, 1500.0))
                .build_eframe(|cc| {
                    let mut app = App::new(cc);
                    app.out_dir = Some(dir.path().to_path_buf());
                    app.salt = Some(salt);
                    app.rows.push(Row {
                        input: input.clone(), output: Some(dir.path().join("synthetic.sc")),
                        status: Status::Converted, progress: None, job: None, size: Some(100),
                        report: Some(json!({"ok": true, "input": {"path": input, "format": "fusion_archive"}})),
                    });
                    app
                });
            harness.run_steps(8);
            harness.get_by_label(present::REPORT_CONVERTED);
            assert!(harness.query_by_label("Review and send report").is_none());
            assert!(harness.query_by_label(present::SEND_DM_BUTTON).is_none());
            harness
                .get_by_label(present::REPORT_CONVERTED)
                .click_accesskit();
            harness.run_steps(8);
            assert!(harness.query_by_label("Diagnose this file").is_none());
            let review = harness.get_by_label("Review and send report").rect();
            let join = harness.get_by_label(present::SEND_JOIN_BUTTON).rect();
            let dm = harness.get_by_label(present::SEND_DM_BUTTON).rect();
            assert!(review.bottom() < join.top());
            assert!(join.top() <= dm.top());
            for rect in [review, join, dm] {
                assert!(rect.left() >= 0.0 && rect.right() <= width);
            }
            harness
                .get_by_label("Review and send report")
                .click_accesskit();
            harness.run_steps(8);
            harness.get_by_label("This file converted, but the result is wrong.");
            harness.get_by_label("Anonymous summary only");
            assert!(harness.query_by_label("Send the bundle").is_none());
            let card = harness.state().consent.card.as_ref().unwrap();
            assert_eq!(card.prepared.report_id, id);
            assert_eq!(
                card.prepared.signature,
                dutchman::report_send::CONVERTED_PROBLEM_SIGNATURE
            );
            assert!(card.prepared.bundle_sha256.is_none());
            assert_eq!(card.folder, old_folder.join("converted"));
            let metadata = card.prepared.metadata("summary", false);
            let text = serde_json::to_string_pretty(&metadata).unwrap();
            harness.get_by_label(&text);
            assert!(!text.contains("synthetic"));
            assert!(!text.contains("old-failure"));
            assert!(!text.contains("claim_secret"));
            assert!(metadata.get("description").is_none());
            assert!(!card.folder.join("submission.json").exists());
        }
    }

    #[test]
    fn whole_file_choice_opens_private_steps_without_sending() {
        use egui_kittest::kittest::Queryable;
        let dir = tempfile::tempdir().unwrap();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1100.0, 1500.0))
            .build_eframe(|cc| {
                let mut app = App::new(cc);
                app.out_dir = Some(dir.path().to_path_buf());
                app.rows.push(Row { input: PathBuf::from("synthetic.f3d"), output: None, status: Status::Failed,
                    progress: None, report: Some(json!({"ok": false, "input": {"path": "synthetic.f3d", "format_name": "Fusion"}, "error": {"stage": "decode", "kind": "decode"}})), job: None, size: Some(100) });
                app.open_consent(0).unwrap();
                app
            });
        harness.run_steps(8);
        let folder = harness
            .state()
            .consent
            .card
            .as_ref()
            .unwrap()
            .folder
            .clone();
        harness
            .get_by_label("Send the whole file privately")
            .click_accesskit();
        harness.step();
        assert!(harness.state().private_send.is_some());
        assert!(harness.state().consent.card.is_none());
        assert!(!folder.join("submission.json").exists());
    }

    #[test]
    fn a_dismissed_unfinished_run_stays_dismissed_until_another_one() {
        let (first, second) = (PathBuf::from("first.jsonl"), PathBuf::from("second.jsonl"));
        assert_eq!(undismissed(Some(first.clone()), Some(&first)), None);
        assert_eq!(
            undismissed(Some(second.clone()), Some(&first)),
            Some(second)
        );
        assert_eq!(undismissed(Some(first.clone()), None), Some(first));
        assert_eq!(undismissed(None, None), None);
    }

    #[test]
    fn part_positions_and_history_choices_default_and_roundtrip() {
        use egui_kittest::kittest::Queryable;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 1800.0))
            .build_eframe(|cc| App::new(cc));
        harness.run_steps(8);
        assert!(!harness.state().options.no_verify);
        assert!(!harness.state().options.stored_placements);
        assert!(!harness.state().options.no_memory_check);
        harness.get_by_label("Advanced").click_accesskit();
        harness.run_steps(8);
        harness.get_by_label("Part positions").click_accesskit();
        harness.run_steps(8);
        harness.get_by_label("As first placed").click_accesskit();
        harness.run_steps(8);
        assert_eq!(
            harness.state().options.part_positions,
            dutchman_core::PartPositions::FirstPlaced
        );
        harness
            .get_by_label("Check feature history in SolidCraft")
            .click_accesskit();
        harness.run_steps(8);
        assert!(harness.state().options.no_verify);
        harness
            .get_by_label("Convert and check history even if memory is low")
            .click_accesskit();
        harness.run_steps(8);
        assert!(harness.state().options.no_memory_check);
        assert!(harness.state().options.no_verify);
    }

    #[test]
    fn preset_and_advanced_choices_map_to_shared_options() {
        let mut options = ConvertOptions::default();
        assert_eq!(options.preset, Preset::Standard);
        assert_eq!(options.choices(), Choices::of(Preset::Standard));
        for preset in [Preset::Fast, Preset::Standard, Preset::Full] {
            select_preset(&mut options, preset);
            assert_eq!(options.choices(), Choices::of(preset));
            for history in [true, false] {
                for sketches in [true, false] {
                    for joints in [true, false] {
                        set_choices(&mut options, history, sketches, joints);
                        assert_eq!(options.preset, preset);
                        assert_eq!(options.import_only, !history);
                        assert_eq!(options.no_sketches, !sketches);
                        assert_eq!(options.no_joints, !joints);
                        let base = Choices::of(preset);
                        assert_eq!(
                            options.choices(),
                            Choices {
                                history: base.history && history,
                                sketches: base.sketches && sketches,
                                joints: base.joints && joints,
                                history_within_budget: base.history_within_budget,
                            }
                        );
                    }
                }
            }
        }
        select_preset(&mut options, Preset::Standard);
        assert_eq!(options.choices(), Choices::of(Preset::Standard));
    }

    #[test]
    fn part_positions_restore_legacy_settings_and_remember_choices() {
        for import_only in [false, true] {
            let mut options = ConvertOptions::default();
            let custom = restore_choices(&mut options, &json!({"import_only": import_only}));
            assert_eq!(options.preset, Preset::Standard);
            assert_eq!(options.choices().history, !import_only);
            assert!(options.choices().sketches);
            assert!(options.choices().joints);
            assert_eq!(custom, import_only);
        }
        let mut options = ConvertOptions::default();
        assert!(!restore_choices(&mut options, &json!({})));
        select_preset(&mut options, Preset::Full);
        set_choices(&mut options, true, false, true);
        options.no_memory_check = true;
        options.no_verify = true;
        options.diagnostics_default = false;
        options.stored_placements = true;
        let saved = saved_choices(&options, true);
        let mut restored = ConvertOptions::default();
        assert!(restore_choices(&mut restored, &saved));
        assert_eq!(restored.preset, Preset::Full);
        assert_eq!(restored.choices(), options.choices());
        assert!(!restored.no_memory_check);
        assert!(restored.no_verify);
        // An older saved "save_diagnostics" choice changes nothing: every run keeps its log.
        assert!(restored.diagnostics_default);
        let mut older = ConvertOptions::default();
        restore_choices(
            &mut older,
            &json!({"preset": "full", "save_diagnostics": false}),
        );
        assert!(older.diagnostics_default);
        assert_eq!(
            restored.effective_part_positions(),
            dutchman_core::PartPositions::FirstPlaced
        );
        assert!(restored.diagnostics.is_none());
        for mode in [
            dutchman_core::PartPositions::Smart,
            dutchman_core::PartPositions::LastShown,
            dutchman_core::PartPositions::FirstPlaced,
        ] {
            options.stored_placements = false;
            options.part_positions = mode;
            let mut restored = ConvertOptions::default();
            restore_choices(&mut restored, &saved_choices(&options, true));
            assert_eq!(restored.effective_part_positions(), mode);
        }
        let mut legacy = ConvertOptions::default();
        restore_choices(&mut legacy, &json!({"stored_placements":true}));
        assert_eq!(
            legacy.effective_part_positions(),
            dutchman_core::PartPositions::FirstPlaced
        );
    }

    #[test]
    fn run_log_completion_renames_only_its_rows_and_marks_stopped() {
        for stopped in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let running = dir
                .path()
                .join("2026-11-02_163710_dutchman-0.2.6_running.jsonl");
            let prior = dir
                .path()
                .join("2026-11-02_150000_dutchman-0.2.6_failed.jsonl");
            std::fs::write(&running, "").unwrap();
            let mut harness = egui_kittest::Harness::builder().build_eframe(|cc| {
                let mut app = App::new(cc);
                app.diagnostic_path = Some(running.clone());
                app.diagnostic_automatic = true;
                app.diagnostic_stopped = stopped;
                app.row_logs.insert(0, prior.clone());
                app.row_logs.insert(1, running.clone());
                app.rows = (0..2)
                    .map(|_| Row {
                        input: PathBuf::from("synthetic.f3d"),
                        output: None,
                        status: Status::Failed,
                        report: Some(json!({"ok": false})),
                        progress: None,
                        job: None,
                        size: None,
                    })
                    .collect();
                let prepared = dutchman::report_send::Prepared::new(
                    "0123456789abcdef",
                    "f3d",
                    "decode",
                    None,
                    dir.path(),
                )
                .unwrap();
                let mut card =
                    crate::report_consent::Card::new(1, prepared, dir.path().to_path_buf());
                card.log = Some(running.clone());
                card.log_ready = false;
                app.consent.card = Some(card);
                app.sender.send(Message::Done).unwrap();
                app
            });
            harness.run_steps(2);
            let app = harness.state();
            assert_eq!(app.row_logs.get(&0), Some(&prior));
            let finished = app.row_logs.get(&1).unwrap();
            assert_eq!(Some(finished), app.diagnostic_path.as_ref());
            assert!(finished.is_file());
            let card = app.consent.card.as_ref().unwrap();
            assert_eq!(card.log.as_ref(), Some(finished));
            assert!(card.log_ready);
            assert!(finished.to_str().unwrap().ends_with(if stopped {
                "_stopped.jsonl"
            } else {
                "_failed.jsonl"
            }));
            assert!(!running.exists());
        }
    }

    #[test]
    fn diagnostics_setting_and_unfinished_notice_are_available_and_dismissable() {
        use egui_kittest::kittest::Queryable;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 1600.0))
            .build_eframe(|cc| {
                let mut app = App::new(cc);
                app.unfinished_diagnostic =
                    Some(PathBuf::from("diagnostics/2026-10-29_201500.jsonl"));
                app
            });
        harness.run_steps(8);
        harness.get_by_label("Open diagnostics folder");
        harness.get_by_label("Open its log folder");
        harness.get_by_label("The last run didn't finish.");
        // Toggle through accessibility rather than a pointer position, which depends on how
        // the platform's fonts lay the form out.
        // Every run keeps its log: there is no checkbox to turn that off.
        assert!(harness
            .query_by_label("Save a diagnostic log (no names or contents from your file)")
            .is_none());
        harness.get_by_label("Dismiss").click_accesskit();
        harness.run_steps(8);
        assert!(harness.state().unfinished_diagnostic.is_none());
    }

    #[test]
    fn changing_a_checkbox_marks_custom_and_selecting_a_preset_resets_it() {
        use egui_kittest::kittest::Queryable;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 1400.0))
            .build_eframe(|cc| App::new(cc));
        harness.run_steps(8);
        harness.get_by_label("Advanced").click_accesskit();
        harness.run_steps(8);
        harness.get_by_label("Feature history").click_accesskit();
        harness.run_steps(8);
        assert!(harness.state().custom);
        assert!(harness.state().options.import_only);
        assert_eq!(harness.state().options.preset, Preset::Standard);
        harness
            .get_by_value("Custom (based on Standard)")
            .click_accesskit();
        // The preset menu can take more than one frame to open on a slow runner.
        for _ in 0..60 {
            harness.step();
            if harness.query_by_label("Full").is_some() {
                break;
            }
        }
        harness.get_by_label("Full").click_accesskit();
        // The menu's closing animation can outlast `run`'s step limit on a slow runner.
        harness.run_steps(8);
        assert!(!harness.state().custom);
        assert_eq!(harness.state().options.choices(), Choices::of(Preset::Full));
    }

    #[test]
    fn tiny_estimates_use_lower_bounds() {
        use egui_kittest::kittest::Queryable;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 1400.0))
            .build_eframe(|cc| {
                let mut app = App::new(cc);
                let input = PathBuf::from("synthetic.f3d");
                app.inputs = vec![input.clone()];
                app.size_inputs = Some(app.inputs.clone());
                app.estimate_key = Some(app.current_estimate_key());
                app.estimates = vec![(
                    input,
                    Some(Estimate {
                        peak_bytes: 1,
                        seconds: 1,
                        seconds_exact: 1.0,
                        band: 0.3,
                        memory_band: 0.3,
                        time_band: None,
                    }),
                )];
                app
            });
        harness.run_steps(8);
        assert!(harness
            .query_by_label("under 0.1 GB, under a minute")
            .is_some());
    }

    #[test]
    fn queued_estimates_ignore_old_choices_and_show_current_numbers() {
        use egui_kittest::kittest::Queryable;
        let input = PathBuf::from("synthetic.f3d");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 1400.0))
            .build_eframe(|cc| {
                let mut app = App::new(cc);
                app.inputs = vec![input.clone()];
                app.size_inputs = Some(app.inputs.clone());
                app.estimate_key = Some(app.current_estimate_key());
                app.estimate_generation = 2;
                app.sender
                    .send(Message::Estimated {
                        sizes: Vec::new(),
                        generation: 1,
                        files: vec![(
                            input.clone(),
                            Some(Estimate {
                                peak_bytes: 90_000_000_000,
                                seconds: 60,
                                seconds_exact: 60.0,
                                band: 0.3,
                                memory_band: 0.3,
                                time_band: Some(0.3),
                            }),
                        )],
                        available: Some(1_000_000_000),
                    })
                    .unwrap();
                app.sender
                    .send(Message::Estimated {
                        sizes: Vec::new(),
                        generation: 2,
                        files: vec![(
                            input.clone(),
                            Some(Estimate {
                                peak_bytes: 2_500_000_000,
                                seconds: 121,
                                seconds_exact: 121.0,
                                band: 0.3,
                                memory_band: 0.3,
                                time_band: Some(0.3),
                            }),
                        )],
                        available: Some(4_000_000_000),
                    })
                    .unwrap();
                app
            });
        harness.run_steps(8);
        assert!(harness
            .query_by_label("about 2.5 GB, about 3 min")
            .is_some());
        assert!(harness.query_by_label("4.0 GB of memory free").is_some());
        assert!(harness
            .query_by_label("about 90.0 GB, about 1 min")
            .is_none());
    }

    #[test]
    fn unknown_or_zero_memory_has_no_free_memory_label() {
        use egui_kittest::kittest::Queryable;
        for available in [None, Some(0)] {
            let mut harness = egui_kittest::Harness::builder().build_eframe(|cc| {
                let mut app = App::new(cc);
                app.available = available;
                app.size_inputs = Some(app.inputs.clone());
                app.estimate_key = Some(app.current_estimate_key());
                app
            });
            harness.run_steps(8);
            assert_eq!(
                harness
                    .query_all_by_label_contains("GB of memory free")
                    .count(),
                0
            );
        }
    }

    mod loss_fixture {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/support/losses.rs"
        ));
    }

    #[test]
    fn expanded_loss_labels_stay_inside_the_card() {
        use egui_kittest::kittest::Queryable;
        let row = Row {
            input: PathBuf::from("synthetic.f3d"),
            output: Some(PathBuf::from("converted/synthetic.sc")),
            status: Status::Converted,
            progress: None,
            job: None,
            size: None,
            report: Some(loss_fixture::report()),
        };
        let view = row.view();
        let lines = &view.components[0].lines;
        assert_eq!(lines.len(), 3);
        for width in [520.0, 1000.0] {
            let mut installed = false;
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(width, 900.0))
                .build_state(
                    |ctx, card_rect| {
                        // Font definitions take effect on the following frame.
                        if !installed {
                            theme::install(ctx);
                            installed = true;
                            return;
                        }
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let p = *theme::palette(ui);
                            *card_rect = card(ui, &p, |ui| {
                                row_ui(
                                    ui,
                                    &p,
                                    0,
                                    &row,
                                    &view,
                                    ReportPolicy::bundled(),
                                    true,
                                    false,
                                    None,
                                    None,
                                );
                            })
                            .rect;
                        });
                    },
                    egui::Rect::NOTHING,
                );
            harness.run_steps(8);
            harness.get_by_label(&view.loss_summary).click_accesskit();
            harness.run_steps(8);
            let card_rect = *harness.state();
            assert!(card_rect.left() >= 0.0 && card_rect.right() <= width);
            for loss in lines {
                let label = format!("{}{}", loss.level, loss.text);
                let rect = harness.get_by_label(&label).rect();
                assert!(
                    rect.left() >= card_rect.left() && rect.right() <= card_rect.right(),
                    "{width}px: loss {label} at {rect:?} exceeds card {card_rect:?}"
                );
            }
        }
    }

    #[test]
    fn loss_prose_wraps_at_spaces() {
        let mut installed = false;
        let mut harness = egui_kittest::Harness::builder().build_ui(|ui| {
            if !installed {
                theme::install(ui.ctx());
                installed = true;
                return;
            }
            let p = *theme::palette(ui);
            ui.set_width(240.0);
            loss_line(
                ui,
                &p,
                &present::LossLine {
                    level: "Info".into(),
                    text: "ordinary words should wrap at spaces whenever they fit on a line".into(),
                    full_text: String::new(),
                },
            );
        });
        harness.run_steps(2);
        // Inspect the actual rendered paragraph, including the coloured severity prefix.
        let output = harness.output();
        let galley = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(&text.galley),
                _ => None,
            })
            .unwrap();
        assert!(galley.rows.len() > 1);
        for row in galley.rows.iter().take(galley.rows.len() - 1) {
            assert!(row.glyphs.last().unwrap().chr.is_whitespace());
        }
    }

    #[test]
    fn history_budget_note_is_visible_with_losses_collapsed() {
        use egui_kittest::kittest::Queryable;
        let message = "feature history left out: this assembly needs about 8 GB with history and 4 GB are free; use the full preset to keep it";
        let row = Row {
            input: PathBuf::from("synthetic.f3z"),
            output: Some(PathBuf::from("out.solidcraft")),
            status: Status::Converted,
            progress: None,
            job: None,
            size: None,
            report: Some(json!({"ok": true, "mode": "import", "losses": [
                {"level": "Info", "subject": "history", "message": message}
            ]})),
        };
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 600.0))
            .build_ui(|ui| {
                theme::install(ui.ctx());
                let p = *theme::palette(ui);
                row_ui(
                    ui,
                    &p,
                    0,
                    &row,
                    &row.view(),
                    ReportPolicy::bundled(),
                    true,
                    false,
                    None,
                    None,
                );
            });
        harness.run_steps(8);
        assert!(harness.query_by_label(message).is_some());
    }

    #[test]
    fn a_design_without_history_says_why_once() {
        let report = json!({"ok": true, "losses": [
            {"level": "Info", "subject": "history", "message": "flattened: the design carries no feature history"},
            {"level": "Info", "subject": "history", "message": "sketches kept as geometry"}]});
        let (outcome, notes) = outcome_and_history("Geometry without history.", Some(&report));
        assert_eq!(
            outcome,
            "Geometry without history: the design carries no feature history."
        );
        assert_eq!(notes, ["sketches kept as geometry"]);
        // A reason already in the outcome line is not repeated under it.
        let (outcome, notes) = outcome_and_history(
            "Geometry without history (flattened: fillet is not mapped yet).",
            Some(&report),
        );
        assert_eq!(
            outcome,
            "Geometry without history (flattened: fillet is not mapped yet)."
        );
        assert_eq!(notes, ["sketches kept as geometry"]);
        // Editable history keeps its notes as they are.
        let (outcome, notes) = outcome_and_history("Editable history; unverified.", Some(&report));
        assert_eq!(outcome, "Editable history; unverified.");
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn result_list_uses_shared_strings() {
        use egui_kittest::kittest::Queryable;
        let reports = [
            json!({"ok": true, "mode": "history", "history": {"mapped": 3, "total": 3},
                "verification": {"status": "verified"}, "losses": []}),
            json!({"ok": true, "mode": "import", "verification": {"status": "flattened", "reason": "fillet is not mapped yet"},
                "losses": [{"level": "Degraded", "subject": "history", "message": "flattened: fillet is not mapped yet"}]}),
            json!({"ok": true, "completeness": {"placed": 1, "placed_empty": 1, "placements": 3, "left_out": [{"file": "pin.ipt", "count": 1, "reason": "not found", "stage": "input"}]},
                "losses": [{"level": "Dropped", "subject": "occurrence", "message": "pin.ipt: not found at its stored path"}]}),
            json!({"ok": false, "error": {"message": "cannot read this design: no geometry could be read", "stage": "decode", "upstream": "cadmpeg"}}),
        ];
        let rows: Vec<Row> = reports
            .into_iter()
            .enumerate()
            .map(|(n, report)| Row {
                input: PathBuf::from(format!("design-{n}.f3d")),
                output: Some(PathBuf::from(format!("converted/design-{n}.solidcraft"))),
                status: if report.get("ok") == Some(&Value::Bool(true)) {
                    Status::Converted
                } else {
                    Status::Failed
                },
                report: Some(report),
                progress: None,
                job: None,
                size: None,
            })
            .collect();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 1200.0))
            .build_ui(|ui| {
                theme::install(ui.ctx());
                let p = *theme::palette(ui);
                for (index, row) in rows.iter().enumerate() {
                    card(ui, &p, |ui| {
                        row_ui(
                            ui,
                            &p,
                            index,
                            row,
                            &row.view(),
                            ReportPolicy::bundled(),
                            true,
                            false,
                            None,
                            None,
                        );
                    });
                    ui.add_space(theme::SPACE_2);
                }
            });
        harness.run_steps(8);
        for label in [
            "Editable history, 3 of 3 features rebuilt; verified in SolidCraft.",
            "Geometry without history (flattened: fillet is not mapped yet).",
            "Incomplete",
            "1 of 3 components placed; 1 placed empty, no joints",
            "1 part file was not found: pin.ipt",
            "Losses: 1 Degraded",
            "Losses: 1 Dropped",
            "cannot read this design: no geometry could be read",
            "Review and send report",
            present::SEND_FOLD,
        ] {
            assert!(harness.query_by_label(label).is_some(), "missing {label}");
        }
        use egui_kittest::kittest::NodeT;
        let labels = harness
            .query_all_by(|node| node.label().is_some())
            .filter_map(|node| node.accesskit_node().label())
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(dir) = std::env::var_os("DUTCHMAN_PRESENT_SCREENSHOTS") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(PathBuf::from(&dir).join("synthetic-results.txt"), &labels).unwrap();

            harness
                .render()
                .unwrap()
                .save(PathBuf::from(&dir).join("synthetic-results.png"))
                .unwrap();
            if let Some(before) = std::env::var_os("DUTCHMAN_PRESENT_COMPARE") {
                assert_eq!(
                    std::fs::read_to_string(PathBuf::from(&before).join("synthetic-results.txt"))
                        .unwrap(),
                    labels
                );
                let before = image::open(PathBuf::from(before).join("synthetic-results.png"))
                    .unwrap()
                    .to_rgba8();
                let after = image::open(PathBuf::from(dir).join("synthetic-results.png"))
                    .unwrap()
                    .to_rgba8();
                assert_eq!(before.dimensions(), after.dimensions());
                let changed = before
                    .pixels()
                    .zip(after.pixels())
                    .filter(|(a, b)| a != b)
                    .count();
                assert_eq!(changed, 0, "result list pixels changed");
            }
        }
    }

    #[test]
    fn converting_cards_show_stage_counts_and_elapsed_time() {
        use egui_kittest::kittest::Queryable;
        for total in [Some(4), None] {
            let event = dutchman_core::ProgressEvent {
                stage: dutchman_core::Stage::Decode,
                elapsed_ms: 15_000,
                designs_read: 2,
                designs_total: total,
                heartbeat: true,
            };
            let row = Row {
                input: PathBuf::from("assembly.f3z"),
                output: None,
                status: Status::Converting,
                progress: Some(event),
                report: None,
                job: None,
                size: None,
            };
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1200.0, 400.0))
                .build_ui(|ui| {
                    theme::install(ui.ctx());
                    let p = *theme::palette(ui);
                    row_ui(
                        ui,
                        &p,
                        0,
                        &row,
                        &row.view(),
                        ReportPolicy::bundled(),
                        true,
                        false,
                        None,
                        None,
                    );
                });
            harness.run_steps(2);
            assert!(harness.query_by_label(&event.to_string()).is_some());
        }
    }

    #[test]
    fn a_blocked_checker_shows_the_recovery_message() {
        use egui_kittest::kittest::Queryable;
        let message =
            "macOS has not yet allowed SolidCraft to run: open SolidCraft once from Applications.";
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(760.0, 680.0))
            .build_eframe(move |cc| {
                let mut app = App::new(cc);
                app.checker = Err(dutchman_solidcraft::NoCli::Blocked(message.to_owned()));
                app
            });
        harness.run_steps(8);
        assert!(harness.query_by_label(message).is_some());
    }
}
