//! The app driven as a person would: designs added, an output folder set, Convert pressed, and
//! the results read back from the window.
//!
//! `screenshots` renders the window in both themes; it needs a GPU, so it is ignored by default:
//! `DUTCHMAN_GUI_SCREENSHOTS=<dir> cargo test -p dutchman-gui --features test-support
//! --test ui -- --ignored screenshots`. Linux isolates diagnostics with XDG_STATE_HOME.
//!
//! `--features test-support --test ui consent_e2e` drives consent against loopback 201/503
//! stubs. With `DUTCHMAN_GUI_SCREENSHOTS=<dir>`, it also renders day-theme evidence there
//! and eleven walkthrough frames in the sibling `walkthrough` directory. The ignored
//! `consent_e2e::consent_e2e_live` test requires `DUTCHMAN_E2E_ENDPOINT` and sends only a
//! synthetic fixture summary, marked with signature `e2e-test-desktop-consent-summary`.

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use dutchman_gui::app::Status;
use dutchman_gui::App;

/// An app whose run logs, and the unfinished-run notice, use a folder of this test process,
/// never the real diagnostics folder (CI runs these tests without the test-support feature).
fn test_app(cc: &eframe::CreationContext<'_>) -> App {
    dutchman_gui::worker::convert_in_process_for_tests();
    static FOLDER: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let folder = FOLDER.get_or_init(|| {
        std::env::temp_dir().join(format!(
            "dutchman-gui-ui-diagnostics-{}",
            std::process::id()
        ))
    });
    dutchman_gui::set_diagnostics_folder(folder.clone());
    App::new(cc)
}
use eframe::egui::{self, ThemePreference};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

#[cfg(feature = "test-support")]
#[path = "support/losses.rs"]
mod loss_fixture;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dutchman-core/tests/fixtures")
        .join(name)
}

fn harness() -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(760.0, 680.0))
        .build_eframe(|cc| test_app(cc))
}

/// Presses Convert and runs the window until the run is over.
fn convert(harness: &mut Harness<'static, App>) {
    harness.run_steps(8);
    harness.get_by_label("Convert").click_accesskit();
    harness.step();
    wait_for_conversion(harness);
}

#[test]
fn the_window_offers_the_file_problem_form_and_names_its_version() {
    let mut harness = harness();
    harness.run_steps(8);
    let report = harness.get_by_label("Report a problem").rect();
    let night = harness.get_by_label("Night").rect();
    assert!((report.center().y - night.center().y).abs() < 1.0);
    assert!(report.left() > night.right());
    assert_eq!(
        dutchman_gui::window_title(),
        format!("Dutchman Convert {}", dutchman::BUILD_VERSION)
    );
    let name = harness.get_by_label("Dutchman Convert").rect();
    let version = harness
        .get_by_label(&format!("v{}", dutchman::BUILD_VERSION))
        .rect();
    assert!(version.left() >= name.right());
    assert!(version.height() < name.height());
    assert_eq!(
        dutchman_gui::REPORT_PROBLEM_URL,
        "https://github.com/dutchman-cad/dutchman/issues/new?template=file-did-not-convert.yml"
    );
}

fn header_harness(width: f32, theme: ThemePreference, update: bool) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(width, 680.0))
        .build_eframe(|cc| test_app(cc));
    h.ctx.set_theme(theme);
    h.state_mut().preview_solidcraft(Some("0.41.9"));
    if update {
        h.state_mut()
            .updater
            .preview(dutchman_gui::update::Release {
                fixes_reports: Vec::new(),
                tag: "v0.2.6".into(),
                notes: "Release notes".into(),
                download: "https://github.com/dutchman-cad/dutchman/releases/tag/v0.2.6".into(),
            });
    }
    h.run_steps(8);
    h
}

fn assert_header_fits(h: &Harness<'_, App>, width: f32, update: bool, theme: &str) {
    let name = h.get_by_label("Dutchman Convert").rect();
    let version = h
        .get_by_label(&format!("v{}", dutchman::BUILD_VERSION))
        .rect();
    assert!(version.left() >= name.right());
    let mut title = name.union(version);
    if let Some(subtitle) = h.query_by_label("Converts native CAD designs into SolidCraft designs.")
    {
        title = title.union(subtitle.rect());
    }
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 680.0));
    assert!(window.contains_rect(title));
    let narrow = width - 64.0 < 850.0;
    let mut labels = vec!["Settings", "Report a problem"];
    let found = h
        .query_by_label(if narrow { "0.41" } else { "SolidCraft 0.41" })
        .is_some();
    labels.push(match (narrow, found) {
        (true, true) => "0.41",
        (false, true) => "SolidCraft 0.41",
        (true, false) => "not found",
        (false, false) => "SolidCraft not found",
    });
    if narrow {
        labels.push(theme);
    } else {
        labels.extend(["Night", "Day", "System"]);
    }
    if update {
        labels.push(if narrow { "Update" } else { "Update to 0.2.6" });
    }
    let mut rects = Vec::new();
    for label in labels {
        let rect = h.get_by_label(label).rect();
        assert!(
            !rect.intersects(title),
            "{width}: {label} {rect:?} overlaps title {title:?}"
        );
        assert!(
            window.contains_rect(rect),
            "{width}: {label} {rect:?} outside window"
        );
        for other in &rects {
            assert!(
                !rect.intersects(*other),
                "{width}: {label} overlaps another control"
            );
        }
        rects.push(rect);
    }
}

#[test]
fn header_controls_fit_without_overlapping_the_title() {
    for width in [
        520.0, 600.0, 760.0, 913.0, 914.0, 1000.0, 1100.0, 1180.0, 1263.0, 1264.0, 1400.0,
    ] {
        for update in [false, true] {
            for (theme, label) in [
                (ThemePreference::Light, "Day"),
                (ThemePreference::Dark, "Night"),
                (ThemePreference::System, "System"),
            ] {
                let mut h = header_harness(width, theme, update);
                assert_header_fits(&h, width, update, label);
                h.state_mut().preview_solidcraft(None);
                h.run_steps(8);
                assert_header_fits(&h, width, update, label);
            }
        }
    }
}

#[test]
#[ignore = "renders real GPU screenshots; set DUTCHMAN_GUI_SCREENSHOTS"]
fn header_screenshots() {
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    for (theme, label) in [
        (ThemePreference::Light, "Day"),
        (ThemePreference::Dark, "Night"),
    ] {
        for width in [520.0, 760.0] {
            let mut h = header_harness(width, theme, true);
            assert_header_fits(&h, width, true, label);
            h.render()
                .unwrap()
                .save(dir.join(format!("header-{width}-{label}.png")))
                .unwrap();
        }
    }
}

#[test]
fn designs_convert_from_the_window() {
    let out = tempfile::tempdir().unwrap();
    // Keep Convert and all three results visible without scrolling.
    let mut harness = Harness::builder()
        .with_size(egui::vec2(760.0, 1280.0))
        .build_eframe(|cc| test_app(cc));
    harness.state_mut().add_inputs([
        fixture("generated_unit_cube.f3d"),
        fixture("synthetic_cube.ipt"),
        fixture("solidworks_cube.sldprt"),
    ]);
    harness.state_mut().set_out_dir(out.path().to_path_buf());
    harness.state_mut().set_verify(false);
    convert(&mut harness);

    let rows = harness.state().rows();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.status == Status::Converted));
    assert!(out.path().join("generated_unit_cube.sc").is_file());
    assert!(out.path().join("synthetic_cube.sc").is_file());
    assert!(out.path().join("solidworks_cube.sc").is_file());
    assert!(harness.query_by_label("3 converted, 0 failed").is_some());
    assert_eq!(harness.query_all_by_label("Open in SolidCraft").count(), 3);
}

#[test]
fn the_format_list_shows_headlines_beside_reader_status() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1200.0, 1800.0))
        .build_eframe(|cc| test_app(cc));
    harness.run_steps(8);
    harness.get_by_label("Formats").click_accesskit();
    harness.run_steps(8);
    assert!(harness.query_by_label("Fusion archive (.f3d)").is_some());
    assert!(harness.query_all_by_label("L5").count() >= 1);
    assert!(harness.query_by_label("IGES (.igs, .iges)").is_some());
    assert!(harness.query_by_label("L9").is_some());
    assert!(harness
        .query_by_label("open it in SolidCraft directly")
        .is_some());
    assert!(harness.query_by_label("Creo or NX part (.prt)").is_some());
}

#[test]
fn a_file_that_cannot_convert_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let broken = dir.path().join("broken.f3d");
    std::fs::write(&broken, b"not a design").unwrap();
    let mut harness = harness();
    harness.state_mut().add_inputs([broken]);
    harness.state_mut().set_out_dir(dir.path().join("out"));
    convert(&mut harness);

    let rows = harness.state().rows();
    assert_eq!(rows[0].status, Status::Failed);
    assert!(harness.query_by_label("Failed").is_some());
    assert!(harness.query_by_label("0 converted, 1 failed").is_some());
    assert!(!dir.path().join("out/broken.sc").exists());

    // The failed row says to report it, and its button opens the form filled in, without
    // the file's name. The other ways to send it are folded until asked for.
    harness.get_by_label("Review and send report");
    assert!(harness
        .query_by_label("Report anonymously on GitHub")
        .is_none());
    open_help(&mut harness);
    harness
        .get_by_label("Report anonymously on GitHub")
        .click_accesskit();
    harness.step();
    let opened = harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::OpenUrl(open) => Some(open.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        opened.url.starts_with(dutchman_gui::REPORT_PROBLEM_URL),
        "{}",
        opened.url
    );
    assert!(opened.url.contains("&stage=stage%3A%20"), "{}", opened.url);
    assert!(opened.url.contains("&report-anon="), "{}", opened.url);
    assert!(!opened.url.contains("broken"), "{}", opened.url);
}

/// A failed run of a file named `name` holding `bytes` bytes, in a window `width` wide. What
/// the file manager would show goes to `SHOWN`.
fn failed_card(name: &str, bytes: usize, width: f32) -> (Harness<'static, App>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let broken = dir.path().join(name);
    std::fs::write(&broken, vec![b'x'; bytes]).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(width, 1100.0))
        .build_eframe(|cc| test_app(cc));
    harness.state_mut().set_file_manager(|path| {
        SHOWN.lock().unwrap().push(path.to_path_buf());
        Ok(())
    });
    harness.state_mut().add_inputs([broken]);
    harness.state_mut().set_out_dir(dir.path().join("out"));
    harness
        .state_mut()
        .set_diagnostic_log(dir.path().join("log.jsonl"));
    convert(&mut harness);
    assert_eq!(harness.state().rows()[0].status, Status::Failed);
    open_help(&mut harness);
    (harness, dir)
}

/// Opens a failed card's folded "Help us fix it" section.
fn open_help(harness: &mut Harness<'static, App>) {
    harness
        .get_by_label(dutchman::present::SEND_FOLD)
        .click_accesskit();
    harness.run_steps(8);
}

static SHOWN: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Presses the DM button: what it copied and opened.
fn press_dm(harness: &mut Harness<'static, App>) -> (String, String) {
    harness.get_by_label("Copy and open DM").click_accesskit();
    harness.step();
    let commands = &harness.output().platform_output.commands;
    let copied = commands.iter().find_map(|command| match command {
        egui::OutputCommand::CopyText(text) => Some(text.clone()),
        _ => None,
    });
    let opened = commands.iter().find_map(|command| match command {
        egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
        _ => None,
    });
    let copied = copied.unwrap();
    assert!(copied.contains("Run log: log.jsonl."));
    assert!(copied.contains("no names or contents from the design"));
    (copied, opened.unwrap())
}

#[test]
fn a_small_failed_file_is_dragged_into_the_dm_with_its_folder_open() {
    let (mut harness, dir) = failed_card("small-broken.f3d", 2048, 760.0);
    assert!(harness
        .query_by_label(
            "If this file opens in the app that made it, it's a bug in Dutchman. Help us fix it:"
        )
        .is_some());
    assert!(harness
        .query_by_label("Already in? Skip this step.")
        .is_some());
    assert!(harness
        .query_by_label("Drag the file in (its folder is open).")
        .is_some());
    assert!(harness.query_by_label("Have a STEP export?").is_some());
    let (message, opened) = press_dm(&mut harness);
    assert!(opened.starts_with("https://discord.com/users/"), "{opened}");
    assert!(message.contains("Size: 0.1 MB."), "{message}");
    assert!(message.contains("STEP: unknown."), "{message}");
    assert!(!message.contains("broken"), "{message}");
    assert!(!message.contains('\n'), "{message}");
    assert!(SHOWN
        .lock()
        .unwrap()
        .contains(&dir.path().join("small-broken.f3d")));

    // The card's id is the log's: the same run, the same salt.
    let log = std::fs::read_to_string(dir.path().join("log.jsonl")).unwrap();
    let first: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    let id = first["file"].as_str().unwrap();
    assert!(message.contains(&format!("Report id: {id}.")), "{message}");
}

#[test]
fn a_large_failed_file_takes_the_private_link_and_no_folder_opens() {
    let (mut harness, dir) = failed_card("large-broken.f3d", 11 << 20, 760.0);
    assert!(harness
        .query_by_label_contains("Dutchy replies with a private upload link (up to 300 MB")
        .is_some());
    assert!(harness
        .query_by_label_contains("Drag the file in")
        .is_none());
    let (message, _) = press_dm(&mut harness);
    assert!(message.contains("Size: 11 MB."), "{message}");
    assert!(!SHOWN
        .lock()
        .unwrap()
        .contains(&dir.path().join("large-broken.f3d")));
}

#[test]
fn the_step_cards_stand_side_by_side_and_wrap_when_narrow() {
    let (harness, _dir) = failed_card("wide.f3d", 10, 1000.0);
    let join = harness.get_by_label("Join our Discord server").rect();
    let dm = harness.get_by_label("DM Dutchy").rect();
    assert!((join.top() - dm.top()).abs() < 1.0 && join.right() < dm.left());
    let (harness, _dir) = failed_card("narrow.f3d", 10, 520.0);
    let join = harness.get_by_label("Join our Discord server").rect();
    let step = harness.get_by_label("Have a STEP export?").rect();
    assert!(step.top() > join.bottom());
}

#[test]
fn a_step_file_is_not_asked_for_a_step_export() {
    let (mut harness, _dir) = failed_card("broken.step", 2048, 760.0);
    assert!(harness.query_by_label("Have a STEP export?").is_none());
    let (message, _) = press_dm(&mut harness);
    assert!(message.contains("STEP: yes."), "{message}");
}

#[test]
fn a_file_converts_again_searching_a_folder_chosen_for_it() {
    let out = tempfile::tempdir().unwrap();
    let mut harness = harness();
    harness
        .state_mut()
        .add_inputs([fixture("synthetic_cube.ipt")]);
    harness.state_mut().set_out_dir(out.path().to_path_buf());
    harness.state_mut().set_verify(false);
    convert(&mut harness);

    let parts = out.path().join("Parts");
    let ctx = harness.ctx.clone();
    harness.state_mut().search_for_row(0, parts.clone(), &ctx);
    assert!(harness.state().running());
    let started = Instant::now();
    while harness.state().running() {
        assert!(started.elapsed() < Duration::from_secs(120));
        std::thread::sleep(Duration::from_millis(20));
        harness.step();
    }
    harness.run_steps(8);
    let row = &harness.state().rows()[0];
    assert_eq!(row.status, Status::Converted);
    assert_eq!(row.job.as_ref().unwrap().search.also, [parts]);
    assert!(harness.query_by_label("1 converted, 0 failed").is_some());
}

