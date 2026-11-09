//! Comparing builds: the newest nightly and the listed candidate builds convert the design too,
//! and the summary says, feature by feature, what each one changes.
//!
//! The builds come from GitHub releases: the rolling `nightly` pre-release, and the rolling
//! `candidates` pre-release, whose `candidates.json` lists the builds made from
//! `docs/candidates.toml`. Each archive is checked against the SHA-256 published beside it and
//! kept in the user's cache folder, so a second run downloads nothing. No Rust toolchain is
//! needed, and nothing about the design leaves the machine: the downloads are plain GETs of
//! public files.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Where the builds are published.
const RELEASES: &str = "https://api.github.com/repos/dutchman-cad/dutchman/releases/tags";

/// The largest archive downloaded.
const MAX_ARCHIVE: u64 = 400 << 20;

/// The largest release description or list read.
const MAX_META: u64 = 4 << 20;

/// How long one build may take to convert the design, by default.
pub const DEFAULT_TIME_LIMIT: Duration = Duration::from_secs(30 * 60);

/// The target triple this program was built for, which the archives are named by.
pub const TARGET: &str = env!("DUTCHMAN_TARGET");

/// One build to compare against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Build {
    /// `nightly`, or the candidate's name.
    pub label: String,
    /// What the release says it is (`Nightly 2026-11-08`, a candidate's note).
    pub title: String,
    /// The archive's download address.
    pub url: String,
    /// The address of its SHA-256.
    pub sha_url: String,
}

/// What one build changed, against the conversion being investigated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Delta {
    pub label: String,
    pub title: String,
    /// Why there is no comparison, when there isn't one.
    pub skipped: Option<String>,
    pub ok_before: bool,
    pub ok_after: bool,
    pub rebuilt_before: u64,
    pub rebuilt_after: u64,
    pub needing: u64,
    /// Features rebuilt before and not now: (ordinal, kind, status now).
    pub worse: Vec<(u64, String, String)>,
    /// Features rebuilt now and not before: (ordinal, kind, status before).
    pub better: Vec<(u64, String, String)>,
}

impl Delta {
    /// Whether this build is worse anywhere.
    pub fn regresses(&self) -> bool {
        (self.ok_before && !self.ok_after) || !self.worse.is_empty()
    }
}

/// The features of a convert report by ordinal: (kind, status).
fn features(report: &Value) -> Vec<(u64, String, String)> {
    report
        .pointer("/history/features")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|feature| {
            let ordinal = feature.get("ordinal")?.as_u64()?;
            let kind = crate::anon::generic(
                feature.get("kind").and_then(Value::as_str).unwrap_or("?"),
                "other",
            );
            let status = feature
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let status =
                if status == "mapped" && feature.get("reapplied") == Some(&Value::Bool(true)) {
                    "reapplied"
                } else {
                    status
                };
            Some((ordinal, kind, word(status)))
        })
        .collect()
}

fn word(text: &str) -> String {
    if !text.is_empty()
        && text.len() <= 20
        && text.chars().all(|c| c.is_ascii_lowercase() || c == '_')
    {
        text.to_owned()
    } else {
        "other".to_owned()
    }
}

/// What `after` (another build's report on the same design) changes against `before`.
pub fn delta(label: &str, title: &str, before: &Value, after: &Value) -> Delta {
    let count = |report: &Value, pointer: &str| {
        report.pointer(pointer).and_then(Value::as_u64).unwrap_or(0)
    };
    let mut delta = Delta {
        label: label.to_owned(),
        title: title.to_owned(),
        ok_before: before.get("ok") == Some(&Value::Bool(true)),
        ok_after: after.get("ok") == Some(&Value::Bool(true)),
        rebuilt_before: count(before, "/history/rebuilt"),
        rebuilt_after: count(after, "/history/rebuilt"),
        needing: count(before, "/history/needing").max(count(after, "/history/needing")),
        ..Delta::default()
    };
    let now = features(after);
    for (ordinal, kind, status) in features(before) {
        let Some((_, later_kind, later)) = now.iter().find(|(o, _, _)| *o == ordinal) else {
            continue;
        };
        if *later_kind != kind || *later == status {
            continue;
        }
        if status == "mapped" && later != "mapped" {
            delta.worse.push((ordinal, kind, later.clone()));
        } else if later == "mapped" && status != "mapped" {
            delta.better.push((ordinal, kind, status));
        }
    }
    delta
}

