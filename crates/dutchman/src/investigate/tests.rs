use super::*;

/// A conversion report shaped like a real one, every name and size invented.
fn report(input: &Path) -> Value {
    json!({
        "schema": wire::CONVERT,
        "ok": true,
        "input": {
            "path": input.display().to_string(),
            "format": "fusion_design",
            "format_name": "Fusion design (.f3d)",
        },
        "output": {"path": input.with_extension("sc").display().to_string()},
        "solidcraft": {"found": "0.30.1"},
        "names": ["Gearbox Housing", "Lid Mount"],
        "verification": {
            "status": "partial",
            "reason": "feature \"Lid Mount Extrude\": its profile region doesn't match the source (swept 76.8 mm², the source 146852.0 mm²)",
        },
        "history": {
            "total": 6, "mapped": 3, "needing": 5, "rebuilt": 2, "reapplied": 0,
            "features": [
                {"name": "Sketch 1", "kind": "sketch", "status": "mapped", "ordinal": 0},
                {"name": "Lid Mount Extrude", "kind": "extrude", "status": "mapped", "ordinal": 1,
                 "variant": "extrude, blind", "profile_unverified": true},
                {"name": "Gearbox Housing Fillet", "kind": "fillet", "status": "blocked", "ordinal": 2,
                 "reason": "its face (182.1342 mm2, 61 boundary edges) on Gearbox Housing is not among the faces SolidCraft rebuilds before it"},
                {"name": "Fillet 7", "kind": "fillet", "status": "blocked", "ordinal": 4,
                 "reason": "a second fillet blocker"},
                {"name": "Chamfer 3", "kind": "chamfer", "status": "blocked", "ordinal": 5,
                 "source": "solidcraft", "category": "blend_failed",
                 "upstream": "https://github.com/tangency-labs/solidcraft/issues/12",
                 "reason": "SolidCraft could not blend 3.25 mm"},
            ],
        },
        "losses": [],
    })
}

fn scrubber(report: &Value, input: &Path) -> Scrubber {
    let mut scrub = Scrubber::default();
    scrub.know_path(input);
    scrub.know_names_in(report);
    scrub
}

fn anonymous(report: &Value, input: &Path, out: &Path) -> Value {
    let salt = Salt::parse("00112233445566778899aabbccddeeff").unwrap();
    anon::report(
        &json!({"results": [report]}),
        &[input.to_path_buf()],
        out,
        &salt,
        &anon::Run::default(),
    )
    .0
}

#[test]
fn numbers_are_found_and_written_n() {
    assert_eq!(
        numbers("v0.2.8, 182.1342 mm2 and f3z."),
        ["0.2.8", "182.1342", "2", "3"]
    );
    assert_eq!(
        without_numbers("182.1342 mm2, 61 edges."),
        "N mmN, N edges."
    );
}

#[test]
fn the_summary_keeps_kinds_and_positions_but_no_names_or_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("Secret Project").join("Gearbox 4471.f3d");
    let report = report(&input);
    let scrub = scrubber(&report, &input);
    let anonymous = anonymous(&report, &input, dir.path());
    let md = summary(&report, &anonymous, None, None, None, &scrub);
    check(&md, &report, &anonymous, &[], None, &scrub).unwrap();
    let text = &md.text;
    for private in [
        "Secret",
        "Gearbox",
        "4471",
        "Lid Mount",
        "Housing",
        "182",
        "1342",
        "76.8",
        "146852",
        "3.25",
        "Chamfer 3",
        "Fillet 7",
    ] {
        assert!(!text.contains(private), "{private} in:\n{text}");
    }
    assert!(
        text.contains("- fillet, feature 3 in the timeline: its face (N mmN, N boundary edges)"),
        "{text}"
    );
    // Only the first blocker of each kind.
    assert!(!text.contains("a second fillet blocker"), "{text}");
    assert!(text.contains("## Replays that may be wrong"), "{text}");
    assert!(
        text.contains("- extrude, feature 2 in the timeline: its profile is the whole of a sketch"),
        "{text}"
    );
    assert!(text.contains("profile region does not match"), "{text}");
    assert!(
        text.contains(
            "- blend_failed: tracked at https://github.com/tangency-labs/solidcraft/issues/12"
        ),
        "{text}"
    );
    assert!(text.contains("- History: 2 of 5 features"), "{text}");
    assert!(text.contains("- Synthetic case: not tried"), "{text}");
    // Replays that may be wrong come before the blockers.
    assert!(text.find("Replays that may be wrong") < text.find("First blocked feature"));
}