#[test]
fn adding_a_design_selects_an_output_folder() {
    let mut harness = harness();
    harness.run_steps(8);
    assert!(harness.query_by_label("Add designs to convert.").is_some());
    harness
        .state_mut()
        .add_inputs([fixture("synthetic_cube.ipt")]);
    harness.run_steps(8);
    assert!(harness.query_by_label("Choose an output folder.").is_none());
    assert!(harness.query_by_label("Convert").is_some());
}

#[test]
#[ignore = "renders with a GPU; set DUTCHMAN_GUI_SCREENSHOTS to a folder"]
fn screenshots() {
    let Some(dir) = std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").map(PathBuf::from) else {
        panic!("set DUTCHMAN_GUI_SCREENSHOTS to a folder");
    };
    std::fs::create_dir_all(&dir).unwrap();
    // Environment changes happen only in a child process, never alongside other tests.
    // In particular, never inspect or write the owner's diagnostic logs.
    if std::env::var_os("DUTCHMAN_GUI_SCREENSHOT_CHILD").is_none() {
        let state = tempfile::tempdir().unwrap();
        if !cfg!(target_os = "linux") {
            panic!("diagnostics isolation uses XDG_STATE_HOME on Linux");
        }
        if !cfg!(feature = "test-support") {
            panic!("enable --features test-support for the screenshot run");
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "screenshots", "--nocapture"])
            .env("DUTCHMAN_GUI_SCREENSHOT_CHILD", "1")
            .env("XDG_STATE_HOME", state.path())
            .status()
            .unwrap();
        assert!(status.success(), "screenshot child failed: {status}");
        return;
    }
    let work = tempfile::tempdir().unwrap();
    let designs = work.path().join("Designs");
    std::fs::create_dir_all(&designs).unwrap();
    let input = designs.join("generated_unit_cube.f3d");
    std::fs::copy(fixture("generated_unit_cube.f3d"), &input).unwrap();
    let failed_input = designs.join("container_metadata_only.f3d");
    std::fs::copy(fixture("container_metadata_only.f3d"), &failed_input).unwrap();
    let mut index = format!("# Dutchman desktop UI review\n\nBuilt from Dutchman {}. Usually 1100 × 800 logical pixels; long-loss frames use 520 or 1000 × 900. One PNG per state and theme. Day = light; Night = dark.\n\n", env!("CARGO_PKG_VERSION"));
    for (theme, name) in [
        (ThemePreference::Light, "light"),
        (ThemePreference::Dark, "dark"),
    ] {
        for (case, size, description) in [
            (
                "small",
                4 << 20,
                "A failed file small enough to drag into the DM.",
            ),
            (
                "large",
                40 << 20,
                "A failed file over Discord's limit: the private link.",
            ),
        ] {
            let mut harness = screenshot_harness(theme);
            harness.state_mut().add_inputs([failed_input.clone()]);
            harness
                .state_mut()
                .set_out_dir(work.path().join(format!("card-{case}-{name}")));
            convert(&mut harness);
            harness.state_mut().preview_size(0, size);
            harness.run_steps(8);
            harness
                .get_by_label("Join our Discord server")
                .scroll_to_me();
            harness.run_steps(8);
            shot(
                &mut harness,
                &dir,
                &mut index,
                &format!("05-send-privately-{case}"),
                name,
                description,
            );
        }
    }
    for (theme, name) in [
        (ThemePreference::Light, "light"),
        (ThemePreference::Dark, "dark"),
    ] {
        #[cfg(feature = "test-support")]
        for width in [520.0, 1000.0] {
            let mut harness = screenshot_harness(theme);
            harness.set_size(egui::vec2(width, 900.0));
            harness.state_mut().set_checker_for_test(true);
            harness
                .state_mut()
                .preview_result_for_test(dutchman_gui::app::Row {
                    input: PathBuf::from("synthetic.f3d"),
                    output: Some(PathBuf::from("converted/synthetic.sc")),
                    status: Status::Converted,
                    progress: None,
                    job: None,
                    size: None,
                    report: Some(loss_fixture::report()),
                });
            harness.run_steps(8);
            harness.get_by_label_contains("Losses:").scroll_to_me();
            harness.run_steps(8);
            harness.get_by_label_contains("Losses:").click_accesskit();
            harness.run_steps(8);
            harness.get_by_label("Save the full log…").scroll_to_me();
            harness.run_steps(8);
            assert!(harness.query_by_label_contains("/home/").is_none());
            shot(
                &mut harness,
                &dir,
                &mut index,
                &format!("15-long-losses-{width}"),
                name,
                "Expanded synthetic losses with long native ids and an oversized unbroken token.",
            );
        }
        let mut harness = screenshot_harness(theme);
        harness.state_mut().updater.preview(dutchman_gui::update::Release {
            fixes_reports: Vec::new(),
            tag: "v0.2.6".into(),
            notes: "Dutchman 0.2.6\n\nImproved desktop conversion progress and clearer reports.\nUpdated native format readers.".into(),
            download: "https://github.com/dutchman-cad/dutchman/releases/tag/v0.2.6".into(),
        });
        harness.run_steps(8);
        shot(
            &mut harness,
            &dir,
            &mut index,
            "00-update-pill",
            name,
            "A newer release is available: the Update pill in the header.",
        );
        for (found, case, update, width) in [
            (Some("0.41.9"), "current", false, 1120.0),
            (Some("0.42.0"), "newer", false, 1120.0),
            (Some("0.42.0"), "newer-update", true, 1120.0),
            (Some("0.17.9"), "too-old", false, 1120.0),
            (Some("0.24.0"), "older-history", false, 1120.0),
            (None, "none-found", false, 1120.0),
            (Some("0.42.0"), "narrow", true, 760.0),
        ] {
            let mut preview = Harness::builder()
                .with_size(egui::vec2(width, 680.0))
                .build_eframe(|cc| test_app(cc));
            preview.ctx.set_theme(theme);
            preview.state_mut().preview_solidcraft(found);
            if update {
                preview
                    .state_mut()
                    .updater
                    .preview(dutchman_gui::update::Release {
                        fixes_reports: Vec::new(),
                        tag: "v0.2.6".into(),
                        notes: "Release notes".into(),
                        download: "https://github.com/dutchman-cad/dutchman/releases/tag/v0.2.6"
                            .into(),
                    });
            }
            preview.run_steps(8);
            let label = if width < 850.0 {
                found
                    .and_then(dutchman_solidcraft::solidcraft_minor)
                    .unwrap_or_else(|| "not found".into())
            } else {
                found.map_or_else(
                    || "SolidCraft not found".into(),
                    |v| {
                        format!(
                            "SolidCraft {}",
                            dutchman_solidcraft::solidcraft_minor(v).unwrap()
                        )
                    },
                )
            };
            preview.get_by_label(&label).click_accesskit();
            preview.run_steps(8);
            assert!(preview
                .query_by_label_contains(match case {
                    "newer-update" | "narrow" => "Update Dutchman",
                    "newer" => "is newer than this Dutchman knows",
                    "too-old" => "is too old to open Dutchman's designs",
                    "older-history" => "A newer SolidCraft keeps more of their history.",
                    "none-found" => "SolidCraft not found. Install SolidCraft",
                    _ => "SolidCraft 0.41 is installed.",
                })
                .is_some());
            let file = format!("00-solidcraft-{case}-{name}.png");
            preview.render().unwrap().save(dir.join(&file)).unwrap();
            index.push_str(&format!(
                "- [{file}]({file}) — SolidCraft version pill and panel: {case} ({width} px wide).\n"
            ));
        }
        harness.get_by_label("Update to 0.2.6").click_accesskit();
        harness.run_steps(8);
        shot(
            &mut harness,
            &dir,
            &mut index,
            "00-update-window",
            name,
            "The update window under the pill.",
        );
        for (installed, state, description) in [
            (
                false,
                "running",
                "Updating with Homebrew: progress and the output tail.",
            ),
            (true, "installed", "Homebrew finished: Restart Dutchman."),
        ] {
            harness.state_mut().updater.preview_homebrew(installed);
            harness.run_steps(4);
            shot(
                &mut harness,
                &dir,
                &mut index,
                &format!("00-update-homebrew-{state}"),
                name,
                description,
            );
        }
        harness.state_mut().updater.preview_scoop();
        harness.run_steps(8);
        shot(
            &mut harness,
            &dir,
            &mut index,
            "00-update-scoop-confirm",
            name,
            "Scoop: confirm closing Dutchman to update it.",
        );
        harness.state_mut().updater.open = false;
        harness.run_steps(8);
        shot(
            &mut harness,
            &dir,
            &mut index,
            "01-start",
            name,
            "Empty start window.",
        );
        harness.state_mut().add_inputs([input.clone()]);
        harness
            .state_mut()
            .set_out_dir(work.path().join(format!("Converted-{name}")));
        harness.state_mut().set_verify(false);
        wait_for_estimate(&mut harness);
        shot(
            &mut harness,
            &dir,
            &mut index,
            "02-file-estimate",
            name,
            "Repository cube chosen, with its live estimate and output folder.",
        );
        for (number, previous, preset) in [
            ("03", "Standard", "Fast"),
            ("04", "Fast", "Standard"),
            ("05", "Standard", "Full"),
        ] {
            harness.get_by_value(previous).click_accesskit();
            harness.run_steps(8);
            harness.get_by_label(preset).click_accesskit();
            wait_for_estimate(&mut harness);
            shot(
                &mut harness,
                &dir,
                &mut index,
                &format!("{number}-preset-{}", preset.to_lowercase()),
                name,
                &format!("{preset} preset selected, with refreshed estimate."),
            );
        }
        harness.get_by_label("Advanced").click_accesskit();
        harness.run_steps(8);
        harness.get_by_label("Sketches").click_accesskit();
        wait_for_estimate(&mut harness);
        assert!(harness.query_by_value("Custom (based on Full)").is_some());
        shot(
            &mut harness,
            &dir,
            &mut index,
            "06-advanced-custom",
            name,
            "Advanced open; Sketches unchecked; Custom based on Full.",
        );
        // Collapse before conversion so the ordinary 800-pixel window shows more results.
        harness.get_by_label("Advanced").click_accesskit();
        harness.run_steps(8);
        harness.get_by_label("Convert").click_accesskit();
        // Do not settle frames here: the tiny cube may finish before the next frame.
        harness.step();
        // The click takes effect after the action bar is drawn. Paint its updated state.
        harness.step();
        let running = if harness.query_by_label("Stop").is_some() {
            "First frame after Convert for a real cube conversion; Stop is visible. The tiny fixture completes too quickly for a reliable mid-run progress frame."
        } else {
            "First frame after Convert for a real cube conversion; the tiny fixture completed before a running frame could be rendered."
        };
        shot(&mut harness, &dir, &mut index, "08-running", name, running);
        wait_for_conversion(&mut harness);
        assert_eq!(harness.state().rows().len(), 1);
        assert_eq!(harness.state().rows()[0].status, Status::Converted);
        harness.get_by_label("RESULTS").scroll_to_me();
        harness.run_steps(8);
        shot(
            &mut harness,
            &dir,
            &mut index,
            "09-done",
            name,
            "One successful conversion, report card and summary; scrolled to results.",
        );

        let mut harness = screenshot_harness(theme);
        harness.state_mut().add_inputs([failed_input.clone()]);
        harness
            .state_mut()
            .set_out_dir(work.path().join(format!("Failed-{name}")));
        harness.state_mut().set_verify(false);
        convert(&mut harness);
        assert_eq!(harness.state().rows()[0].status, Status::Failed);
        harness.get_by_label("RESULTS").scroll_to_me();
        harness.run_steps(8);
        shot(
            &mut harness,
            &dir,
            &mut index,
            "10-failed",
            name,
            "Repository metadata-only Fusion fixture fails conversion; report card explains why.",
        );

        let folder = dutchman::diagnostics::default_folder().unwrap();
        std::fs::create_dir_all(&folder).unwrap();
        let unfinished = folder.join("2026-10-29_201500.jsonl");
        std::fs::write(&unfinished, "{\"type\":\"start\",\"file\":\"f1\"}\n").unwrap();
        let mut harness = screenshot_harness(theme);
        harness.state_mut().add_inputs([input.clone()]);
        harness
            .state_mut()
            .set_out_dir(work.path().join(format!("Converted-{name}")));
        harness.run_steps(8);
        harness.get_by_label("The last run didn't finish. Its diagnostic log is here:");
        shot(
            &mut harness,
            &dir,
            &mut index,
            "11-unfinished-log",
            name,
            "Unfinished-run banner detected from an isolated temporary default diagnostic log.",
        );
        harness
            .get_by_label("Open diagnostics folder")
            .scroll_to_me();
        harness.run_steps(8);
        shot(
            &mut harness,
            &dir,
            &mut index,
            "12-diagnostics",
            name,
            "Open diagnostics folder button; scrolled to show the controls.",
        );
        harness
            .get_by_label("The last run didn't finish. Its diagnostic log is here:")
            .scroll_to_me();
        harness.run_steps(8);
        harness.get_by_label("Dismiss").click_accesskit();
        harness.run_steps(8);
        std::fs::remove_file(unfinished).unwrap();
        harness.get_by_label("OUTPUT FOLDER").scroll_to_me();
        harness.run_steps(8);
        shot(&mut harness, &dir, &mut index, "13-settings", name, "Output folder, SolidCraft output and target, history checking, assembly project folder, and diagnostics settings; scrolled to the output section.");
        harness.get_by_label("Formats").click_accesskit();
        harness.run_steps(8);
        harness.get_by_label("Fusion archive (.f3d)");
        shot(
            &mut harness,
            &dir,
            &mut index,
            "14-formats",
            name,
            "Formats menu open with reader status and capability levels.",
        );
    }
    index.push_str("\nAll conversions use repository fixtures copied into a temporary folder. Native file/folder pickers and external folder/app launches are not invoked. Diagnostics are read from a temporary XDG_STATE_HOME in a child test process.\n");
    std::fs::write(dir.join("INDEX.md"), index).unwrap();
}

fn screenshot_harness(theme: ThemePreference) -> Harness<'static, App> {
    let harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_eframe(|cc| test_app(cc));
    harness.ctx.set_theme(theme);
    harness
}

fn shot(
    harness: &mut Harness<'static, App>,
    dir: &Path,
    index: &mut String,
    state: &str,
    theme: &str,
    description: &str,
) {
    let file = format!("{state}-{theme}.png");
    let image = harness.render().unwrap();
    let (width, height) = image.dimensions();
    use image::ImageEncoder;
    let output = std::fs::File::create(dir.join(&file)).unwrap();
    image::codecs::png::PngEncoder::new_with_quality(
        output,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        image.as_raw(),
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    index.push_str(&format!("- [{file}]({file}) — {description}\n"));
}

fn wait_for_estimate(harness: &mut Harness<'static, App>) {
    harness.run_steps(8);
    let started = Instant::now();
    while harness.query_all_by_label_contains("GB, ").count() == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "estimate did not arrive"
        );
        std::thread::sleep(Duration::from_millis(20));
        harness.step();
    }
    harness.run_steps(8);
}

