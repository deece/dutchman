//! `dutchman`: the command-line converter.
//!
//! Exit codes: 0 when the design was converted (or inspected), 1 when it could not be, 2 for a
//! usage error or memory-budget refusal. See `docs/json.md` for the `--json` reports.

use std::ffi::OsString;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use dutchman::{anon, batch, formats, json, worker, ConvertOptions};
use dutchman_core::{Check, FeatureStatus, Format, Mode};
use serde_json::Value;

/// Convert CAD files: dutchman part.f3d writes part.sc.
#[derive(Parser)]
#[command(
    name = "dutchman",
    version,
    about,
    allow_external_subcommands = true,
    after_help = "Examples:\n  dutchman part.f3d\n  dutchman part.f3d out.sc\n  dutchman convert part.f3d out.sc"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Print each loss note in full (the text reports cut them to their first sentence).
    #[arg(long, global = true)]
    verbose: bool,
    /// Hide conversion progress and the volunteer invitation after conversion results.
    #[arg(long, global = true)]
    quiet: bool,
    /// Never prompt to switch presets; refuse a conversion that exceeds free memory.
    /// With report --send-summary, explicitly consent to sending the reviewed summary.
    #[arg(long, global = true)]
    yes: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Review a local problem report, including a file that converted with a wrong result.
    Report {
        /// Folder holding prepared.json, or a fresh folder for a summary-only report.
        #[arg(long)]
        folder: PathBuf,
        /// Reuse the anonymous file id from the diagnostic log or report key.
        #[arg(long)]
        report_id: Option<String>,
        #[arg(long)]
        source_format: Option<String>,
        /// Use converted-with-problem for a converted file with a wrong result.
        #[arg(long)]
        signature: Option<String>,
        /// Send only the anonymous summary after reviewing it; also requires --yes.
        #[arg(long, requires = "yes")]
        send_summary: bool,
        /// Allow this report to be used in a public test case.
        #[arg(long, requires = "send_summary")]
        public_test: bool,
        #[arg(long)]
        json: bool,
    },
    /// Check a folder or project ZIP locally, resuming completed checks on the next run.
    Doctor {
        input: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        /// Maximum concurrent designs, capped by available CPUs.
        #[arg(long)]
        jobs: Option<std::num::NonZeroUsize>,
        /// Total converter memory budget in MiB, shared across jobs.
        #[arg(long, default_value_t = 8192)]
        memory_mb: u64,
        /// Longest one part may take.
        #[arg(long, value_parser = dutchman::diagnose::parse_duration)]
        time_limit: Option<std::time::Duration>,
        /// Matching STEP for a project with one design; otherwise discovered beside each file.
        #[arg(long)]
        step: Option<PathBuf>,
        /// Anonymous part number to target in the report.
        #[arg(long, value_parser = clap::value_parser!(usize))]
        part: Option<usize>,
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        options: ConvertOptions,
    },
    /// Make a private local reproduction kit; only kit/anonymous.json is safe to share.
    Repro {
        input: PathBuf,
        /// Fresh folder for the kit and temporary local trials. A `dutchman diagnose` report
        /// already in it (its `diagnose` folder) sends the kit straight to the failing part.
        #[arg(long)]
        out: PathBuf,
        /// Use the report `dutchman diagnose` wrote to this folder (its --out folder) instead.
        #[arg(long, value_name = "FOLDER")]
        from_diagnose: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Find which parts of a large design fail, run slowly or take the memory, on this machine,
    /// and write a ranked report that names parts by number only; nothing leaves the machine.
    Diagnose {
        /// Matching STEP export; a same-name export beside the file is found automatically.
        #[arg(long, requires = "input")]
        step: Option<PathBuf>,
        /// Make this anonymous part number the report target.
        #[arg(long, requires = "input")]
        part: Option<usize>,
        /// The native design file to read.
        #[arg(required_unless_present_any = ["same_failure", "make_report"])]
        input: Option<PathBuf>,
        /// Run the whole narrowing: rank the parts, isolate a failing part, find the first
        /// feature it fails at and record the failure's signature. Run it again with the same
        /// --out to carry on after a stop.
        #[arg(long, requires = "input", conflicts_with_all = ["same_failure", "make_report"])]
        narrow: bool,
        /// Cut and prove a private reproduction bundle without narrowing. Nothing is sent.
        #[arg(long, requires = "input", conflicts_with_all = ["narrow", "same_failure", "make_report"])]
        bundle: bool,
        /// Convert this small rebuilt file and say whether it fails the way the design narrowed
        /// into --out did.
        #[arg(long, value_name = "FILE", conflicts_with_all = ["input", "make_report"])]
        same_failure: Option<PathBuf>,
        /// Make the report for this small rebuilt file: one zip and a prefilled issue link, in
        /// the --out folder. Nothing is sent.
        #[arg(long, value_name = "FILE", conflicts_with = "input")]
        make_report: Option<PathBuf>,
        /// With --narrow or --same-failure: the longest isolating or bisecting may take in all.
        #[arg(long, value_name = "TIME", value_parser = dutchman::diagnose::parse_duration)]
        step_time_limit: Option<std::time::Duration>,
        /// Fresh folder to write `diagnose/report.json`, `report.txt` and `local-key.json` in
        /// (default: the current folder).
        #[arg(long, value_name = "FOLDER")]
        out: Option<PathBuf>,
        /// The longest one part may take: seconds, or a number with s, m or h.
        #[arg(long, value_name = "TIME", value_parser = dutchman::diagnose::parse_duration)]
        time_limit: Option<std::time::Duration>,
        /// The most memory, in gigabytes, the converter may hold while it works on one part
        /// (default: four fifths of what is free).
        #[arg(long, value_name = "GB")]
        memory_limit: Option<f64>,
        /// The project folder an assembly's part files are searched in when they are not at
        /// their stored paths.
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
        /// Another folder to search, with its subfolders, for part files; may be given more
        /// than once.
        #[arg(long = "search", value_name = "DIR")]
        search: Vec<PathBuf>,
        /// Print one machine-readable result on stdout instead of the summary.
        #[arg(long)]
        json: bool,
    },
    /// Convert files dropped into an inbox, polling until shutdown.
    Watch(dutchman::watch::Command),
    #[command(external_subcommand)]
    External(Vec<OsString>),
    /// Drive SolidCraft from plain-language requests with Pilot.
    #[cfg(feature = "pilot")]
    #[command(name = "pilot")]
    Pilot(dutchman_pilot::cli::Command),
    /// Convert CAD files, a folder or a project ZIP into SolidCraft designs or other output.
    Convert {
        /// A design file, folder of designs or project ZIP.
        input: PathBuf,
        /// The output file to write, matching --to; DXF also accepts an existing directory.
        /// Omit to write beside the input; folders and ZIPs get a new (SolidCraft) folder.
        /// An explicit output is replaced, unless --no-clobber; an existing default needs --force.
        #[arg(conflicts_with = "output_flag")]
        output: Option<PathBuf>,
        /// Output path (an alternative to the positional output).
        #[arg(short = 'o', long = "output", value_name = "FILE")]
        output_flag: Option<PathBuf>,
        /// Replace an existing default output beside the input.
        #[arg(long, conflicts_with = "no_clobber")]
        force: bool,
        #[command(flatten)]
        options: ConvertOptions,
        /// Select one TechDraw page by object name (requires --to dxf).
        #[arg(long, value_name = "NAME")]
        page: Option<String>,
        /// Convert only this part of the design, by the number `dutchman diagnose` gave it.
        #[arg(long, value_name = "N")]
        part: Option<usize>,
        /// With --part: also convert the parts inside it, when it is a sub-assembly.
        #[arg(long, requires = "part")]
        with_children: bool,
        /// With --part: cut the part's history after this feature (0: no history), counting
        /// the features not suppressed, in timeline order.
        #[arg(long, value_name = "N", requires = "part")]
        stop_at_feature: Option<usize>,
        /// Also write the whole text report, every loss in full, to FILE.
        #[arg(long, value_name = "FILE")]
        log: Option<PathBuf>,
        /// Also write an anonymous report to FILE: counts, causes and timings with no file
        /// names, paths or geometry, safe to share (see docs/report-anon.md).
        #[arg(long, value_name = "FILE")]
        report_anon: Option<PathBuf>,
        /// The salt for the anonymous report's file ids (32 hex digits), to get the same ids as
        /// an earlier run (default: a new one, printed when the run ends).
        #[arg(long, value_name = "HEX", requires = "report_anon")]
        report_salt: Option<String>,
        /// Write the key from the anonymous id to the file's path to FILE. Keep it private.
        #[arg(long, value_name = "FILE", requires = "report_anon")]
        report_key: Option<PathBuf>,
        /// Print one machine-readable report on stdout instead of the text report.
        #[arg(long)]
        json: bool,
    },
    /// Convert CAD files, folders and project ZIPs into designs under one output folder.
    ConvertAll {
        /// Design files, project ZIPs, and folders to search with their subfolders. Every
        /// positional argument is an input file or folder; use --out-dir for the output folder.
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Output folder, keeping each project's layout (default: a new (SolidCraft) folder
        /// beside the first input). Existing designs are replaced unless --no-clobber.
        #[arg(long, value_name = "DIR")]
        out_dir: Option<PathBuf>,
        /// How many files to convert at once (default: the machine's cores, at most 8).
        #[arg(long, value_name = "N")]
        jobs: Option<std::num::NonZeroUsize>,
        /// Seconds one file may take before it is stopped and reported as failed.
        #[arg(long, value_name = "SECONDS", default_value_t = worker::DEFAULT_TIMEOUT.as_secs())]
        timeout: u64,
        /// Megabytes of memory one file may hold before it is stopped and reported as failed
        /// (0: no limit).
        #[arg(long, value_name = "MB", default_value_t = worker::DEFAULT_MAX_MEMORY_MB)]
        max_memory: u64,
        /// Record each file's result in FILE as it finishes; files already recorded there,
        /// unchanged since, are not converted again, so a stopped run carries on.
        #[arg(long, value_name = "FILE")]
        journal: Option<PathBuf>,
        /// Also write an anonymous report to FILE: counts, causes and timings with no file
        /// names, paths or geometry, safe to share (see docs/report-anon.md).
        #[arg(long, value_name = "FILE")]
        report_anon: Option<PathBuf>,
        /// The salt for the anonymous report's file ids (32 hex digits), to get the same ids as
        /// an earlier run (default: a new one, printed when the run ends).
        #[arg(long, value_name = "HEX", requires = "report_anon")]
        report_salt: Option<String>,
        /// Write the key from each anonymous id to its file's path to FILE. Keep it private.
        #[arg(long, value_name = "FILE", requires = "report_anon")]
        report_key: Option<PathBuf>,
        #[command(flatten)]
        options: ConvertOptions,
        /// Write the whole text report, every loss of every file in full, to FILE (default:
        /// dutchman.log in the output folder).
        #[arg(long, value_name = "FILE")]
        log: Option<PathBuf>,
        /// Print one machine-readable report on stdout instead of the text report.
        #[arg(long)]
        json: bool,
    },
    /// List the native formats Dutchman knows and what converts from each.
    Formats {
        /// Print one machine-readable report on stdout instead of the text listing.
        #[arg(long)]
        json: bool,
        /// Print the listing as a Markdown table, as the README and docs/formats.md show it.
        #[arg(long, conflicts_with = "json")]
        markdown: bool,
    },
    /// Show what converting a native CAD file would carry across, without writing anything.
    Inspect {
        /// The native design file to read.
        input: PathBuf,
        /// The project folder an assembly's part files are searched in when they are not at
        /// their stored paths (default: the nearest folder above the assembly holding an
        /// Inventor .ipj, else two folders up).
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
        /// Another folder to search, with its subfolders, for part files the project folder
        /// does not hold; may be given more than once.
        #[arg(long = "search", value_name = "DIR")]
        search: Vec<PathBuf>,
        /// Print one machine-readable report on stdout instead of the text report.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    if let Some(code) = worker::run_if_worker() {
        return code;
    }
    let mut args: Vec<OsString> = std::env::args_os().collect();
    if let Some((index, arg)) = args
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, arg)| *arg != "--verbose" && *arg != "--quiet" && *arg != "--yes")
    {
        let name = arg.to_str().unwrap_or("");
        let builtin = Cli::command()
            .get_subcommands()
            .any(|cmd| cmd.get_name() == name)
            || matches!(name, "pilot" | "help");
        if name == "pilot" || !builtin && !name.starts_with('-') {
            if let Some(path) =
                dutchman_launcher::find_tool(if name == "printers" { "print" } else { name })
            {
                let mut rest: Vec<_> = args.iter().skip(index + 1).cloned().collect();
                if matches!(name, "printers" | "print") {
                    rest.insert(0, OsString::from(name));
                }
                return run_tool(&path, &rest);
            }
        }
        if !builtin && (!name.starts_with('-') || name.is_empty()) && is_input(Path::new(arg)) {
            args.insert(index, OsString::from("convert"));
        }
    }
    // Name the command `dutchman` in every usage line, also where argv[0] is `dutchman.exe`.
    let mut command = Cli::command().bin_name("dutchman");
    if args.len() == 1
        || args
            .iter()
            .any(|arg| arg == "--help" || arg == "-h" || arg == "help")
    {
        let listing = dutchman_launcher::tools_listing();
        if !listing.is_empty() {
            let examples = command
                .get_after_help()
                .map(ToString::to_string)
                .unwrap_or_default();
            command = command.after_help(format!("{examples}\n\n{listing}"));
        }
    }
    #[cfg(feature = "pilot")]
    let mut command = command.mut_subcommand("pilot", |c| {
        dutchman_pilot::cli::with_bin_name(c, "dutchman pilot")
    });
    if args.len() == 1 {
        let _ = command.print_help();
        println!();
        return ExitCode::SUCCESS;
    }
    let suggestion_command = command
        .clone()
        .external_subcommand_value_parser(None::<clap::builder::ValueParser>)
        .allow_external_subcommands(false);
    let matches = command.get_matches_from(&args);
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(err) => err.exit(),
    };
    let verbose = cli.verbose;
    let quiet = cli.quiet;
    let yes = cli.yes;
    match cli.command {
        Command::Report {
            folder,
            report_id,
            source_format,
            signature,
            send_summary,
            public_test,
            json: as_json,
        } => {
            let result = (|| {
                let prepared = if folder.join("prepared.json").exists() {
                    dutchman::report_send::Prepared::load(&folder)?
                } else {
                    dutchman::report_send::Prepared::new(
                        report_id
                            .as_deref()
                            .ok_or("A new report needs --report-id.")?,
                        source_format
                            .as_deref()
                            .ok_or("A new report needs --source-format.")?,
                        signature
                            .as_deref()
                            .ok_or("A new report needs --signature.")?,
                        None,
                        &folder,
                    )?
                };
                if send_summary {
                    dutchman::report_send::submit(
                        &prepared,
                        &folder,
                        "summary",
                        public_test,
                        yes,
                        dutchman::report_send::endpoint().as_deref(),
                    )
                } else {
                    Ok(prepared.preview())
                }
            })();
            match result {
                Ok(report) => {
                    print_json(&report);
                    exit(&report)
                }
                Err(message) => {
                    if as_json {
                        print_json(
                            &serde_json::json!({"schema": dutchman::report_send::SCHEMA, "ok": false, "summary": message}),
                        );
                    } else {
                        eprintln!("dutchman: {message}");
                    }
                    ExitCode::FAILURE
                }
            }
        }
        Command::Doctor {
            input,
            out,
            jobs,
            memory_mb,
            time_limit,
            step,
            part,
            json: as_json,
            options,
        } => {
            let out = out.unwrap_or_else(|| dutchman::doctor::default_output(&input));
            let mut settings = dutchman::doctor::Settings {
                jobs,
                memory: memory_mb.saturating_mul(1 << 20),
                diagnose: dutchman::diagnose::Settings {
                    options,
                    step_path: step,
                    target_part: part,
                    ..dutchman::diagnose::Settings::default()
                },
            };
            if let Some(time) = time_limit {
                settings.diagnose.time_limit = time;
            }
            narrow_outcome(
                dutchman::doctor::SCHEMA,
                &input,
                dutchman::doctor::run(&input, &out, &settings, &|done, total| {
                    if !quiet && !as_json {
                        eprintln!("dutchman: checked {done} of {total} designs");
                    }
                }),
                as_json,
                &|r| r.get("ok") == Some(&Value::Bool(true)),
            )
        }
        Command::Diagnose {
            step,
            part,
            input,
            narrow,
            bundle,
            same_failure,
            make_report,
            step_time_limit,
            out,
            time_limit,
            memory_limit,
            project,
            search,
            json,
        } => {
            let job = match (input, same_failure, make_report) {
                (Some(input), _, _) if bundle => DiagnoseJob::Bundle(input),
                (Some(input), _, _) if narrow => DiagnoseJob::Narrow(input),
                (Some(input), _, _) => DiagnoseJob::Rank(input),
                (_, Some(small), _) => DiagnoseJob::SameFailure(small),
                (_, _, Some(small)) => DiagnoseJob::MakeReport(small),
                // clap requires one of them.
                (None, None, None) => return ExitCode::from(2),
            };
            diagnose(
                &job,
                step,
                part,
                out.as_deref().unwrap_or(Path::new(".")),
                time_limit,
                memory_limit,
                step_time_limit,
                ConvertOptions {
                    project,
                    search,
                    ..ConvertOptions::default()
                },
                json,
                quiet,
            )
        }
        Command::Repro {
            input,
            out,
            from_diagnose,
            json,
        } => match dutchman::repro::run_from(&input, &out, false, from_diagnose.as_deref()) {
            Ok(report) => {
                if json {
                    println!("{report}");
                } else {
                    println!(
                        "{}",
                        report
                            .get("summary")
                            .and_then(Value::as_str)
                            .unwrap_or("Reproduction kit written.")
                    );
                }
                ExitCode::SUCCESS
            }
            Err(error) => {
                if json {
                    println!(
                        "{}",
                        json::failure(dutchman::repro::SCHEMA, json::input(&input), &error)
                    );
                } else {
                    eprintln!("dutchman: {error}");
                }
                ExitCode::FAILURE
            }
        },
        Command::Watch(command) => match dutchman::watch::run_with_progress(command, quiet) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(err) => {
                eprintln!("dutchman: {err}");
                ExitCode::FAILURE
            }
        },
        Command::External(args) => {
            let mut args = args.into_iter();
            let name = args.next().unwrap_or_default();
            let name = name.to_str().unwrap_or("");
            if let Some(path) =
                dutchman_launcher::find_tool(if name == "printers" { "print" } else { name })
            {
                let mut rest: Vec<_> = args.collect();
                if matches!(name, "printers" | "print") {
                    rest.insert(0, OsString::from(name));
                }
                return run_tool(&path, &rest);
            }
            // A known Dutchman CAD tool that isn't installed says so, rather than suggesting a
            // built-in command with a similar name (`pilot` is close to `report`).
            let known_tool = matches!(name, "pilot" | "fixtures" | "print" | "printers");
            if known_tool {
                eprintln!("{}", dutchman_launcher::not_installed_message(name));
                return ExitCode::FAILURE;
            }
            if let Err(error) = suggestion_command.try_get_matches_from(std::env::args_os()) {
                if error
                    .get(clap::error::ContextKind::SuggestedSubcommand)
                    .is_some()
                {
                    error.exit();
                }
            }
            eprintln!("{}", dutchman_launcher::not_installed_message(name));
            ExitCode::FAILURE
        }
        #[cfg(feature = "pilot")]
        Command::Pilot(cmd) => match dutchman_pilot::cli::run_named(cmd, "dutchman pilot") {
            Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            Err(err) => {
                eprintln!("dutchman: {err}");
                ExitCode::FAILURE
            }
        },
        Command::Convert {
            input,
            output,
            output_flag,
            force,
            mut options,
            page,
            part,
            with_children,
            stop_at_feature,
            log,
            report_anon,
            report_salt,
            report_key,
            json,
        } => {
            let explicit = output.or(output_flag);
            options.part = part.map(|number| dutchman_core::PartSelection {
                number,
                children: with_children,
                stop_at_feature,
            });
            if part.is_some() && options.to == dutchman::OutputFormat::Dxf {
                eprintln!("dutchman: --part does not apply to --to dxf");
                return ExitCode::from(2);
            }
            if page.is_some() && options.to != dutchman::OutputFormat::Dxf {
                eprintln!("dutchman: --page requires --to dxf");
                return ExitCode::from(2);
            }
            let mut anon = match anon_request(report_anon, report_salt, report_key, &mut options) {
                Ok(anon) => anon,
                Err(code) => return code,
            };
            if input.is_dir() || batch::extension_of(&input) == "zip" {
                let out_dir = explicit.unwrap_or_else(|| batch::default_output(&input));
                let settings = match batch::Settings::isolated() {
                    Ok(settings) => settings,
                    Err(err) => {
                        eprintln!("dutchman: {err}");
                        return ExitCode::FAILURE;
                    }
                };
                if !json && !quiet {
                    println!(
                        "{}",
                        batch::plan(std::slice::from_ref(&input), &out_dir).description
                    );
                }
                let report = match batch::convert_all(
                    std::slice::from_ref(&input),
                    &out_dir,
                    &options,
                    &settings,
                ) {
                    Ok(report) => report,
                    Err(err) => {
                        eprintln!("dutchman: {err}");
                        return ExitCode::FAILURE;
                    }
                };
                if json {
                    print_json(&report);
                } else {
                    print!("{}", batch::text(&report, verbose));
                }
                let log = log.unwrap_or_else(|| out_dir.join(batch::LOG_NAME));
                if let Err(err) = std::fs::write(&log, batch::full_log(&report)) {
                    eprintln!("dutchman: cannot write {}: {err}", log.display());
                }
                if let Some(request) = anon.take() {
                    let written = anon::write(
                        &request,
                        &report,
                        std::slice::from_ref(&input),
                        &out_dir,
                        &options,
                        &settings,
                    );
                    if let Err(code) = anon_notice(&request, written) {
                        return code;
                    }
                }
                return exit(&report);
            }
            let derived = explicit.is_none();
            if options.to == dutchman::OutputFormat::Dxf {
                let output = explicit.unwrap_or_else(|| {
                    input
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new("."))
                        .to_path_buf()
                });
                let report = dutchman::dxf::convert_with_diagnostics(
                    &input,
                    &output,
                    page.as_deref(),
                    !options.no_clobber && (!derived || force),
                    &options,
                );
                if let Some(log) = log {
                    if let Err(err) =
                        std::fs::write(&log, json::full_log(&report, &batch::line(&report)))
                    {
                        eprintln!("dutchman: cannot write {}: {err}", log.display());
                    }
                }
                if let Some(request) = anon.take() {
                    let written = anon::write_one(&request, &report, &input, &output, &options);
                    if let Err(code) = anon_notice(&request, written) {
                        return code;
                    }
                }
                if json {
                    print_json(&report);
                } else if let Some(message) =
                    report.pointer("/error/message").and_then(Value::as_str)
                {
                    eprintln!("dutchman: {message}");
                } else {
                    if let Some(pages) = report.get("pages").and_then(Value::as_array) {
                        for page in pages {
                            println!(
                                "Converted {} → {}",
                                input.display(),
                                page.get("path").and_then(Value::as_str).unwrap_or_default()
                            );
                        }
                    }
                    if let Some(losses) = report.get("losses").and_then(Value::as_array) {
                        print_losses(losses, verbose);
                    }
                }
                return exit(&report);
            }
            let output = explicit.unwrap_or_else(|| {
                dutchman_core::Format::part_revision(&input)
                    .map_or_else(|| input.clone(), |(part, _)| part)
                    .with_extension(options.output_extension())
            });
            if derived {
                if input == output
                    || std::fs::canonicalize(&input)
                        .ok()
                        .zip(std::fs::canonicalize(&output).ok())
                        .is_some_and(|(input, output)| input == output)
                {
                    eprintln!("dutchman: default output {} is the input file; choose another output with -o PATH", output.display());
                    return ExitCode::from(2);
                }
                if output.try_exists().unwrap_or(true) && !force {
                    eprintln!("dutchman: {} already exists; choose another output with -o PATH, or replace it with --force", output.display());
                    return ExitCode::from(2);
                }
                options.no_clobber |= !force;
            }
            if !json {
                if let Some(status) =
                    json::status_line(&serde_json::json!({"input": json::input(&input)}))
                {
                    println!("{status}");
                }
            }
            if options.part.is_none()
                && !memory_cli(&input, &mut options, json, quiet, yes, json::CONVERT)
            {
                return ExitCode::from(2);
            }
            convert(
                &input,
                &output,
                &options,
                log.as_deref(),
                anon.as_ref(),
                Output {
                    json,
                    verbose,
                    quiet,
                },
            )
        }
        Command::ConvertAll {
            inputs,
            out_dir,
            jobs,
            timeout,
            max_memory,
            journal,
            report_anon,
            report_salt,
            report_key,
            mut options,
            log,
            json,
        } => {
            let out_dir = out_dir.unwrap_or_else(|| {
                inputs
                    .first()
                    .map(|input| batch::default_output(input))
                    .unwrap_or_else(|| PathBuf::from("SolidCraft"))
            });
            warn_missing_inputs(&inputs);
            let preview = batch::plan(&inputs, &out_dir);
            if !json && !quiet {
                println!("{}", preview.description);
            }
            for job in preview.jobs {
                if !memory_cli(&job.input, &mut options, json, quiet, yes, batch::SCHEMA) {
                    return ExitCode::from(2);
                }
            }
            let anon = match anon_request(report_anon, report_salt, report_key, &mut options) {
                Ok(anon) => anon,
                Err(code) => return code,
            };
            let settings = match batch::Settings::isolated() {
                Ok(settings) => batch::Settings {
                    jobs,
                    journal,
                    limits: worker::Limits {
                        timeout: std::time::Duration::from_secs(timeout.max(1)),
                        max_memory: (max_memory > 0).then_some(max_memory.saturating_mul(1 << 20)),
                    },
                    ..settings
                },
                Err(err) => {
                    eprintln!("dutchman: {err}");
                    return ExitCode::FAILURE;
                }
            };
            let progress = dutchman::progress::StderrProgress::new(quiet, json, verbose);
            let report = match batch::convert_all_with_progress(
                &inputs,
                &out_dir,
                &options,
                &settings,
                &|input, event| progress.report(input, event),
            ) {
                Ok(report) => report,
                Err(err) => {
                    progress.finish();
                    eprintln!("dutchman: {err}");
                    return ExitCode::FAILURE;
                }
            };
            progress.finish();
            if let Some(message) = report.pointer("/error/message").and_then(Value::as_str) {
                if json {
                    print_json(&report);
                } else {
                    eprintln!("dutchman: {message}");
                }
                return exit(&report);
            }
            if json {
                print_json(&report);
            } else {
                print!("{}", batch::text(&report, verbose));
                if !quiet {
                    if let Some(pitch) = json::fixtures_pitch(
                        report
                            .get("results")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten(),
                    ) {
                        println!("{pitch}");
                    }
                }
            }
            let log = log.unwrap_or_else(|| out_dir.join(batch::LOG_NAME));
            let written = log
                .parent()
                .filter(|folder| !folder.as_os_str().is_empty())
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&log, batch::full_log(&report)));
            match written {
                Ok(()) if !json => eprintln!("dutchman: every loss, in full: {}", log.display()),
                Ok(()) => {}
                Err(err) => eprintln!("dutchman: cannot write {}: {err}", log.display()),
            }
            if let Some(request) = anon {
                let written =
                    anon::write(&request, &report, &inputs, &out_dir, &options, &settings);
                if let Err(code) = anon_notice(&request, written) {
                    return code;
                }
            }
            exit(&report)
        }
        Command::Inspect {
            input,
            project,
            search,
            json,
        } => inspect(
            &input,
            &dutchman::PartSearch {
                threads: 0,
                part_positions: dutchman_core::PartPositions::Smart,
                project,
                also: search,
            },
            json,
            verbose,
            quiet,
        ),
        Command::Formats { json, markdown } => {
            if json {
                print_json(&formats::json());
            } else if markdown {
                print!("{}", formats::markdown());
            } else {
                print!("{}", formats::text());
            }
            ExitCode::SUCCESS
        }
    }
}

