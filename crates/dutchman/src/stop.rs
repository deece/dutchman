//! Stopping a conversion early, with a message, when this machine cannot finish it.
//!
//! Once a design is read the memory it holds is measured, not estimated. Mapping and writing
//! it needs more on top (the work Dutchman does on a decoded model keeps further copies of
//! it), so a machine with less free than a share of what the design already holds stops here
//! instead of running for hours and then running out. The message names the largest parts by
//! their `dutchman diagnose` numbers and their share of the memory.

use dutchman_core::{parts, Model};

/// What mapping and writing a decoded model needs on top of what it holds, as a share of it.
/// This existing post-read mapping margin is separate from the calibrated preflight
/// stages and check allowance. A swap file does not count as free.
pub const EXTRA_SHARE: f64 = 0.15;

/// Additional checking memory from the same metadata model the desktop displays.
/// Unknown metadata leaves checking enabled. The read model's resident copies are
/// already allocated when free memory is measured; do not reserve them again.
pub fn history_check_bytes(
    input: &std::path::Path,
    options: &crate::ConvertOptions,
) -> Option<u64> {
    crate::choices::history_check_estimate(input, options).map(|estimate| estimate.peak_bytes)
}

/// Shared card and report text for a check skipped to keep the design usable.
pub fn history_check_message(needed: u64, available: u64) -> String {
    format!(
        "History check skipped: needs about {:.1} GB, {:.1} GB free",
        crate::choices::gb(needed),
        crate::choices::gb(available)
    )
}

/// A design holding less than this is never stopped: its extra work cannot matter.
pub const MIN_HELD: u64 = 1 << 30;

/// Free memory below which a running conversion that holds at least [`MIN_HELD`] stops. An
/// estimate from a file's size can be far off in either direction, so this measures instead.
pub const LOW_MEMORY_FLOOR: u64 = 1 << 30;

/// Whether free memory has fallen below the floor. Cheap: it reads one system figure, so it can
/// be asked often; measuring what a conversion holds is slower and waits for this to say yes.
pub fn memory_is_low(available: Option<u64>) -> bool {
    #[cfg(debug_assertions)]
    if std::env::var_os("DUTCHMAN_TEST_LOW_MEMORY").is_some() {
        return true;
    }
    available.is_some_and(|available| available < LOW_MEMORY_FLOOR) || os_says_critical()
}

/// What `kern.memorystatus_vm_pressure_level` reports at critical pressure.
#[cfg(any(target_os = "macos", test))]
const MACOS_CRITICAL_PRESSURE: u32 = 4;

/// Free space on the boot volume below which a Mac has no room left to swap to.
#[cfg(any(target_os = "macos", test))]
const MACOS_LOW_DISK: u64 = 2 << 30;

/// Whether a Mac's own figures say memory has run out: it compresses and swaps instead of
/// reporting little free, so its free memory alone doesn't show it. `pressure` is the text of
/// `sysctl -n kern.memorystatus_vm_pressure_level`.
#[cfg(any(target_os = "macos", test))]
fn macos_critical(pressure: Option<&str>, boot_volume_free: Option<u64>) -> bool {
    pressure.and_then(|text| text.trim().parse::<u32>().ok()) == Some(MACOS_CRITICAL_PRESSURE)
        || boot_volume_free.is_some_and(|free| free < MACOS_LOW_DISK)
}

/// [`macos_critical`], measured at most once a second, since it starts a program.
#[cfg(target_os = "macos")]
fn os_says_critical() -> bool {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LAST: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let Ok(mut last) = LAST.lock() else {
        return false;
    };
    if let Some((at, critical)) = *last {
        if at.elapsed() < Duration::from_secs(1) {
            return critical;
        }
    }
    let pressure = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.memorystatus_vm_pressure_level"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok());
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let boot_volume_free = disks
        .iter()
        .find(|disk| disk.mount_point() == std::path::Path::new("/"))
        .map(sysinfo::Disk::available_space);
    let critical = macos_critical(pressure.as_deref(), boot_volume_free);
    *last = Some((Instant::now(), critical));
    critical
}

#[cfg(not(target_os = "macos"))]
fn os_says_critical() -> bool {
    false
}