fn wait_for_conversion(harness: &mut Harness<'static, App>) {
    let started = Instant::now();
    while harness.state().running() || harness.state().rows().is_empty() {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "conversion did not finish"
        );
        std::thread::sleep(Duration::from_millis(20));
        harness.step();
    }
    harness.run_steps(8);
}

#[test]
fn dropped_files_and_results_show_format_status_and_one_invitation() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("creo.prt");
    let second = dir.path().join("nx.prt");
    std::fs::write(&first, b"#UGC:2").unwrap();
    std::fs::write(&second, b"SPLMSSTR").unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_eframe(|cc| test_app(cc));
    harness.state_mut().add_inputs([first, second]);
    harness.state_mut().set_out_dir(dir.path().join("out"));
    harness.run_steps(8);
    for program in ["Creo", "NX"] {
        assert!(harness
            .query_by_label(&format!("{program} .prt: opens, no geometry yet  L1"))
            .is_some());
    }
    convert(&mut harness);
    assert!(harness
        .state()
        .rows()
        .iter()
        .all(|r| r.status == Status::Failed));
    assert_eq!(
        harness
            .query_all_by_label_contains("Want to help Dutchman read Creo, NX better?")
            .count(),
        1
    );
    assert!(harness
        .query_by_label(dutchman::json::FIXTURES_URL)
        .is_some());
    assert!(harness
        .query_by_label(dutchman::json::DISCORD_URL)
        .is_some());
}

/// Opening a missing-CLI preview retries discovery. Isolate its environment without
/// changing the process-wide environment of the other, concurrently running tests.
fn isolate_missing_cli_preview(test: &str) -> bool {
    if std::env::var_os("SOLIDCRAFT_CLI").is_none() {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env_remove("SOLIDCRAFT_CLI")
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[test]
fn solidcraft_status_panel_waits_for_a_click_and_shares_version_notices() {
    if isolate_missing_cli_preview(
        "solidcraft_status_panel_waits_for_a_click_and_shares_version_notices",
    ) {
        return;
    }
    for (found, expected) in [
        (Some("0.42.0"), "is newer than this Dutchman knows"),
        (Some("0.17.9"), "is too old to open Dutchman's designs"),
        (
            Some("0.24.0"),
            "A newer SolidCraft keeps more of their history.",
        ),
        (Some("0.41.0"), "SolidCraft 0.41 is installed."),
        (None, "SolidCraft not found. Install SolidCraft"),
    ] {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1120.0, 680.0))
            .build_eframe(|cc| test_app(cc));
        harness.state_mut().preview_solidcraft(found);
        harness.run_steps(8);
        assert!(harness.query_by_label("SolidCraft version").is_none());
        let label = found.map_or_else(
            || "SolidCraft not found".into(),
            |v| {
                format!(
                    "SolidCraft {}",
                    dutchman_solidcraft::solidcraft_minor(v).unwrap()
                )
            },
        );
        harness.get_by_label(&label).click_accesskit();
        harness.run_steps(8);
        assert!(harness.query_by_label_contains(expected).is_some());
        if found.is_none() || found == Some("0.17.9") {
            assert!(harness.query_by_label("Download SolidCraft").is_some());
        }
    }
}

#[cfg(unix)]
#[test]
fn solidcraft_panel_takes_a_chosen_cli_and_explains_the_app_alone() {
    if isolate_missing_cli_preview("solidcraft_panel_takes_a_chosen_cli_and_explains_the_app_alone")
    {
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let script = |path: &std::path::Path, line: &str| {
        std::fs::write(path, format!("#!/bin/sh\necho '{line}'\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("solidcraft");
    script(&app, "solidcraft 0.30.0");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1120.0, 680.0))
        .build_eframe(|cc| test_app(cc));
    harness.state_mut().preview_solidcraft(None);
    harness.state_mut().preview_solidcraft_app(true);
    harness.run_steps(8);
    harness
        .get_by_label("SolidCraft not found")
        .click_accesskit();
    harness.run_steps(8);
    assert!(harness
        .query_by_label_contains("Found the SolidCraft app, but not solidcraft-cli")
        .is_some());
    assert!(harness.query_by_label("Choose solidcraft-cli…").is_some());
    assert!(harness.query_by_label("Find automatically").is_none());

    harness.state_mut().choose_solidcraft_cli(&app);
    harness.run_steps(8);
    assert!(harness
        .query_by_label_contains("no solidcraft-cli beside it")
        .is_some());
    assert!(harness
        .query_by_label_contains("Found the SolidCraft app")
        .is_none());

    script(&dir.path().join("solidcraft-cli"), "solidcraft-cli 0.41.0");
    harness.state_mut().choose_solidcraft_cli(&app);
    harness.run_steps(8);
    assert!(harness.query_by_label("SolidCraft 0.41").is_some());
    assert!(harness.query_by_label("Find automatically").is_some());
    assert!(harness
        .query_by_label_contains("the portable ZIP and the MSI")
        .is_none());
}

#[test]
fn update_pill_adapts_to_width_and_waits_for_a_click() {
    for (width, label) in [(760.0, "Update"), (1000.0, "Update to 0.2.6")] {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(width, 680.0))
            .build_eframe(|cc| test_app(cc));
        harness
            .state_mut()
            .updater
            .preview(dutchman_gui::update::Release {
                fixes_reports: Vec::new(),
                tag: "v0.2.6".into(),
                notes: "Release notes".into(),
                download: "https://github.com/dutchman-cad/dutchman/releases/tag/v0.2.6".into(),
            });
        harness.run_steps(8);
        assert!(harness
            .query_by_label("Dutchman 0.2.6 is available")
            .is_none());
        harness.get_by_label(label).click_accesskit();
        harness.run_steps(8);
        assert!(harness
            .query_by_label("Dutchman 0.2.6 is available")
            .is_some());
        harness.get_by_label("Skip this version").click_accesskit();
        harness.run_steps(8);
        assert!(harness.query_by_label(label).is_none());
        assert!(harness
            .query_by_label("Dutchman 0.2.6 is available")
            .is_none());
    }
}

#[test]
fn update_window_stays_clear_of_the_header_controls() {
    for width in [520.0, 900.0, 1440.0] {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(width, 900.0))
            .build_eframe(|cc| App::new(cc));
        harness
            .state_mut()
            .updater
            .preview(dutchman_gui::update::Release {
                fixes_reports: Vec::new(),
                tag: "v0.2.6".into(),
                notes: "Release notes".into(),
                download: "https://github.com/dutchman-cad/dutchman/releases/tag/v0.2.6".into(),
            });
        harness.run_steps(8);
        let pill = if width < 920.0 {
            "Update"
        } else {
            "Update to 0.2.6"
        };
        harness.get_by_label(pill).click_accesskit();
        harness.run_steps(8);
        assert!(harness
            .query_by_label("Dutchman 0.2.6 is available")
            .is_some());
        let window = harness
            .ctx
            .memory(|m| m.area_rect(egui::Id::new("Dutchman update")))
            .expect("the update window is open");
        let mut checked = 0;
        for label in [
            "Settings",
            "Report a problem",
            "System",
            "Day",
            "Night",
            "SolidCraft",
            "not found",
            pill,
        ] {
            for node in harness.query_all_by_label_contains(label) {
                let rect = node.rect();
                if rect.height() > 80.0 {
                    continue;
                }
                checked += 1;
                assert!(
                    !window.intersects(rect),
                    "{width}: window {window:?} covers {label:?} at {rect:?}"
                );
            }
        }
        assert!(
            checked >= 4,
            "{width}: only {checked} header controls found"
        );
    }
}

#[test]
fn update_notes_open_at_the_top_each_time() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 760.0))
        .build_eframe(|cc| App::new(cc));
    let long: String = (1..=60)
        .map(|n| format!("Line number {n} of the notes.\n\n"))
        .collect();
    for tag in ["v0.2.6", "v0.2.7", "v0.2.7"] {
        harness
            .state_mut()
            .updater
            .preview(dutchman_gui::update::Release {
                fixes_reports: Vec::new(),
                tag: tag.into(),
                notes: long.clone(),
                download: format!("https://github.com/dutchman-cad/dutchman/releases/tag/{tag}"),
            });
        harness.state_mut().updater.open = true;
        harness.run_steps(8);
        harness
            .get_by_label_contains("Line number 40 of")
            .scroll_to_me();
        harness.run_steps(8);
        // Closing and opening again, or opening another version, starts at the top.
        harness.state_mut().updater.open = false;
        harness.run_steps(4);
    }
    harness.state_mut().updater.open = true;
    harness.run_steps(8);
    let top = harness.get_by_label_contains("Line number 1 of").rect();
    let window = harness
        .ctx
        .memory(|m| m.area_rect(egui::Id::new("Dutchman update")))
        .unwrap();
    assert!(
        window.contains_rect(top),
        "first line {top:?} not in {window:?}"
    );
}

#[test]
fn homebrew_job_survives_window_close_and_offers_restart_on_success() {
    let mut harness = harness();
    harness
        .state_mut()
        .updater
        .preview(dutchman_gui::update::Release {
            fixes_reports: Vec::new(),
            tag: "v0.2.6".into(),
            notes: "Release notes".into(),
            download: "https://github.com/dutchman-cad/dutchman/releases/tag/v0.2.6".into(),
        });
    harness.state_mut().updater.preview_homebrew(false);
    harness.run_steps(3);
    assert!(harness.query_by_label("Updating Dutchman…").is_some());
    harness.state_mut().updater.open = false;
    harness.run_steps(3);
    harness.get_by_label("Update").click_accesskit();
    harness.run_steps(3);
    assert!(harness.query_by_label("Updating Dutchman…").is_some());
    assert!(harness.query_by_label("Restart Dutchman").is_none());
    // While the update runs, offering the same command again would only invite running it twice.
    assert!(harness.query_by_label("Copy command").is_none());
    harness.state_mut().updater.preview_homebrew(true);
    harness.run_steps(3);
    assert!(harness
        .query_by_label("Dutchman 0.2.6 is installed")
        .is_some());
    assert!(harness.query_by_label("Restart Dutchman").is_some());
    assert!(harness.query_by_label("Update output").is_some());
    // After success only the result and Restart apply.
    for gone in ["Copy command", "Skip this version", "Remind me later"] {
        assert!(
            harness.query_by_label(gone).is_none(),
            "{gone} after success"
        );
    }
}

#[test]
fn scoop_confirmation_can_be_cancelled_without_starting_a_process() {
    let mut harness = harness();
    harness
        .state_mut()
        .updater
        .preview(dutchman_gui::update::Release {
            fixes_reports: Vec::new(),
            tag: "v0.2.6".into(),
            notes: "Release notes".into(),
            download: "https://github.com/dutchman-cad/dutchman/releases/tag/v0.2.6".into(),
        });
    harness.state_mut().updater.preview_scoop();
    harness.run_steps(8);
    assert!(harness
        .query_by_label("Dutchman closes, Scoop updates it, and Dutchman opens again.")
        .is_some());
    assert!(harness.query_by_label("Update and restart").is_some());
    harness.get_by_label("Cancel").click_accesskit();
    harness.run_steps(8);
    assert!(harness.query_by_label("Update and restart").is_none());
    harness.get_by_label("Update with Scoop").click_accesskit();
    harness.run_steps(8);
    assert!(harness.query_by_label("Update and restart").is_some());
}

/// Public documentation images: real CC0 conversion, isolated paths, and both themes.
#[cfg(all(target_os = "linux", feature = "freecad"))]
#[test]
#[ignore = "renders README images with a GPU; set DUTCHMAN_GUI_SCREENSHOTS"]
fn readme_screenshots() {
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    if std::env::var_os("DUTCHMAN_GUI_SCREENSHOT_CHILD").is_none() {
        let home = tempfile::tempdir().unwrap();
        let designs = home.path().join("~/Designs");
        std::fs::create_dir_all(&designs).unwrap();
        std::fs::copy(
            fixture("design_history.FCStd"),
            designs.join("Bracket.FCStd"),
        )
        .unwrap();
        let tools = home.path().join("~/Applications/SolidCraft");
        std::fs::create_dir_all(&tools).unwrap();
        // Preserve the real check while keeping the machine's installation path private.
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            std::env::var_os("SOLIDCRAFT_CLI").unwrap(),
            tools.join("solidcraft-cli"),
        )
        .unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "readme_screenshots", "--nocapture"])
            .current_dir(home.path())
            .env("DUTCHMAN_GUI_SCREENSHOT_CHILD", "1")
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("XDG_STATE_HOME", home.path().join("state"))
            .env("SOLIDCRAFT_CLI", "~/Applications/SolidCraft/solidcraft-cli")
            .status()
            .unwrap();
        assert!(status.success(), "README screenshot child failed: {status}");
        return;
    }
    let mut index = String::new();
    for (theme, name) in [
        (ThemePreference::Light, "day"),
        (ThemePreference::Dark, "night"),
    ] {
        let mut harness = screenshot_harness(theme);
        harness.run_steps(8);
        if name == "day" {
            shot(
                &mut harness,
                &dir,
                &mut index,
                "readme-drop",
                name,
                "Drop designs here.",
            );
        }
        harness
            .state_mut()
            .add_inputs([PathBuf::from("~/Designs/Bracket.FCStd")]);
        harness
            .state_mut()
            .set_out_dir(PathBuf::from("~/Designs/Converted"));
        wait_for_estimate(&mut harness);
        if name == "day" {
            shot(
                &mut harness,
                &dir,
                &mut index,
                "readme-preset",
                name,
                "Standard preset and live estimate.",
            );
        }
        convert(&mut harness);
        let row = &harness.state().rows()[0];
        assert_eq!(row.status, Status::Converted);
        let report = row.report.as_ref().unwrap();
        assert!(
            report["history"]["mapped"].as_u64().unwrap_or(0) > 0,
            "{report}"
        );
        harness.get_by_label("RESULTS").scroll_to_me();
        harness.run_steps(8);
        shot(
            &mut harness,
            &dir,
            &mut index,
            "readme-result",
            name,
            "CC0 bracket with mapped feature history.",
        );
        std::fs::remove_dir_all("~/Designs/Converted").unwrap();
    }
}

