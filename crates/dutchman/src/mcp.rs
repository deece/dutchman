//! A Model Context Protocol server: Dutchman's commands as tools an AI agent can call.
//!
//! It speaks JSON-RPC 2.0 over stdio, one message per line, as the MCP stdio transport
//! defines. The tools are `convert`, `inspect`, `formats` and `convert_all`; each returns the
//! JSON report the command's `--json` prints (`docs/json.md`), so an agent and a script see
//! the same documents. A conversion that fails is a tool error carrying its failure report.

use std::io::{self, BufRead, Write};
use std::num::NonZeroUsize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::{ConvertOptions, PartSearch};

/// The protocol versions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// The longest message read, in bytes; a longer line is answered with an error and skipped.
pub const MAX_MESSAGE: usize = 1 << 20;

// JSON-RPC error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

/// Answers every message on `input` on `output` until `input` ends.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let read =
            io::Read::take(&mut input, MAX_MESSAGE as u64 + 1).read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(());
        }
        let reply = if line.len() > MAX_MESSAGE && line.last() != Some(&b'\n') {
            skip_line(&mut input)?;
            Some(error(
                Value::Null,
                INVALID_REQUEST,
                &format!("message over {MAX_MESSAGE} bytes"),
            ))
        } else {
            let text = String::from_utf8_lossy(&line);
            if text.trim().is_empty() {
                continue;
            }
            handle(&text)
        };
        if let Some(reply) = reply {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
}

/// Reads and drops the rest of an over-long line.
fn skip_line(input: &mut impl BufRead) -> io::Result<()> {
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            return Ok(());
        }
        match buffer.iter().position(|b| *b == b'\n') {
            Some(end) => {
                input.consume(end + 1);
                return Ok(());
            }
            None => {
                let len = buffer.len();
                input.consume(len);
            }
        }
    }
}

/// The reply to one message, or `None` for a notification.
pub fn handle(message: &str) -> Option<Value> {
    let request: Value = match serde_json::from_str(message) {
        Ok(request) => request,
        Err(err) => return Some(error(Value::Null, PARSE_ERROR, &err.to_string())),
    };
    let Some(fields) = request.as_object() else {
        return Some(error(
            Value::Null,
            INVALID_REQUEST,
            "a message must be one JSON object",
        ));
    };
    let Some(method) = fields.get("method").and_then(Value::as_str) else {
        // A response from the client: this server sends no requests, so there is nothing to
        // match it with.
        return None;
    };
    let id = fields.get("id").cloned()?;
    let params = fields.get("params").cloned().unwrap_or(Value::Null);
    // Dutchman guards every call into cadmpeg; this guards the server's own code as well.
    let outcome = catch_unwind(AssertUnwindSafe(|| answer(method, &params)))
        .unwrap_or_else(|_| Err((INTERNAL_ERROR, "the server failed unexpectedly".to_owned())));
    Some(match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => error(id, code, &message),
    })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn answer(method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = PROTOCOL_VERSIONS
                .iter()
                .find(|v| Some(**v) == asked)
                .unwrap_or(&PROTOCOL_VERSIONS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "dutchman", "version": env!("CARGO_PKG_VERSION")},
                "instructions": format!(
                    "Converts native CAD files ({}) into SolidCraft designs (.sc). Use \
                     read_report to diagnose an existing desktop or watch result without decoding; \
                     its full report contains private local paths and must stay local. Use \
                     inspect to see what a file would carry across, convert for one file, \
                     convert_all for many. Paths are on the machine running this server; give \
                     absolute paths. Use diagnose_narrow for anonymous local failure narrowing and rebuild recipes; no diagnostic tool sends files.",
                    dutchman_core::Format::supported_names()
                ),
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or((INVALID_PARAMS, "tools/call needs a tool name".to_owned()))?;
            let empty = Map::new();
            let arguments = match params.get("arguments") {
                None | Some(Value::Null) => &empty,
                Some(Value::Object(arguments)) => arguments,
                Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".to_owned())),
            };
            let (report, schema) = match name {
                "convert" => (convert(arguments), crate::json::CONVERT),
                "inspect" => (inspect(arguments), crate::json::INSPECT),
                "read_report" => (read_report(arguments), crate::json::CONVERT),
                "formats" => (
                    only(arguments, &[]).map(|()| crate::formats::json()),
                    crate::formats::SCHEMA,
                ),
                "convert_all" => (convert_all(arguments), crate::batch::SCHEMA),
                "repro" => (repro(arguments), crate::repro::SCHEMA),
                "diagnose" | "diagnose_narrow" | "convert_part" | "stop_at_feature"
                | "same_failure" | "make_report" => {
                    (diagnostic(name, arguments), diagnostic_schema(name))
                }
                "compare_step" => (compare_step(arguments), "dutchman.compare-step/1"),
                "doctor" => (doctor_tool(arguments), crate::doctor::SCHEMA),

                "extract_repro" => (extract_repro(arguments), crate::diagnose::bundle::SCHEMA),
                "verify_repro" => (verify_repro(arguments), crate::diagnose::bundle::SCHEMA),
                "report_preview" => (report_preview(arguments), crate::report_send::SCHEMA),
                "report_submit" => (report_submit(arguments), crate::report_send::SCHEMA),
                _ => return Err((INVALID_PARAMS, format!("no tool named {name}"))),
            };
            // Arguments the tool cannot take fail as the tool fails: a report with the reason,
            // so a client that reads only the structured content sees it too.
            Ok(result(&report.unwrap_or_else(|message| {
                if is_diagnostic(name) {
                    let safe = ["input is required", "input must be a path", "out is required", "out must be a path",
                        "part is required", "part must be a whole number of at least 1",
                        "feature must be a whole number of at least 0", "feature number is beyond this part's history",
                        "time_limit must be a duration",
                        "the time limit is too large; use a smaller number of seconds", "memory_limit_mb must be a whole number of at least 1",
                        "memory_limit_mb is too large", "neighbours must be true or false"];
                    let message = if safe.contains(&message.as_str()) { &message } else {
                        "The diagnostic step could not finish. Check the arguments and local diagnose folder, then retry; no file was sent."
                    };
                    return crate::json::failure(schema, Value::Null, message);
                }
                let input = arguments
                    .get("input")
                    .and_then(Value::as_str)
                    .map_or(Value::Null, |input| {
                        crate::json::input(std::path::Path::new(input))
                    });
                if matches!(name, "diagnose" | "doctor" | "compare_step") {
                    crate::json::failure(schema, Value::Null, "The local check could not finish. Check the input, output folder and resource limits, then retry.")
                } else { crate::json::failure(schema, input, &message) }
            })))
        }
        _ => Err((METHOD_NOT_FOUND, format!("no method {method}"))),
    }
}

