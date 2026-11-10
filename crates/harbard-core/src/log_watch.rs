//! Wake a reader the moment the store changes, instead of on its next poll.
//!
//! A reader that tails the log on a timer sees an append only when the timer
//! fires: at a 250 ms tick that is 125 ms late on average, every time. On
//! macOS and the BSDs the kernel can say when a file changes (kqueue), so
//! [`LogWatch::wait`] returns as soon as anything under the store root moves:
//! an append to the newest segment, a new day's segment, or one of the JSON
//! state files being replaced. Elsewhere it sleeps for the timeout and reports
//! a change, which is the old polling behaviour.
//!
//! A wake says only that something may have changed. The caller still reads
//! by byte position ([`crate::store::Store::read_since`]), so a spurious wake
//! costs one empty read and a missed one is caught by the timeout.

use crate::store::Store;
use std::time::Duration;

/// Waits for the store to change. One per reader thread.
pub struct LogWatch {
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    inner: kq::Watch,
    #[cfg(not(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )))]
    _store: Store,
}

impl LogWatch {
    /// Watch the store root, its events folder and the newest segment.
    pub fn new(store: &Store) -> std::io::Result<Self> {
        #[cfg(any(
            target_os = "macos",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        {
            Ok(LogWatch {
                inner: kq::Watch::new(store.clone())?,
            })
        }
        #[cfg(not(any(
            target_os = "macos",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        )))]
        {
            Ok(LogWatch {
                _store: store.clone(),
            })
        }
    }

    /// Block until something under the store changes or `timeout` passes.
    /// Returns `true` on a change, `false` on a quiet timeout. Without kqueue
    /// it sleeps for `timeout` and returns `true`.
    pub fn wait(&mut self, timeout: Duration) -> bool {
        #[cfg(any(
            target_os = "macos",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        {
            self.inner.wait(timeout)
        }
        #[cfg(not(any(
            target_os = "macos",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        )))]
        {
            std::thread::sleep(timeout);
            true
        }
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
mod kq {
    use crate::store::Store;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// Open a path for change notification only. On macOS `O_EVTONLY` keeps
    /// the open from holding the volume busy.
    fn open_for_events(path: &Path) -> Option<libc::c_int> {
        let c = CString::new(path.as_os_str().as_bytes()).ok()?;
        #[cfg(target_os = "macos")]
        let flags = libc::O_EVTONLY;
        #[cfg(not(target_os = "macos"))]
        let flags = libc::O_RDONLY;
        // SAFETY: `c` is a valid NUL-terminated path for the whole call.
        let fd = unsafe { libc::open(c.as_ptr(), flags | libc::O_CLOEXEC) };
        (fd >= 0).then_some(fd)
    }

    fn close(fd: libc::c_int) {
        // SAFETY: `fd` came from `open_for_events` and is closed once.
        unsafe { libc::close(fd) };
    }

    pub struct Watch {
        store: Store,
        kq: libc::c_int,
        root: Option<libc::c_int>,
        events_dir: Option<libc::c_int>,
        segment: Option<(PathBuf, libc::c_int)>,
    }

    impl Watch {
        pub fn new(store: Store) -> std::io::Result<Self> {
            // SAFETY: plain syscall; checked below.
            let kq = unsafe { libc::kqueue() };
            if kq < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut w = Watch {
                store,
                kq,
                root: None,
                events_dir: None,
                segment: None,
            };
            w.arm();
            Ok(w)
        }