#[test]
fn solidworks_folder_shows_the_plan_and_hides_inventor_settings() {
    let dir = tempfile::tempdir().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dutchman-core/tests/fixtures");
    for (source, name) in [
        ("solidworks_cube.sldprt", "solidworks_cube.sldprt"),
        ("placed_cubes.sldasm", "main.SLDASM"),
    ] {
        std::fs::copy(fixtures.join(source), dir.path().join(name)).unwrap();
    }
    std::fs::write(dir.path().join("~$main.SLDASM"), b"lock").unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1200.0))
        .build_eframe(|cc| test_app(cc));
    harness.state_mut().add_inputs([dir.path().to_path_buf()]);
    let started = Instant::now();
    loop {
        harness.run_steps(2);
        if harness
            .query_all_by_label_contains(
                "1 assembly, 1 part, 0 STEP files; converting assemblies with their parts",
            )
            .count()
            > 0
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        harness
            .query_all_by_label_contains("Inventor project folder:")
            .count(),
        0
    );
    assert!(harness.query_by_label("Choose an output folder.").is_none());
    harness
        .state_mut()
        .set_out_dir(dir.path().join("converted"));
    harness.state_mut().set_verify(false);
    convert(&mut harness);
    assert_eq!(harness.state().rows().len(), 1);
    let row = &harness.state().rows()[0];
    assert_eq!(row.status, Status::Converted);
    assert_eq!(row.input.file_name().unwrap(), "main.SLDASM");
    let report = row.report.as_ref().unwrap();
    assert_eq!(report["completeness"]["left_out"], serde_json::json!([]));
}

#[test]
#[ignore = "renders a supplied project zip with a GPU; set DUTCHMAN_GUI_SCREENSHOTS and DUTCHMAN_PROJECT_ZIP"]
fn project_zip_screenshots() {
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    let input = PathBuf::from(std::env::var_os("DUTCHMAN_PROJECT_ZIP").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut harness = screenshot_harness(ThemePreference::Light);
    let expected_plan =
        dutchman::batch::plan(std::slice::from_ref(&input), Path::new("")).description;
    harness.state_mut().add_inputs([input]);
    harness.state_mut().set_verify(false);
    let started = Instant::now();
    loop {
        harness.run_steps(2);
        if harness.query_by_label(&expected_plan).is_some() {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(120));
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut index = String::new();
    shot(
        &mut harness,
        &dir,
        &mut index,
        "project-added",
        "day",
        "Project plan before conversion.",
    );
    harness.state_mut().set_out_dir(dir.join("converted"));
    harness.run_steps(2);
    harness.get_by_label("Convert").click_accesskit();
    harness.step();
    let started = Instant::now();
    let mut reported = None;
    while harness.state().running() || harness.state().rows().is_empty() {
        if harness.query_by_label("Use Fast").is_some() {
            eprintln!("Using Fast for the project memory check");
            harness.get_by_label("Use Fast").click_accesskit();
            harness.step();
        }
        assert!(
            harness.query_by_label("Convert anyway").is_none(),
            "project exceeds the available memory for this screenshot run"
        );
        let rows = harness.state().rows();
        let completed = rows
            .iter()
            .filter(|row| matches!(row.status, Status::Converted | Status::Failed))
            .count();
        let counts = (completed, rows.len());
        if reported != Some(counts) {
            eprintln!("Project conversion: {completed} of {} finished", rows.len());
            reported = Some(counts);
        }
        assert!(
            started.elapsed() < Duration::from_secs(3600),
            "project conversion did not finish"
        );
        std::thread::sleep(Duration::from_millis(20));
        harness.run_steps(2);
    }
    harness.get_by_label("RESULTS").scroll_to_me();
    harness.run_steps(8);
    shot(
        &mut harness,
        &dir,
        &mut index,
        "project-converted",
        "day",
        "Project conversion results.",
    );
    std::fs::write(dir.join("project-screenshots.md"), index).unwrap();
}

#[cfg(feature = "test-support")]
mod consent_e2e {
    use super::*;
    use egui_kittest::kittest::NodeT;
    use serde_json::{json, Value};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{channel, Receiver, Sender};

    const SENT: &str = "Sent. The report was received.";

    struct Stub {
        endpoint: String,
        requests: Receiver<Value>,
        release: Sender<()>,
        thread: std::thread::JoinHandle<()>,
    }

    impl Stub {
        fn new(status: u16) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/api/reports", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let (tx, requests) = channel();
            let (release, gate) = channel();
            let thread = std::thread::spawn(move || {
                for attempt in 0..if status == 503 { 2 } else { 1 } {
                    let started = Instant::now();
                    let mut stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(started.elapsed() < Duration::from_secs(30));
                                std::thread::sleep(Duration::from_millis(10));
                            }
                            Err(e) => panic!("{e}"),
                        }
                    };
                    // On Windows an accepted socket inherits the listener's non-blocking mode;
                    // this server reads with a timeout instead.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    let (header_end, size) = loop {
                        let mut chunk = [0; 4096];
                        let n = stream.read(&mut chunk).unwrap();
                        assert!(n > 0);
                        bytes.extend_from_slice(&chunk[..n]);
                        assert!(bytes.len() < 32 * 1024);
                        if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                            assert!(headers.starts_with("POST /api/reports HTTP/1.1"));
                            let size = headers
                                .lines()
                                .find_map(|line| {
                                    let (key, value) = line.split_once(':')?;
                                    key.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().unwrap())
                                })
                                .unwrap();
                            break (end + 4, size);
                        }
                    };
                    while bytes.len() < header_end + size {
                        let mut chunk = [0; 4096];
                        let n = stream.read(&mut chunk).unwrap();
                        assert!(n > 0);
                        bytes.extend_from_slice(&chunk[..n]);
                        assert!(bytes.len() < 32 * 1024);
                    }
                    let body = std::str::from_utf8(&bytes[header_end..]).unwrap();
                    assert!(body.contains("name=\"report\""));
                    assert!(!body.contains("name=\"bundle\""));
                    assert!(!body.contains("container_metadata_only"));
                    let metadata = body
                        .split_once("\r\n\r\n")
                        .unwrap()
                        .1
                        .split("\r\n--")
                        .next()
                        .unwrap();
                    let report: Value = serde_json::from_str(metadata).unwrap();
                    tx.send(report.clone()).unwrap();
                    if attempt == 0 {
                        gate.recv_timeout(Duration::from_secs(20)).unwrap();
                    }
                    let receipt = if status == 201 {
                        json!({"report_id": report["report_id"], "received": 1793534400u64})
                    } else {
                        json!({"error": "Storage full"})
                    }
                    .to_string();
                    write!(stream, "HTTP/1.1 {status} Stub\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{receipt}", receipt.len()).unwrap();
                }
            });
            Self {
                endpoint,
                requests,
                release,
                thread,
            }
        }
    }

    fn review(
        endpoint: String,
        signature: Option<String>,
        capture_walkthrough: bool,
    ) -> (Harness<'static, App>, tempfile::TempDir, PathBuf) {
        let out = tempfile::tempdir().unwrap();
        let mut harness = screenshot_harness(ThemePreference::Light);
        harness
            .state_mut()
            .set_report_transport_for_test(endpoint, signature);
        let input = out.path().join("container_metadata_only.f3d");
        std::fs::copy(fixture("container_metadata_only.f3d"), &input).unwrap();
        harness.state_mut().add_inputs([input]);
        harness.state_mut().set_out_dir(out.path().to_path_buf());
        harness.state_mut().set_verify(false);
        // A log path as long as a Windows temp folder's, so every platform checks that a
        // long "Run log:" line keeps to one line and leaves the buttons beside it alone.
        let logs = [
            "Users",
            "RUNNERADMIN",
            "AppData",
            "Local",
            "Temp",
            ".tmpAbCdEf",
        ]
        .iter()
        .fold(out.path().to_path_buf(), |dir, part| dir.join(part));
        std::fs::create_dir_all(&logs).unwrap();
        harness
            .state_mut()
            .set_diagnostic_log(logs.join("diagnostics.jsonl"));
        convert(&mut harness);
        assert_eq!(harness.state().rows()[0].status, Status::Failed);
        harness.get_by_label("RESULTS").scroll_to_me();
        harness.run_steps(8);
        assert!(harness.query_by_label("0 converted, 1 failed").is_some());
        let diagnose = harness.get_by_label("Diagnose this file").rect();
        assert!(
            diagnose.top() > 70.0 && diagnose.bottom() < 730.0,
            "{diagnose:?}"
        );
        let show = harness.get_by_label("Show this run's log").rect();
        assert!(
            show.height() < 40.0,
            "Show this run's log is squeezed: {show:?}"
        );
        if capture_walkthrough {
            walkthrough(&mut harness, "01-failed-card");
        }
        harness
            .get_by_label("Review and send report")
            .click_accesskit();
        harness.run_steps(8);
        harness
            .get_by_label("Send the whole file privately")
            .click_accesskit();
        harness.run_steps(8);
        assert!(harness.query_by_label("Send privately").is_some());
        if capture_walkthrough {
            walkthrough(&mut harness, "02-private-send-card");
        }
        // The private card and the row both offer review; select the window's button.
        harness
            .get_by_label("Send privately")
            .get_by_label("Review and send report")
            .click_accesskit();
        harness.run_steps(8);
        assert!(harness.query_by_label("Send a problem report").is_some());
        assert!(harness.query_by_label(SENT).is_none());
        assert!(harness.query_by_label("Send the bundle").is_none());
        assert!(harness.query_by_label("Retry").is_none());
        assert!(harness.query_by_label_contains("JSON").is_none());
        assert!(harness.query_by_label_contains("claim hash").is_none());
        assert_eq!(
            harness
                .get_by_label("You may use this report in a public test case")
                .accesskit_node()
                .toggled(),
            Some(egui::accesskit::Toggled::False)
        );
        assert!(!harness.get_by_label("Send").accesskit_node().is_disabled());
        let folder = std::fs::read_dir(out.path().join("reports"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(folder.join("prepared.json").is_file());
        assert!(!folder.join("submission.json").exists());
        harness
            .get_by_label("Anonymous summary only")
            .click_accesskit();
        harness.run_steps(3);
        assert!(!harness.get_by_label("Send").accesskit_node().is_disabled());
        if capture_walkthrough {
            walkthrough(&mut harness, "03-consent-screen");
        }
        (harness, out, folder)
    }

    fn walkthrough(harness: &mut Harness<'static, App>, name: &str) {
        if let Some(dir) = std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS") {
            let dir = PathBuf::from(dir).parent().unwrap().join("walkthrough");
            std::fs::create_dir_all(&dir).unwrap();
            let image = harness.render().unwrap();
            assert_eq!(image.dimensions(), (1100, 800));
            image.save(dir.join(format!("{name}.png"))).unwrap();
        }
    }

    fn snapshot(harness: &mut Harness<'static, App>, name: &str) {
        if let Some(dir) = std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS") {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            shot(
                harness,
                &dir,
                &mut String::new(),
                name,
                "day",
                "Consent E2E",
            );
        }
    }

    fn wait_for(harness: &mut Harness<'static, App>, label: &str) {
        let started = Instant::now();
        loop {
            harness.run_steps(2);
            if harness.query_all_by_label_contains(label).count() > 0 {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "waiting for {label}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        harness.run_steps(8);
    }

    /// Reports a converted result as `losses` describes it, through the card and the consent
    /// screen to a stub endpoint; returns what the endpoint received.
    fn send_converted(report: Value) -> Value {
        let stub = Stub::new(201);
        let out = tempfile::tempdir().unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1100.0, 1500.0))
            .build_eframe(|cc| test_app(cc));
        harness
            .state_mut()
            .set_report_transport_for_test(stub.endpoint.clone(), None);
        harness.state_mut().set_out_dir(out.path().to_path_buf());
        let input = out.path().join("Bracket.f3d");
        let mut report = report;
        report["input"] = json!({"path": input, "format": "fusion_archive"});
        harness
            .state_mut()
            .preview_result_for_test(dutchman_gui::app::Row {
                input: input.clone(),
                output: Some(out.path().join("Bracket.sc")),
                status: Status::Converted,
                progress: None,
                report: Some(report),
                job: None,
                size: Some(100),
            });
        harness.run_steps(8);
        harness
            .get_by_label(dutchman::present::REPORT_CONVERTED)
            .click_accesskit();
        harness.run_steps(8);
        harness
            .get_by_label("Review and send report")
            .click_accesskit();
        harness.run_steps(8);
        harness
            .get_by_label("Anonymous summary only")
            .click_accesskit();
        harness.run_steps(3);
        harness.get_by_label("Send").click_accesskit();
        harness.run_steps(3);
        let sent = stub.requests.recv_timeout(Duration::from_secs(10)).unwrap();
        stub.release.send(()).unwrap();
        wait_for(&mut harness, SENT);
        stub.thread.join().unwrap();
        sent
    }

    #[test]
    fn a_converted_result_with_any_loss_can_be_reported() {
        let cases = [
            // Only a note that history was not kept (Info).
            json!({"ok": true, "mode": "import", "losses": [{"level": "Info", "subject": "history",
                "message": "The original geometry is kept alongside the resolved sketches."}]}),
            // Partial history: some features kept, some not.
            json!({"ok": true, "mode": "partial_history", "history": {"mapped": 2, "total": 9},
                "losses": [{"level": "Info", "subject": "history", "message": "7 features were not kept."}]}),
            // Features blocked or flattened.
            json!({"ok": true, "mode": "history", "verification": {"status": "flattened", "reason": "fillet is not mapped yet"},
                "losses": [{"level": "Dropped", "subject": "feature", "message": "Fillet cannot be mapped yet"}]}),
            json!({"ok": true, "mode": "history", "losses": [{"level": "Degraded", "subject": "feature", "message": "edge selection cannot resolve"}]}),
            // Parts unplaced.
            json!({"ok": true, "mode": "import", "completeness": {"placed": 2, "placed_empty": 1, "placements": 3, "left_out": []}}),
            // Colours dropped.
            json!({"ok": true, "mode": "import", "losses": [{"level": "Dropped", "subject": "appearance", "message": "Colour omitted"}]}),
        ];
        for report in cases {
            let sent = send_converted(report.clone());
            assert_eq!(sent["signature"], "converted-with-problem", "{report}");
            assert_eq!(sent["source_format"], "f3d");
            assert_eq!(sent["consent"]["choice"], "summary");
            assert!(!sent.to_string().contains("Bracket"));
        }
    }

    fn local_case(status: u16) {
        let stub = Stub::new(status);
        let (mut harness, _out, folder) = review(stub.endpoint, None, status == 201);
        assert!(stub.requests.try_recv().is_err());
        snapshot(&mut harness, &format!("consent-{status}-before"));
        harness.get_by_label("Send").click_accesskit();
        harness.run_steps(3);
        let report = stub.requests.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            report["consent"],
            json!({"choice": "summary", "text_version": 1, "public_test": false})
        );
        assert_eq!(report["source_format"], "f3d");
        assert_eq!(report["source_app"], "Fusion");
        assert_eq!(report["dutchman_version"], dutchman::BUILD_VERSION);
        assert_eq!(report["claim_hash"].as_str().unwrap().len(), 64);
        assert_eq!(
            serde_json::from_slice::<Value>(
                &std::fs::read(folder.join("submission.json")).unwrap()
            )
            .unwrap(),
            report
        );
        assert!(harness.query_by_label("Sending…").is_some());
        assert!(harness.query_by_label(SENT).is_none());
        assert!(!folder.join("receipt.json").exists());
        assert!(harness.get_by_label("Send").accesskit_node().is_disabled());
        if status == 201 {
            walkthrough(&mut harness, "04-sending");
        }
        stub.release.send(()).unwrap();
        if status == 201 {
            // Observe every frame: the UI must never say sent before the durable receipt.
            let started = Instant::now();
            loop {
                harness.step();
                if harness.query_by_label(SENT).is_some() {
                    assert!(folder.join("receipt.json").is_file());
                    break;
                }
                assert!(started.elapsed() < Duration::from_secs(20));
                std::thread::sleep(Duration::from_millis(10));
            }
            let receipt: Value =
                serde_json::from_slice(&std::fs::read(folder.join("receipt.json")).unwrap())
                    .unwrap();
            assert_eq!(
                receipt,
                json!({"report_id": report["report_id"], "received": 1793534400u64})
            );
            harness.run_steps(8);
            assert!(harness.get_by_label("Send").accesskit_node().is_disabled());
            assert!(harness.query_by_label("Retry").is_none());
        } else {
            wait_for(&mut harness, "It did not go through.");
            assert!(harness.query_by_label("Send").is_none());
            assert_eq!(
                stub.requests.recv_timeout(Duration::from_secs(5)).unwrap(),
                report
            );
            assert!(!folder.join("receipt.json").exists());
            assert!(folder.join("prepared.json").is_file());
            assert!(folder.join("submission.json").is_file());
            assert!(harness.query_by_label(SENT).is_none());
            assert!(!harness.get_by_label("Retry").accesskit_node().is_disabled());
            assert!(harness
                .query_by_label("Send the whole file privately")
                .is_some());
            assert!(harness
                .query_by_label("Open GitHub with anonymous summary only")
                .is_some());
        }
        snapshot(&mut harness, &format!("consent-{status}-after"));
        stub.thread.join().unwrap();
        if status == 201 {
            walkthrough(&mut harness, "05-sent-receipt");
            walkthrough_extras(&mut harness, &report);
        } else {
            walkthrough(&mut harness, "06-send-failed-fallbacks");
        }
    }

    fn walkthrough_extras(harness: &mut Harness<'static, App>, report: &Value) {
        harness
            .get_by_label("Open GitHub with anonymous summary only")
            .click_accesskit();
        harness.step();
        let url = harness
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|command| {
                if let egui::OutputCommand::OpenUrl(open) = command {
                    Some(open.url.clone())
                } else {
                    None
                }
            })
            .unwrap();
        assert!(url.starts_with(dutchman_gui::REPORT_PROBLEM_URL));
        assert!(url.len() < 2000);
        harness.run_steps(8);
        assert!(harness
            .query_by_label(dutchman::present::REPORT_COPIED)
            .is_some());
        walkthrough(harness, "07-github-short-link");
        harness.get_by_label("Diagnose this file").click_accesskit();
        harness.run_steps(8);
        assert!(harness
            .query_by_label("Works locally. Keep the original and local-key.json private.")
            .is_some());
        assert!(harness
            .query_by_label(dutchman::present::REPORT_COPIED)
            .is_none());
        walkthrough(harness, "08-diagnose-wizard");
        let started = Instant::now();
        while harness.get_by_label("Close").accesskit_node().is_disabled() {
            assert!(started.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(20));
            harness.run_steps(2);
        }
        harness.get_by_label("Close").click_accesskit();
        harness.run_steps(8);
        harness.get_by_label("Add matching STEP").click_accesskit();
        harness.run_steps(8);
        assert!(harness.query_by_label("Part to check").is_some());
        walkthrough(harness, "09-add-matching-step");
        harness.get_by_label("Back to conversion").click_accesskit();
        harness.run_steps(8);
        harness.get_by_label("Check a folder").scroll_to_me();
        harness.run_steps(8);
        harness.get_by_label("Check a folder").click_accesskit();
        harness.run_steps(8);
        assert!(harness.query_by_label("Choose folder…").is_some());
        harness.get_by_label(dutchman_gui::APP_NAME);
        walkthrough(harness, "10-check-a-folder");
        harness.get_by_label("Back to conversion").click_accesskit();
        harness.run_steps(8);
        harness
            .state_mut()
            .updater
            .preview(dutchman_gui::update::Release {
                fixes_reports: vec![dutchman::report_send::hash(
                    report["report_id"].as_str().unwrap().as_bytes(),
                )],
                tag: "v99.0.0".into(),
                notes: "Synthetic release for desktop report testing.".into(),
                download: "https://github.com/dutchman-cad/dutchman/releases/tag/v99.0.0".into(),
            });
        harness.run_steps(8);
        harness.get_by_label("Update to 99.0.0").click_accesskit();
        harness.run_steps(8);
        assert!(harness
            .query_by_label_contains("This update fixes a problem you reported")
            .is_some());
        walkthrough(harness, "11-update-fixes-your-report");
    }

    #[test]
    fn consent_e2e_201_receipt() {
        local_case(201);
    }

    #[test]
    fn consent_e2e_503_keeps_report_and_fallbacks() {
        local_case(503);
    }

    #[test]
    #[ignore = "sends a synthetic summary to DUTCHMAN_E2E_ENDPOINT after UI consent"]
    fn consent_e2e_live() {
        let endpoint = std::env::var("DUTCHMAN_E2E_ENDPOINT").unwrap();
        let (mut harness, _out, folder) = review(
            endpoint,
            Some("e2e-test-desktop-consent-summary".into()),
            false,
        );
        snapshot(&mut harness, "consent-live-before");
        harness.get_by_label("Send").click_accesskit();
        harness.step();
        wait_for(&mut harness, SENT);
        let receipt: Value =
            serde_json::from_slice(&std::fs::read(folder.join("receipt.json")).unwrap()).unwrap();
        assert!(receipt["received"].as_u64().is_some());
        let saved: Value =
            serde_json::from_slice(&std::fs::read(folder.join("submission.json")).unwrap())
                .unwrap();
        assert_eq!(saved["signature"], "e2e-test-desktop-consent-summary");
        assert_eq!(receipt["report_id"], saved["report_id"]);
        eprintln!("Live receipt: {receipt}");
        snapshot(&mut harness, "consent-live-after");
    }
}

