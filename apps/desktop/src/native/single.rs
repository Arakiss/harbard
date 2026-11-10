//! One harbard at a time. A second launch hands over to the first (which
//! shows its main window) and exits before opening anything.
//!
//! Same socket and wire as the old shell's single-instance plugin
//! (`/tmp/<bundle id with . and - as _>_si.sock`; the client writes its cwd,
//! `\0\0`, then its arguments joined by `\0`), so an old build and this one
//! still exclude each other during the switch. A dev build that turns the
//! feature on uses its own socket and never reaches the installed app.

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The installed app's bundle id.
pub const BUNDLE_ID: &str = "com.harbard.desktop";
/// What a dev build calls itself on the socket.
pub const DEV_ID: &str = "com.harbard.desktop.dev";

pub fn socket_path(identifier: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/{}_si.sock", identifier.replace(['.', '-'], "_")))
}

/// What the launch found.
pub enum Claim {
    /// This is the only harbard: it listens for the next launch.
    First(UnixListener),
    /// Another harbard was told to come forward; this one must exit.
    Handed,
    /// The socket could not be used either way; run anyway.
    Alone,
}

pub fn claim(socket: &Path) -> Claim {
    match UnixStream::connect(socket) {
        Ok(mut stream) => {
            let cwd = std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let args = std::env::args().collect::<Vec<_>>().join("\0");
            let _ = stream.write_all(format!("{cwd}\0\0{args}").as_bytes());
            Claim::Handed
        }
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
            let _ = std::fs::remove_file(socket);
            match UnixListener::bind(socket) {
                Ok(l) => Claim::First(l),
                Err(_) => Claim::Alone,
            }
        }
        Err(_) => Claim::Alone,
    }
}

/// Accept later launches on a thread; each one calls `on_launch`.
pub fn listen(listener: UnixListener, on_launch: impl Fn() + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("harbard-single".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut sink = String::new();
                let _ = stream.read_to_string(&mut sink);
                on_launch();
            }
        });
}

pub fn release(socket: &Path) {
    let _ = std::fs::remove_file(socket);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_is_the_old_shells_and_a_dev_build_has_its_own() {
        assert_eq!(
            socket_path(BUNDLE_ID),
            PathBuf::from("/tmp/com_harbard_desktop_si.sock")
        );
        assert_ne!(socket_path(DEV_ID), socket_path(BUNDLE_ID));
    }

    #[test]
    fn a_second_launch_hands_over_to_the_first() {
        let _fork = crate::native::FORK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let socket = std::env::temp_dir().join(format!("hb-si-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let Claim::First(listener) = claim(&socket) else {
            panic!("the first launch owns the socket");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        listen(listener, move || {
            let _ = tx.send(());
        });
        assert!(matches!(claim(&socket), Claim::Handed));
        assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).is_ok());
        release(&socket);
    }

    #[test]
    fn a_socket_left_by_a_crash_is_taken_over() {
        let _fork = crate::native::FORK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let socket = std::env::temp_dir().join(format!("hb-si-stale-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        drop(UnixListener::bind(&socket).unwrap());
        assert!(socket.exists());
        assert!(matches!(claim(&socket), Claim::First(_)));
        release(&socket);
    }
}