#[test]
fn a_number_from_the_design_written_as_fixed_text_is_caught() {
    let input = Path::new("/somewhere/part.f3d");
    let report = report(input);
    let scrub = scrubber(&report, input);
    let anonymous = json!({});
    let mut md = Md::default();
    md.fixed("a size of 182.1342 slipped through\n");
    let error = check(&md, &report, &anonymous, &[], None, &scrub).unwrap_err();
    assert!(error.contains("number from the design"), "{error}");
}

#[test]
fn a_name_from_the_design_is_caught() {
    let input = Path::new("/somewhere/part.f3d");
    let report = report(input);
    let scrub = scrubber(&report, input);
    let mut md = Md::default();
    // As if the scrubbing had been skipped.
    md.text.push_str("Lid Mount");
    md.designed.push_str("Lid Mount");
    let error = check(&md, &report, &json!({}), &[], None, &scrub).unwrap_err();
    assert!(error.contains("name or path"), "{error}");
}

#[test]
fn a_given_report_is_summarised_kept_private_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("Gearbox 4471.f3d");
    std::fs::write(&input, b"not read: the report is given").unwrap();
    let saved = dir.path().join("saved.report.json");
    std::fs::write(&saved, serde_json::to_vec(&report(&input)).unwrap()).unwrap();
    let out = dir.path().join("inv");
    let settings = Settings {
        out: Some(out.clone()),
        report: Some(saved.clone()),
        send: Consent::No,
        compare: false,
        synthetic: false,
        ..Settings::default()
    };
    let asked = std::cell::Cell::new(false);
    let ask = |_: &str| {
        asked.set(true);
        true
    };
    let result = run(&input, &settings, &ask, &|_| {}).unwrap();
    assert!(!asked.get());
    assert_eq!(result["schema"], SCHEMA);
    assert_eq!(result["sent"], false);
    assert_eq!(result["report"], "given");
    let summary = std::fs::read_to_string(out.join(SUMMARY)).unwrap();
    assert!(!summary.contains("Gearbox"), "{summary}");
    for private in ["local-key.json", "anonymous.json", STATE] {
        assert!(out.join(private).is_file(), "{private}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
        let mode = std::fs::metadata(out.join(SUMMARY))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }
    // A second run, to send after asking, reuses the summary without the report.
    std::fs::remove_file(&saved).unwrap();
    let again = run(&input, &settings, &ask, &|_| {}).unwrap();
    assert_eq!(again["summary_path"], result["summary_path"]);
    assert_eq!(std::fs::read_to_string(out.join(SUMMARY)).unwrap(), summary);
}

#[test]
fn sending_without_a_service_keeps_the_summary_locally() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("part.f3d");
    std::fs::write(&input, b"x").unwrap();
    let out = dir.path().join("inv");
    std::fs::create_dir_all(&out).unwrap();
    let id = Salt::random().unwrap().file_id(&input);
    let sent = send_summary(&out, &id, "fusion_design", "# summary\n", &[], None).unwrap();
    assert_eq!(sent["sent"], false);
    let folder = report_send::local_folder(&out, &id);
    let prepared = report_send::Prepared::load(&folder).unwrap();
    assert_eq!(prepared.signature, SIGNATURE);
    assert_eq!(prepared.parts.len(), 1);
    assert_eq!(prepared.parts.first().unwrap().role, "summary");
    // The same consent is reused on a retry.
    let again = send_summary(&out, &id, "fusion_design", "# summary\n", &[], None).unwrap();
    assert_eq!(again["sent"], false);
}