/// A synthetic design whose estimate is far above the free memory given.
#[cfg(feature = "test-support")]
fn memory_harness(free: u64) -> (Harness<'static, App>, PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("Synthetic assembly.f3d");
    std::fs::copy(fixture("generated_unit_cube.f3d"), &input).unwrap();
    let sizes = dutchman::choices::read_sizes(&input).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(760.0, 900.0))
        .build_eframe(|cc| test_app(cc));
    harness.state_mut().add_inputs([input.clone()]);
    harness.state_mut().set_out_dir(dir.path().join("out"));
    harness.state_mut().set_verify(false);
    harness
        .state_mut()
        .set_sizes_for_test(input.clone(), sizes, free);
    harness.run_steps(8);
    (harness, input, dir)
}

#[test]
#[cfg(feature = "test-support")]
fn an_estimate_above_free_memory_starts_converting_at_once() {
    let (mut h, _input, dir) = memory_harness(1);
    h.run_steps(8);
    h.get_by_label("Convert").click_accesskit();
    h.step();
    assert!(h.state().running());
    assert!(h
        .query_by_label("This conversion may need more memory")
        .is_none());
    assert!(h.query_by_label("Convert with these choices").is_none());
    wait_for_conversion(&mut h);
    assert_eq!(h.state().rows().len(), 1);
    assert_eq!(h.state().rows()[0].status, Status::Converted);
    assert!(dir.path().join("out").exists());
}

#[test]
#[cfg(feature = "test-support")]
fn memory_presets_expose_help_to_focus_and_accessibility() {
    use egui_kittest::kittest::NodeT;
    let (mut h, _, _dir) = memory_harness(1);
    let selector = h
        .query_all_by_value("Standard")
        .find(|node| !node.accesskit_node().is_disabled())
        .unwrap();
    let description = selector.accesskit_node().description().unwrap().to_owned();
    assert!(description.contains("leaves history out"));
    selector.focus();
    h.run_steps(4);
    h.get_by_label(&description);
    h.key_press(egui::Key::Space);
    h.run_steps(4);
    for (label, text) in [
        ("Fast", "Shape only"),
        ("Standard", "leaves history out"),
        ("Full", "keeps history regardless"),
    ] {
        let entry = h
            .query_all_by_label(label)
            .find(|node| !node.accesskit_node().is_disabled())
            .unwrap();
        assert!(entry.accesskit_node().description().unwrap().contains(text));
        let description = entry.accesskit_node().description().unwrap().to_owned();
        entry.focus();
        h.run_steps(4);
        h.get_by_label(&description);
    }
}

#[test]
#[cfg(feature = "test-support")]
fn choices_reach_the_conversion() {
    let (mut h, input, dir) = memory_harness(1);
    h.get_by_label("Advanced").click_accesskit();
    h.run_steps(8);
    for label in ["Sketches", "Joints", "Feature history"] {
        h.get_by_label(label).click_accesskit();
        h.run_steps(4);
    }
    convert(&mut h);
    let report = h.state().rows().first().unwrap().report.as_ref().unwrap();
    for choice in ["history", "sketches", "joints"] {
        assert_eq!(
            report.pointer(&format!("/choices/{choice}")),
            Some(&serde_json::json!(false))
        );
    }
    let choices = dutchman::choices::Choices {
        history: false,
        sketches: false,
        joints: false,
        history_within_budget: true,
    };
    let unchecked = dutchman::choices::estimate(&input, &choices).unwrap();
    assert_eq!(
        report.pointer("/estimate/peak_bytes"),
        Some(&serde_json::json!(unchecked.peak_bytes))
    );
    assert!(dir.path().join("out").exists());
}

#[test]
fn run_log_buttons_share_the_batch_log_and_review_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = screenshot_harness(ThemePreference::Light);
    let log = dir
        .path()
        .join("2026-11-02_163710_dutchman-0.2.6_failed.jsonl");
    for name in ["first.f3d", "second.f3d"] {
        let input = dir.path().join(name);
        std::fs::write(&input, "synthetic refusal").unwrap();
        harness.state_mut().add_inputs([input]);
    }
    harness.state_mut().set_diagnostic_log(log.clone());
    harness
        .state_mut()
        .set_out_dir(dir.path().join("converted"));
    convert(&mut harness);
    harness.get_by_label("RESULTS").scroll_to_me();
    harness.run_steps(8);
    assert_eq!(harness.query_all_by_label("Show this run's log").count(), 2);
    assert_eq!(harness.query_all_by_label("Copy log to…").count(), 2);
    assert_eq!(
        harness
            .query_all_by_label(&format!(
                "Run log: {}",
                dutchman_gui::platform::display_path(&log)
            ))
            .count(),
        2
    );
    assert!(log.is_file()); // Explicit paths remain exactly as supplied, even result-like names.
    harness
        .query_all_by_label("Review and send report")
        .next()
        .unwrap()
        .click_accesskit();
    harness.run_steps(8);
    harness
        .get_by_label("Send a problem report")
        .get_by_label(&format!(
            "Run log: {}",
            dutchman_gui::platform::display_path(&log)
        ));
    harness.get_by_label("This run's log contains no names or contents from the design. It is kept locally; Send uploads only the summary or bundle reviewed below.");
}

#[test]
#[ignore = "renders the failed run-log card with a GPU"]
fn run_log_screenshots() {
    // Isolate recovery notices too, so screenshots never display the owner's log paths.
    if std::env::var_os("DUTCHMAN_RUNLOG_SCREENSHOT_CHILD").is_none() {
        let state = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "run_log_screenshots"])
            .env("DUTCHMAN_RUNLOG_SCREENSHOT_CHILD", "1")
            .env("HOME", state.path())
            .env("LOCALAPPDATA", state.path())
            .env("XDG_STATE_HOME", state.path())
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("synthetic-part.f3d");
    std::fs::copy(fixture("container_metadata_only.f3d"), &input).unwrap();
    let mut index = String::new();
    for (theme, name) in [
        (ThemePreference::Light, "day"),
        (ThemePreference::Dark, "night"),
    ] {
        let mut harness = screenshot_harness(theme);
        harness.state_mut().add_inputs([input.clone()]);
        harness.state_mut().set_out_dir(work.path().join(name));
        harness.state_mut().set_verify(false);
        harness.state_mut().set_diagnostic_log(
            work.path()
                .join("2026-11-02_163710_dutchman-0.2.6_failed.jsonl"),
        );
        convert(&mut harness);
        assert_eq!(harness.state().rows()[0].status, Status::Failed);
        harness
            .get_by_label(dutchman::present::REPORT_BUTTON)
            .scroll_to_me();
        harness.run_steps(8);
        for label in [
            "Show this run's log",
            "Copy log to…",
            dutchman::present::REPORT_BUTTON,
        ] {
            let rect = harness.get_by_label(label).rect();
            assert!(
                rect.left() >= 0.0
                    && rect.right() < 1100.0
                    && rect.top() > 60.0
                    && rect.bottom() < 730.0,
                "{label}: {rect:?}"
            );
        }
        shot(
            &mut harness,
            &dir,
            &mut index,
            "run-log-failed",
            name,
            "Failed synthetic conversion with its own log selected or copied.",
        );
    }
    std::fs::write(dir.join("run-logs.md"), index).unwrap();
}

fn ai_result_card_at(
    theme: ThemePreference,
    width: f32,
) -> (Harness<'static, App>, tempfile::TempDir) {
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("synthetic-part.f3d");
    std::fs::copy(fixture("generated_unit_cube.f3d"), &input).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(width, 800.0))
        .build_eframe(|cc| test_app(cc));
    harness.ctx.set_theme(theme);
    harness.state_mut().add_inputs([input]);
    harness
        .state_mut()
        .set_out_dir(work.path().join("converted"));
    harness.state_mut().set_verify(false);
    // No diagnostic log is chosen: the run still keeps one in the diagnostics folder.
    convert(&mut harness);
    assert_eq!(harness.state().rows()[0].status, Status::Converted);
    harness
        .get_by_label("Ask your AI about this file")
        .scroll_to_me();
    harness.run_steps(8);
    (harness, work)
}

fn ai_result_card(theme: ThemePreference) -> (Harness<'static, App>, tempfile::TempDir) {
    ai_result_card_at(theme, 1100.0)
}

const COPIED: &str = "Copied. Paste it into your AI assistant.";

