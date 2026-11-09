//! Minimal throwaway temp directory for tests, so the crate keeps a `tempfile`
//! dev-dependency out of the graph. Unique per process + atomic counter.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp directory removed on drop. Use as a harbard `HARBARD_HOME`.
pub struct TempHome {
    path: PathBuf,
}

impl TempHome {
    pub fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("harbard-test-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp home");
        TempHome { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Default for TempHome {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