/// A plain GET with the size limited, refusing redirects to anywhere but GitHub's own hosts.
fn get(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(120))
        .redirects(5)
        .build();
    let response = agent
        .get(url)
        .set(
            "User-Agent",
            concat!("dutchman/", env!("CARGO_PKG_VERSION")),
        )
        .set("Accept", "application/octet-stream, application/json")
        .call()
        .map_err(|e| format!("could not download {url}: {e}"))?;
    let host = url::Url::parse(response.get_url())
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default();
    if !(host == "github.com"
        || host.ends_with(".github.com")
        || host.ends_with(".githubusercontent.com"))
    {
        return Err(format!("{url} led to a host that is not GitHub's"));
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("could not download {url}: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{url} is larger than expected"));
    }
    Ok(bytes)
}

/// The archive extension a target's builds use.
fn extension(target: &str) -> &'static str {
    if target.contains("windows") {
        "zip"
    } else {
        "tar.gz"
    }
}

/// The builds a release offers for `target`: `name` is the archive name without extension.
fn asset(release: &Value, name: &str) -> Option<(String, String)> {
    let assets = release.get("assets")?.as_array()?;
    let url_of = |file: &str| {
        assets
            .iter()
            .find(|a| a.get("name").and_then(Value::as_str) == Some(file))
            .and_then(|a| a.get("browser_download_url"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let archive = url_of(name)?;
    let sha = url_of(&format!("{name}.sha256"))?;
    Some((archive, sha))
}

/// The builds to compare against: the newest nightly, then every candidate listed for this
/// target. A release that can't be read is left out with the reason.
pub fn builds(target: &str) -> (Vec<Build>, Vec<String>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    let ext = extension(target);
    match get(&format!("{RELEASES}/nightly"), MAX_META)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string()))
    {
        Ok(release) => {
            let name = format!("dutchman-nightly-{target}.{ext}");
            match asset(&release, &name) {
                Some((url, sha_url)) => found.push(Build {
                    label: "nightly".to_owned(),
                    title: release
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("Nightly")
                        .to_owned(),
                    url,
                    sha_url,
                }),
                None => missing.push(format!("the nightly has no build for {target}")),
            }
        }
        Err(_) => missing.push("the nightly could not be read".to_owned()),
    }
    if let Ok(release) = get(&format!("{RELEASES}/candidates"), MAX_META)
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string()))
    {
        let list = asset_bytes(&release, "candidates.json")
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        for candidate in list
            .as_ref()
            .and_then(|l| l.get("candidates"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = candidate
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| valid_name(n))
            else {
                continue;
            };
            let archive = format!("dutchman-candidate-{name}-{target}.{ext}");
            if let Some((url, sha_url)) = asset(&release, &archive) {
                found.push(Build {
                    label: name.to_owned(),
                    title: candidate
                        .get("note")
                        .and_then(Value::as_str)
                        .unwrap_or(name)
                        .to_owned(),
                    url,
                    sha_url,
                });
            }
        }
    }
    (found, missing)
}

fn asset_bytes(release: &Value, file: &str) -> Option<Vec<u8>> {
    let url = release
        .get("assets")?
        .as_array()?
        .iter()
        .find(|a| a.get("name").and_then(Value::as_str) == Some(file))?
        .get("browser_download_url")?
        .as_str()?
        .to_owned();
    get(&url, MAX_META).ok()
}

/// A candidate name: lower-case letters, digits and dashes, as the manifest requires.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The user's cache folder for Dutchman's downloaded builds.
pub fn cache_dir() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        var("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library").join("Caches"))
    } else {
        var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|home| home.join(".cache")))
    }?;
    Some(base.join("dutchman").join("builds"))
}