/// A tool result carrying `report`: as text for every client, and as structured content;
/// an error when the report says the run failed.
fn result(report: &Value) -> Value {
    json!({
        "content": [{"type": "text", "text": report.to_string()}],
        "structuredContent": report,
        "isError": report.get("ok") != Some(&Value::Bool(true)),
    })
}

/// The options every converting tool takes, as JSON Schema properties.
fn option_properties() -> Map<String, Value> {
    let properties = json!({
        "threads": {"type": "integer", "minimum": 0, "default": 0, "description": "Maximum assembly part workers (0: automatic, at most two; 1: serial); capped by available memory."},
        "diagnostics_default": {"type": "boolean", "default": false, "description": "Save in the platform diagnostics folder, retaining the newest 50 logs. Cannot be combined with diagnostics."},
        "diagnostics": {"type": "string", "description": "Absolute path to append an anonymous live diagnostic JSONL log. Includes progress and resource samples even if a conversion never finishes; no names or file contents."},
        "to": {"type": "string", "enum": ["solidcraft", "freecad"], "default": "solidcraft", "description": "Output format: SolidCraft design or self-contained FreeCAD rebuild macro."},
        "preset": {"type": "string", "enum": ["fast", "standard", "full"], "default": "standard", "description": "Fast keeps shape only; Standard keeps history, sketches and joints but budgets history before decoding; Full never drops history for memory."},
        "history": {"type": "boolean", "default": true, "description": "Keep feature history when the preset allows it; false narrows the preset like import_only."},
        "sketches": {"type": "boolean", "default": true, "description": "Keep reference sketches when the preset allows it. Sketches needed by retained history stay."},
        "joints": {"type": "boolean", "default": true, "description": "Keep assembly joints when the preset allows it; false keeps the selected placements before joint solving."},
        "part_positions": {"type": "string", "enum": ["smart", "last-shown", "first-placed"], "default": "smart", "description": "Part positions: Smart, As last shown in the original program, or As first placed."},
        "stored_placements": {"type": "boolean", "default": false, "description": "Place parts where they were first placed, ignoring later moves"},
        "no_memory_check": {"type": "boolean", "default": false, "description": "Run and check history even when estimated peak memory exceeds available memory; also disables Standard's history budget and history-check skipping."},
        "import_only": {
            "type": "boolean",
            "description": "Leave feature history out; keep geometry and the preset's reference sketches.",
        },
        "verify": {
            "type": "boolean",
            "description": "Replay retained history in SolidCraft and compare a complete history with the original geometry (default true; history_check_skipped reports a check that cannot fit in free memory, unless no_memory_check forces it; the report also says when no SolidCraft CLI is available).",
        },
        "solidcraft": {
            "type": "string",
            "description": "Path of the SolidCraft CLI for the check (default: $SOLIDCRAFT_CLI, else solidcraft-cli on PATH, beside this program, or inside an installed SolidCraft app).",
        },
        "overwrite": {
            "type": "boolean",
            "description": "Replace an existing .sc file at the output (default true). When false, an existing file is left alone and that file's conversion fails, saying so.",
        },
        "solidcraft_version": {
            "type": "string",
            "description": "Target SolidCraft release (x.y.z, at least 0.18.2). Defaults to the found CLI version, else the newest known at build time, even when verification is off."
        },
        "verify_timeout": {
            "type": "integer",
            "minimum": 1,
            "description": "Seconds each SolidCraft check may run before the history is written unverified (default 120).",
        },
        "project": project_property(),
        "search": search_property(),
    });
    match properties {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// The project folder an Inventor assembly's part files are searched in.
fn project_property() -> Value {
    json!({
        "type": "string",
        "description": "Absolute path of the project folder an assembly's part files are searched in, when they are not at the paths the assembly stores (default: the nearest folder above the assembly holding an Inventor .ipj, else two folders up; for convert_all, each folder given).",
    })
}

/// More folders an Inventor assembly's part files are searched in.
fn search_property() -> Value {
    json!({
        "type": "array",
        "items": {"type": "string"},
        "description": "Absolute paths of more folders to search, with their subfolders, for part files the project folder does not hold.",
    })
}

fn tools() -> Value {
    let input = json!({
        "type": "string",
        "description": "Absolute path of a CAD file. Call formats for supported extensions, reader status and what Dutchman converts. Creo and NX parts share .prt and are distinguished by contents; reading their geometry is not supported yet.",
    });
    let mut convert = option_properties();
    convert.insert("input".to_owned(), input.clone());
    convert.insert("dry_run".to_owned(), json!({"type": "boolean", "default": false, "description": "Return estimate, choices and fits from a quick pre-read, without decoding or writing. Output is optional."}));
    convert.insert(
        "output".to_owned(),
        json!({"type": "string", "description": "Absolute path of the .sc (legacy .solidcraft or .solvecraft) or .FCMacro file to write, matching to. An existing file there is replaced, unless overwrite is false. If supplied, diagnostics is also written at its specified path."}),
    );
    let mut all = option_properties();
    all.insert(
        "inputs".to_owned(),
        json!({
            "type": "array",
            "items": {"type": "string"},
            "minItems": 1,
            "description": "Absolute paths of design files, project ZIPs, and folders to search with their subfolders.",
        }),
    );
    all.insert(
        "out_dir".to_owned(),
        json!({"type": "string", "description": "Absolute path of the folder to write the designs to, keeping each input folder's layout. Designs already there are replaced, unless overwrite is false. Converted designs are written only inside out_dir. If supplied, diagnostics, journal, report_anon, and report_key are also written at their specified paths."}),
    );
    all.insert(
        "jobs".to_owned(),
        json!({"type": "integer", "minimum": 1, "description": "How many files to convert at once (default: the machine's cores, at most 8)."}),
    );
    all.insert(
        "timeout".to_owned(),
        json!({"type": "integer", "minimum": 1, "description": "Seconds one file may take before it is stopped and reported as failed (default 900)."}),
    );
    all.insert(
        "max_memory_mb".to_owned(),
        json!({"type": "integer", "minimum": 0, "description": "Megabytes of memory one file may hold before it is stopped and reported as failed (default 8192; 0: no limit)."}),
    );
    all.insert(
        "journal".to_owned(),
        json!({"type": "string", "description": "Absolute path of a file recording each file's result as it finishes. Files already recorded there, unchanged since, are not converted again, so a stopped run carries on."}),
    );
    all.insert(
        "report_anon".to_owned(),
        json!({"type": "string", "description": "Absolute path to also write an anonymous report to: counts, causes and timings with no file names, paths or geometry, safe to share (docs/report-anon.md). The result's report_anon gives its path and the salt of its file ids, which stays private."}),
    );
    all.insert(
        "report_salt".to_owned(),
        json!({"type": "string", "description": "The salt for the anonymous report's file ids (32 hex digits), to get the same ids as an earlier run (default: a new one)."}),
    );
    all.insert(
        "report_key".to_owned(),
        json!({"type": "string", "description": "Absolute path to write the key from each anonymous id to its file's path to. Keep it private."}),
    );
    let diagnostic = json!({
        "input": input, "out": {"type": "string", "description": "Local checkpoint folder; keep private."},
        "step_path": {"type": "string", "description": "Matching STEP export; otherwise an adjacent same-name STEP is found."},
        "part": {"type": "integer", "minimum": 1, "description": "Anonymous report target."},
        "time_limit": {"type": ["integer", "string"], "description": "Positive whole seconds or a duration such as 30s or 5m."},
        "memory_mb": {"type": "integer", "minimum": 256, "description": "Converter memory budget in MiB."},
        "jobs": {"type": "integer", "minimum": 1, "description": "Doctor only: maximum concurrent designs."},
        "narrow": {"type": "boolean", "description": "Diagnose only: narrow a conversion failure."}
    });
    let mut diagnose_properties = diagnostic.as_object().cloned().unwrap_or_default();
    diagnose_properties.remove("jobs");
    let mut doctor_properties = diagnostic.as_object().cloned().unwrap_or_default();
    doctor_properties.remove("narrow");
    let mut compare_properties = diagnose_properties.clone();
    compare_properties.remove("narrow");
    compare_properties.remove("out");
    compare_properties.insert(
        "memory_limit_mb".into(),
        json!({"type":"integer","minimum":1,"description":"Alias of memory_mb; choose one."}),
    );
    let mut listed = json!([
        {
            "name": "read_report",
            "description": "Read a saved per-file conversion report without decoding or converting. Returns the full dutchman.convert/1 report plus loss_groups by code and stage, including feature outcomes, placements and verification when recorded. LOCAL USE ONLY: like convert and inspect, this reply contains private names and paths; keep it out of remote assistants and public reports. Anonymous diagnostic tools remain anonymous.",
            "inputSchema": {"type": "object", "properties": {
                "input": {"type": "string", "description": "Absolute path of a .sc design (reads its adjacent <stem>.report.json), .report.json, or .failed.json. The original and design need not exist."}
            }, "required": ["input"], "additionalProperties": false},
            "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false}
        },
        {"name": "doctor", "title": "Check a folder", "description": "Check a folder or project ZIP locally with CPU and total memory budgets and private checkpoints. Returns anonymous findings grouped by signature. Nothing is sent.",
            "inputSchema": {"type": "object", "properties": doctor_properties, "required": ["input", "out"], "additionalProperties": false}},
        {"name": "compare_step", "title": "Compare matching STEP", "description": "Compare stored geometry with a STEP export under resource limits. Anonymous body numbers and measurements only; advisory, worst first. Nothing is sent.",
            "inputSchema": {"type": "object", "properties": compare_properties, "required": ["input"], "additionalProperties": false}},
        {
            "name": "convert",
            "title": "Convert a CAD file",
            "description": "Convert one native CAD file into a SolidCraft design or FreeCAD macro (to: freecad). If a feature blocks mapping or replay differs, SolidCraft keeps the editable prefix and resolved sketches hidden beside the visible original shape. Parts without body-building history keep resolved reference sketches. Returns the dutchman.convert/1 report: the mode written, the history check, each feature's status and every loss.",
            "inputSchema": {"type": "object", "properties": convert, "required": ["input"], "anyOf": [{"required": ["output"]}, {"required": ["dry_run"], "properties": {"dry_run": {"const": true}}}], "additionalProperties": false},
        },
        {
            "name": "inspect",
            "title": "Inspect a CAD file",
            "description": "Read one native CAD file and report what converting it would carry across, without writing anything. Returns the dutchman.inspect/1 report. Check conversion.possible and conversion.reason: ok means inspection succeeded, not that conversion is possible.",
            "inputSchema": {"type": "object", "properties": {"input": input, "project": project_property(), "search": search_property()}, "required": ["input"], "additionalProperties": false},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "formats",
            "title": "List formats",
            "description": "List the native formats Dutchman knows and what converts from each. Returns the dutchman.formats/1 report.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "convert_all",
            "title": "Convert many CAD files",
            "description": "Convert CAD files, folders and project ZIPs into SolidCraft designs under one folder. Plans top-level assemblies and unused parts, ignores lock files, and keeps adjacent STEP exports as native-file fallbacks. Returns the dutchman.convert-all/1 report.",
            "inputSchema": {"type": "object", "properties": all, "required": ["inputs", "out_dir"], "additionalProperties": false},
        },
        {
            "name": "report_preview", "description": "Prepare or preview a local report. Returns only sizes, roles and signature; never bundle contents. Nothing is sent. Converted files can use signature converted-with-problem in a fresh folder. Reuse the diagnostic file id. Bundle is the default when a proven bundle exists. Decode refusals have no bundle yet.",
            "inputSchema": {"type": "object", "properties": {"folder": {"type": "string"}, "report_id": {"type": "string"}, "source_format": {"type": "string"}, "signature": {"type": "string"}}, "required": ["folder"], "additionalProperties": false}
        },
        {
            "name": "report_submit", "description": "Send the previewed report only after the user's explicit yes to this choice and public-test permission. Never infer consent. No automatic resend after failure.",
            "inputSchema": {"type": "object", "properties": {"folder": {"type": "string"}, "choice": {"enum": ["bundle", "summary"], "default": "bundle"}, "public_test": {"type": "boolean", "default": false}, "explicit_yes": {"type": "boolean"}}, "required": ["folder", "choice", "explicit_yes"], "additionalProperties": false}
        },
        {
            "name": "repro", "title": "Make a minimum reproduction kit",
            "description": "Run a local conversion and narrow structured check failures. Writes a kit under out and returns anonymous.json, a summary and kit paths. Never upload the original or local-key.json. include_local_key is only for a client keeping the returned names local.",
            "inputSchema": {"type": "object", "properties": {
                "input": input, "out": {"type": "string", "description": "Absolute path of a fresh local output folder."},
                "include_local_key": {"type": "boolean", "default": false, "description": "Return private names for local use only. Keep this response local; never include it in an issue or send it to a remote assistant."}
            }, "required": ["input", "out"], "additionalProperties": false}
        },
    ]);
    if let Some(list) = listed.as_array_mut() {
        list.extend(diagnostic_tools());
    }
    listed
}

type Arguments = Map<String, Value>;

/// Refuses arguments outside `known`, so a misspelt option is not silently ignored.
fn repro(arguments: &Arguments) -> Result<Value, String> {
    only(arguments, &["input", "out", "include_local_key"])?;
    crate::repro::run(
        &path(arguments, "input")?,
        &path(arguments, "out")?,
        flag(arguments, "include_local_key", false)?,
    )
}

fn read_report(arguments: &Arguments) -> Result<Value, String> {
    only(arguments, &["input"])?;
    crate::saved_report::read(&path(arguments, "input")?)
}

fn diagnostic_settings(arguments: &Arguments) -> Result<crate::diagnose::Settings, String> {
    let mut settings = crate::diagnose::Settings::default();
    if let Some(value) = arguments.get("time_limit").filter(|v| !v.is_null()) {
        settings.time_limit = if let Some(text) = value.as_str() {
            crate::diagnose::parse_duration(text)?
        } else {
            let seconds = value
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or("time_limit must be a duration")?;
            crate::diagnose::parse_duration(&seconds.to_string())?
        };
    }
    if arguments.get("memory_mb").is_some_and(|v| !v.is_null())
        && arguments
            .get("memory_limit_mb")
            .is_some_and(|v| !v.is_null())
    {
        return Err("Choose memory_mb or memory_limit_mb, not both".into());
    }
    let key = if arguments.get("memory_mb").is_some_and(|v| !v.is_null()) {
        "memory_mb"
    } else {
        "memory_limit_mb"
    };
    if let Some(mb) = count(arguments, key)? {
        if key == "memory_mb" && mb.get() < 256 {
            return Err("memory_mb must be at least 256".into());
        }
        settings.memory_limit = Some(
            (mb.get() as u64)
                .checked_mul(1 << 20)
                .ok_or("memory_limit_mb is too large")?,
        );
    }
    if arguments.get("step_path").is_some_and(|v| !v.is_null()) {
        settings.step_path = Some(path(arguments, "step_path")?);
    }
    settings.target_part = count(arguments, "part")?.map(NonZeroUsize::get);
    Ok(settings)
}

fn compare_step(arguments: &Arguments) -> Result<Value, String> {
    only(
        arguments,
        &[
            "input",
            "step_path",
            "time_limit",
            "memory_mb",
            "memory_limit_mb",
            "part",
        ],
    )?;
    let input = path(arguments, "input")?;
    let settings = diagnostic_settings(arguments)?;
    let scratch = tempfile::tempdir().map_err(|e| e.to_string())?;
    let result = crate::diagnose::run(&input, scratch.path(), &settings, &|_| {})?;
    Ok(json!({"schema": "dutchman.compare-step/1", "ok": true,
        "comparison": result.pointer("/report/step_comparison").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!({"status": "unavailable", "parts": [], "reason": "No unique matching STEP was found. Choose a matching STEP export."}))}))
}

fn doctor_tool(arguments: &Arguments) -> Result<Value, String> {
    only(
        arguments,
        &[
            "input",
            "out",
            "jobs",
            "memory_mb",
            "time_limit",
            "step_path",
            "part",
        ],
    )?;
    let diagnose = diagnostic_settings(arguments)?;
    let settings = crate::doctor::Settings {
        jobs: count(arguments, "jobs")?,
        memory: diagnose.memory_limit.unwrap_or(8 << 30),
        diagnose,
    };
    crate::doctor::run(
        &path(arguments, "input")?,
        &path(arguments, "out")?,
        &settings,
        &|_, _| {},
    )
}

fn only(arguments: &Arguments, known: &[&str]) -> Result<(), String> {
    match arguments.keys().find(|key| !known.contains(&key.as_str())) {
        Some(key) => Err(format!("unknown argument {key}")),
        None => Ok(()),
    }
}

fn path(arguments: &Arguments, key: &str) -> Result<PathBuf, String> {
    match arguments.get(key) {
        Some(Value::String(path)) if !path.is_empty() => Ok(PathBuf::from(path)),
        Some(_) => Err(format!("{key} must be a path")),
        None => Err(format!("{key} is required")),
    }
}

fn flag(arguments: &Arguments, key: &str, default: bool) -> Result<bool, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{key} must be true or false")),
    }
}

