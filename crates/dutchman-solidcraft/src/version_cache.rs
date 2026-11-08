//! Best-effort CLI version caching. No design data is stored here.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

use super::{parse_version, probe_version};

type Answer = Result<Option<String>, String>;
static VERSIONS: OnceLock<Mutex<HashMap<PathBuf, Answer>>> = OnceLock::new();
const MAX_BYTES: u64 = 64 * 1024;
const MAX_ENTRIES: usize = 32;

pub(super) fn version(cli: &Path, live: bool) -> Answer {
    version_at(cli, live, cache_path())
}

fn version_at(cli: &Path, live: bool, cache: Option<PathBuf>) -> Answer {
    let cli = resolve(cli);
    // Hold the lock through discovery so concurrent conversions share one probe too.
    let mut versions = VERSIONS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(answer) = versions.get(&cli) {
        return answer.clone();
    }
    let key = identity(&cli);
    if let Some(line) = cache.as_deref().and_then(|path| disk_version(path, &key)) {
        let answer = Ok(Some(line));
        versions.insert(cli, answer.clone());
        return answer;
    }
    if !live {
        return Ok(None);
    }
    let started = std::time::Instant::now();
    let probed = probe_version(&cli);
    // A first start can be slow (the system scanning a fresh install); a probe that ran out of
    // time is asked again next time rather than remembered as "no version" for the whole session.
    let timed_out = matches!(probed, Ok(None)) && started.elapsed() >= super::VERSION_TIMEOUT;
    if let (Some(path), Ok(Some((line, true)))) = (cache, &probed) {
        if parse_version(line).is_some() && identity(&cli) == key {
            let _ = save(&path, key, line);
        }
    }
    let answer = probed.map(|answer| answer.map(|(line, _)| line));
    if !timed_out {
        versions.insert(cli, answer.clone());
    }
    answer
}

fn resolve(cli: &Path) -> PathBuf {
    let path = if cli.components().count() == 1 && !cli.is_absolute() {
        std::env::var_os("PATH")
            .and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|folder| folder.join(cli))
                    .find(|path| path.is_file())
            })
            .unwrap_or_else(|| cli.to_owned())
    } else {
        cli.to_owned()
    };
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn cache_path() -> Option<PathBuf> {
    let env_path = |name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let folder = if cfg!(windows) {
        env_path("LOCALAPPDATA")?.join("Dutchman")
    } else if cfg!(target_os = "macos") {
        env_path("HOME")?.join("Library/Caches/Dutchman")
    } else {
        env_path("XDG_CACHE_HOME")
            .or_else(|| env_path("HOME").map(|home| home.join(".cache")))?
            .join("dutchman")
    };
    Some(folder.join("solidcraft-versions.json"))
}

fn identity(cli: &Path) -> Option<Value> {
    let metadata = std::fs::metadata(cli).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(json!({
        "path": cli.to_str()?,
        "size": metadata.len(),
        "modified": modified.as_nanos().to_string(),
    }))
}

fn entries(path: &Path) -> Option<Vec<Value>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let entries: Vec<Value> = serde_json::from_slice(&bytes).ok()?;
    (entries.len() <= MAX_ENTRIES).then_some(entries)
}

fn disk_version(path: &Path, key: &Option<Value>) -> Option<String> {
    let key = key.as_ref()?;
    entries(path)?.into_iter().find_map(|entry| {
        if entry.get("cli")? != key {
            return None;
        }
        let line = entry.get("version")?.as_str()?;
        parse_version(line)?;
        Some(line.to_owned())
    })
}

