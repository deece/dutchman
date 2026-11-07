//! Conversions, off the window's thread. A run plans its files and converts them with the
//! shared batch code `dutchman convert-all` uses, sending each step back as a message and
//! asking the window to repaint.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use dutchman::batch::{self, Event, Job};
use dutchman::choices::{self, Estimate};
use dutchman::ConvertOptions;
use dutchman_core::Writer;
use eframe::egui;
use serde_json::Value;

/// What the background work tells the window.
#[derive(Debug)]
pub enum Message {
    /// Project plan and whether Inventor settings apply.
    ProjectSummary {
        generation: u64,
        text: String,
        inventor: bool,
    },
    /// Files and folders chosen in a dialog, for what it was opened for.
    Picked { pick: Pick, paths: Vec<PathBuf> },
    /// Estimates for one snapshot of the queued inputs and choices.
    Estimated {
        generation: u64,
        files: Vec<(PathBuf, Option<Estimate>)>,
        available: Option<u64>,
        sizes: Vec<(PathBuf, Option<choices::Sizes>)>,
    },
    /// A run's plan: inputs that could not be planned (as failed reports), then its jobs.
    Planned {
        problems: Vec<Value>,
        jobs: Vec<Job>,
    },
    /// A finished file, and its size when it can be read.
    Sized { input: PathBuf, size: Option<u64> },
    /// The design result is still available, but its local report could not be saved.
    ReportSaveFailed(String),
    /// A job of the run started or finished.
    Progress(Event),
    /// The run is over: every job finished, or it was stopped.
    Done,
}

/// The size of the file at `input`, read here so the window never waits on a slow disk.
fn sized(input: &Path) -> Message {
    Message::Sized {
        input: input.to_path_buf(),
        size: std::fs::metadata(input).ok().map(|file| file.len()),
    }
}

fn save_report(output: &Path, report: &Value, send: &impl Fn(Message)) {
    if let Err(message) = dutchman::saved_report::save(output, report) {
        send(Message::ReportSaveFailed(message));
    }
}

/// A run in progress.
pub struct Run {
    stop: Arc<AtomicBool>,
}

/// Set by tests, whose own program is not a converter, so they convert here instead.
static IN_PROCESS: AtomicBool = AtomicBool::new(cfg!(test));

/// Converts in this process from now on, for tests.
pub fn convert_in_process_for_tests() {
    IN_PROCESS.store(true, Ordering::Relaxed);
}

/// The program each file converts in, so running low on memory or crashing ends that file and
/// not the app.
fn converter_program() -> Option<std::path::PathBuf> {
    if IN_PROCESS.load(Ordering::Relaxed) {
        None
    } else {
        std::env::current_exe().ok()
    }
}

impl Run {
    /// Converts `inputs` into `out_dir` in the background.
    pub fn start(
        inputs: Vec<PathBuf>,
        out_dir: PathBuf,
        options: &ConvertOptions,
        messages: Sender<Message>,
        ctx: egui::Context,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let writer = options.writer();
        let options = options.clone();
        let flag = Arc::clone(&stop);
        std::thread::spawn(move || {
            let send = |message: Message| {
                let _ = messages.send(message);
                ctx.request_repaint();
            };
            // A panic here must not take the window down; it ends the run.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let batch::Plan {
                    mut jobs, problems, ..
                } = batch::plan(&inputs, &out_dir);
                batch::prepare(&mut jobs, &options);
                let refused: Vec<PathBuf> = problems
                    .iter()
                    .filter_map(|report| report.pointer("/input/path")?.as_str())
                    .map(PathBuf::from)
                    .collect();
                for report in &problems {
                    if let Some(name) = report
                        .pointer("/input/path")
                        .and_then(Value::as_str)
                        .and_then(|path| Path::new(path).file_name())
                    {
                        save_report(
                            &out_dir.join(name).with_extension(writer.extension()),
                            report,
                            &send,
                        );
                    }
                }
                send(Message::Planned {
                    problems,
                    jobs: jobs.clone(),
                });
                for input in &refused {
                    send(sized(input));
                }
                let on_event = |event: Event| {
                    if let Event::Finished(index, report) = &event {
                        if let Some(job) = jobs.get(*index) {
                            save_report(&job.output, report, &send);
                            send(sized(&job.input));
                        }
                    }
                    send(Message::Progress(event));
                };
                match converter_program() {
                    Some(program) => batch::run_isolated(
                        &jobs,
                        &options,
                        &program,
                        batch::workers(None),
                        &flag,
                        on_event,
                    ),
                    None => batch::run_each(&jobs, &writer, batch::workers(None), &flag, on_event),
                }
            }));
            send(Message::Done);
        });
        Run { stop }
    }

    /// Converts `job`, the run's job at `index`, again in the background, as it now says.
    pub fn retry(
        job: Job,
        index: usize,
        options: &ConvertOptions,
        messages: Sender<Message>,
        ctx: egui::Context,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let writer = options.writer();
        let options = options.clone();
        let flag = Arc::clone(&stop);
        let input = job.input.clone();
        let output = job.output.clone();
        std::thread::spawn(move || {
            let send = |message: Message| {
                let _ = messages.send(message);
                ctx.request_repaint();
            };
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let on_event = |event: Event| {
                    if let Event::Finished(_, report) = &event {
                        save_report(&output, report, &send);
                    }
                    if matches!(&event, Event::Finished(_, _)) {
                        send(sized(&input));
                    }
                    send(Message::Progress(match event {
                        Event::Started(_) => Event::Started(index),
                        Event::Progress(_, event) => Event::Progress(index, event),
                        Event::Finished(_, report) => Event::Finished(index, report),
                    }));
                };
                match converter_program() {
                    Some(program) => {
                        batch::run_isolated(&[job], &options, &program, 1, &flag, on_event)
                    }
                    None => batch::run_each(&[job], &writer, 1, &flag, on_event),
                }
            }));
            send(Message::Done);
        });
        Run { stop }
    }

    /// Starts no further file; those converting finish.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Plans folders and estimates their files without blocking a frame.