fn count(arguments: &Arguments, key: &str) -> Result<Option<NonZeroUsize>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .and_then(NonZeroUsize::new)
            .map(Some)
            .ok_or_else(|| format!("{key} must be a whole number of at least 1")),
    }
}

const OPTIONS: [&str; 19] = [
    "diagnostics",
    "diagnostics_default",
    "threads",
    "preset",
    "history",
    "sketches",
    "joints",
    "stored_placements",
    "part_positions",
    "no_memory_check",
    "to",
    "import_only",
    "verify",
    "solidcraft",
    "solidcraft_version",
    "verify_timeout",
    "overwrite",
    "project",
    "search",
];

/// The `project` and `search` arguments.
fn part_search(arguments: &Arguments) -> Result<PartSearch, String> {
    let project = match arguments.get("project") {
        None | Some(Value::Null) => None,
        Some(_) => Some(path(arguments, "project")?),
    };
    let also = match arguments.get("search") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(folders)) => folders
            .iter()
            .map(|folder| match folder {
                Value::String(folder) => Ok(PathBuf::from(folder)),
                _ => Err("search must be a list of paths".to_owned()),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("search must be a list of paths".to_owned()),
    };
    Ok(PartSearch {
        threads: 0,
        part_positions: dutchman_core::PartPositions::Smart,
        project,
        also,
    })
}

