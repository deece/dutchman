//! Command-line progress presentation on stderr only.
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::Mutex;

use dutchman_core::ProgressEvent;

/// Serializes concurrent files' progress on one updating terminal line.
pub struct StderrProgress {
    quiet: bool,
    json: bool,
    terminal: bool,
    plain: bool,
    line: Mutex<Option<usize>>,
}

impl StderrProgress {
    /// Progress shows as an updating line on a terminal. Off a terminal it is silent, unless
    /// `json` asks for machine-readable lines or `verbose` for plain ones; `quiet` hides all.
    pub fn new(quiet: bool, json: bool, verbose: bool) -> Self {
        let terminal = std::io::stderr().is_terminal();
        Self {
            quiet,
            json,
            terminal,
            plain: verbose,
            line: Mutex::new(None),
        }
    }

    pub fn report(&self, input: &Path, event: ProgressEvent) {
        if self.quiet || !(self.json || self.terminal || self.plain) {
            return;
        }
        if let Ok(mut line) = self.line.lock() {
            let mut err = std::io::stderr().lock();
            if self.json {
                let _ = writeln!(err, "{}", crate::json::progress(input, event));
            } else if self.terminal {
                let name = input
                    .file_name()
                    .unwrap_or(input.as_os_str())
                    .to_string_lossy();
                let name: String = name.chars().take(20).collect();
                let text = format!("{name}: {event}");
                let width = line.unwrap_or(0).max(text.chars().count());
                // Padding clears the previous update without console-specific escapes.
                let _ = write!(err, "\r{text:<width$}");
                let _ = err.flush();
                *line = Some(width);
            } else {
                let _ = writeln!(err, "{}: {event}", input.display());
            }
        }
    }

    pub fn finish(&self) {
        if let Ok(mut line) = self.line.lock() {
            if line.take().is_some() {
                eprintln!();
            }
        }
    }
}

impl Drop for StderrProgress {
    fn drop(&mut self) {
        self.finish();
    }
}
