//! Whether a transcript root can be read now, without ever waiting on macOS.
//!
//! On macOS a directory on a removable volume is gated by the privacy
//! database: the first `opendir` by a binary that has no decision yet blocks
//! inside the kernel until the operator answers the system dialog. Nobody
//! answers while the operator is away, so such an open must never run on a
//! request thread or under the projector's lock.
//!
//! [`readable`] makes that first open on a detached thread and waits at most
//! [`PROBE_WAIT`]. An open still blocked means "not readable yet": the caller
//! reads nothing under that root and the view says the root was not found.
//! No second probe starts while one is blocked, so a gated volume costs one
//! parked thread, not one per request. When the operator answers, the blocked
//! open returns, the answer is recorded, and the next call sees it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

/// Longest a caller waits for a probe before treating the root as unreadable.
pub const PROBE_WAIT: Duration = Duration::from_millis(1500);

/// Roots whose probe is still blocked in the kernel. Answers are not
/// cached: an open that returns costs one short-lived thread, and a root
/// created or granted a moment ago is seen on the next call.
fn pending() -> &'static Mutex<HashSet<PathBuf>> {
    static PENDING: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashSet::new()))
}

/// True when `dir` can be listed now. Never blocks longer than [`PROBE_WAIT`].
pub fn readable(dir: &Path) -> bool {
    readable_with(dir, PROBE_WAIT, |p| std::fs::read_dir(p).is_ok())
}

/// True while an open of `dir` is still waiting on the operator.
pub fn waiting(dir: &Path) -> bool {
    pending()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(dir)
}

fn readable_with<F>(dir: &Path, wait: Duration, open: F) -> bool
where
    F: FnOnce(&Path) -> bool + Send + 'static,
{
    let key = dir.to_path_buf();
    if !pending()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.clone())
    {
        return false;
    }
    let (tx, rx) = mpsc::channel();
    let probe_key = key.clone();
    let spawned = thread::Builder::new()
        .name("harbard-reach".into())
        .spawn(move || {
            let readable = open(&probe_key);
            pending()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&probe_key);
            let _ = tx.send(readable);
        });
    if spawned.is_err() {
        pending()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
        return false;
    }
    rx.recv_timeout(wait).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    fn unique(name: &str) -> PathBuf {
        PathBuf::from(format!("/reach-test/{name}/{}", std::process::id()))
    }

    #[test]
    fn an_open_that_never_returns_costs_one_wait_and_one_thread() {
        let dir = unique("gated");
        let (release, gate) = mpsc::channel::<()>();
        let opens = Arc::new(AtomicUsize::new(0));
        let counted = opens.clone();
        let started = Instant::now();
        assert!(!readable_with(&dir, Duration::from_millis(50), move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = gate.recv();
            true
        }));
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(waiting(&dir));
        // Later calls answer at once and start no second open.
        for _ in 0..5 {
            let again = Instant::now();
            assert!(!readable_with(&dir, Duration::from_secs(5), |_| panic!(
                "second open"
            )));
            assert!(again.elapsed() < Duration::from_millis(100));
        }
        assert_eq!(opens.load(Ordering::SeqCst), 1);
        // The operator answers: the parked open returns and the root is open again.
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while waiting(&dir) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!waiting(&dir));
        assert!(readable_with(&dir, Duration::from_secs(1), |_| true));
    }

    #[test]
    fn a_refused_root_is_unreadable_and_a_present_one_readable() {
        assert!(!readable_with(&unique("refused"), PROBE_WAIT, |_| false));
        let here = std::env::temp_dir();
        assert!(readable(&here));
        assert!(!readable(&here.join("harbard-reach-missing-dir-for-test")));
    }
}