fn options(arguments: &Arguments) -> Result<ConvertOptions, String> {
    let defaults = ConvertOptions::default();
    let solidcraft = match arguments.get("solidcraft") {
        None | Some(Value::Null) => None,
        Some(_) => Some(path(arguments, "solidcraft")?),
    };
    let search = part_search(arguments)?;
    let to = match arguments.get("to").and_then(Value::as_str) {
        None if !arguments.contains_key("to") => crate::OutputFormat::Solidcraft,
        Some("solidcraft") => crate::OutputFormat::Solidcraft,
        Some("freecad") => crate::OutputFormat::Freecad,
        _ => return Err("to must be solidcraft or freecad".to_owned()),
    };
    let solidcraft_version = match arguments.get("solidcraft_version") {
        None | Some(Value::Null) => None,
        Some(Value::String(version)) => Some(version.parse()?),
        Some(_) => {
            return Err("solidcraft_version must be a version string such as 0.25.1".to_owned())
        }
    };
    let threads = match arguments.get("threads") {
        None | Some(Value::Null) => 0,
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| "threads must be a whole number of at least 0".to_owned())?,
    };
    let part_positions = match arguments.get("part_positions") {
        None | Some(Value::Null) => dutchman_core::PartPositions::Smart,
        Some(Value::String(value)) => value.parse()?,
        Some(_) => {
            return Err("part_positions must be smart, last-shown, or first-placed".to_owned())
        }
    };
    if flag(arguments, "stored_placements", false)?
        && arguments.contains_key("part_positions")
        && part_positions != dutchman_core::PartPositions::FirstPlaced
    {
        return Err(
            "stored_placements selects first-placed; choose one part_positions setting".to_owned(),
        );
    }
    let import_only = flag(arguments, "import_only", false)?;
    let history = flag(arguments, "history", true)?;
    let diagnostics_default = flag(arguments, "diagnostics_default", false)?;
    if diagnostics_default && arguments.get("diagnostics").is_some_and(|v| !v.is_null()) {
        return Err("diagnostics and diagnostics_default cannot be combined".into());
    }
    Ok(ConvertOptions {
        threads,
        diagnostics_default,
        diagnostics: match arguments.get("diagnostics") {
            None | Some(Value::Null) => None,
            Some(_) => Some(path(arguments, "diagnostics")?),
        },
        diagnostics_salt: None,
        diagnostics_threads: 1,
        to,
        verify: false,
        freecad_timeout: 300,
        preset: match arguments.get("preset") {
            None => defaults.preset,
            Some(Value::String(name)) => match name.as_str() {
                "fast" => crate::choices::Preset::Fast,
                "standard" => crate::choices::Preset::Standard,
                "full" => crate::choices::Preset::Full,
                _ => return Err("preset must be fast, standard or full".to_owned()),
            },
            Some(_) => return Err("preset must be fast, standard or full".to_owned()),
        },
        import_only: import_only || !history,
        no_sketches: !flag(arguments, "sketches", true)?,
        no_joints: !flag(arguments, "joints", true)?,
        stored_placements: flag(arguments, "stored_placements", false)?,
        part_positions,
        no_memory_check: flag(arguments, "no_memory_check", false)?,
        no_verify: !flag(arguments, "verify", true)?,
        write_replay: false,
        no_clobber: !flag(arguments, "overwrite", true)?,
        solidcraft,
        solidcraft_version,
        verify_timeout: count(arguments, "verify_timeout")?
            .map_or(defaults.verify_timeout, |n| n.get() as u64),
        project: search.project,
        search: search.also,
        part: None,
    })
}