/// The SHA-256 a `.sha256` file names: its first word, 64 hex digits.
pub fn published_sha(text: &str) -> Option<String> {
    let word = text.split_whitespace().next()?.to_ascii_lowercase();
    (word.len() == 64 && word.chars().all(|c| c.is_ascii_hexdigit())).then_some(word)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The program from `build`, downloaded, checked and unpacked into the cache once.
pub fn fetch(build: &Build, cache: &Path) -> Result<PathBuf, String> {
    let sha = String::from_utf8(get(&build.sha_url, 4096)?)
        .ok()
        .and_then(|text| published_sha(&text))
        .ok_or("its SHA-256 file is malformed")?;
    let program = if cfg!(windows) {
        "dutchman.exe"
    } else {
        "dutchman"
    };
    let folder = cache.join(&sha);
    let path = folder.join(program);
    if path.is_file() {
        return Ok(path);
    }
    let bytes = get(&build.url, MAX_ARCHIVE)?;
    if hex(&Sha256::digest(&bytes)) != sha {
        return Err("its download does not match its published SHA-256".to_owned());
    }
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let found = if build.url.ends_with(".zip") {
        from_zip(&bytes, program)?
    } else {
        from_tar_gz(&bytes, program)?
    };
    let partial = folder.join(format!("{program}.partial"));
    std::fs::write(&partial, found).map_err(|e| format!("{}: {e}", partial.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("{}: {e}", partial.display()))?;
    }
    std::fs::rename(&partial, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// The bytes of the file called `program` in a zip archive's top folder.
pub fn from_zip(bytes: &[u8], program: &str) -> Result<Vec<u8>, String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        // An entry whose name doesn't decode is not the program.
        let wanted = file.name().is_ok_and(|name| is_program(&name, program));
        if wanted {
            let mut out = Vec::new();
            file.by_ref()
                .take(MAX_ARCHIVE)
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            return Ok(out);
        }
    }
    Err(format!("the archive holds no {program}"))
}

/// Whether an archive entry is the program itself, at the top or in one folder.
fn is_program(entry: &str, program: &str) -> bool {
    let parts: Vec<&str> = entry.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    parts.len() <= 2 && parts.last() == Some(&program)
}

/// The bytes of the file called `program` in a gzip-compressed tar archive's top folder.
pub fn from_tar_gz(bytes: &[u8], program: &str) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(MAX_ARCHIVE * 3)
        .read_to_end(&mut data)
        .map_err(|e| format!("the archive is not gzip: {e}"))?;
    let mut at = 0usize;
    let mut long_name: Option<String> = None;
    while let Some(header) = data.get(at..at + 512) {
        if header.iter().all(|b| *b == 0) {
            break;
        }
        let field = |range: std::ops::Range<usize>| {
            let raw = header.get(range).unwrap_or(&[]);
            let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(raw.get(..end).unwrap_or(&[])).into_owned()
        };
        let size = usize::from_str_radix(field(124..136).trim(), 8)
            .map_err(|_| "the archive is not a tar file".to_owned())?;
        let kind = header.get(156).copied().unwrap_or(b'0');
        let start = at + 512;
        let body = data
            .get(start..start + size)
            .ok_or("the archive is cut short")?;
        let name = match long_name.take() {
            Some(name) => name,
            None => {
                let prefix = field(345..500);
                let name = field(0..100);
                if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                }
            }
        };
        match kind {
            b'L' => {
                let end = body.iter().position(|b| *b == 0).unwrap_or(body.len());
                long_name =
                    Some(String::from_utf8_lossy(body.get(..end).unwrap_or(&[])).into_owned());
            }
            b'0' | 0 if is_program(&name, program) => return Ok(body.to_vec()),
            _ => {}
        }
        at = start + size.div_ceil(512) * 512;
    }
    Err(format!("the archive holds no {program}"))
}

/// Converts `input` with `program` into `folder`, within `limit`, and returns its report.
pub fn convert_with(
    program: &Path,
    input: &Path,
    folder: &Path,
    limit: Duration,
) -> Result<Value, String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let output = folder.join("design.sc");
    let stdout = folder.join("report.json");
    let file = std::fs::File::create(&stdout).map_err(|e| format!("{}: {e}", stdout.display()))?;
    let mut child = Command::new(program)
        .arg("convert")
        .arg(input)
        .arg("-o")
        .arg(&output)
        .args(["--force", "--json", "--quiet"])
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start it: {e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("it ran past the time limit".to_owned());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let _ = std::fs::remove_file(&output);
    let bytes = crate::report_send::bounded_read(&stdout, crate::saved_report::MAX_BYTES as usize)?;
    serde_json::from_slice(&bytes).map_err(|_| "it printed no report".to_owned())
}

