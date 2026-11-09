//! Records the cadmpeg version Dutchman is built with, from the workspace's lock file, for
//! the reports to name.

use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let mut version = None;
    for folder in Path::new(&manifest).ancestors() {
        let lock = folder.join("Cargo.lock");
        if let Ok(text) = std::fs::read_to_string(&lock) {
            println!("cargo:rerun-if-changed={}", lock.display());
            version = cadmpeg_version(&text);
            break;
        }
    }
    // The target the archives of newer builds are named by (`dutchman investigate`).
    println!(
        "cargo:rustc-env=DUTCHMAN_TARGET={}",
        std::env::var("TARGET").unwrap_or_default()
    );
    println!(
        "cargo:rustc-env=DUTCHMAN_CADMPEG_VERSION={}",
        version.unwrap_or_else(|| "unknown".to_owned())
    );
}

/// The version of the `cadmpeg-core` package a lock file holds, with the commit it is taken
/// from when it comes from git (`0.6.3+dbf8cd4`).
fn cadmpeg_version(lock: &str) -> Option<String> {
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "name = \"cadmpeg-core\"" {
            let version = lines
                .next()?
                .trim()
                .strip_prefix("version = \"")
                .and_then(|rest| rest.strip_suffix('"'))?
                .to_owned();
            let commit = lines
                .next()
                .and_then(|source| source.trim().strip_prefix("source = \"git+"))
                .and_then(|source| source.rsplit_once('#'))
                .map(|(_, commit)| commit.trim_end_matches('"'))
                .and_then(|commit| commit.get(..7));
            return Some(match commit {
                Some(commit) => format!("{version}+{commit}"),
                None => version,
            });
        }
    }
    None
}