fn copied_texts(harness: &Harness<'static, App>) -> Vec<String> {
    harness
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Clicks `label` and returns what the click copied.
fn copy_with(harness: &mut Harness<'static, App>, label: &str) -> String {
    harness.get_by_label(label).click_accesskit();
    harness.step();
    let mut copied = copied_texts(harness);
    assert_eq!(copied.len(), 1, "{label}: {copied:?}");
    assert!(!harness
        .output()
        .platform_output
        .commands
        .iter()
        .any(|command| matches!(command, egui::OutputCommand::OpenUrl(_))));
    harness.run_steps(8);
    harness.get_by_label(COPIED);
    copied.remove(0)
}

fn copy_ai_prompt(harness: &mut Harness<'static, App>) -> String {
    copy_with(harness, "Ask your AI about this file")
}

/// The text of the prompt box under the button.
#[cfg(feature = "test-support")]
fn shown_prompt(harness: &Harness<'static, App>) -> String {
    harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .value()
        .unwrap()
}

#[cfg(feature = "test-support")]
#[test]
fn ask_ai_copies_a_short_prompt_naming_files_that_exist() {
    let (mut harness, _work) = ai_result_card(ThemePreference::Light);
    assert!(harness.query_by_label(COPIED).is_none());
    assert!(harness
        .query_by_role(egui::accesskit::Role::MultilineTextInput)
        .is_none());
    let prompt = copy_ai_prompt(&mut harness);
    assert!(!prompt.contains('\n'));
    let row = &harness.state().rows()[0];
    let output = row.output.as_ref().unwrap();
    let report = output.with_extension("report.json");
    // The run's log is in the diagnostics folder (isolated under test), not beside the design.
    let folder = harness.state().diagnostics_folder_for_test().unwrap();
    let log = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .expect("a log in the diagnostics folder");
    assert!(log.starts_with(folder));
    assert!(std::fs::read_dir(output.parent().unwrap())
        .unwrap()
        .all(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_none_or(|ext| ext != "jsonl")));
    for path in [&row.input, output, &report, &log] {
        assert!(path.exists(), "{path:?}");
        assert!(prompt.contains(&path.display().to_string()), "{prompt}");
    }
    // The guide is beside the app when installed, else on GitHub.
    let guide = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("AGENTS.md");
    if guide.is_file() {
        assert!(prompt.contains(&guide.display().to_string()));
    } else {
        assert!(prompt.contains(&dutchman_gui::app::agents_url(dutchman::BUILD_VERSION)));
    }
    assert!(prompt.contains("Don't send any files anywhere without asking me"));
    assert!(
        prompt.contains("`dutchman investigate` on the original file"),
        "{prompt}"
    );
    // Short apart from the paths it names (these are long temporary ones).
    let paths: usize = [&row.input, output, &report, &log]
        .iter()
        .map(|path| path.display().to_string().len())
        .sum();
    assert!(
        prompt.len() - paths < 600,
        "{} bytes: {prompt}",
        prompt.len()
    );
    assert!(prompt.matches(". ").count() <= 6, "{prompt}");
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report).unwrap()).unwrap();
    assert_eq!(Some(&saved), row.report.as_ref());
    // The same text is shown under the button, and Copy copies it again.
    assert_eq!(shown_prompt(&harness), prompt);
    assert_eq!(copy_with(&mut harness, "Copy"), prompt);
    assert_eq!(shown_prompt(&harness), prompt);
    harness.get_by_label("Close").click_accesskit();
    harness.run_steps(4);
    assert!(harness
        .query_by_role(egui::accesskit::Role::MultilineTextInput)
        .is_none());
}

#[test]
fn ask_ai_copies_every_time_and_after_a_new_conversion() {
    let (mut harness, _work) = ai_result_card(ThemePreference::Light);
    let first = copy_ai_prompt(&mut harness);
    // Copied goes away after a short while and the button is as before.
    std::thread::sleep(Duration::from_millis(2200));
    harness.run_steps(4);
    assert!(harness.query_by_label(COPIED).is_none());
    assert_eq!(copy_ai_prompt(&mut harness), first);
    // Straight away again: it copies again, without waiting.
    harness
        .get_by_label("Ask your AI about this file")
        .click_accesskit();
    harness.step();
    assert_eq!(copied_texts(&harness), std::slice::from_ref(&first));
    harness.run_steps(8);
    // Converting again replaces the card: no stale prompt or Copied, and the button works.
    harness.get_by_label("Convert").click_accesskit();
    harness.step();
    wait_for_conversion(&mut harness);
    harness.run_steps(8);
    assert!(harness.query_by_label(COPIED).is_none());
    assert!(harness
        .query_by_role(egui::accesskit::Role::MultilineTextInput)
        .is_none());
    let again = copy_ai_prompt(&mut harness);
    assert!(again.starts_with("Help me with a Dutchman conversion."));
}

#[cfg(feature = "test-support")]
#[test]
fn a_report_button_opens_the_section_for_a_result_with_losses_only() {
    for width in [1100.0, 520.0] {
        // With losses: the cube. Without: a synthetic clean result.
        let (mut harness, work) = ai_result_card_at(ThemePreference::Light, width);
        assert!(harness
            .query_by_label(dutchman::present::REPORT_CONVERTED)
            .is_some());
        assert!(harness.query_by_label("Review and send report").is_none());
        let button = harness.get_by_label("Report this file…").rect();
        assert!(
            button.left() >= 0.0 && button.right() <= width,
            "{button:?}"
        );
        let ask = harness.get_by_label("Ask your AI about this file").rect();
        assert!(ask.left() >= 0.0 && ask.right() <= width, "{ask:?}");
        harness.get_by_label("Report this file…").click_accesskit();
        harness.run_steps(8);
        let review = harness.get_by_label("Review and send report").rect();
        assert!(
            review.left() >= 0.0 && review.right() <= width,
            "{review:?}"
        );
        let _ = work;
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 800.0))
        .build_eframe(|cc| test_app(cc));
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("Clean.f3d");
    harness
        .state_mut()
        .preview_result_for_test(dutchman_gui::app::Row {
            input: input.clone(),
            output: Some(dir.path().join("Clean.sc")),
            status: Status::Converted,
            progress: None,
            report: Some(serde_json::json!({"ok": true, "mode": "history",
            "input": {"path": input, "format": "fusion_archive"}, "losses": []})),
            job: None,
            size: Some(100),
        });
    harness.run_steps(8);
    assert!(harness
        .query_by_label(dutchman::present::REPORT_CONVERTED)
        .is_some());
    assert!(harness.query_by_label("Report this file…").is_none());
    assert!(harness
        .query_by_label("Ask your AI about this file")
        .is_none());
}

#[test]
fn ask_ai_prompt_fits_a_narrow_window() {
    let (mut harness, _work) = ai_result_card_at(ThemePreference::Light, 520.0);
    copy_ai_prompt(&mut harness);
    harness.get_by_label("Close").scroll_to_me();
    harness.run_steps(8);
    for label in ["Copy", "Close", "Ask your AI about this file"] {
        let rect = harness.get_by_label(label).rect();
        assert!(
            rect.left() >= 0.0 && rect.right() <= 520.0,
            "{label}: {rect:?}"
        );
    }
    let prompt = harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .rect();
    assert!(
        prompt.left() >= 0.0 && prompt.right() <= 520.0,
        "{prompt:?}"
    );
}

#[test]
#[ignore = "renders the AI result card in Day and Night with a GPU"]
fn ask_ai_screenshots() {
    // Keep paths private and out of the pictures: work in a fixed folder under /tmp.
    let root = PathBuf::from("/tmp/dutchman-shots/ask-ai");
    if std::env::var_os("DUTCHMAN_AI_SCREENSHOT_CHILD").is_none() {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("state")).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "ask_ai_screenshots", "--nocapture"])
            .env("DUTCHMAN_AI_SCREENSHOT_CHILD", "1")
            .env("XDG_STATE_HOME", root.join("state"))
            .env("LOCALAPPDATA", root.join("state"))
            .env("TMPDIR", &root)
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut index = String::new();
    for (theme, name) in [
        (ThemePreference::Light, "day"),
        (ThemePreference::Dark, "night"),
    ] {
        let (mut harness, _work) = ai_result_card(theme);
        // Show neutral checker metadata rather than the machine's installed CLI path.
        #[cfg(feature = "test-support")]
        harness.state_mut().set_checker_for_test(true);
        harness.run_steps(8);
        let button = harness.get_by_label("Ask your AI about this file").rect();
        assert!(button.top() > 60.0 && button.bottom() < 800.0, "{button:?}");
        shot(
            &mut harness,
            &dir,
            &mut index,
            "ask-ai-before",
            name,
            "Converted synthetic file with losses: the button before any click.",
        );
        let prompt = copy_ai_prompt(&mut harness);
        std::fs::write(dir.join(format!("copied-prompt-{name}.txt")), &prompt).unwrap();
        if name == "day" {
            std::fs::write(dir.join("copied-prompt.txt"), &prompt).unwrap();
        }
        assert_eq!(harness.query_all_by_label_contains("/home/").count(), 0);
        harness.get_by_label("Close").scroll_to_me();
        harness.run_steps(8);
        shot(
            &mut harness,
            &dir,
            &mut index,
            "ask-ai-copied",
            name,
            "Right after the click: Copied, and the prompt open under the button.",
        );
        std::thread::sleep(Duration::from_millis(2200));
        harness.run_steps(8);
        assert!(harness.query_by_label(COPIED).is_none());
        shot(
            &mut harness,
            &dir,
            &mut index,
            "ask-ai-after",
            name,
            "Two seconds later: Copied gone, the prompt still open.",
        );
    }
    std::fs::write(dir.join("ask-ai.md"), index).unwrap();
}

#[test]
#[ignore = "renders real GPU screenshots"]
fn converted_report_screenshots() {
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    if std::env::var_os("DUTCHMAN_GUI_SCREENSHOT_CHILD").is_none() {
        let state = tempfile::tempdir().unwrap();
        if !cfg!(target_os = "linux") {
            panic!("diagnostics isolation uses XDG_STATE_HOME on Linux");
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "converted_report_screenshots",
                "--nocapture",
            ])
            .env("DUTCHMAN_GUI_SCREENSHOT_CHILD", "1")
            .env("XDG_STATE_HOME", state.path())
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let work = tempfile::tempdir().unwrap();
    // Use the real checker through a temporary alias so its displayed path has no home name.
    let screenshot_cli = work.path().join("solidcraft-cli");
    #[cfg(unix)]
    if let Ok(cli) = dutchman_solidcraft::usable_cli(&dutchman_solidcraft::Verification::Auto) {
        std::os::unix::fs::symlink(cli, &screenshot_cli).unwrap();
    }
    let input = work.path().join("Synthetic cube.f3d");
    std::fs::copy(fixture("generated_unit_cube.f3d"), &input).unwrap();
    let mut index =
        String::from("# Converted file reporting\n\nSynthetic cube, report section expanded.\n\n");
    for (theme, name) in [
        (ThemePreference::Light, "day"),
        (ThemePreference::Dark, "night"),
    ] {
        for (width, label) in [(1100.0, "wide"), (520.0, "520")] {
            let mut h = Harness::builder()
                .with_size(egui::vec2(width, 1100.0))
                .build_eframe(|cc| test_app(cc));
            h.ctx.set_theme(theme);
            h.state_mut()
                .set_solidcraft_cli(Some(screenshot_cli.clone()));
            h.state_mut().set_verify(false);
            h.state_mut().add_inputs([input.clone()]);
            h.state_mut()
                .set_out_dir(work.path().join(format!("converted-{name}-{label}")));
            convert(&mut h);
            assert_eq!(h.state().rows()[0].status, Status::Converted);
            assert!(h.state().rows()[0].size.is_some_and(|bytes| bytes > 0));
            h.get_by_label("Report this file…").scroll_to_me();
            h.run_steps(8);
            shot(
                &mut h,
                &dir,
                &mut index,
                &format!("converted-button-{label}"),
                name,
                "Converted synthetic file with losses: the Report this file button in the card's row.",
            );
            h.get_by_label("Report this file…").click_accesskit();
            h.run_steps(8);
            h.get_by_label(dutchman::present::REPORT_BUTTON)
                .scroll_to_me();
            h.run_steps(8);
            h.get_by_label(dutchman::present::SEND_DM_BUTTON);
            h.get_by_label("Drag the file in (its folder is open).");
            assert!(h.query_by_label("Diagnose this file").is_none());
            assert_eq!(h.query_all_by_label_contains("/home/").count(), 0);
            assert!(h.get_by_label("Converted").rect().top() > 70.0);
            assert!(
                h.get_by_label(dutchman::present::REPORT_BUTTON)
                    .rect()
                    .bottom()
                    < 1030.0
            );
            shot(
                &mut h,
                &dir,
                &mut index,
                &format!("converted-report-{label}"),
                name,
                "Converted synthetic file with report review and private-send steps expanded.",
            );
        }
    }
    std::fs::write(dir.join("index.md"), index).unwrap();
}

#[cfg(feature = "test-support")]
#[test]
#[ignore = "renders GPU screenshots of a card with unchecked extrude regions"]
fn unchecked_regions_screenshots() {
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let extrude = |unchecked: bool| {
        let mut row = serde_json::json!({"name": "Extrude", "kind": "extrude", "status": "mapped"});
        if unchecked {
            row["profile_unverified"] = serde_json::Value::Bool(true);
        }
        row
    };
    let report = serde_json::json!({
        "ok": true,
        "mode": "partial_history",
        "verification": {"status": "partial", "reason": "history stopped at feature \"Fillet 1\""},
        "history": {"mapped": 5, "total": 7, "rebuilt": 5, "needing": 7, "features": [
            {"name": "Sketch 1", "kind": "sketch", "status": "mapped"},
            extrude(true), extrude(true), extrude(false),
            {"name": "Sketch 2", "kind": "sketch", "status": "mapped"},
            {"name": "Fillet 1", "kind": "fillet", "status": "blocked"},
            {"name": "Chamfer 1", "kind": "chamfer", "status": "blocked"}
        ]},
        "losses": []
    });
    let mut index = String::from("# Unchecked extrude regions on the card\n\n");
    for (theme, name) in [
        (ThemePreference::Light, "day"),
        (ThemePreference::Dark, "night"),
    ] {
        for width in [520.0, 1000.0] {
            let mut harness = screenshot_harness(theme);
            harness.set_size(egui::vec2(width, 700.0));
            harness.state_mut().set_checker_for_test(true);
            harness
                .state_mut()
                .preview_result_for_test(dutchman_gui::app::Row {
                    input: PathBuf::from("synthetic.f3d"),
                    output: Some(PathBuf::from("converted/synthetic.sc")),
                    status: Status::Converted,
                    progress: None,
                    job: None,
                    size: None,
                    report: Some(report.clone()),
                });
            harness.run_steps(8);
            harness
                .get_by_label_contains("may use the wrong sketch region")
                .scroll_to_me();
            harness.run_steps(8);
            shot(
                &mut harness,
                &dir,
                &mut index,
                &format!("unchecked-regions-{width}"),
                name,
                "Synthetic partial history with two rebuilt extrudes whose regions couldn't be checked.",
            );
        }
    }
    std::fs::write(dir.join("index.md"), index).unwrap();
}