fn save(path: &Path, key: Option<Value>, line: &str) -> Option<()> {
    let key = key?;
    let mut entries = entries(path).unwrap_or_default();
    entries.retain(|entry| entry.get("cli") != Some(&key));
    entries.truncate(MAX_ENTRIES - 1);
    entries.insert(0, json!({"cli": key, "version": line}));
    let bytes = serde_json::to_vec(&entries).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let parent = path.parent()?;
    std::fs::create_dir_all(parent).ok()?;
    let mut file = tempfile::NamedTempFile::new_in(parent).ok()?;
    std::io::Write::write_all(&mut file, &bytes).ok()?;
    file.persist(path).ok()?;
    Some(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn script(folder: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let cli = folder.join(name);
        std::fs::write(
            &cli,
            format!("#!/bin/sh\necho run >> \"$0.count\"\n{body}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::canonicalize(cli).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn process_cache_shares_success_and_no_answer_across_callers() {
        let _turn = crate::testing::scripts();
        let dir = tempfile::tempdir().unwrap();
        for (name, body, expected) in [
            (
                "healthy",
                "echo 'solidcraft-cli 0.26.0'",
                Some("solidcraft-cli 0.26.0".to_owned()),
            ),
            ("silent", "exit 0", None),
        ] {
            let cli = script(dir.path(), name, body);
            let first = std::thread::spawn({
                let cli = cli.clone();
                move || version_at(&cli, true, None)
            });
            assert_eq!(version_at(&cli, true, None), Ok(expected.clone()));
            assert_eq!(first.join().unwrap(), Ok(expected.clone()));
            assert_eq!(version_at(&cli, false, None), Ok(expected));
            assert_eq!(
                std::fs::read_to_string(cli.with_extension("count")).unwrap(),
                "run\n"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn disk_hit_and_cache_only_lookup_never_spawn() {
        let _turn = crate::testing::scripts();
        let dir = tempfile::tempdir().unwrap();
        let cli = script(dir.path(), "cli", "echo 'solidcraft-cli 0.26.0'");
        let cache = dir.path().join("cache.json");
        assert_eq!(version_at(&cli, false, Some(cache.clone())), Ok(None));
        assert!(!cli.with_extension("count").exists());
        save(&cache, identity(&cli), "solidcraft-cli 0.25.1").unwrap();
        assert_eq!(
            version_at(&cli, false, Some(cache)),
            Ok(Some("solidcraft-cli 0.25.1".to_owned()))
        );
        assert_eq!(
            version_at(&cli, true, None),
            Ok(Some("solidcraft-cli 0.25.1".to_owned()))
        );
        assert!(!cli.with_extension("count").exists());
    }

    #[cfg(unix)]
    #[test]
    fn only_successful_versions_are_persisted_and_bad_cache_probes_live() {
        let _turn = crate::testing::scripts();
        let dir = tempfile::tempdir().unwrap();
        for (name, body, persists) in [
            ("healthy", "echo 'solidcraft-cli 0.26.0'", true),
            ("failed", "echo 'solidcraft-cli 0.26.0'; exit 3", false),
            ("garbage", "echo unknown", false),
        ] {
            let cli = script(dir.path(), name, body);
            let cache = dir.path().join(format!("{name}.json"));
            std::fs::write(&cache, "garbage").unwrap();
            assert!(version_at(&cli, true, Some(cache.clone()))
                .unwrap()
                .is_some());
            assert_eq!(disk_version(&cache, &identity(&cli)).is_some(), persists);
            assert_eq!(
                std::fs::read_to_string(cli.with_extension("count")).unwrap(),
                "run\n"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn timeout_is_bounded_and_asked_again_next_time() {
        let _turn = crate::testing::scripts();
        let dir = tempfile::tempdir().unwrap();
        let cli = script(dir.path(), "sleepy", "exec sleep 30");
        let cache = dir.path().join("cache.json");
        let start = std::time::Instant::now();
        assert_eq!(version_at(&cli, true, Some(cache.clone())), Ok(None));
        // The script sleeps 30 s: finishing well before that proves the probe was cut off,
        // with room for a slow runner to start and stop it.
        assert!(
            start.elapsed() < super::super::VERSION_TIMEOUT + std::time::Duration::from_secs(10),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(version_at(&cli, true, Some(cache.clone())), Ok(None));
        assert!(!cache.exists());
        assert_eq!(
            std::fs::read_to_string(cli.with_extension("count")).unwrap(),
            "run\nrun\n"
        );
    }

    #[test]
    fn disk_cache_matches_path_size_and_modification_time() {
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("cli");
        let cache = dir.path().join("versions.json");
        std::fs::write(&cli, "one").unwrap();
        let key = identity(&cli);
        save(&cache, key.clone(), "solidcraft-cli 0.26.0").unwrap();
        assert_eq!(
            disk_version(&cache, &key).as_deref(),
            Some("solidcraft-cli 0.26.0")
        );
        std::fs::File::options()
            .write(true)
            .open(&cli)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(10))
            .unwrap();
        assert!(disk_version(&cache, &identity(&cli)).is_none());
        save(&cache, identity(&cli), "solidcraft-cli 0.26.0").unwrap();
        std::fs::write(&cli, "longer").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&cli)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(10))
            .unwrap();
        assert!(disk_version(&cache, &identity(&cli)).is_none());
        assert!(disk_version(&cache, &identity(&dir.path().join("other"))).is_none());
        for bytes in [b"garbage".to_vec(), vec![b' '; MAX_BYTES as usize + 1]] {
            std::fs::write(&cache, bytes).unwrap();
            assert!(disk_version(&cache, &key).is_none());
        }
    }
}
