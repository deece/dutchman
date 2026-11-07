//! `dutchman-mcp`: Dutchman as a Model Context Protocol server on stdio, for AI agents.
//!
//! Start it from an MCP client; it reads requests on stdin and answers on stdout until stdin
//! closes. See the README for client configuration and `docs/json.md` for the reports its
//! tools return.

use std::process::ExitCode;

fn main() -> ExitCode {
    if let Some(code) = dutchman::worker::run_if_worker() {
        return code;
    }
    if let Some(text) = info_text(std::env::args_os().skip(1)) {
        println!("{text}");
        return ExitCode::SUCCESS;
    }
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    match dutchman::mcp::serve(stdin.lock(), stdout.lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("dutchman-mcp: {err}");
            ExitCode::FAILURE
        }
    }
}

/// The text for a lone `--version` or `--help` argument, which is answered without starting
/// the server.
fn info_text(mut args: impl Iterator<Item = std::ffi::OsString>) -> Option<String> {
    let flag = args.next()?;
    if args.next().is_some() {
        return None;
    }
    let version = env!("CARGO_PKG_VERSION");
    match flag.to_str()? {
        "--version" | "-V" => Some(format!("dutchman-mcp {version}")),
        "--help" | "-h" => Some(format!(
            "dutchman-mcp {version}\n\
             Dutchman as a Model Context Protocol server on stdio.\n\n\
             Usage: dutchman-mcp\n\n\
             Start it from an MCP client; it reads requests on stdin and answers on stdout\n\
             until stdin closes.\n\n\
             Options:\n  -h, --help     Print help\n  -V, --version  Print version"
        )),
        _ => None,
    }
}