        fn register(&self, fd: libc::c_int) {
            let fflags = libc::NOTE_WRITE
                | libc::NOTE_EXTEND
                | libc::NOTE_ATTRIB
                | libc::NOTE_RENAME
                | libc::NOTE_DELETE
                | libc::NOTE_LINK;
            let change = libc::kevent {
                ident: fd as libc::uintptr_t,
                filter: libc::EVFILT_VNODE,
                flags: libc::EV_ADD | libc::EV_CLEAR,
                fflags,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            // SAFETY: one change, no event buffer; `kq` and `fd` are open.
            unsafe {
                libc::kevent(
                    self.kq,
                    &change,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                );
            }
        }

        /// (Re)open what is missing and follow the newest segment. Closing a
        /// descriptor removes its registration, so nothing is left behind.
        fn arm(&mut self) {
            if self.root.is_none()
                && let Some(fd) = open_for_events(self.store.root())
            {
                self.register(fd);
                self.root = Some(fd);
            }
            if self.events_dir.is_none()
                && let Some(fd) = open_for_events(&self.store.events_dir())
            {
                self.register(fd);
                self.events_dir = Some(fd);
            }
            let newest = self.store.segments().into_iter().next_back();
            let current = self.segment.as_ref().map(|(p, _)| p.clone());
            if newest != current {
                if let Some((_, fd)) = self.segment.take() {
                    close(fd);
                }
                if let Some(path) = newest
                    && let Some(fd) = open_for_events(&path)
                {
                    self.register(fd);
                    self.segment = Some((path, fd));
                }
            }
        }

        pub fn wait(&mut self, timeout: Duration) -> bool {
            self.arm();
            let ts = libc::timespec {
                tv_sec: timeout.as_secs() as libc::time_t,
                tv_nsec: timeout.subsec_nanos() as libc::c_long,
            };
            // SAFETY: `kevent` is plain data; zeroed is a valid empty value.
            let mut out: [libc::kevent; 8] = unsafe { std::mem::zeroed() };
            // SAFETY: the buffer outlives the call and its length is passed.
            let n = unsafe {
                libc::kevent(
                    self.kq,
                    std::ptr::null(),
                    0,
                    out.as_mut_ptr(),
                    out.len() as libc::c_int,
                    &ts,
                )
            };
            if n <= 0 {
                return false;
            }
            // A deleted or renamed folder is reopened on the next wait.
            for ev in &out[..n as usize] {
                if ev.fflags & (libc::NOTE_DELETE | libc::NOTE_RENAME) != 0 {
                    let fd = ev.ident as libc::c_int;
                    if self.root == Some(fd) {
                        self.root = None;
                        close(fd);
                    } else if self.events_dir == Some(fd) {
                        self.events_dir = None;
                        close(fd);
                    }
                }
            }
            true
        }
    }

    impl Drop for Watch {
        fn drop(&mut self) {
            for fd in [self.root, self.events_dir].into_iter().flatten() {
                close(fd);
            }
            if let Some((_, fd)) = self.segment.take() {
                close(fd);
            }
            close(self.kq);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, EventKind, NoteKind, Role, SCHEMA_VERSION};
    use crate::test_util::TempHome;
    use std::time::Instant;

    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    fn note(text: &str) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: format!(
                "{:026}",
                COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            ts: "2026-07-15T10:00:00Z".to_string(),
            event: EventKind::Note,
            session: "s".to_string(),
            harness: "codex".to_string(),
            host: "h".to_string(),
            cwd: "/tmp".to_string(),
            repo: None,
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: text.to_string(),
            to: None,
            kind: NoteKind::Fyi,
        }
    }

    #[test]
    fn an_append_wakes_the_watch_before_the_timeout() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        store.append(&note("first")).unwrap();
        let mut watch = LogWatch::new(&store).unwrap();
        while watch.wait(Duration::from_millis(50)) {}
        let writer = {
            let store = store.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                store.append(&note("second")).unwrap();
            })
        };
        let started = Instant::now();
        let woke = watch.wait(Duration::from_secs(5));
        writer.join().unwrap();
        assert!(woke, "the append should wake the watch");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "woke by the append, not the timeout"
        );
    }

    #[test]
    fn a_quiet_store_times_out() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        store.append(&note("only")).unwrap();
        let mut watch = LogWatch::new(&store).unwrap();
        while watch.wait(Duration::from_millis(50)) {}
        #[cfg(any(
            target_os = "macos",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        assert!(!watch.wait(Duration::from_millis(100)));
    }
}