/// Recognise files, folders and known CAD suffixes for the conversion shorthand.
fn is_input(path: &Path) -> bool {
    path.is_file() || path.is_dir() || Format::from_path(path).is_some()
}

fn run_tool(path: &Path, args: &[OsString]) -> ExitCode {
    match dutchman_launcher::exec(path, args) {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(err) => {
            eprintln!("dutchman: cannot run {}: {err}", path.display());
            ExitCode::FAILURE
        }
    }
}

/// What `dutchman diagnose` was asked to do.
enum DiagnoseJob {
    Rank(PathBuf),
    Bundle(PathBuf),
    Narrow(PathBuf),
    SameFailure(PathBuf),
    MakeReport(PathBuf),
}

/// The settings every diagnose job shares, or the message for a bad `--memory-limit`.
fn diagnose_settings(
    time_limit: Option<std::time::Duration>,
    memory_limit: Option<f64>,
    options: ConvertOptions,
) -> Result<dutchman::diagnose::Settings, &'static str> {
    let mut settings = dutchman::diagnose::Settings {
        options,
        ..dutchman::diagnose::Settings::default()
    };
    if let Some(limit) = time_limit {
        settings.time_limit = limit;
    }
    if let Some(gb) = memory_limit {
        if !gb.is_finite() || gb <= 0.0 {
            return Err("--memory-limit must be above zero (gigabytes)");
        }
        settings.memory_limit = Some((gb * 1e9) as u64);
    }
    Ok(settings)
}