#[cfg(feature = "test-support")]
#[test]
#[ignore = "renders GPU screenshots of a card whose rebuilt features don't match the source"]
fn body_mismatch_screenshots() {
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let why = "the retained features give 0 bodies where the source has 1 body at the same point";
    let report = serde_json::json!({
        "ok": true,
        "mode": "partial_history",
        "verification": {"status": "partial", "reason": format!(
            "history stopped at feature \"Chamfer 1\"; retained features replay in SolidCraft, but {why}"
        )},
        "history": {"mapped": 5, "total": 7, "rebuilt": 5, "needing": 7, "features": [
            {"name": "Sketch 1", "kind": "sketch", "status": "mapped"},
            {"name": "Extrude 1", "kind": "extrude", "status": "mapped"},
            {"name": "Extrude 2", "kind": "extrude", "status": "mapped"},
            {"name": "Sketch 2", "kind": "sketch", "status": "mapped"},
            {"name": "Extrude 3", "kind": "extrude", "status": "mapped"},
            {"name": "Chamfer 1", "kind": "chamfer", "status": "blocked"},
            {"name": "Fillet 1", "kind": "fillet", "status": "blocked"}
        ]},
        "losses": [{"code": "history/body-mismatch", "level": "Degraded",
            "subject": "synthetic.f3d", "message": why}]
    });
    let mut index = String::from("# Rebuilt features that don't match the source, on the card\n\n");
    for (theme, name) in [
        (ThemePreference::Light, "day"),
        (ThemePreference::Dark, "night"),
    ] {
        for width in [520.0, 1000.0] {
            let mut harness = screenshot_harness(theme);
            harness.set_size(egui::vec2(width, 700.0));
            harness.state_mut().set_checker_for_test(true);
            harness
                .state_mut()
                .preview_result_for_test(dutchman_gui::app::Row {
                    input: PathBuf::from("synthetic.f3d"),
                    output: Some(PathBuf::from("converted/synthetic.sc")),
                    status: Status::Converted,
                    progress: None,
                    job: None,
                    size: None,
                    report: Some(report.clone()),
                });
            harness.run_steps(8);
            harness
                .get_by_label_contains("don't match the original")
                .scroll_to_me();
            harness.run_steps(8);
            shot(
                &mut harness,
                &dir,
                &mut index,
                &format!("body-mismatch-{width}"),
                name,
                "Synthetic partial history whose rebuilt features give no body where the source has one.",
            );
        }
    }
    std::fs::write(dir.join("index.md"), index).unwrap();
}

#[test]
#[ignore = "renders a GPU screenshot of the Part positions choices"]
fn part_positions_screenshot() {
    // Isolate settings and paths so the image never shows the owner's home folder.
    if std::env::var_os("DUTCHMAN_PART_POSITIONS_CHILD").is_none() {
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "part_positions_screenshot"])
            .env("DUTCHMAN_PART_POSITIONS_CHILD", "1")
            .env("HOME", home.path())
            .env("LOCALAPPDATA", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("XDG_STATE_HOME", home.path().join("state"))
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let work = tempfile::tempdir().unwrap();
    let input = work.path().join("synthetic-part.f3d");
    std::fs::copy(fixture("container_metadata_only.f3d"), &input).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 800.0))
        .build_eframe(|cc| test_app(cc));
    harness.ctx.set_theme(ThemePreference::Light);
    harness.state_mut().add_inputs([input]);
    harness
        .state_mut()
        .set_out_dir(work.path().join("converted"));
    wait_for_estimate(&mut harness);
    harness.get_by_label("Advanced").click_accesskit();
    harness.run_steps(8);
    harness.get_by_label("Part positions").click_accesskit();
    harness.run_steps(8);
    harness.get_by_label("As Fusion last showed them");
    harness.get_by_label("As first placed");
    let mut index = String::new();
    shot(
        &mut harness,
        &dir,
        &mut index,
        "part-positions",
        "day",
        "Advanced open with the Part positions choices listed.",
    );
}

#[test]
fn part_positions_last_shown_names_the_program() {
    let cases: [(&[&str], &str); 5] = [
        (&["a.f3d", "b.f3d"], "As Fusion last showed them"),
        (&["a.ipt", "b.iam"], "As Inventor last showed them"),
        (&["a.sldprt"], "As SolidWorks last showed them"),
        (&["a.f3d", "b.ipt"], "As last shown in the original program"),
        (&[], "As last shown in the original program"),
    ];
    for (files, expected) in cases {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 800.0))
            .build_eframe(|cc| test_app(cc));
        harness
            .state_mut()
            .add_inputs(files.iter().map(PathBuf::from));
        harness.run_steps(8);
        harness.get_by_label("Advanced").click_accesskit();
        harness.run_steps(8);
        harness.get_by_label("Part positions").click_accesskit();
        harness.run_steps(8);
        harness.get_by_label(expected);
        assert_eq!(
            harness
                .query_all_by_label_contains("last showed them")
                .count()
                + harness
                    .query_all_by_label_contains("last shown in the original")
                    .count(),
            1,
            "{files:?}"
        );
    }
}

/// A window of the default size for a monitor of `monitor` points, with one converted design
/// as the only result, and the Advanced choices open.
fn fit_harness(size: [f32; 2], theme: ThemePreference, work: &Path) -> Harness<'static, App> {
    fit_harness_for(size, theme, work, "generated_unit_cube.f3d")
}

/// [`fit_harness`] with a file that cannot convert: its failed card is the one result.
fn failed_fit_harness(
    size: [f32; 2],
    theme: ThemePreference,
    work: &Path,
) -> Harness<'static, App> {
    let h = fit_harness_for(size, theme, work, "container_metadata_only.f3d");
    assert_eq!(h.state().rows()[0].status, Status::Failed);
    h
}

/// A failed card fits the window at once: its title, log, Diagnose and the report button
/// sit between the RESULTS heading and the Convert row, with nothing scrolling.
fn assert_failed_card_in_view(h: &Harness<'_, App>, name: impl std::fmt::Debug) {
    assert!(!h.state().page_scrolls(), "{name:?}: the page scrolls");
    let heading = h.get_by_label("RESULTS").rect();
    let convert_row = h.get_by_label("Convert").rect().top();
    for label in [
        "Failed",
        "Show this run's log",
        "Diagnose this file",
        "Review and send report",
        dutchman::present::SEND_FOLD,
    ] {
        let rect = h.get_by_label(label).rect();
        assert!(
            rect.top() >= heading.bottom() && rect.bottom() <= convert_row,
            "{name:?}: {label} {rect:?} out of view between {heading:?} and {convert_row}"
        );
    }
}

fn fit_harness_for(
    size: [f32; 2],
    theme: ThemePreference,
    work: &Path,
    name: &str,
) -> Harness<'static, App> {
    let input = work.join(name);
    std::fs::copy(fixture(name), &input).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size[0], size[1]))
        .build_eframe(|cc| test_app(cc));
    harness.ctx.set_theme(theme);
    harness.state_mut().preview_solidcraft(Some("0.41.9"));
    // A fixed checker, so no path of the machine running the test is shown.
    #[cfg(feature = "test-support")]
    harness.state_mut().set_checker_for_test(true);
    harness.state_mut().add_inputs([input]);
    harness.state_mut().set_out_dir(work.join("converted"));
    harness.state_mut().set_verify(false);
    harness.run_steps(8);
    harness.get_by_label("Advanced").click_accesskit();
    harness.run_steps(8);
    convert(&mut harness);
    harness
}

/// Every control and heading of the main screen sits inside the window, above the Convert row.
fn assert_main_screen_in_view(h: &Harness<'_, App>, size: [f32; 2]) {
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0], size[1]));
    let convert_row = h.get_by_label("Convert").rect().top();
    for label in [
        "DESIGNS",
        "Add files…",
        "OUTPUT FOLDER",
        "Preset",
        "Part positions",
        "Feature history",
        "Joints",
        "Output format",
        "Replace designs already in the output folder",
        "Open diagnostics folder",
        "RESULTS",
        "Open in SolidCraft",
    ] {
        let rect = h.get_by_label(label).rect();
        assert!(
            window.contains_rect(rect),
            "{size:?}: {label} {rect:?} outside the window"
        );
        assert!(
            rect.bottom() <= convert_row,
            "{size:?}: {label} {rect:?} runs under the Convert row at {convert_row}"
        );
    }
}

#[test]
fn the_default_size_fits_the_main_screen_without_scrolling() {
    for monitor in [[1440.0, 900.0], [1920.0, 1080.0]] {
        let size = dutchman_gui::fit_size(monitor);
        assert!(size[0] <= monitor[0] * 0.9 && size[1] <= monitor[1] * 0.9);
        assert!(size[0] >= dutchman_gui::WIDE_BREAKPOINT);
        let work = tempfile::tempdir().unwrap();
        let h = fit_harness(size, ThemePreference::Light, work.path());
        assert_eq!(h.state().rows().len(), 1);
        assert!(
            !h.state().page_scrolls(),
            "{monitor:?}: the page scrolls at {size:?}"
        );
        assert_main_screen_in_view(&h, size);
        // After a run that didn't finish, its notice still fits: nothing scrolls or moves out.
        let mut h = h;
        h.state_mut()
            .preview_unfinished_run(work.path().join("2026-11-05_010203_dutchman_running.jsonl"));
        h.run_steps(4);
        assert!(h.get_by_label("The last run didn't finish.").rect().width() > 0.0);
        assert!(
            !h.state().page_scrolls(),
            "{monitor:?}: the unfinished-run notice makes the page scroll at {size:?}"
        );
        assert_main_screen_in_view(&h, size);
        // Designs and the output choices sit beside the results, not above them.
        let designs = h.get_by_label("DESIGNS").rect();
        let results = h.get_by_label("RESULTS").rect();
        assert!(
            results.left() > designs.right(),
            "{monitor:?}: not side by side"
        );
        // With the Ask-your-AI prompt open, the whole card still shows without scrolling:
        // the box keeps to a few lines and scrolls inside.
        let mut h = h;
        h.get_by_label("Ask your AI about this file")
            .click_accesskit();
        h.run_steps(8);
        assert!(
            !h.state().page_scrolls(),
            "{monitor:?}: the open prompt makes the page scroll at {size:?}"
        );
        assert_main_screen_in_view(&h, size);
        let heading = h.get_by_label("RESULTS").rect();
        let top = h.get_by_label("Open in SolidCraft").rect();
        assert!(
            top.top() >= heading.bottom(),
            "{monitor:?}: the card's top {top:?} scrolled under RESULTS {heading:?}"
        );
        let convert_row = h.get_by_label("Convert").rect().top();
        for label in ["Copy", "Close"] {
            let rect = h.get_by_label(label).rect();
            assert!(
                rect.bottom() <= convert_row,
                "{monitor:?}: {label} {rect:?} below the Convert row at {convert_row}"
            );
        }
        // The box's own rect is its whole text; what shows is the space between the button
        // and the Copy row under it: the Report row, "Copied" and about five lines (uncapped,
        // the whole prompt took about 400 px).
        let ask = h.get_by_label("Ask your AI about this file").rect();
        let copy = h.get_by_label("Copy").rect();
        assert!(
            copy.top() - ask.bottom() < 220.0,
            "{monitor:?}: the prompt takes {} px",
            copy.top() - ask.bottom()
        );
        // A failed file's card fits too, with its other ways to report folded.
        let work = tempfile::tempdir().unwrap();
        let h = failed_fit_harness(size, ThemePreference::Light, work.path());
        assert_failed_card_in_view(&h, monitor);
        assert!(h.query_by_label("Open invite").is_none());
    }
}

#[test]
fn the_default_size_is_centred_and_capped_at_the_monitor() {
    assert_eq!(
        dutchman_gui::fit_size([1920.0, 1080.0]),
        dutchman_gui::FIT_SIZE
    );
    assert_eq!(
        dutchman_gui::fit_size([1440.0, 900.0]),
        dutchman_gui::FIT_SIZE
    );
    // A small monitor takes 90% of it, never less than the smallest usable window.
    assert_eq!(dutchman_gui::fit_size([1280.0, 720.0]), [1152.0, 648.0]);
    assert_eq!(dutchman_gui::fit_size([400.0, 300.0]), [520.0, 420.0]);
    assert_eq!(
        dutchman_gui::centred_position([1920.0, 1080.0], [1180.0, 800.0]),
        [370.0, 140.0]
    );
    assert_eq!(
        dutchman_gui::centred_position([400.0, 300.0], [520.0, 420.0]),
        [0.0, 0.0]
    );
}

#[test]
fn many_results_scroll_in_their_own_column_and_not_the_page() {
    let out = tempfile::tempdir().unwrap();
    let size = dutchman_gui::FIT_SIZE;
    let mut h = Harness::builder()
        .with_size(egui::vec2(size[0], size[1]))
        .build_eframe(|cc| test_app(cc));
    // One folder of three designs gives three results from one entry in Designs.
    let folder = out.path().join("Designs");
    std::fs::create_dir_all(&folder).unwrap();
    for name in [
        "generated_unit_cube.f3d",
        "synthetic_cube.ipt",
        "solidworks_cube.sldprt",
    ] {
        std::fs::copy(fixture(name), folder.join(name)).unwrap();
    }
    h.state_mut().add_inputs([folder]);
    h.state_mut().set_out_dir(out.path().join("converted"));
    h.state_mut().set_verify(false);
    h.run_steps(8);
    h.get_by_label("Advanced").click_accesskit();
    h.run_steps(8);
    convert(&mut h);
    assert_eq!(h.state().rows().len(), 3);
    assert!(!h.state().page_scrolls());
    let convert_row = h.get_by_label("Convert").rect().top();
    assert!(h.get_by_label("Open diagnostics folder").rect().bottom() <= convert_row);
    assert!(h.get_by_label("RESULTS").rect().bottom() <= convert_row);
}

#[test]
fn a_narrow_window_keeps_the_single_column() {
    let work = tempfile::tempdir().unwrap();
    let size = [520.0, 1600.0];
    let h = fit_harness(size, ThemePreference::Light, work.path());
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0], size[1]));
    let designs = h.get_by_label("DESIGNS").rect();
    let output = h.get_by_label("OUTPUT FOLDER").rect();
    let results = h.get_by_label("RESULTS").rect();
    assert!(
        output.top() > designs.bottom(),
        "Output folder is below Designs"
    );
    assert!(
        results.top() > output.bottom(),
        "Results is below the options"
    );
    for label in [
        "DESIGNS",
        "Add files…",
        "OUTPUT FOLDER",
        "Preset",
        "RESULTS",
    ] {
        let rect = h.get_by_label(label).rect();
        assert!(
            window.contains_rect(rect),
            "{label} {rect:?} outside the window"
        );
    }
    // The column keeps its width: nothing runs past the right edge.
    assert!(h.get_by_label("Open in SolidCraft").rect().right() <= size[0]);
}

#[test]
fn the_remove_button_does_not_overlap_the_row_text() {
    for width in [520.0, 1180.0] {
        let work = tempfile::tempdir().unwrap();
        let h = fit_harness([width, 1500.0], ThemePreference::Light, work.path());
        let remove = h.get_by_label("Remove").rect();
        let mut seen = 0;
        for node in h.get_all_by_role(egui::accesskit::Role::Label) {
            let rect = node.rect();
            // Text on the Remove button's own line.
            if rect.bottom() > remove.top()
                && rect.top() < remove.bottom()
                && rect.right() > 0.0
                && rect != remove
            {
                seen += 1;
                assert!(
                    rect.right() <= remove.left() + 0.5 || rect.left() >= remove.right() - 0.5,
                    "{width}: text {rect:?} overlaps Remove {remove:?}"
                );
            }
        }
        assert!(seen > 0, "{width}: found no text beside Remove");
    }
}