pub fn estimates(
    inputs: Vec<PathBuf>,
    options: ConvertOptions,
    checker_found: bool,
    generation: u64,
    messages: Sender<Message>,
    ctx: egui::Context,
) {
    std::thread::spawn(move || {
        let plan = batch::plan(&inputs, std::path::Path::new(""));
        let _ = messages.send(Message::ProjectSummary {
            generation,
            text: plan.description,
            inventor: plan
                .jobs
                .iter()
                .any(|job| matches!(batch::extension_of(&job.input).as_str(), "ipt" | "iam")),
        });
        let available = choices::available_memory();
        let mut sizes = Vec::new();
        let files = plan
            .jobs
            .into_iter()
            .map(|job| {
                let size = choices::read_sizes(&job.input);
                let estimate = size
                    .and_then(|sizes| choices::desktop_estimate(sizes, &options, checker_found));
                sizes.push((job.input.clone(), size));
                (job.input, estimate)
            })
            .collect();
        let _ = messages.send(Message::Estimated {
            generation,
            files,
            available,
            sizes,
        });
        ctx.request_repaint();
    });
}

/// Opens a file dialog in the background and sends what was picked.
pub fn pick(kind: Pick, messages: Sender<Message>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let dialog = rfd::AsyncFileDialog::new();
        let paths: Vec<PathBuf> = pollster::block_on(async {
            match kind {
                Pick::Files => dialog
                    .set_title("Add designs")
                    .add_filter(
                        "CAD designs",
                        &[
                            "f3d", "f3z", "ipt", "iam", "sldprt", "sldasm", "FCStd", "fcstd",
                            "CATPart", "catpart", "prt", "pvz", "3dm", "sat", "sab", "smt", "smb",
                            "step", "stp", "igs", "iges", "zip",
                        ],
                    )
                    .pick_files()
                    .await
                    .unwrap_or_default(),
                Pick::Folder => dialog
                    .set_title("Add a folder of designs")
                    .pick_folders()
                    .await
                    .unwrap_or_default(),
                Pick::Output => dialog
                    .set_title("Choose the output folder")
                    .pick_folder()
                    .await
                    .into_iter()
                    .collect(),
                Pick::Project => dialog
                    .set_title("Choose the project folder for assemblies' part files")
                    .pick_folder()
                    .await
                    .into_iter()
                    .collect(),
                Pick::SearchFolder(_) => dialog
                    .set_title("Choose a folder holding the missing part files")
                    .pick_folder()
                    .await
                    .into_iter()
                    .collect(),
                Pick::SaveLog(_) => dialog
                    .set_title("Save the full log")
                    .set_file_name("dutchman.log")
                    .add_filter("Log", &["log", "txt"])
                    .save_file()
                    .await
                    .into_iter()
                    .collect(),
                Pick::SolidCraftCli => dialog
                    .set_title("Choose solidcraft-cli")
                    .pick_file()
                    .await
                    .into_iter()
                    .collect(),
                Pick::SearchFile(_) => dialog
                    .set_title("Find a missing part file")
                    .add_filter("Inventor parts and assemblies", &["ipt", "iam"])
                    .pick_file()
                    .await
                    .into_iter()
                    .collect(),
            }
            .into_iter()
            .map(|handle| handle.path().to_path_buf())
            .collect()
        });
        if !paths.is_empty() {
            let _ = messages.send(Message::Picked { pick: kind, paths });
            ctx.request_repaint();
        }
    });
}