fn convert(arguments: &Arguments) -> Result<Value, String> {
    only(
        arguments,
        &[&["input", "output", "dry_run"][..], &OPTIONS].concat(),
    )?;
    let input = path(arguments, "input")?;
    let options = options(arguments)?;
    if flag(arguments, "dry_run", false)? {
        let mut choices = options.choices();
        let full = crate::choices::estimate_with_options(&input, &choices, &options);
        let available = crate::choices::available_memory();
        let check = crate::choices::memory_check(&input, &options);
        let estimate = match check {
            crate::choices::MemoryCheck::Ok(estimate) => estimate,
            crate::choices::MemoryCheck::TooBig { estimate, .. } => Some(estimate),
        };
        if estimate.map(|e| e.peak_bytes) != full.map(|e| e.peak_bytes) {
            choices.history = false;
        }
        return Ok(json!({
            "ok": true,
            "estimate": crate::json::estimate(estimate),
            "choices": crate::json::placement_choices(&options, choices),
            "fits": estimate.zip(available).map(|(e, free)| e.peak_bytes <= free),
        }));
    }
    let output = path(arguments, "output")?;
    // A unit test's own program is not a converter.
    #[cfg(test)]
    return Ok(crate::convert_options_with_progress(
        &input,
        &output,
        &options,
        !options.no_clobber,
        &options.part_search(),
        &|_| {},
    ));
    // In a child, so that running low on memory stops the conversion and not this server.
    #[cfg(not(test))]
    {
        let program =
            std::env::current_exe().map_err(|err| format!("cannot find this program: {err}"))?;
        let job = crate::batch::Job {
            project_files: None,
            fallback: None,
            input,
            output,
            replace: !options.no_clobber,
            search: options.part_search(),
        };
        let limits = crate::worker::Limits {
            timeout: std::time::Duration::from_secs(u64::MAX / 4),
            max_memory: None,
            design: Some(crate::worker::DEFAULT_DESIGN_LIMIT),
        };
        Ok(crate::worker::convert_isolated(
            &program, &job, &options, &limits,
        ))
    }
}

fn inspect(arguments: &Arguments) -> Result<Value, String> {
    only(arguments, &["input", "project", "search"])?;
    Ok(crate::inspect_report(
        &path(arguments, "input")?,
        &part_search(arguments)?,
    ))
}