#[test]
fn newer_builds_are_summarised_regressions_first_and_pass_the_check() {
    let input = Path::new("/somewhere/Gearbox 4471.f3d");
    let report = report(input);
    let scrub = scrubber(&report, input);
    let anonymous = json!({});
    let deltas = [
        compare::Delta {
            label: "fix-12".to_owned(),
            title: "Fillets on closed edges".to_owned(),
            ok_before: true,
            ok_after: true,
            rebuilt_before: 2,
            rebuilt_after: 3,
            needing: 5,
            better: vec![(2, "fillet".to_owned(), "blocked".to_owned())],
            ..compare::Delta::default()
        },
        compare::Delta {
            label: "nightly".to_owned(),
            title: "Nightly 2026-11-08".to_owned(),
            ok_before: true,
            ok_after: true,
            rebuilt_before: 2,
            rebuilt_after: 1,
            needing: 5,
            worse: vec![(1, "extrude".to_owned(), "blocked".to_owned())],
            ..compare::Delta::default()
        },
        compare::Delta {
            label: "fix-13".to_owned(),
            title: "x".to_owned(),
            skipped: Some("/home/someone/.cache: no space".to_owned()),
            ..compare::Delta::default()
        },
    ];
    let md = summary(&report, &anonymous, None, Some(&deltas), None, &scrub);
    check(&md, &report, &anonymous, &deltas, None, &scrub).unwrap();
    let text = &md.text;
    let nightly = text.find("- nightly (Nightly 2026-11-08)").unwrap();
    let fix = text.find("- fix-12 (Fillets on closed edges)").unwrap();
    assert!(nightly < fix, "{text}");
    assert!(
        text.contains("  - worse: extrude, feature 2 in the timeline is now blocked"),
        "{text}"
    );
    assert!(
        text.contains("  - better: fillet, feature 3 in the timeline is rebuilt"),
        "{text}"
    );
    assert!(
        text.contains("rebuilds 3 features (this version 2)"),
        "{text}"
    );
    assert!(
        text.contains("- fix-13 (x): could not be compared"),
        "{text}"
    );
    assert!(!text.contains("someone"), "{text}");
    // Without the deltas the build numbers are not allowed.
    assert!(check(&md, &report, &anonymous, &[], None, &scrub).is_err());
}

#[test]
fn the_synthetic_line_says_why_there_is_no_case_and_how_long_it_took() {
    let input = Path::new("/somewhere/part.f3d");
    let report = report(input);
    let scrub = scrubber(&report, input);
    let none = synthetic::Synthetic::from_json(&json!({
        "made": false,
        "reason": "SolidCraft can't yet export a design with components as a command script",
        "seconds": 37,
    }))
    .unwrap();
    let md = summary(&report, &json!({}), None, None, Some(&none), &scrub);
    check(&md, &report, &json!({}), &[], Some(&none), &scrub).unwrap();
    assert!(
        md.text.contains(
            "- Synthetic case: none made: SolidCraft can't yet export a design with components \
             as a command script (https://github.com/tangency-labs/solidcraft/issues/6562) \
             (took 37 s)"
        ),
        "{}",
        md.text
    );
    let made = synthetic::Synthetic {
        folder: Some(PathBuf::from("synthetic")),
        steps: Some(3),
        reason: None,
        seconds: 412,
    };
    let md = summary(&report, &json!({}), None, None, Some(&made), &scrub);
    check(&md, &report, &json!({}), &[], Some(&made), &scrub).unwrap();
    assert!(
        md.text.contains(
            "- Synthetic case: made, 3 steps on invented dimensions, in synthetic/ (took 412 s)"
        ),
        "{}",
        md.text
    );
    // Without the record the step's numbers are not allowed.
    assert!(check(&md, &report, &json!({}), &[], None, &scrub).is_err());
}