/// Every build converts the design; the deltas, in the order of `builds`. A build that
/// can't be fetched or run gets a delta that says why.
pub fn compare(
    input: &Path,
    out: &Path,
    report: &Value,
    builds: &[Build],
    limit: Duration,
    on_progress: &(dyn Fn(&str) + Sync),
) -> Vec<Delta> {
    let Some(cache) = cache_dir() else {
        return builds
            .iter()
            .map(|b| skipped(b, "there is no cache folder to keep it in"))
            .collect();
    };
    builds
        .iter()
        .map(|build| {
            on_progress(&format!("comparing with the {} build", build.label));
            let program = match fetch(build, &cache) {
                Ok(program) => program,
                Err(error) => return skipped(build, &error),
            };
            let folder = out.join("builds").join(&build.label);
            match convert_with(&program, input, &folder, limit) {
                Ok(after) => delta(&build.label, &build.title, report, &after),
                Err(error) => skipped(build, &error),
            }
        })
        .collect()
}

fn skipped(build: &Build, why: &str) -> Delta {
    Delta {
        label: build.label.clone(),
        title: build.title.clone(),
        skipped: Some(why.to_owned()),
        ..Delta::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report(ok: bool, rebuilt: u64, statuses: &[(&str, &str)]) -> Value {
        let features: Vec<Value> = statuses
            .iter()
            .enumerate()
            .map(|(i, (kind, status))| json!({"ordinal": i, "kind": kind, "status": status}))
            .collect();
        json!({"ok": ok, "history": {"rebuilt": rebuilt, "needing": statuses.len(), "features": features}})
    }

    #[test]
    fn a_delta_lists_what_got_worse_and_what_got_better() {
        let before = report(
            true,
            2,
            &[
                ("sketch", "mapped"),
                ("extrude", "mapped"),
                ("fillet", "blocked"),
            ],
        );
        let after = report(
            true,
            2,
            &[
                ("sketch", "mapped"),
                ("extrude", "blocked"),
                ("fillet", "mapped"),
            ],
        );
        let delta = delta("nightly", "Nightly", &before, &after);
        assert_eq!(
            delta.worse,
            [(1, "extrude".to_owned(), "blocked".to_owned())]
        );
        assert_eq!(
            delta.better,
            [(2, "fillet".to_owned(), "blocked".to_owned())]
        );
        assert!(delta.regresses());
        let same = super::delta("nightly", "Nightly", &before, &before);
        assert!(!same.regresses() && same.better.is_empty());
        let fails = super::delta("nightly", "Nightly", &before, &json!({"ok": false}));
        assert!(fails.regresses());
    }

    #[test]
    fn published_shas_and_names_are_checked() {
        let sha = "a".repeat(64);
        assert_eq!(published_sha(&format!("{sha}  dutchman.zip\n")), Some(sha));
        assert_eq!(published_sha("not a sha"), None);
        assert!(valid_name("fix-1234"));
        assert!(!valid_name("Fix 1234"));
        assert!(!valid_name("../x"));
        assert!(is_program("dutchman-nightly-x/dutchman", "dutchman"));
        assert!(is_program("dutchman", "dutchman"));
        assert!(!is_program("a/b/dutchman", "dutchman"));
        assert!(!is_program("dutchman-nightly-x/dutchman-mcp", "dutchman"));
    }

    /// A tar entry: a ustar header, then the body padded to 512 bytes.
    fn tar_entry(name: &str, body: &[u8], kind: u8) -> Vec<u8> {
        let mut header = vec![0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        let size = format!("{:011o}\0", body.len());
        header[124..136].copy_from_slice(size.as_bytes());
        header[156] = kind;
        header[257..263].copy_from_slice(b"ustar\0");
        let mut out = header;
        out.extend_from_slice(body);
        out.resize(out.len().div_ceil(512) * 512, 0);
        out
    }

    #[test]
    fn the_program_comes_out_of_either_archive() {
        use std::io::Write;
        let mut tar = tar_entry("pkg/README.md", b"readme", b'0');
        tar.extend(tar_entry("pkg/dutchman-mcp", b"other", b'0'));
        tar.extend(tar_entry("pkg/dutchman", b"the program", b'0'));
        tar.extend([0u8; 1024]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar).unwrap();
        let gz = gz.finish().unwrap();
        assert_eq!(from_tar_gz(&gz, "dutchman").unwrap(), b"the program");
        assert!(from_tar_gz(&gz, "dutchman.exe").is_err());

        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("pkg/dutchman.exe", options).unwrap();
        zip.write_all(b"windows program").unwrap();
        let zip = zip.finish().unwrap().into_inner();
        assert_eq!(from_zip(&zip, "dutchman.exe").unwrap(), b"windows program");
    }
}