fn convert_all(arguments: &Arguments) -> Result<Value, String> {
    only(
        arguments,
        &[
            &[
                "inputs",
                "out_dir",
                "jobs",
                "timeout",
                "max_memory_mb",
                "journal",
                "report_anon",
                "report_salt",
                "report_key",
            ][..],
            &OPTIONS,
        ]
        .concat(),
    )?;
    let inputs = match arguments.get("inputs") {
        Some(Value::Array(items)) if !items.is_empty() => items
            .iter()
            .map(|item| match item {
                Value::String(path) if !path.is_empty() => Ok(PathBuf::from(path)),
                _ => Err("inputs must be paths".to_owned()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("inputs must be a list of one or more paths".to_owned()),
        None => return Err("inputs is required".to_owned()),
    };
    let out_dir = path(arguments, "out_dir")?;
    let mut options = options(arguments)?;
    let optional = |key: &str| match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => path(arguments, key).map(Some),
    };
    let mut anon = match optional("report_anon")? {
        Some(path) => Some(crate::anon::Request {
            path,
            salt: match arguments.get("report_salt") {
                None | Some(Value::Null) => None,
                Some(Value::String(hex)) => Some(
                    crate::anon::Salt::parse(hex).map_err(|err| format!("report_salt: {err}"))?,
                ),
                Some(_) => return Err("report_salt must be 32 hex digits".to_owned()),
            },
            key: optional("report_key")?,
        }),
        None if arguments.contains_key("report_salt") || arguments.contains_key("report_key") => {
            return Err("report_salt and report_key need report_anon".to_owned())
        }
        None => None,
    };
    if let Some(request) = &mut anon {
        let salt = request
            .salt
            .clone()
            .map_or_else(crate::anon::Salt::random, Ok)?;
        request.salt = Some(salt.clone());
        options.diagnostics_salt = Some(salt);
    }
    let max_memory = match arguments.get("max_memory_mb") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_u64() {
            Some(0) => None,
            Some(mb) => Some(mb.saturating_mul(1 << 20)),
            None => return Err("max_memory_mb must be a whole number".to_owned()),
        },
    };
    let settings = crate::batch::Settings {
        jobs: count(arguments, "jobs")?,
        journal: optional("journal")?,
        limits: crate::worker::Limits {
            timeout: count(arguments, "timeout")?.map_or(crate::worker::DEFAULT_TIMEOUT, |n| {
                std::time::Duration::from_secs(n.get() as u64)
            }),
            max_memory,
            design: Some(crate::worker::DEFAULT_DESIGN_LIMIT),
        },
        ..crate::batch::Settings::isolated()?
    };
    let mut report = crate::batch::convert_all(&inputs, &out_dir, &options, &settings)?;
    if let Some(request) = anon {
        let salt = crate::anon::write(&request, &report, &inputs, &out_dir, &options, &settings)
            .map_err(|err| format!("cannot write the anonymous report: {err}"))?;
        if let Some(fields) = report.as_object_mut() {
            fields.insert(
                "report_anon".to_owned(),
                json!({"path": request.path.display().to_string(), "salt": salt.hex()}),
            );
        }
    }
    Ok(report)
}

// Test helpers outside `#[test]` functions are not covered by clippy.toml's test allowances.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_limits_preserve_both_argument_styles() {
        for arguments in [
            json!({"time_limit": "30s", "memory_limit_mb": 512}),
            json!({"time_limit": 30, "memory_mb": 512}),
        ] {
            let settings = diagnostic_settings(arguments.as_object().unwrap()).unwrap();
            assert_eq!(settings.time_limit, std::time::Duration::from_secs(30));
            assert_eq!(settings.memory_limit, Some(512 << 20));
        }
        for arguments in [
            json!({"time_limit": u64::MAX}),
            json!({"memory_mb": 512, "memory_limit_mb": 512}),
            json!({"memory_mb": 255}),
        ] {
            assert!(diagnostic_settings(arguments.as_object().unwrap()).is_err());
        }
    }

    #[test]
    fn placement_setting_reaches_workers_search_and_reports() {
        use dutchman_core::PartPositions;
        for mode in [
            PartPositions::Smart,
            PartPositions::LastShown,
            PartPositions::FirstPlaced,
        ] {
            let arguments = json!({"part_positions": mode.name()});
            let parsed = options(arguments.as_object().unwrap()).unwrap();
            assert_eq!(parsed.effective_part_positions(), mode);
            assert_eq!(parsed.part_search().part_positions, mode);
            assert!(parsed
                .to_args()
                .windows(2)
                .any(|pair| pair[0] == "--part-positions" && pair[1] == mode.name()));
            assert_eq!(
                crate::json::placement_choices(&parsed, parsed.choices())["part_positions"],
                mode.name()
            );
        }
        let alias = options(json!({"stored_placements":true}).as_object().unwrap()).unwrap();
        assert_eq!(alias.effective_part_positions(), PartPositions::FirstPlaced);
        assert_eq!(
            options(&Arguments::new())
                .unwrap()
                .effective_part_positions(),
            PartPositions::Smart
        );
        for bad in [
            json!({"stored_placements":"true"}),
            json!({"part_positions":"maybe"}),
            json!({"part_positions":true}),
            json!({"part_positions":"smart","stored_placements":true}),
        ] {
            assert!(options(bad.as_object().unwrap()).is_err());
        }
    }

    #[test]
    fn part_threads_are_validated_and_forwarded() {
        for requested in [0, 1, 4] {
            let arguments = json!({"threads": requested});
            let parsed = options(arguments.as_object().unwrap()).unwrap();
            assert_eq!(parsed.threads, requested);
            assert_eq!(parsed.part_search().threads, requested);
            assert!(parsed.to_args().windows(2).any(|pair| {
                pair[0] == "--threads" && pair[1] == requested.to_string().as_str()
            }));
        }
        for bad in [json!(-1), json!(1.5), json!("4"), json!(true)] {
            let arguments = json!({"threads": bad});
            assert!(options(arguments.as_object().unwrap())
                .unwrap_err()
                .contains("threads must be"));
        }
    }

    #[test]
    fn batch_schema_describes_every_write_destination() {
        let tools = tools();
        let batch = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "convert_all")
            .unwrap();
        let properties = &batch["inputSchema"]["properties"];
        let description = properties["out_dir"]["description"].as_str().unwrap();
        assert!(description.contains("Converted designs are written only inside out_dir"));
        assert!(!description.contains("Nothing is written outside it"));
        for option in ["journal", "report_anon", "report_key"] {
            assert!(description.contains(option), "{description}");
            assert_eq!(properties[option]["type"], "string");
        }
        let inspect = tools
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "inspect")
            .unwrap();
        assert!(inspect["description"]
            .as_str()
            .unwrap()
            .contains("conversion.possible and conversion.reason"));
    }

    #[test]
    fn an_unreadable_archive_has_recovery_advice_and_no_output() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("incomplete.f3d");
        let output = dir.path().join("out.solidcraft");
        std::fs::write(&input, b"not an archive").unwrap();
        let result = call("convert", json!({"input": input, "output": output}));
        let report = &result["result"]["structuredContent"];
        assert_eq!(result["result"]["isError"], true);
        assert_eq!(report["error"]["stage"], "decode");
        assert_eq!(report["error"]["upstream"], "cadmpeg");
        let message = report["error"]["message"].as_str().unwrap();
        assert!(message.contains("download or copy it again"), "{message}");
        assert!(
            message.contains("template=file-did-not-convert.yml"),
            "{message}"
        );
        assert!(!output.exists());
        assert_eq!(call("formats", json!({}))["result"]["isError"], false);
    }

    fn call(name: &str, arguments: Value) -> Value {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}});
        handle(&message.to_string()).unwrap()
    }

    #[test]
    fn report_tools_preview_without_secrets_and_require_yes() {
        let dir = tempfile::tempdir().unwrap();
        let preview = call(
            "report_preview",
            json!({"folder": dir.path(), "report_id": "f0123456789abcdef", "source_format": "f3d", "signature": "decode"}),
        );
        assert_eq!(
            preview["result"]["structuredContent"]["bundle_available"],
            false
        );
        assert!(!preview.to_string().contains("claim_secret"));
        assert!(!preview.to_string().contains("bytes_base64url"));
        let refused = call(
            "report_submit",
            json!({"folder": dir.path(), "choice": "summary", "explicit_yes": false}),
        );
        assert_eq!(refused["result"]["isError"], true);
        assert!(!dir.path().join("submission.json").exists());
    }

    #[test]
    fn report_preview_accepts_a_converted_file_without_a_failure_or_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let preview = call(
            "report_preview",
            json!({"folder": dir.path(),
            "report_id": "f0123456789abcdef", "source_format": "f3d",
            "signature": crate::report_send::CONVERTED_PROBLEM_SIGNATURE}),
        );
        let report = &preview["result"]["structuredContent"];
        assert_eq!(preview["result"]["isError"], false);
        assert_eq!(report["default_choice"], "summary");
        assert_eq!(report["metadata"]["signature"], "converted-with-problem");
        assert_eq!(report["metadata"]["consent"]["choice"], "summary");
        assert_eq!(report["bundle_available"], false);
        assert!(!dir.path().join("submission.json").exists());
    }

    #[test]
    fn notifications_and_responses_get_no_reply() {
        assert!(handle(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        assert!(handle(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#).is_none());
    }

    #[test]
    fn malformed_messages_are_protocol_errors() {
        assert_eq!(handle("{").unwrap()["error"]["code"], PARSE_ERROR);
        assert_eq!(handle("[]").unwrap()["error"]["code"], INVALID_REQUEST);
        let unknown = handle(r#"{"jsonrpc":"2.0","id":"a","method":"nope"}"#).unwrap();
        assert_eq!(unknown["id"], "a");
        assert_eq!(unknown["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(
            call("no_such_tool", json!({}))["error"]["code"],
            INVALID_PARAMS
        );
    }

    #[test]
    fn bad_arguments_are_tool_errors() {
        for (name, arguments, says) in [
            ("convert", json!({"input": "a.f3d"}), "output is required"),
            (
                "convert",
                json!({"input": "a.f3d", "output": 3}),
                "output must be a path",
            ),
            (
                "convert",
                json!({"input": "a.f3d", "output": "b.solidcraft", "verfy": false}),
                "unknown argument verfy",
            ),
            (
                "convert_all",
                json!({"inputs": [], "out_dir": "x"}),
                "one or more paths",
            ),
            (
                "convert_all",
                json!({"inputs": ["a"], "out_dir": "x", "jobs": 0}),
                "jobs must be",
            ),
            ("formats", json!({"x": 1}), "unknown argument x"),
            (
                "inspect",
                json!({"input": "a.iam", "project": 3}),
                "project must be a path",
            ),
        ] {
            let result = &call(name, arguments)["result"];
            assert_eq!(result["isError"], true, "{name}");
            // The same shape as a failed conversion: a report, as structured content and text.
            let report = &result["structuredContent"];
            assert_eq!(report["ok"], false, "{name}");
            let message = report["error"]["message"].as_str().unwrap();
            assert!(message.contains(says), "{name}: {message}");
            let text: Value =
                serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(&text, report, "{name}");
        }
        let report = &call("convert", json!({"input": "/x/a.f3d"}))["result"]["structuredContent"];
        assert_eq!(report["schema"], "dutchman.convert/1");
        assert_eq!(report["input"]["path"], "/x/a.f3d");
        // A project folder is taken; the file is what fails.
        let report = &call("inspect", json!({"input": "/x/a.iam", "project": "/x"}))["result"]
            ["structuredContent"];
        assert_eq!(report["schema"], "dutchman.inspect/1");
        assert_eq!(report["input"]["path"], "/x/a.iam");
        let report = &call("formats", json!({"x": 1}))["result"]["structuredContent"];
        assert_eq!(report["schema"], "dutchman.formats/1");
    }

    #[test]
    fn a_failed_conversion_is_a_tool_error_with_its_report() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out.solidcraft");
        let result = &call(
            "convert",
            json!({"input": dir.path().join("missing.f3d"), "output": output}),
        )["result"];
        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["schema"], "dutchman.convert/1");
        assert_eq!(result["structuredContent"]["ok"], false);
        assert!(!output.exists());
    }

    #[test]
    fn an_over_long_message_is_refused_and_the_next_one_answered() {
        let mut input = vec![b' '; MAX_MESSAGE + 10];
        input.push(b'\n');
        input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n");
        let mut output = Vec::new();
        serve(&input[..], &mut output).unwrap();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0]["error"]["code"], INVALID_REQUEST);
        assert_eq!(replies[1]["id"], 2);
        assert_eq!(replies[1]["result"], json!({}));
    }
}

#[cfg(test)]
mod freecad_output_tests {
    use super::*;

    #[test]
    fn solidcraft_target_option_is_validated_and_forwarded() {
        for version in ["0.18.2", "0.25.1"] {
            let arguments = json!({"solidcraft_version": version, "verify": false});
            let parsed = options(arguments.as_object().unwrap()).unwrap();
            assert_eq!(parsed.solidcraft_version.unwrap().to_string(), version);
            assert!(parsed.to_args().windows(2).any(|pair| {
                pair.first()
                    .is_some_and(|arg| arg == "--solidcraft-version")
                    && pair.get(1).is_some_and(|arg| arg == version)
            }));
        }
        for value in [json!("0.18.1"), json!("bad"), json!(25)] {
            let arguments = json!({"solidcraft_version": value});
            assert!(options(arguments.as_object().unwrap()).is_err());
        }
        let tools = tools();
        for name in ["convert", "convert_all"] {
            let tool = tools
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap();
            assert_eq!(
                tool["inputSchema"]["properties"]["solidcraft_version"]["type"],
                "string"
            );
        }
    }

    #[test]
    fn output_selection_is_validated_and_forwarded_to_workers() {
        let arguments = json!({"to": "freecad"});
        let options = options(arguments.as_object().unwrap()).unwrap();
        assert_eq!(options.to, crate::OutputFormat::Freecad);
        assert!(options.to_args().windows(2).any(|pair| {
            pair.first().is_some_and(|arg| arg == "--to")
                && pair.get(1).is_some_and(|arg| arg == "freecad")
        }));
        let bad = json!({"to": "python"});
        assert!(super::options(bad.as_object().unwrap()).is_err());
    }
}

fn is_diagnostic(name: &str) -> bool {
    matches!(
        name,
        "extract_repro"
            | "verify_repro"
            | "diagnose"
            | "diagnose_narrow"
            | "convert_part"
            | "stop_at_feature"
            | "same_failure"
            | "make_report"
    )
}

fn diagnostic_schema(name: &str) -> &'static str {
    match name {
        "diagnose" => crate::json::DIAGNOSE,
        "convert_part" | "stop_at_feature" => crate::json::PART_TRIAL,
        "same_failure" => crate::json::SAME_FAILURE,
        "make_report" => crate::json::MADE_REPORT,
        _ => crate::json::NARROW,
    }
}

