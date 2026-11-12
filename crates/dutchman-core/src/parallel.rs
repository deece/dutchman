//! Bounded independent work, with results consumed in input order.

#[cfg(not(target_family = "wasm"))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(target_family = "wasm"))]
use std::sync::Mutex;

/// Prepares independent items on scoped workers. A missing result means the coordinator
/// must retry that item serially: a worker failed, panicked, or could not be started.
/// Preparations must leave shared caller state unchanged.
#[cfg(not(target_family = "wasm"))]
pub fn prepare_ordered<T: Sync, R: Send, E, F>(
    items: &[T],
    workers: usize,
    prepare: F,
) -> Vec<Option<R>>
where
    F: Fn(&T) -> Result<R, E> + Sync,
{
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..workers.min(items.len()) {
            let work = || {
                while let Some((item, slot)) = {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    items.get(index).zip(results.get(index))
                } {
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare(item)))
                            .ok()
                            .and_then(Result::ok);
                    if let Ok(mut slot) = slot.lock() {
                        *slot = result;
                    }
                }
            };
            if let Ok(handle) = std::thread::Builder::new().spawn_scoped(scope, work) {
                handles.push(handle);
            }
        }
        for handle in handles {
            let _ = handle.join();
        }
    });
    results
        .into_iter()
        .map(|slot| slot.into_inner().ok().flatten())
        .collect()
}

/// Serial preparation in a plugin, preserving the native retry contract.
#[cfg(target_family = "wasm")]
pub fn prepare_ordered<T: Sync, R: Send, E, F>(
    items: &[T],
    _workers: usize,
    prepare: F,
) -> Vec<Option<R>>
where
    F: Fn(&T) -> Result<R, E> + Sync,
{
    items
        .iter()
        .map(|item| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare(item)))
                .ok()
                .and_then(Result::ok)
        })
        .collect()
}

/// Conservative temporary peak for preparing a part, including STEP text, mapping copies,
/// and a SolidCraft replay process. Body count alone misses dense single-body parts.
/// The 3.5 GB serial / 8.6 GB parallel assembly peaks motivate keeping automatic work
/// small; entity count scales that allowance for detailed parts.
pub fn part_peak_bytes(entities: u64, bodies: usize) -> u64 {
    entities
        .saturating_mul(16 << 10)
        .max((bodies as u64).saturating_mul(1 << 20))
        .max(256 << 20)
}

/// Limits workers by CPU availability, item count, and current free memory. The estimate
/// covers one worker's temporary allocations, including child processes. Workers share only
/// what stays free after a margin for the rest of the conversion (a quarter of free memory,
/// at least 1 GiB), so extra workers never take memory a single worker would have needed.
/// Unknown memory keeps the CPU limit; even a budget smaller than one item allows one serial
/// worker.
pub fn workers(requested: usize, parts: usize, bytes_per_part: u64) -> usize {
    workers_with_memory(requested, parts, bytes_per_part, crate::available_memory())
}

/// The same worker limit using one free-memory snapshot for an entire preparation plan.
pub fn workers_with_memory(
    requested: usize,
    parts: usize,
    bytes_per_part: u64,
    available: Option<u64>,
) -> usize {
    #[cfg(target_family = "wasm")]
    let requested = requested.min(1);
    #[cfg(target_family = "wasm")]
    let cpus = 1;
    #[cfg(not(target_family = "wasm"))]
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    worker_limit(requested, parts, bytes_per_part, cpus, available)
}

/// The least free memory kept back from parallel workers.
const MARGIN_FLOOR: u64 = 1 << 30;

fn worker_limit(
    requested: usize,
    parts: usize,
    bytes_per_part: u64,
    cpus: usize,
    available: Option<u64>,
) -> usize {
    // The measured assembly saved only 3 s (25 s to 22 s) while parallel preparation
    // added 5.1 GB. Automatic work therefore uses at most two workers; an explicit
    // request can use more, still bounded by the per-part peak and free memory.
    let requested = if requested == 0 {
        cpus.min(2)
    } else {
        requested
    };
    let memory = available
        .map(|bytes| {
            let margin = (bytes / 4).max(MARGIN_FLOOR);
            let budget = bytes.saturating_sub(margin);
            usize::try_from(budget / bytes_per_part.max(1)).unwrap_or(usize::MAX)
        })
        .unwrap_or(requested);
    requested.min(parts).min(memory).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_leave_only_their_own_slot_for_serial_retry() {
        let items = [0, 1, 2, 3, 4, 5];
        let results = prepare_ordered(&items, 4, |&item| {
            if item == 1 {
                panic!("forced preparation panic");
            }
            if item == 3 {
                return Err("forced preparation error");
            }
            Ok(item * 2)
        });
        assert_eq!(results, [Some(0), None, Some(4), None, Some(8), Some(10)]);
    }

    #[test]
    fn workers_leave_a_margin_of_free_memory() {
        const GIB: u64 = 1 << 30;
        // 16 GiB free: a quarter (4 GiB) is kept back, 12 GiB at 1 GiB per worker.
        assert_eq!(worker_limit(32, 100, GIB, 32, Some(16 * GIB)), 12);
        // Automatic work also respects the smaller throughput/memory tradeoff.
        assert_eq!(worker_limit(0, 100, GIB, 32, Some(16 * GIB)), 2);
        // 2 GiB free: the 1 GiB floor is kept back, one worker fits.
        assert_eq!(worker_limit(0, 100, GIB, 32, Some(2 * GIB)), 1);
        // Less free than the margin still allows one serial worker.
        assert_eq!(worker_limit(0, 100, GIB, 32, Some(GIB / 2)), 1);
    }

    #[test]
    fn dense_single_body_parts_and_retained_memory_limit_automatic_work() {
        let simple = part_peak_bytes(100, 1);
        let dense = part_peak_bytes(200_000, 1);
        assert!(dense > simple);
        // A 14 GB budget with 3.5 GB held leaves 10.5 GB free. Two dense workers
        // fit with the normal margin; less headroom chooses serial work.
        assert_eq!(worker_limit(0, 256, dense, 32, Some(10_500_000_000)), 2);
        assert_eq!(worker_limit(0, 256, dense, 32, Some(5_000_000_000)), 1);
        assert_eq!(worker_limit(8, 256, dense, 32, Some(10_500_000_000)), 2);
    }

    #[test]
    fn serial_limits_allow_at_least_one_worker() {
        const GIB: u64 = 1 << 30;
        let plenty = Some(100 * GIB);
        assert_eq!(worker_limit(1, 100, 1, 8, plenty), 1);
        assert_eq!(worker_limit(0, 100, 1, 8, plenty), 2);
        assert_eq!(worker_limit(4, 100, 1, 8, None), 4);
        // 32 GiB free keeps 8 back; 24 GiB at 12 GiB per worker is two.
        assert_eq!(worker_limit(4, 100, 12 * GIB, 8, Some(32 * GIB)), 2);
        assert_eq!(worker_limit(4, 0, 1, 8, plenty), 1);
        assert_eq!(worker_limit(4, 3, u64::MAX, 8, plenty), 1);
        assert_eq!(worker_limit(0, 2, 1, 8, None), 2);
    }
}
