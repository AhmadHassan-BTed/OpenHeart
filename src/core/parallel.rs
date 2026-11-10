//! Minimal data-parallel helpers on scoped std threads (no external runtime).
//!
//! Work is distributed dynamically through an atomic cursor, so long files do not stall
//! a statically assigned chunk, and results are returned in input order, which keeps
//! every downstream id assignment deterministic regardless of the thread count.
//!
//! On `wasm32` (the WebAssembly build has no threads) every helper runs on the calling
//! thread.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Whether this target can spawn threads at all.
pub const THREADS_AVAILABLE: bool = !cfg!(target_arch = "wasm32");

/// Resolves a requested worker count (0 = one per available core) against the amount of
/// work, never returning less than 1 or more than `work_items`. Always 1 on `wasm32`.
pub fn resolve_worker_count(requested: usize, work_items: usize) -> usize {
    if !THREADS_AVAILABLE {
        return 1;
    }
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let wanted = if requested == 0 { available } else { requested };
    wanted.max(1).min(work_items.max(1))
}

/// Applies `f` to every item on up to `workers` threads and returns the results in input
/// order. `init` builds per-worker state (for example a parser) that is reused for all
/// items that worker processes. A panic in any worker is propagated to the caller.
pub fn parallel_map_with<T, S, R, I, F>(items: &[T], workers: usize, init: I, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    I: Fn() -> S + Sync,
    F: Fn(&mut S, usize, &T) -> R + Sync,
{
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    let workers = if THREADS_AVAILABLE { workers.max(1).min(n) } else { 1 };
    if workers == 1 {
        let mut state = init();
        return items
            .iter()
            .enumerate()
            .map(|(i, item)| f(&mut state, i, item))
            .collect();
    }

    let next = AtomicUsize::new(0);
    let per_worker: Vec<Vec<(usize, R)>> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut state = init();
                    let mut local = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= n {
                            break;
                        }
                        local.push((i, f(&mut state, i, &items[i])));
                    }
                    local
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| match h.join() {
                Ok(results) => results,
                Err(panic) => std::panic::resume_unwind(panic),
            })
            .collect()
    });

    let mut slots: Vec<Option<R>> = (0..n).map(|_| None).collect();
    for results in per_worker {
        for (i, r) in results {
            slots[i] = Some(r);
        }
    }
    slots
        .into_iter()
        .map(|slot| slot.expect("the atomic work cursor hands out every index exactly once"))
        .collect()
}