/// The message to stop a running conversion with, when what all running conversions hold
/// together (`total`) means they are what is using the memory.
pub fn low_memory(total: u64, own: u64, available: Option<u64>) -> Option<String> {
    #[cfg(debug_assertions)]
    if std::env::var_os("DUTCHMAN_TEST_LOW_MEMORY").is_some() {
        return Some(low_memory_message(2_000_000_000, 500_000_000));
    }
    let available = available?;
    (total >= MIN_HELD && (available < LOW_MEMORY_FLOOR || os_says_critical()))
        .then(|| low_memory_message(own, available))
}

/// Which running conversion stops when memory runs low: the one holding the most, so the others
/// can finish. Equal holdings stop the first, so exactly one is chosen.
pub fn stops_first(holders: &std::collections::BTreeMap<u32, u64>, own: u32) -> bool {
    holders
        .iter()
        .max_by_key(|(pid, held)| (**held, std::cmp::Reverse(**pid)))
        .is_some_and(|(pid, _)| *pid == own)
}

/// How a report says a conversion stopped because memory ran low.
const LOW_MEMORY_STOP: &str = "stopped before this computer ran out of memory";

/// Whether this report is of a conversion stopped because memory ran low.
pub fn stopped_for_memory(report: &serde_json::Value) -> bool {
    report
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|message| message.starts_with(LOW_MEMORY_STOP))
}

/// Records on a report that its file was stopped for memory alongside others, and converted
/// again alone.
pub fn note_retried_alone(report: &mut serde_json::Value) {
    let converted = report.get("ok") == Some(&serde_json::json!(true));
    let Some(fields) = report.as_object_mut() else {
        return;
    };
    fields.insert(
        "retried_after_low_memory".to_owned(),
        serde_json::json!(true),
    );
    if converted {
        let note = serde_json::json!({
            "level": "Info", "subject": "memory", "stage": "input", "code": null, "upstream": null,
            "message": "ran out of memory alongside other files, then converted on its own",
        });
        if let Some(losses) = fields
            .get_mut("losses")
            .and_then(serde_json::Value::as_array_mut)
        {
            losses.push(note);
        }
    }
}

fn low_memory_message(held: u64, available: u64) -> String {
    format!(
        "{LOW_MEMORY_STOP}: {:.1} GB left free while this conversion held {:.1} GB. Close other programs and try again, or use --preset fast",
        crate::choices::gb(available),
        crate::choices::gb(held)
    )
}

/// How many of the largest parts the message names.
const NAMED: usize = 3;

/// The message to stop with, when `held` bytes (what this process holds with the design
/// read) and `available` bytes (still free) say the conversion cannot finish.
pub fn check(model: &Model, held: Option<u64>, available: Option<u64>) -> Option<String> {
    let (held, available) = (held?, available?);
    stop_message(held, available, &parts::catalog(model))
}

/// [`check`] over a catalog of parts.
pub fn stop_message(held: u64, available: u64, catalog: &[parts::PartInfo]) -> Option<String> {
    if held < MIN_HELD {
        return None;
    }
    let extra = (held as f64 * EXTRA_SHARE) as u64;
    if extra <= available {
        return None;
    }
    let gb = |bytes: u64| bytes as f64 / 1_000_000_000.0;
    let mut text = format!(
        "this conversion needs about {:.1} GB more memory and {:.1} GB are free (the design \
         already takes {:.1} GB)",
        gb(extra),
        gb(available),
        gb(held)
    );
    let total: u64 = catalog
        .iter()
        .filter(|part| !part.assembly)
        .map(|part| part.entities)
        .sum();
    let mut largest: Vec<_> = catalog
        .iter()
        .filter(|part| !part.assembly && part.entities > 0)
        .collect();
    largest.sort_by(|a, b| b.entities.cmp(&a.entities).then(a.number.cmp(&b.number)));
    largest.truncate(NAMED);
    if total > 0 && !largest.is_empty() {
        let named: Vec<String> = largest
            .iter()
            .map(|part| {
                let share = (held as f64 * part.entities as f64 / total as f64) as u64;
                format!("{} (about {:.1} GB)", part.number, gb(share))
            })
            .collect();
        text.push_str(&format!(
            "; part{} {} {} the largest",
            if named.len() == 1 { "" } else { "s" },
            join(&named),
            if named.len() == 1 { "is" } else { "are" }
        ));
    }
    text.push_str(
        ". Run `dutchman diagnose` on the file to see which parts fail, run slowly or take the \
         most memory, or convert one part with --part N (--no-memory-check converts anyway)",
    );
    Some(text)
}