/// Prints what a `--narrow`, `--same-failure` or `--make-report` job came to.
fn narrow_outcome(
    schema: &str,
    file: &Path,
    result: Result<Value, String>,
    as_json: bool,
    good: &dyn Fn(&Value) -> bool,
) -> ExitCode {
    match result {
        Ok(result) => {
            if as_json {
                print_json(&result);
            } else {
                println!(
                    "{}",
                    result
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                );
                if let Some(link) = result.get("issue_url").and_then(Value::as_str) {
                    let line = |key: &str, field: &str| {
                        result
                            .pointer(&format!("/{key}/{field}"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned()
                    };
                    println!(
                        "\nThe zip: {}\nOpen a prefilled issue: {link}\n{} {}",
                        line("paths", "zip"),
                        line("private_send", "text"),
                        line("private_send", "url"),
                    );
                } else if let Some(steps) =
                    result.pointer("/paths/next_steps").and_then(Value::as_str)
                {
                    println!("\nWhat to do next: {steps}");
                }
            }
            if good(&result) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            if as_json {
                print_json(&json::failure(schema, json::input(file), &error));
            } else {
                eprintln!("dutchman: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn diagnose(
    job: &DiagnoseJob,
    step_path: Option<PathBuf>,
    target_part: Option<usize>,
    out: &Path,
    time_limit: Option<std::time::Duration>,
    memory_limit: Option<f64>,
    step_time_limit: Option<std::time::Duration>,
    options: ConvertOptions,
    as_json: bool,
    quiet: bool,
) -> ExitCode {
    use dutchman::diagnose::{self, narrow, Event};
    let settings = match diagnose_settings(time_limit, memory_limit, options) {
        Ok(mut settings) => {
            settings.step_path = step_path;
            settings.target_part = target_part;
            settings
        }
        Err(message) => {
            eprintln!("dutchman: {message}");
            return ExitCode::from(2);
        }
    };
    let on_progress = move |progress: narrow::Progress| {
        if !as_json && !quiet {
            eprintln!("dutchman: {progress}");
        }
    };
    let steps = narrow::NarrowSettings::new(settings.clone(), step_time_limit);
    let input = match job {
        DiagnoseJob::Rank(input) => input,
        DiagnoseJob::Bundle(input) => {
            return narrow_outcome(
                diagnose::bundle::SCHEMA,
                input,
                diagnose::bundle::extract(input, out, &settings, None, None, &|| false),
                as_json,
                &|_| true,
            );
        }
        DiagnoseJob::Narrow(input) => {
            return narrow_outcome(
                json::NARROW,
                input,
                narrow::run(input, out, &steps, &on_progress),
                as_json,
                &|result| {
                    !matches!(
                        result.get("status").and_then(Value::as_str),
                        Some("stopped" | "not_repeated")
                    )
                },
            );
        }
        DiagnoseJob::SameFailure(small) => {
            return narrow_outcome(
                json::SAME_FAILURE,
                small,
                narrow::same_failure(small, out, &steps, &on_progress),
                as_json,
                &|result| result.get("same") == Some(&Value::Bool(true)),
            );
        }
        DiagnoseJob::MakeReport(small) => {
            return narrow_outcome(
                json::MADE_REPORT,
                small,
                narrow::make_report(small, out),
                as_json,
                &|_| true,
            );
        }
    };
    let show = !as_json && !quiet;
    let on_event = move |event: Event| {
        if !show {
            return;
        }
        match event {
            Event::Reading => eprintln!("dutchman: reading the design..."),
            Event::Read { parts, ms } => eprintln!(
                "dutchman: read {parts} part{} in {}",
                if parts == 1 { "" } else { "s" },
                dutchman_core::elapsed(ms / 1000)
            ),
            Event::ReadFailed { .. } => {
                eprintln!("dutchman: the design could not be read")
            }
            Event::Part {
                done,
                total,
                number,
                outcome,
                ms,
            } => {
                if outcome != "ok" && outcome != "container" {
                    eprintln!(
                        "dutchman: part {number}: {} ({} ms)",
                        outcome.replace('_', " "),
                        ms
                    );
                } else if done % 25 == 0 {
                    eprintln!("dutchman: {done} of {total} parts done");
                }
            }
            Event::Restarted => eprintln!("dutchman: reading the design again for the next part"),
        }
    };
    match diagnose::run(input, out, &settings, &on_event) {
        Ok(result) => {
            if as_json {
                print_json(&result);
            } else {
                let path = |key: &str| {
                    result
                        .pointer(&format!("/paths/{key}"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                println!(
                    "{}\n\nSafe to share: {} and {}\nPrivate, do not share: {}",
                    result
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    path("report"),
                    path("text"),
                    path("local_key"),
                );
            }
            if result.get("status") == Some(&Value::from("complete")) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            if as_json {
                print_json(&json::failure(diagnose::SCHEMA, json::input(input), &error));
            } else {
                eprintln!("dutchman: {error}");
            }
            ExitCode::FAILURE
        }
    }
}

fn memory_cli(
    input: &Path,
    options: &mut ConvertOptions,
    as_json: bool,
    quiet: bool,
    yes: bool,
    schema: &str,
) -> bool {
    use dutchman::choices::{self, MemoryCheck};
    loop {
        let check = choices::memory_check(input, options);
        match check {
            MemoryCheck::Ok(estimate) => {
                if !as_json && !quiet {
                    if let Some(estimate) = estimate {
                        println!("Estimate: {}", choices::estimate_text_with_bands(estimate));
                    }
                }
                return true;
            }
            MemoryCheck::TooBig {
                conversion,
                lighter,
                ..
            } => {
                let message = check.refusal().unwrap_or_default();
                if !as_json && !yes && std::io::stdin().is_terminal() {
                    if let Some(lighter) = lighter {
                        if !quiet {
                            println!(
                                "Estimate: {}",
                                choices::estimate_text_with_bands(conversion)
                            );
                        }
                        eprint!("{message}\nSwitch to --preset {}? [y/N] ", lighter.name());
                        let _ = std::io::stderr().flush();
                        let mut answer = String::new();
                        if std::io::stdin().read_line(&mut answer).is_ok()
                            && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
                        {
                            options.preset = lighter;
                            continue;
                        }
                    }
                }
                if as_json {
                    print_json(&json::memory_failure(schema, input, options, &message));
                } else {
                    eprintln!("dutchman: {message}");
                }
                return false;
            }
        }
    }
}

/// Reads `--report-anon`, `--report-salt` and `--report-key` into a request, choosing the salt
/// (a new one unless given) and handing it to `options` so diagnostics use the same ids.
fn anon_request(
    report_anon: Option<PathBuf>,
    report_salt: Option<String>,
    report_key: Option<PathBuf>,
    options: &mut ConvertOptions,
) -> Result<Option<anon::Request>, ExitCode> {
    let salt = match report_salt.as_deref().map(anon::Salt::parse).transpose() {
        Ok(salt) => salt,
        Err(err) => {
            eprintln!("dutchman: --report-salt: {err}");
            return Err(ExitCode::from(2));
        }
    };
    let Some(path) = report_anon else {
        return Ok(None);
    };
    let salt = match salt.map_or_else(anon::Salt::random, Ok) {
        Ok(salt) => salt,
        Err(err) => {
            eprintln!("dutchman: {err}");
            return Err(ExitCode::from(2));
        }
    };
    options.diagnostics_salt = Some(salt.clone());
    Ok(Some(anon::Request {
        path,
        salt: Some(salt),
        key: report_key,
    }))
}

/// Says where the anonymous report went and which salt its ids use, or why it was not written.
fn anon_notice(
    request: &anon::Request,
    written: Result<anon::Salt, String>,
) -> Result<(), ExitCode> {
    match written {
        Ok(salt) => {
            eprintln!(
                "dutchman: anonymous report written to {}. Its file ids use the salt {}: \
                 keep the salt to yourself, and give --report-salt {} to get the same ids \
                 next time.",
                request.path.display(),
                salt.hex(),
                salt.hex()
            );
            Ok(())
        }
        Err(err) => {
            eprintln!("dutchman: cannot write the anonymous report: {err}");
            Err(ExitCode::FAILURE)
        }
    }
}

/// Output extensions a mistaken positional argument of convert-all is likely to carry.
const OUTPUT_EXTENSIONS: [&str; 6] = ["sc", "solidcraft", "step", "stp", "fcstd", "fcmacro"];

/// Warns, before anything is converted, about a positional argument of convert-all that does
/// not exist and looks like an output path: every positional argument is an input.
fn warn_missing_inputs(inputs: &[PathBuf]) {
    for input in inputs {
        if input.try_exists().unwrap_or(true) {
            continue;
        }
        let extension = batch::extension_of(input);
        let looks_like_output = extension.is_empty()
            || OUTPUT_EXTENSIONS.contains(&extension.as_str())
            || (input
                .parent()
                .is_some_and(|parent| parent.as_os_str().is_empty() || parent.is_dir())
                && dutchman_core::Format::from_path(input).is_none());
        if looks_like_output {
            eprintln!(
                "dutchman: {} does not exist; every positional argument is an input \u{2014} to choose the output folder use --out-dir",
                input.display()
            );
        }
    }
}

/// How a command prints: JSON or text, and how much of it.
#[derive(Clone, Copy)]
struct Output {
    json: bool,
    verbose: bool,
    quiet: bool,
}

fn convert(
    input: &Path,
    output: &Path,
    options: &ConvertOptions,
    log: Option<&Path>,
    anon: Option<&anon::Request>,
    printing: Output,
) -> ExitCode {
    let Output {
        json: as_json,
        verbose,
        quiet,
    } = printing;
    let progress = dutchman::progress::StderrProgress::new(quiet, as_json, verbose);
    let report = dutchman::convert_options_with_progress(
        input,
        output,
        options,
        !options.no_clobber,
        &options.part_search(),
        &|event| progress.report(input, event),
    );
    progress.finish();
    if let Some(log) = log {
        let text = json::full_log(&report, &batch::line(&report));
        if let Err(err) = std::fs::write(log, text) {
            eprintln!("dutchman: cannot write {}: {err}", log.display());
        }
    }
    if let Some(request) = anon {
        let written = anon::write_one(request, &report, input, output, options);
        if let Err(code) = anon_notice(request, written) {
            return code;
        }
    }
    if !as_json {
        if let Some(line) = json::solidcraft_line(&report) {
            println!("{line}");
            if let Some(notice) = json::solidcraft_notice(&report) {
                if report.pointer("/solidcraft/status").and_then(Value::as_str) == Some("too_old") {
                    println!("{notice} {}", json::SOLIDCRAFT_DOWNLOAD);
                } else {
                    println!("{notice}");
                }
            }
        }
    }
    if as_json {
        print_json(&report);
    } else if let Some(message) = report.pointer("/error/message").and_then(Value::as_str) {
        eprintln!("dutchman: {message}");
    } else {
        // What was written first, on stdout; then what it could not carry across.
        println!("{}", batch::line(&report));
        for line in json::completeness_lines(&report) {
            println!("{line}");
        }
        let losses = report
            .get("losses")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        if verbose || losses.len() <= json::LISTED {
            print_losses(losses, verbose);
        } else {
            for line in json::grouped_lines(&report) {
                eprintln!("dutchman: {line}");
            }
            match log {
                Some(log) => eprintln!("dutchman: every loss, in full: {}", log.display()),
                None => eprintln!(
                    "dutchman: (--verbose prints every loss; --log FILE writes them to FILE)"
                ),
            }
        }
    }
    if !as_json && !quiet {
        if let Some(pitch) = json::fixtures_pitch([&report]) {
            println!("{pitch}");
        }
    }
    exit(&report)
}

fn inspect(
    input: &Path,
    search: &dutchman::PartSearch,
    as_json: bool,
    verbose: bool,
    quiet: bool,
) -> ExitCode {
    let progress = dutchman::progress::StderrProgress::new(quiet, as_json, verbose);
    let result =
        dutchman::inspect_with_progress(input, search, &|event| progress.report(input, event));
    progress.finish();
    let inspection = match result {
        Ok(inspection) => inspection,
        Err(err) => {
            if as_json {
                print_json(&json::error(json::INSPECT, json::input(input), &err));
            } else {
                eprintln!("dutchman: {err}");
            }
            return ExitCode::FAILURE;
        }
    };
    if as_json {
        print_json(&inspection.json(input));
        return ExitCode::SUCCESS;
    }
    let format = Format::from_path(input).map_or("design", Format::name);
    let count =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match inspection.assembly {
        Some((parts, placements)) => println!(
            "{format}: {} placed, from {}",
            count(placements, "part", "parts"),
            count(parts, "part file", "part files")
        ),
        None => println!(
            "{format}: {}, {}",
            count(inspection.bodies, "body", "bodies"),
            count(inspection.faces, "face", "faces")
        ),
    }
    if let Some(outcome) = inspection.report.outcome() {
        match (&inspection.conversion, &outcome.mode, &outcome.check) {
            (Err(reason), _, _) => println!("would write: nothing; converting it fails: {reason}"),
            (Ok(()), Mode::PartialHistory, _) => {
                println!("would write: partial editable history and the imported final shape")
            }
            (Ok(()), Mode::History, _) => println!("would write: editable history"),
            (Ok(()), Mode::Import, Check::Flattened(reason) | Check::NotApplicable(reason)) => {
                println!("would write: geometry without history ({reason})");
            }
            (Ok(()), Mode::Import, _) => println!("would write: geometry without history"),
        }
        for feature in &outcome.features {
            let status = match feature.status {
                FeatureStatus::Mapped => "mapped",
                FeatureStatus::NotNeeded => "not needed",
                FeatureStatus::LeftOut => "left out",
                FeatureStatus::Blocked => "blocked",
            };
            let name = feature.name.as_deref().unwrap_or("(unnamed)");
            match &feature.reason {
                Some(reason) => println!("  {status:<10} {} \"{name}\": {reason}", feature.kind),
                None => println!("  {status:<10} {} \"{name}\"", feature.kind),
            }
        }
    }
    if let Some(completeness) = inspection.report.completeness() {
        let lines = json::completeness_lines(&serde_json::json!({
            "completeness": json::completeness_json(completeness),
        }));
        for line in lines {
            println!("would place: {}", line.trim_start_matches("assembly: "));
        }
    }
    let losses = json::losses(&inspection.report);
    print_losses(losses.as_array().map_or(&[][..], Vec::as_slice), verbose);
    ExitCode::SUCCESS
}

/// Success when the report says the run succeeded.
fn exit(report: &Value) -> ExitCode {
    if report.get("ok") == Some(&Value::Bool(true)) {
        ExitCode::SUCCESS
    } else if memory_refusal(report)
        || report
            .get("results")
            .and_then(Value::as_array)
            .is_some_and(|results| results.iter().any(memory_refusal))
    {
        ExitCode::from(2)
    } else {
        ExitCode::FAILURE
    }
}

fn memory_refusal(report: &Value) -> bool {
    report
        .pointer("/error/message")
        .and_then(Value::as_str)
        .is_some_and(|message| message.starts_with("this conversion needs about "))
}

fn print_json(value: &Value) {
    println!("{value}");
}

/// Losses as the text reports print them, one line each on stderr, then once a note on how
/// to see the ones cut short in full.
fn print_losses(losses: &[Value], verbose: bool) {
    let mut cut = false;
    for (line, shortened) in json::loss_lines(losses, verbose) {
        cut |= shortened;
        eprintln!("dutchman: {line}");
    }
    if cut {
        eprintln!("dutchman: ({})", json::CUT_NOTE);
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;

    #[test]
    fn report_summary_requires_explicit_consent_and_accepts_converted_signature() {
        let args = [
            "dutchman",
            "report",
            "--folder",
            "report",
            "--report-id",
            "f0123456789abcdef",
            "--source-format",
            "f3d",
            "--signature",
            "converted-with-problem",
        ];
        assert!(matches!(
            Cli::try_parse_from(args).unwrap().command,
            Command::Report {
                send_summary: false,
                ..
            }
        ));
        assert!(Cli::try_parse_from(args.into_iter().chain(["--send-summary"])).is_err());
        assert!(matches!(
            Cli::try_parse_from(args.into_iter().chain(["--send-summary", "--yes"]))
                .unwrap()
                .command,
            Command::Report {
                send_summary: true,
                ..
            }
        ));
    }

    #[test]
    fn memory_refusals_exit_two_in_single_and_batch_reports() {
        let refusal = serde_json::json!({"ok": false, "error": {"message": "this conversion needs about 2.0 GB and 1.0 GB are free; try --preset fast (or --no-memory-check to run anyway)"}});
        assert_eq!(exit(&refusal), ExitCode::from(2));
        assert_eq!(
            exit(&serde_json::json!({"ok": false, "results": [refusal]})),
            ExitCode::from(2)
        );
        assert_eq!(exit(&serde_json::json!({"ok": false})), ExitCode::FAILURE);
    }

    #[test]
    fn solidcraft_target_versions_parse_and_reach_workers() {
        for command in ["convert", "convert-all", "watch"] {
            let mut args = vec!["dutchman", command, "in.f3d"];
            if command != "convert" {
                args.extend(["--out-dir", "out"]);
            }
            for version in ["0.18.2", "0.25.1", "0.30.0"] {
                let mut valid = args.clone();
                valid.extend(["--solidcraft-version", version, "--no-verify"]);
                let cli = Cli::try_parse_from(valid).unwrap();
                let options = match cli.command {
                    Command::Convert { options, .. } | Command::ConvertAll { options, .. } => {
                        options
                    }
                    Command::Watch(command) => command.options,
                    _ => panic!("expected conversion command"),
                };
                assert_eq!(options.solidcraft_version.unwrap().to_string(), version);
                let mut worker = vec![
                    std::ffi::OsString::from("dutchman"),
                    "convert".into(),
                    "in.f3d".into(),
                ];
                worker.extend(options.to_args());
                let Command::Convert {
                    options: forwarded, ..
                } = Cli::try_parse_from(worker).unwrap().command
                else {
                    panic!("expected worker")
                };
                assert_eq!(forwarded.solidcraft_version, options.solidcraft_version);
            }
            for version in [
                "0.18.1",
                "0.17.9",
                "0.25",
                "0.25.1.0",
                "v0.25.1",
                "0.25.1-rc1",
                "a.b.c",
                "0.25.-1",
            ] {
                let mut malformed = args.clone();
                malformed.extend(["--solidcraft-version", version]);
                assert!(
                    Cli::try_parse_from(malformed).is_err(),
                    "{command}: {version}"
                );
            }
        }
    }

    #[test]
    fn freecad_output_accepts_short_output_flag_and_positional_output() {
        for args in [
            vec![
                "dutchman",
                "convert",
                "in.f3d",
                "--to",
                "freecad",
                "-o",
                "out.FCMacro",
            ],
            vec![
                "dutchman",
                "convert",
                "in.f3d",
                "out.FCMacro",
                "--to",
                "freecad",
            ],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();
            let Command::Convert {
                options,
                output,
                output_flag,
                ..
            } = cli.command
            else {
                panic!("expected convert");
            };
            assert_eq!(options.to, dutchman::OutputFormat::Freecad);
            assert_eq!(output.or(output_flag), Some(PathBuf::from("out.FCMacro")));
        }
    }

    #[test]
    fn output_path_is_optional_and_cannot_be_given_twice() {
        assert!(Cli::try_parse_from(["dutchman", "convert", "in.f3d", "--to", "freecad"]).is_ok());
        assert!(Cli::try_parse_from([
            "dutchman",
            "convert",
            "in.f3d",
            "a.FCMacro",
            "-o",
            "b.FCMacro",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "dutchman",
            "convert",
            "in.f3d",
            "-o",
            "a.FCMacro",
            "--to",
            "unknown",
        ])
        .is_err());
    }

    #[test]
    fn freecad_native_and_verified_macro_flags_reach_batch_workers() {
        for (format, extension, verify) in [("fcstd", "FCStd", false), ("freecad", "FCMacro", true)]
        {
            let mut args = vec![
                "dutchman",
                "convert",
                "in.f3d",
                "-o",
                "out",
                "--to",
                format,
                "--freecad-timeout",
                "17",
            ];
            if verify {
                args.push("--verify");
            }
            let cli = Cli::try_parse_from(args).unwrap();
            let Command::Convert { options, .. } = cli.command else {
                panic!("expected convert");
            };
            assert_eq!(options.output_extension(), extension);
            assert_eq!(options.verify, verify);
            assert_eq!(options.freecad_timeout, 17);
            let mut worker_args = vec![
                std::ffi::OsString::from("dutchman"),
                "convert".into(),
                "in.f3d".into(),
                "out".into(),
            ];
            worker_args.extend(options.to_args());
            let worker = Cli::try_parse_from(worker_args).unwrap();
            let Command::Convert {
                options: forwarded, ..
            } = worker.command
            else {
                panic!("expected convert");
            };
            assert_eq!(forwarded.to, options.to);
            assert_eq!(forwarded.verify, options.verify);
            assert_eq!(forwarded.freecad_timeout, options.freecad_timeout);
        }
    }
}