fn diagnostic_tools() -> Vec<Value> {
    [
        ("extract_repro", "Cut, prove and shrink a private bundle locally without narrowing. Nothing is sent.", vec!["input", "out"]),
        ("verify_repro", "Replay a saved bundle locally. Returns sizes, manifest and reproduction status only.", vec!["folder"]),
        ("diagnose", "Rank numbered parts locally under time and memory limits.", vec!["input", "out"]),
        ("diagnose_narrow", "Resume automatic narrowing, returning the anonymous signature and rebuild recipe.", vec!["input", "out"]),
        ("convert_part", "Try one numbered part, optionally with neighbours; discard the converted geometry.", vec!["input", "part"]),
        ("stop_at_feature", "Try a numbered part through a history feature (0 means stored shape only); discard geometry.", vec!["input", "part", "feature"]),
        ("same_failure", "Check a freshly rebuilt small file against the signature in out.", vec!["input", "out"]),
        ("make_report", "Prepare a local ZIP from the checked small file. It contains geometry. Nothing is sent; ask the user to review it locally before sharing.", vec!["input", "out"]),
    ].into_iter().map(|(name, description, required)| {
        let mut properties = Map::new();
        for key in &required {
            properties.insert((*key).to_owned(), if matches!(*key, "part" | "feature") {
                json!({"type": "integer", "minimum": if *key == "part" { 1 } else { 0 }})
            } else { json!({"type": "string", "description": "Absolute local path; never returned to the assistant."}) });
        }
        if matches!(name, "convert_part" | "stop_at_feature") {
            properties.insert("neighbours".into(), json!({"type": "boolean", "default": false}));
        }
        if !matches!(name, "make_report" | "verify_repro") {
            properties.insert("time_limit".into(), json!({"type": ["string", "integer"], "description": "Time per trial, such as 30s or 5m, or positive whole seconds."}));
            properties.insert("memory_limit_mb".into(), json!({"type": "integer", "minimum": 1}));
        }
        if matches!(name,"diagnose" | "diagnose_narrow") {
            properties.insert("step_path".into(),json!({"type":"string","description":"Matching STEP export; otherwise a same-name adjacent export is found."}));
            properties.insert("part".into(),json!({"type":"integer","minimum":1,"description":"Anonymous report target."}));
            properties.insert("memory_mb".into(),json!({"type":"integer","minimum":256,"description":"Alias of memory_limit_mb; choose one."}));
        }
        if name == "diagnose" { properties.insert("narrow".into(),json!({"type":"boolean","default":false})); }
        json!({"name": name, "description": format!("{description} Results carry no names, paths or geometry."),
            "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false}})
    }).collect()
}

fn diagnostic(name: &str, arguments: &Arguments) -> Result<Value, String> {
    use crate::diagnose::{self, narrow};
    let mut allowed = vec!["input"];
    if matches!(name, "convert_part" | "stop_at_feature") {
        allowed.extend(["part", "neighbours"]);
        if name == "stop_at_feature" {
            allowed.push("feature");
        }
    } else {
        allowed.push("out");
    }
    if matches!(name, "diagnose" | "diagnose_narrow") {
        allowed.extend(["step_path", "part", "memory_mb"]);
    }
    if name == "diagnose" {
        allowed.push("narrow");
    }
    if !matches!(name, "make_report" | "verify_repro") {
        allowed.extend(["time_limit", "memory_limit_mb"]);
    }
    only(arguments, &allowed)?;
    let input = path(arguments, "input")?;
    let rank = diagnostic_settings(arguments)?;
    let settings = narrow::NarrowSettings::new(rank, None);
    if matches!(name, "convert_part" | "stop_at_feature") {
        let part = count(arguments, "part")?.ok_or("part is required")?.get();
        let feature = if name == "stop_at_feature" {
            Some(
                arguments
                    .get("feature")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or("feature must be a whole number of at least 0")?,
            )
        } else {
            None
        };
        return narrow::trial(
            &input,
            part,
            flag(arguments, "neighbours", false)?,
            feature,
            &settings,
        );
    }
    let out = path(arguments, "out")?;
    let result = match name {
        "diagnose" if flag(arguments, "narrow", false)? => {
            narrow::run(&input, &out, &settings, &|_| {})?
        }
        "diagnose" => diagnose::run(&input, &out, &settings.rank, &|_| {})?,
        "diagnose_narrow" => narrow::run(&input, &out, &settings, &|_| {})?,
        "same_failure" => return narrow::same_failure(&input, &out, &settings, &|_| {}),
        "make_report" => narrow::make_report(&input, &out)?,
        _ => return Err("unknown diagnostic step".into()),
    };
    // Explicit selection: paths and local keys never enter a diagnostic tool result.
    let mut answer = Map::new();
    for key in [
        "schema",
        "ok",
        "summary",
        "status",
        "report",
        "result",
        "signature",
        "bytes",
        "bundle",
    ] {
        if let Some(value) = result.get(key) {
            answer.insert(key.into(), value.clone());
        }
    }
    Ok(Value::Object(answer))
}

fn report_preview(arguments: &Arguments) -> Result<Value, String> {
    only(
        arguments,
        &["folder", "report_id", "source_format", "signature"],
    )?;
    let folder = path(arguments, "folder")?;
    let report = if folder.join("prepared.json").exists() {
        crate::report_send::Prepared::load(&folder)?
    } else {
        let text = |key| {
            arguments
                .get(key)
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{key} is required"))
        };
        crate::report_send::Prepared::new(
            text("report_id")?,
            text("source_format")?,
            text("signature")?,
            None,
            &folder,
        )?
    };
    Ok(report.preview())
}

fn report_submit(arguments: &Arguments) -> Result<Value, String> {
    only(
        arguments,
        &["folder", "choice", "public_test", "explicit_yes"],
    )?;
    let folder = path(arguments, "folder")?;
    let prepared = crate::report_send::Prepared::load(&folder)?;
    crate::report_send::submit(
        &prepared,
        &folder,
        arguments
            .get("choice")
            .and_then(Value::as_str)
            .unwrap_or("bundle"),
        flag(arguments, "public_test", false)?,
        flag(arguments, "explicit_yes", false)?,
        crate::report_send::endpoint().as_deref(),
    )
}

fn extract_repro(arguments: &Arguments) -> Result<Value, String> {
    only(
        arguments,
        &["input", "out", "time_limit", "memory_limit_mb"],
    )?;
    crate::diagnose::bundle::extract(
        &path(arguments, "input")?,
        &path(arguments, "out")?,
        &diagnostic_settings(arguments)?,
        None,
        None,
        &|| false,
    )
}
fn verify_repro(arguments: &Arguments) -> Result<Value, String> {
    only(arguments, &["folder"])?;
    crate::diagnose::bundle::verify(&path(arguments, "folder")?)
}

#[cfg(test)]
mod bundle_tools_tests {
    use super::*;
    #[test]
    fn bundle_tools_are_listed_and_errors_are_anonymous() {
        let tools = diagnostic_tools();
        for name in ["extract_repro", "verify_repro"] {
            assert!(tools.iter().any(|t| t["name"] == name));
            assert!(is_diagnostic(name));
        }
        let response = handle(
            &json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"verify_repro", "arguments":{"folder":"/private/design-name/no-bundle"}}}).to_string(),
        );
        assert!(response.is_some());
        assert!(!format!("{response:?}").contains("design-name"));
    }
}