#[test]
fn a_result_title_has_its_own_line_above_one_row_of_buttons() {
    for size in [[1180.0, 800.0], [520.0, 1500.0]] {
        let work = tempfile::tempdir().unwrap();
        let h = fit_harness(size, ThemePreference::Light, work.path());
        let name = h
            .get_all_by_label("generated_unit_cube.f3d")
            .rfind(|node| node.rect().top() > 80.0)
            .expect("result title");
        let name = name.rect();
        // The whole name is laid out on one line: nothing is elided.
        assert!(name.width() > 150.0, "{size:?}: name {name:?}");
        let open = h.get_by_label("Open in SolidCraft").rect();
        let show = h.get_by_label("Show in folder").rect();
        let step = h.get_by_label("Add matching STEP").rect();
        for b in [open, show, step] {
            assert!(
                b.top() >= name.bottom() - 0.5,
                "{size:?}: {b:?} not below {name:?}"
            );
        }
        // One row, in order.
        assert!((open.center().y - show.center().y).abs() < 1.0);
        assert!(open.right() <= show.left());
        if size[0] > 1000.0 {
            assert!((open.center().y - step.center().y).abs() < 1.0);
            assert!(show.right() <= step.left());
        }
    }
}

#[test]
fn a_very_long_result_name_wraps_instead_of_eliding() {
    let work = tempfile::tempdir().unwrap();
    let long = "a_very_long_generated_unit_cube_design_name_".repeat(3) + ".f3d";
    let input = work.path().join(&long);
    std::fs::copy(fixture("generated_unit_cube.f3d"), &input).unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(520.0, 1500.0))
        .build_eframe(|cc| test_app(cc));
    #[cfg(feature = "test-support")]
    h.state_mut().set_checker_for_test(true);
    h.state_mut().add_inputs([input]);
    h.state_mut().set_out_dir(work.path().join("converted"));
    h.state_mut().set_verify(false);
    h.run_steps(8);
    convert(&mut h);
    let titles: Vec<_> = h
        .get_all_by_label(&long)
        .filter(|node| node.rect().top() > 300.0)
        .collect();
    let title = titles.last().expect("result title");
    // Wrapped onto several lines, so taller than one heading line.
    assert!(title.rect().height() > 40.0, "{:?}", title.rect());
    assert!(title.rect().right() <= 520.0);
}

#[test]
#[ignore = "renders GPU screenshots of the default window size; set DUTCHMAN_GUI_SCREENSHOTS"]
fn fit_screenshots() {
    // Isolate settings and paths so no image shows the owner's home folder.
    if std::env::var_os("DUTCHMAN_FIT_CHILD").is_none() {
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "fit_screenshots", "--nocapture"])
            .env("DUTCHMAN_FIT_CHILD", "1")
            .env("HOME", home.path())
            .env("LOCALAPPDATA", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("XDG_STATE_HOME", home.path().join("state"))
            .status()
            .unwrap();
        assert!(status.success(), "screenshot child failed: {status}");
        return;
    }
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let cases = [
        ("fit-1440x900", dutchman_gui::fit_size([1440.0, 900.0])),
        ("fit-1920x1080", dutchman_gui::fit_size([1920.0, 1080.0])),
        ("fit-520", [520.0, 1500.0]),
    ];
    for (theme, label) in [
        (ThemePreference::Light, "Day"),
        (ThemePreference::Dark, "Night"),
    ] {
        for (name, size) in cases {
            let work = tempfile::tempdir().unwrap();
            let mut h = fit_harness(size, theme, work.path());
            if size[0] >= dutchman_gui::WIDE_BREAKPOINT {
                assert!(!h.state().page_scrolls(), "{name}: the page scrolls");
            }
            h.render()
                .unwrap()
                .save(dir.join(format!("{name}-{label}.png")))
                .unwrap();
            if name == "fit-1440x900" {
                h.state_mut().preview_unfinished_run(
                    work.path().join("2026-11-05_010203_dutchman_running.jsonl"),
                );
                h.run_steps(4);
                assert!(
                    !h.state().page_scrolls(),
                    "{name}: the notice makes the page scroll"
                );
                h.render()
                    .unwrap()
                    .save(dir.join(format!("{name}-unfinished-{label}.png")))
                    .unwrap();
            }
            if size[0] >= dutchman_gui::WIDE_BREAKPOINT {
                let work = tempfile::tempdir().unwrap();
                let mut h = failed_fit_harness(size, theme, work.path());
                assert_failed_card_in_view(&h, name);
                h.render()
                    .unwrap()
                    .save(dir.join(format!("{name}-failed-{label}.png")))
                    .unwrap();
                open_help(&mut h);
                h.render()
                    .unwrap()
                    .save(dir.join(format!("{name}-failed-help-{label}.png")))
                    .unwrap();
            }
        }
    }
}

#[cfg(feature = "test-support")]
#[test]
#[ignore = "renders the SolidCraft version window with a GPU; set DUTCHMAN_GUI_SCREENSHOTS"]
fn solidcraft_window_screenshots() {
    // Isolate settings and paths so no image shows the owner's home folder.
    if std::env::var_os("DUTCHMAN_FIT_CHILD").is_none() {
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "solidcraft_window_screenshots",
                "--nocapture",
            ])
            .env("DUTCHMAN_FIT_CHILD", "1")
            .env("HOME", home.path())
            .env("LOCALAPPDATA", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("XDG_STATE_HOME", home.path().join("state"))
            .status()
            .unwrap();
        assert!(status.success(), "screenshot child failed: {status}");
        return;
    }
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let size = dutchman_gui::fit_size([1440.0, 900.0]);
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0], size[1]));
    for found in [true, false] {
        let work = tempfile::tempdir().unwrap();
        let mut h = fit_harness(size, ThemePreference::Light, work.path());
        if !found {
            h.state_mut().preview_solidcraft(None);
            h.state_mut().preview_search_for_test(&[
                ("SOLIDCRAFT_CLI", "unset"),
                ("PATH", "not_found"),
                ("beside Dutchman", "not_found"),
                ("Program Files", "not_found"),
                ("Local app data", "not_found"),
                ("Windows registry", "not_found"),
            ]);
        }
        h.run_steps(8);
        h.get_by_label("SolidCraft version…").click_accesskit();
        h.run_steps(8);
        let window = h
            .ctx
            .memory(|m| m.area_rect(egui::Id::new("SolidCraft version")))
            .expect("the version window is open");
        assert!(
            viewport.contains_rect(window),
            "{window:?} outside {viewport:?}"
        );
        let texts: &[&str] = if found {
            &["SolidCraft 0.41 is installed."]
        } else {
            &[
                "SolidCraft not found.",
                "Looked in:\n• SOLIDCRAFT_CLI: not set",
            ]
        };
        for text in texts {
            let rect = h.get_by_label_contains(text).rect();
            assert!(
                window.contains_rect(rect),
                "{text}: {rect:?} outside the window {window:?}"
            );
        }
        let name = if found { "found" } else { "not-found" };
        h.render()
            .unwrap()
            .save(dir.join(format!("solidcraft-window-{name}-Day.png")))
            .unwrap();
    }
}

#[test]
#[ignore = "renders the update window with a GPU; set DUTCHMAN_GUI_SCREENSHOTS to a folder"]
fn update_dialog_screenshots() {
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    // Settings, window size and diagnostics come from a temporary home, never the reader's.
    if std::env::var_os("DUTCHMAN_GUI_SCREENSHOT_CHILD").is_none() {
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "update_dialog_screenshots",
                "--nocapture",
            ])
            .env("DUTCHMAN_GUI_SCREENSHOT_CHILD", "1")
            .env("HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_CACHE_HOME", home.path().join("cache"))
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("XDG_STATE_HOME", home.path().join("state"))
            .status()
            .unwrap();
        assert!(status.success(), "screenshot child failed: {status}");
        return;
    }
    let feed = include_bytes!("../src/update/fixtures/releases.atom");
    let mut index = String::new();
    for state in ["checking", "available", "failed"] {
        let mut harness = screenshot_harness(ThemePreference::Light);
        harness.set_size(egui::vec2(900.0, 760.0));
        #[cfg(feature = "test-support")]
        harness.state_mut().set_checker_for_test(true);
        match state {
            "checking" => harness.state_mut().updater.preview_checking(),
            "available" => harness.state_mut().updater.preview_feed(feed),
            _ => harness
                .state_mut()
                .updater
                .preview_failed("dns error: failed to lookup address information"),
        }
        harness.run_steps(8);
        if state == "available" {
            // The notes open at their first line, as the reader sees them.
            let window = harness
                .ctx
                .memory(|m| m.area_rect(egui::Id::new("Dutchman update")))
                .unwrap();
            let first = harness
                .get_by_label_contains("Dutchman Convert 0.2.7 adds")
                .rect();
            assert!(
                window.contains_rect(first),
                "intro {first:?} not in {window:?}"
            );
        }
        match state {
            "checking" => assert!(harness
                .query_by_label_contains("Checking for updates")
                .is_some()),
            "available" => {
                assert!(harness
                    .query_by_label_contains("Dutchman 0.2.7 is available")
                    .is_some());
                assert!(harness.query_by_label_contains("What's new").is_some());
            }
            _ => assert!(harness.query_by_label("Retry").is_some()),
        }
        let homes: Vec<_> = harness
            .query_all_by_label_contains("/home/")
            .map(|n| {
                format!(
                    "{:?} {:?}",
                    egui_kittest::kittest::NodeT::accesskit_node(&n).value(),
                    egui_kittest::kittest::NodeT::accesskit_node(&n).role()
                )
            })
            .collect();
        assert!(homes.is_empty(), "{homes:?}");
        shot(
            &mut harness,
            &dir,
            &mut index,
            &format!("update-{state}"),
            "light",
            state,
        );
        if state == "available" {
            harness.get_by_label("What's new").scroll_to_me();
            harness.run_steps(8);
            harness
                .get_by_label_contains("Report problems")
                .scroll_to_me();
            harness.run_steps(8);
            shot(
                &mut harness,
                &dir,
                &mut index,
                "update-available-code",
                "light",
                "inline code",
            );
        }
    }
}

#[test]
fn the_guide_link_names_this_builds_version() {
    assert_eq!(
        dutchman_gui::app::agents_url("0.2.7"),
        "https://github.com/dutchman-cad/dutchman/blob/v0.2.7/assets/install/AGENTS.md"
    );
    assert_eq!(
        dutchman_gui::app::agents_url("0.2.8-nightly.abc1234"),
        "https://github.com/dutchman-cad/dutchman/blob/abc1234/assets/install/AGENTS.md"
    );
}

#[cfg(feature = "test-support")]
#[test]
#[ignore = "renders running-step GPU screenshots; set DUTCHMAN_GUI_SCREENSHOTS"]
fn running_step_screenshots() {
    let dir = PathBuf::from(std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS").unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    for (theme, label) in [
        (ThemePreference::Light, "Day"),
        (ThemePreference::Dark, "Night"),
    ] {
        for seconds in [0, 61, 120, 121] {
            let mut harness = screenshot_harness(theme);
            harness.state_mut().preview_solidcraft(Some("0.41.0"));
            harness.state_mut().set_checker_for_test(true);
            harness
                .state_mut()
                .preview_running_for_test(dutchman_gui::app::Row {
                    input: PathBuf::from("Assembly.iam"),
                    output: None,
                    status: Status::Converting,
                    report: None,
                    job: None,
                    size: None,
                    progress: Some(dutchman_core::ProgressEvent {
                        stage: dutchman_core::Stage::Map,
                        elapsed_ms: (seconds + 30) * 1000,
                        designs_read: 261,
                        designs_total: Some(264),
                        heartbeat: true,
                        running: Some(dutchman_core::RunningStep {
                            kind: dutchman_core::StepKind::CheckingHistory,
                            name: Some("Bracket".into()),
                            elapsed_ms: seconds * 1000,
                            usual_ms: dutchman_core::StepKind::Checking.usual_ms(),
                            limit_ms: Some(dutchman_solidcraft::DEFAULT_TIMEOUT.as_secs() * 1000),
                            consequence: Some(
                                dutchman_core::progress_words::history_limit(false, false).into(),
                            ),
                        }),
                    }),
                });
            harness.run_steps(8);
            let caption = harness
                .state()
                .rows()
                .first()
                .unwrap()
                .progress
                .as_ref()
                .unwrap()
                .running_line();
            assert!(harness.query_by_label(&caption).is_some());
            assert!(harness.query_by_label("261 of 264 designs read").is_some());
            assert!(harness.query_by_label("Stop").is_some());
            harness
                .render()
                .unwrap()
                .save(dir.join(format!("running-{seconds}s-{label}.png")))
                .unwrap();
        }
    }
}

#[cfg(feature = "test-support")]
#[test]
fn folder_run_keeps_the_oldest_file_visible_below_the_counts() {
    for (theme, label, width) in [
        (ThemePreference::Light, "Day", 1100.0),
        (ThemePreference::Dark, "Night", 1100.0),
        (ThemePreference::Light, "Day", 520.0),
        (ThemePreference::Dark, "Night", 520.0),
    ] {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(width, 800.0))
            .build_eframe(|cc| test_app(cc));
        harness.ctx.set_theme(theme);
        harness.state_mut().preview_solidcraft(Some("0.41.0"));
        harness.state_mut().set_checker_for_test(true);
        let rows = (0..1293)
            .map(|index| {
                let active = (1261..1264).contains(&index);
                dutchman_gui::app::Row {
                    input: PathBuf::from(if index == 1261 {
                        "Assembly.iam".into()
                    } else {
                        format!("Part-{index}.ipt")
                    }),
                    output: None,
                    status: if index < 1261 {
                        Status::Converted
                    } else if active {
                        Status::Converting
                    } else {
                        Status::Waiting
                    },
                    report: None,
                    job: None,
                    size: None,
                    progress: active.then(|| dutchman_core::ProgressEvent {
                        stage: dutchman_core::Stage::Map,
                        elapsed_ms: if index == 1261 { 151_000 } else { 30_000 },
                        designs_read: 261,
                        designs_total: Some(264),
                        heartbeat: true,
                        running: Some(dutchman_core::RunningStep {
                            kind: dutchman_core::StepKind::CheckingHistory,
                            name: Some("Bracket".into()),
                            elapsed_ms: 61_000,
                            usual_ms: 60_000,
                            limit_ms: Some(120_000),
                            consequence: Some(
                                dutchman_core::progress_words::history_limit(false, false).into(),
                            ),
                        }),
                    }),
                }
            })
            .collect();
        harness
            .state_mut()
            .preview_folder_running_for_test(PathBuf::from("Project"), rows);
        harness.run_steps(8);
        assert!(harness.query_by_label("1261 of 1293 done").is_some());
        assert!(harness.query_by_label("Assembly.iam: building history (2 other files running)\nStill checking Bracket (rebuilt history): 1 min 1 s").is_some());
        let line = harness.get_by_label("Assembly.iam: building history (2 other files running)\nStill checking Bracket (rebuilt history): 1 min 1 s").rect();
        let count = harness.get_by_label("1261 of 1293 done").rect();
        assert!(line.top() >= count.bottom());
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 800.0))
                .contains_rect(line)
        );
        if let Some(dir) = std::env::var_os("DUTCHMAN_GUI_SCREENSHOTS") {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            harness
                .render()
                .unwrap()
                .save(dir.join(format!("folder-1261-of-1293-{width:.0}-{label}.png")))
                .unwrap();
        }
    }
}
