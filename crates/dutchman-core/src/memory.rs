//! Available memory for choosing bounded part concurrency.

/// Bytes currently available to this process, when the operating system reports them.
/// On Linux, use the smaller of MemAvailable and remaining cgroup v2 headroom,
/// including ancestor limits (for example a systemd scope's MemoryMax).
pub fn available_memory() -> Option<u64> {
    // Share the debug-only memory fixture with post-read checks, so preflight and part
    // workers can be exercised on a synthetic low-memory system too. Releases measure.
    #[cfg(debug_assertions)]
    if let Some(bytes) = std::env::var("DUTCHMAN_TEST_MEMORY").ok().and_then(|text| {
        let (held, available) = text.split_once(',')?;
        held.parse::<u64>().ok()?;
        available.parse::<u64>().ok()
    }) {
        return Some(bytes);
    }
    platform_available_memory()
}

#[cfg(target_os = "linux")]
fn platform_available_memory() -> Option<u64> {
    linux_available_memory(
        std::path::Path::new("/proc/meminfo"),
        std::path::Path::new("/proc/self/cgroup"),
        std::path::Path::new("/sys/fs/cgroup"),
    )
}

#[cfg(target_os = "linux")]
fn linux_available_memory(
    meminfo: &std::path::Path,
    groups: &std::path::Path,
    root: &std::path::Path,
) -> Option<u64> {
    let host = std::fs::read_to_string(meminfo).ok().and_then(|text| {
        text.lines().find_map(|line| {
            let rest = line.strip_prefix("MemAvailable:")?;
            rest.split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()?
                .checked_mul(1024)
        })
    });
    let mut available = host;
    if let Ok(groups) = std::fs::read_to_string(groups) {
        if let Some(group) = groups.lines().find_map(|line| line.strip_prefix("0::")) {
            let mut path = root.join(group.trim_start_matches('/'));
            while path.starts_with(root) {
                let read = |name| {
                    std::fs::read_to_string(path.join(name))
                        .ok()?
                        .trim()
                        .parse::<u64>()
                        .ok()
                };
                if let (Some(max), Some(current)) = (read("memory.max"), read("memory.current")) {
                    // File cache counts in `memory.current` and is given back when needed; a
                    // conversion reads large files, so it would look like memory running out.
                    let cache = std::fs::read_to_string(path.join("memory.stat"))
                        .map_or(0, |stat| reclaimable_file_cache(&stat));
                    let remaining = max.saturating_sub(current.saturating_sub(cache));
                    available = Some(available.map_or(remaining, |host| host.min(remaining)));
                }
                if !path.pop() {
                    break;
                }
            }
        }
    }
    available
}

/// The file cache a cgroup's `memory.stat` reports.
#[cfg(target_os = "linux")]
fn reclaimable_file_cache(stat: &str) -> u64 {
    stat.lines()
        .filter_map(|line| line.split_once(' '))
        .filter(|(name, _)| matches!(*name, "active_file" | "inactive_file"))
        .filter_map(|(_, bytes)| bytes.trim().parse::<u64>().ok())
        .fold(0, u64::saturating_add)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn platform_available_memory() -> Option<u64> {
    let mut system = sysinfo::System::new();
    system.refresh_memory_specifics(sysinfo::MemoryRefreshKind::nothing().with_ram());
    choose_available(
        system.total_memory(),
        system.used_memory(),
        system.available_memory(),
        system.free_memory(),
    )
}

/// What can be used now, from the figures `sysinfo` reports.
///
/// On macOS, `sysinfo` computes available memory as free + inactive + purgeable pages minus the
/// pages the memory compressor occupies. On a machine that is compressing, the subtraction
/// leaves almost nothing (a Mac with half its memory unused reported 0.02 GB), and "free" pages
/// alone are small by design. Total minus used (active, wired, compressed) is the figure the
/// system itself calls free plus cached, so the larger of the two is taken.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn choose_available(total: u64, used: u64, available: u64, free: u64) -> Option<u64> {
    let unused = total.saturating_sub(used);
    let best = available.max(unused);
    let best = if best == 0 { free } else { best };
    (best > 0).then_some(best)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_available_memory() -> Option<u64> {
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    #[test]
    fn available_memory_ignores_a_compressor_that_swallows_the_free_pages() {
        // An 8 GB Mac: 0.02 GB available by sysinfo, 2.84 GB neither used nor compressed.
        let gb = 1_000_000_000;
        assert_eq!(
            super::choose_available(8_590 * gb / 1000, 5_750 * gb / 1000, gb / 50, 270_000_000),
            Some(8_590 * gb / 1000 - 5_750 * gb / 1000)
        );
        // Where sysinfo's figure is the larger, it stands.
        assert_eq!(
            super::choose_available(16 * gb, 10 * gb, 7 * gb, gb),
            Some(7 * gb)
        );
        // Nothing measurable falls back to free pages, then to unknown.
        assert_eq!(super::choose_available(8 * gb, 8 * gb, 0, gb), Some(gb));
        assert_eq!(super::choose_available(0, 0, 0, 0), None);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_cgroup_headroom_gives_back_file_cache() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let child = root.join("scope");
        std::fs::create_dir_all(&child).unwrap();
        let meminfo = root.join("meminfo");
        std::fs::write(&meminfo, "MemAvailable: 1000000 kB\n").unwrap();
        let groups = root.join("cgroup");
        std::fs::write(&groups, "0::/scope\n").unwrap();
        std::fs::write(child.join("memory.max"), "1000000").unwrap();
        std::fs::write(child.join("memory.current"), "900000").unwrap();
        std::fs::write(
            child.join("memory.stat"),
            "anon 100000\nfile 800000\nactive_file 300000\ninactive_file 500000\n",
        )
        .unwrap();
        assert_eq!(
            super::linux_available_memory(&meminfo, &groups, root),
            Some(900_000)
        );
        // More cache than `memory.current` shows, in a race between the two reads, is no room.
        std::fs::write(child.join("memory.current"), "100").unwrap();
        assert_eq!(
            super::linux_available_memory(&meminfo, &groups, root),
            Some(1_000_000)
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_prefers_cgroup_headroom_including_ancestors() {
        let dir = tempfile::tempdir().unwrap();
        let meminfo = dir.path().join("meminfo");
        let groups = dir.path().join("cgroup");
        let root = dir.path().join("groups");
        let child = root.join("scope/worker");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(&meminfo, "MemAvailable: 1000 kB\n").unwrap();
        std::fs::write(&groups, "0::/scope/worker\n").unwrap();
        std::fs::write(child.join("memory.max"), "900000").unwrap();
        std::fs::write(child.join("memory.current"), "100000").unwrap();
        let read = || super::linux_available_memory(&meminfo, &groups, &root);
        assert_eq!(read(), Some(800000));
        std::fs::write(root.join("scope/memory.max"), "500000").unwrap();
        std::fs::write(root.join("scope/memory.current"), "200000").unwrap();
        assert_eq!(read(), Some(300000));
        std::fs::write(&meminfo, "MemAvailable: 100 kB\n").unwrap();
        assert_eq!(read(), Some(102400));
        std::fs::write(root.join("scope/memory.max"), "max").unwrap();
        assert_eq!(read(), Some(102400));
        std::fs::write(child.join("memory.current"), "900001").unwrap();
        assert_eq!(read(), Some(0));
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn available_memory_is_plausible() {
        let bytes = super::available_memory().unwrap();
        assert!(bytes > 0 && bytes < 1_000_000_000_000_000);
    }
}
