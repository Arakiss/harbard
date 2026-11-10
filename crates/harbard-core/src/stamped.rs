//! Small files read on every view, parsed again only when they change.
//!
//! A workspace read opens a couple of hundred cursor files whose contents
//! rarely move between two reads. A stat is far cheaper than an open, a read
//! and a parse, so each file's parsed value is kept beside the stamp it was
//! read under (device, inode, size, modification and change times, to the
//! nanosecond) and read again only when the stamp moves. A file is stamped
//! before it is read: a write that lands between the two leaves the newer
//! contents under the older stamp, which the next read sees as a change.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What tells one version of a file from the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl Stamp {
    pub fn of(meta: &fs::Metadata) -> Stamp {
        Stamp {
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
            ctime: (meta.ctime(), meta.ctime_nsec()),
        }
    }

    /// The stamp of `path`, following links. `None` when it cannot be read.
    pub fn read(path: &Path) -> Option<Stamp> {
        fs::metadata(path).ok().map(|m| Stamp::of(&m))
    }
}

/// Above this many files the cache starts over: it only ever holds what
/// the reads ask for, so this is a bound for a pathological store.
const MAX_FILES: usize = 8_192;

/// Parsed values by path, each kept under the stamp it was read with.
pub struct Stamped<T> {
    files: Mutex<HashMap<PathBuf, (Stamp, T)>>,
}

impl<T> Default for Stamped<T> {
    fn default() -> Self {
        Stamped {
            files: Mutex::new(HashMap::new()),
        }
    }
}

impl<T: Clone> Stamped<T> {
    /// `read(path)`, or the value it gave last time when `path` has the
    /// same stamp. A file that cannot be stamped is read directly.
    pub fn get(&self, path: &Path, read: impl FnOnce(&Path) -> T) -> T {
        let Some(stamp) = Stamp::read(path) else {
            return read(path);
        };
        if let Some((held, v)) = self
            .files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(path)
            && *held == stamp
        {
            return v.clone();
        }
        let v = read(path);
        let mut files = self.files.lock().unwrap_or_else(|e| e.into_inner());
        if files.len() >= MAX_FILES {
            files.clear();
        }
        files.insert(path.to_path_buf(), (stamp, v.clone()));
        v
    }
}

/// A JSON file parsed into `T` (missing or unparsable reads as the
/// default), kept in `cache` while its stamp holds.
pub fn json<T>(cache: &Stamped<T>, path: &Path) -> T
where
    T: serde::de::DeserializeOwned + Default + Clone,
{
    cache.get(path, |p| {
        fs::read_to_string(p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    })
}

/// A process-wide [`Stamped`] cache, for a `static`.
pub type Shared<T> = std::sync::LazyLock<Stamped<T>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn a_file_is_read_again_only_when_it_changes() {
        let dir = crate::test_util::TempHome::new();
        let path = dir.path().join("cursor");
        let cache: Stamped<String> = Stamped::default();
        let reads = Cell::new(0);
        let read = |p: &Path| {
            reads.set(reads.get() + 1);
            fs::read_to_string(p).unwrap_or_default()
        };
        // Missing: read directly every time, never kept.
        assert_eq!(cache.get(&path, read), "");
        assert_eq!(cache.get(&path, read), "");
        assert_eq!(reads.get(), 2);
        fs::write(&path, "one").unwrap();
        assert_eq!(cache.get(&path, read), "one");
        assert_eq!(cache.get(&path, read), "one");
        assert_eq!(reads.get(), 3);
        // Rewritten in place with the same length: a new stamp.
        fs::write(&path, "two").unwrap();
        assert_eq!(cache.get(&path, read), "two");
        // Replaced by a rename.
        let tmp = dir.path().join("tmp");
        fs::write(&tmp, "three").unwrap();
        fs::rename(&tmp, &path).unwrap();
        assert_eq!(cache.get(&path, read), "three");
        assert_eq!(reads.get(), 5);
    }
}