/// What a file dialog picks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    /// Designs to convert.
    Files,
    Folder,
    /// The folder designs are written to.
    Output,
    /// The project folder an assembly's part files are searched in.
    Project,
    /// The `solidcraft-cli` that checks histories.
    SolidCraftCli,
    /// A folder to search for the part files the result row at this index is missing.
    SearchFolder(usize),
    /// One of those files, whose folder is then searched.
    SearchFile(usize),
    /// Where to save the full log of the result row at this index.
    SaveLog(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_plan_files_in_queued_folders_off_thread() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("synthetic.f3d");
        std::fs::write(&input, b"not a design").unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        estimates(
            vec![dir.path().to_path_buf()],
            ConvertOptions::default(),
            false,
            7,
            send,
            egui::Context::default(),
        );
        let _ = receive
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        match receive
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
        {
            Message::Estimated {
                generation, files, ..
            } => {
                assert_eq!(generation, 7);
                assert_eq!(files.len(), 1);
                assert_eq!(files.first().unwrap().0, input);
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }

    #[test]
    fn conversion_worker_forwards_counts_before_the_final_report() {
        let dir = tempfile::tempdir().unwrap();
        let input = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../dutchman-core/tests/fixtures/generated_unit_cube.f3d");
        let (send, receive) = std::sync::mpsc::channel();
        let options = ConvertOptions {
            verify: false,
            ..ConvertOptions::default()
        };
        let _run = Run::start(
            vec![input],
            dir.path().to_path_buf(),
            &options,
            send,
            egui::Context::default(),
        );
        let mut events = Vec::new();
        let mut finished = false;
        let mut size_received = false;
        loop {
            match receive
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap()
            {
                Message::Progress(Event::Progress(index, event)) => {
                    assert_eq!(index, 0);
                    assert!(!finished);
                    events.push(event);
                }
                Message::Sized { size, .. } => {
                    assert!(size.is_some_and(|bytes| bytes > 0));
                    size_received = true;
                }
                Message::Progress(Event::Finished(index, report)) => {
                    assert!(size_received);
                    assert_eq!(index, 0);
                    assert_eq!(report["ok"], true);
                    let output = PathBuf::from(report["output"]["path"].as_str().unwrap());
                    assert!(output.is_file());
                    let saved: Value = serde_json::from_slice(
                        &std::fs::read(dutchman::saved_report::path(&output, true)).unwrap(),
                    )
                    .unwrap();
                    assert_eq!(saved, report);
                    finished = true;
                }
                Message::Done => break,
                _ => {}
            }
        }
        assert!(finished);
        let last = events.last().unwrap();
        assert_eq!(last.designs_read, 1);
        assert_eq!(last.designs_total, Some(1));
        assert_eq!(last.stage, dutchman_core::Stage::Write);
    }

    #[test]
    fn iges_in_a_folder_returns_the_shared_refusal_to_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let inputs = dir.path().join("inputs");
        let outputs = dir.path().join("outputs");
        std::fs::create_dir_all(&inputs).unwrap();
        std::fs::write(inputs.join("design.igs"), b"not IGES data").unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let _run = Run::start(
            vec![inputs],
            outputs.clone(),
            &ConvertOptions::default(),
            send,
            egui::Context::default(),
        );
        let mut reports = Vec::new();
        loop {
            match receive
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
            {
                Message::Progress(Event::Finished(_, report)) => reports.push(report),
                Message::Done => break,
                _ => {}
            }
        }
        assert_eq!(reports.len(), 1);
        let report = reports.first().unwrap();
        assert_eq!(report.pointer("/ok"), Some(&serde_json::json!(false)));
        assert_eq!(report.pointer("/error/message").and_then(Value::as_str), Some("Dutchman doesn't convert IGES yet: open it in SolidCraft directly (File > Import)."));
        assert!(dutchman::json::fixtures_pitch(&reports).is_none());
        assert!(!outputs.join("design.solidcraft").exists());
        let saved: Value =
            serde_json::from_slice(&std::fs::read(outputs.join("design.failed.json")).unwrap())
                .unwrap();
        assert_eq!(&saved, report);
    }
}