fn join(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first, last] => format!("{first} and {last}"),
        [init @ .., last] => format!("{}, and {last}", init.join(", ")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_mac_is_critical_on_its_own_pressure_level_or_a_full_boot_volume() {
        const GB: u64 = 1 << 30;
        assert!(macos_critical(Some("4\n"), Some(100 * GB)));
        for level in ["1\n", "2\n", "0", "", "not a number"] {
            assert!(!macos_critical(Some(level), Some(100 * GB)), "{level:?}");
        }
        assert!(macos_critical(Some("1\n"), Some(2 * GB - 1)));
        assert!(!macos_critical(Some("1\n"), Some(2 * GB)));
        assert!(!macos_critical(None, None));
    }

    #[test]
    fn workers_count_together_and_the_largest_stops_alone() {
        const GB: u64 = 1 << 30;
        // Four workers under the floor each still add up to a conversion's worth.
        let small = GB * 9 / 10;
        assert!(low_memory(small * 4, small, Some(GB / 2)).is_some());
        assert!(low_memory(GB / 2, GB / 2, Some(GB / 2)).is_none());
        assert!(low_memory(GB * 8, GB * 8, Some(GB)).is_none());
        let holders = [(10, GB), (11, 3 * GB), (12, 2 * GB)].into_iter().collect();
        assert_eq!(
            [10, 11, 12].map(|pid| stops_first(&holders, pid)),
            [false, true, false]
        );
        let equal = [(10, GB), (11, GB)].into_iter().collect();
        assert_eq!([10, 11].map(|pid| stops_first(&equal, pid)), [true, false]);
    }

    fn part(number: usize, entities: u64, assembly: bool) -> parts::PartInfo {
        parts::PartInfo {
            number,
            parent: None,
            depth: 0,
            assembly,
            bodies: 1,
            entities,
            subtree_entities: entities,
            name: format!("secret name {number}"),
        }
    }

    const GB: u64 = 1_000_000_000;

    #[test]
    fn history_check_allowance_uses_the_displayed_cost() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("synthetic.ipt");
        std::fs::write(&input, [0; 1024]).unwrap();
        let options = crate::ConvertOptions::default();
        let sizes = crate::choices::read_sizes(&input).unwrap();
        let parts = crate::choices::desktop_estimate_parts(sizes, &options, true).unwrap();
        assert_eq!(
            history_check_bytes(&input, &options),
            parts.check.map(|e| e.peak_bytes)
        );
        assert_eq!(
            history_check_message(500_000_000, 2 * GB),
            "History check skipped: needs about 0.5 GB, 2.0 GB free"
        );
        assert_eq!(
            history_check_bytes(&dir.path().join("missing.f3d"), &options),
            None
        );
    }

    #[test]
    fn a_machine_with_room_is_not_stopped() {
        let catalog = [part(1, 10, false)];
        assert_eq!(stop_message(8 * GB, 20 * GB, &catalog), None);
        // A small design is never stopped, however little is free.
        assert_eq!(stop_message(GB / 2, 0, &catalog), None);
    }

    #[test]
    fn the_message_names_the_largest_parts_by_number_and_never_by_name() {
        let catalog = [
            part(1, 100, false),
            part(2, 5, true),
            part(3, 300, false),
            part(4, 50, false),
            part(17, 550, false),
        ];
        let text = stop_message(10 * GB, GB, &catalog).unwrap();
        assert!(
            text.starts_with("this conversion needs about 1.5 GB more memory and 1.0 GB are free")
        );
        assert!(
            text.contains(
                "parts 17 (about 5.5 GB), 3 (about 3.0 GB), and 1 (about 1.0 GB) are the largest"
            ),
            "{text}"
        );
        assert!(text.contains("dutchman diagnose"), "{text}");
        assert!(text.contains("--part N"), "{text}");
        assert!(!text.contains("secret"), "{text}");
    }

    #[test]
    fn two_parts_read_as_a_pair() {
        let catalog = [part(4, 60, false), part(17, 90, false)];
        let text = stop_message(10 * GB, 0, &catalog).unwrap();
        assert!(
            text.contains("parts 17 (about 6.0 GB) and 4 (about 4.0 GB) are the largest"),
            "{text}"
        );
    }
}
