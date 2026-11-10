//! Isolated, bounded-stack-safe parallel execution of per-function work (Phases 4-6).
//!
//! Every job runs on a worker thread created with an explicit, large stack, so analyses
//! whose recursion depth follows AST nesting cannot overflow the (1 MiB on Windows) main
//! or server thread. Each job is wrapped in `catch_unwind`, so a panic in one function is
//! reported as an error for that job instead of tearing down the process. Results are
//! returned in input order, which keeps the merged artifacts deterministic.
//!
//! `wasm32` has no threads: every job runs on the calling thread, whose stack the module
//! reserves at link time (`-zstack-size`, see `.cargo/config.toml`). `catch_unwind` is a
//! no-op there (panics abort), so isolation reduces to the jobs' own error results.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Stack size for analysis worker threads.
pub const WORKER_STACK_BYTES: usize = 64 * 1024 * 1024;

/// Whether worker threads can be spawned on this target.
const THREADS_AVAILABLE: bool = !cfg!(target_arch = "wasm32");

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

fn run_isolated<T, R, F>(item: &T, f: &F) -> Result<R, String>
where
    F: Fn(&T) -> Result<R, String>,
{
    match catch_unwind(AssertUnwindSafe(|| f(item))) {
        Ok(r) => r,
        Err(payload) => Err(format!("panic: {}", panic_message(payload.as_ref()))),
    }
}

/// Applies `f` to every item on a pool of large-stack worker threads and returns the
/// results in input order. Falls back to the calling thread only when no worker thread
/// can be spawned at all.
pub fn par_map<T, R, F>(items: &[T], f: F) -> Vec<Result<R, String>>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> Result<R, String> + Sync,
{
    if items.is_empty() {
        return Vec::new();
    }
    if !THREADS_AVAILABLE {
        return items.iter().map(|item| run_isolated(item, &f)).collect();
    }

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, items.len());
    let next = AtomicUsize::new(0);
    let f_ref = &f;
    let next_ref = &next;

    let mut slots: Vec<Option<Result<R, String>>> = (0..items.len()).map(|_| None).collect();

    let any_spawned = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let builder = std::thread::Builder::new()
                .name("openheart-analysis".to_string())
                .stack_size(WORKER_STACK_BYTES);
            let spawned = builder.spawn_scoped(scope, move || {
                let mut local: Vec<(usize, Result<R, String>)> = Vec::new();
                loop {
                    let idx = next_ref.fetch_add(1, Ordering::Relaxed);
                    if idx >= items.len() {
                        break;
                    }
                    local.push((idx, run_isolated(&items[idx], f_ref)));
                }
                local
            });
            match spawned {
                Ok(h) => handles.push(h),
                Err(_) => break,
            }
        }
        let any = !handles.is_empty();
        for h in handles {
            match h.join() {
                Ok(local) => {
                    for (idx, r) in local {
                        slots[idx] = Some(r);
                    }
                }
                Err(payload) => {
                    // A worker can only unwind outside `run_isolated` if the result
                    // bookkeeping itself panicked; its unfinished items are retried below.
                    let _ = panic_message(payload.as_ref());
                }
            }
        }
        any
    });

    if !any_spawned {
        for (idx, item) in items.iter().enumerate() {
            slots[idx] = Some(run_isolated(item, &f));
        }
    }

    slots
        .into_iter()
        .enumerate()
        .map(|(idx, slot)| match slot {
            Some(r) => r,
            None => run_isolated(&items[idx], &f),
        })
        .collect()
}

/// Runs a single closure on a large-stack worker thread with panic isolation. On `wasm32`
/// it runs on the calling thread (the module's stack is sized at link time).
pub fn run_on_large_stack<R, F>(f: F) -> Result<R, String>
where
    R: Send,
    F: FnOnce() -> Result<R, String> + Send,
{
    if !THREADS_AVAILABLE {
        return match catch_unwind(AssertUnwindSafe(f)) {
            Ok(r) => r,
            Err(payload) => Err(format!("panic: {}", panic_message(payload.as_ref()))),
        };
    }
    std::thread::scope(|scope| {
        let builder = std::thread::Builder::new()
            .name("openheart-analysis".to_string())
            .stack_size(WORKER_STACK_BYTES);
        match builder.spawn_scoped(scope, move || match catch_unwind(AssertUnwindSafe(f)) {
            Ok(r) => r,
            Err(payload) => Err(format!("panic: {}", panic_message(payload.as_ref()))),
        }) {
            Ok(handle) => match handle.join() {
                Ok(r) => r,
                Err(payload) => Err(format!("panic: {}", panic_message(payload.as_ref()))),
            },
            Err(e) => Err(format!("failed to spawn analysis worker thread: {}", e)),
        }
    })
}
