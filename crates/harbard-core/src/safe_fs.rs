//! File access that cannot be raced into another file.
//!
//! "Check the path, then open the path" is a TOCTOU hole: between the two,
//! any process running as the user can rename a symlink to `~/Documents/x`
//! over the checked name, and the open follows it. Every function here
//! instead walks from a directory it opened itself, one component at a time,
//! with `O_NOFOLLOW` on each step, and decides on the descriptor it holds:
//!
//! - directories are opened `O_DIRECTORY | O_NOFOLLOW` and walked with
//!   `openat`, so a component swapped for a symlink is refused, not followed;
//! - a file is opened `O_NOFOLLOW | O_NONBLOCK` (a FIFO never blocks the
//!   reader), then `fstat`ed: it must be a regular file with one link
//!   (`nlink == 1`: a hardlink to a file elsewhere names that file's bytes
//!   without naming its path) and no larger than the cap; the read is from
//!   that descriptor, `take(cap + 1)`, so a file that grows is still capped;
//! - writes create with `O_CREAT | O_EXCL | O_NOFOLLOW` inside a directory
//!   descriptor; a replacement is written to a fresh temp name and moved
//!   with `renameat`, a removal is `unlinkat`, both relative to the
//!   descriptor, never to a path re-resolved later.
//!
//! The root passed in is trusted to be the caller's own folder (the harbard
//! home); only its last component is refused when it is a symlink.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::path::Path;

/// Why an access was refused.
#[derive(Debug)]
pub enum SafeError {
    /// Nothing there (or a component is missing).
    NotFound,
    /// There, but not what is allowed: a symlink, a hardlink, a directory
    /// where a file was expected, a FIFO, a device, a bad name.
    Refused(&'static str),
    /// Larger than the cap.
    TooLarge,
    Io(io::Error),
}

impl std::fmt::Display for SafeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SafeError::NotFound => write!(f, "not found"),
            SafeError::Refused(why) => write!(f, "refused: {why}"),
            SafeError::TooLarge => write!(f, "too large"),
            SafeError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SafeError {}

impl From<nix::errno::Errno> for SafeError {
    fn from(e: nix::errno::Errno) -> Self {
        use nix::errno::Errno;
        match e {
            Errno::ENOENT => SafeError::NotFound,
            // O_NOFOLLOW on a symlink is ELOOP; O_DIRECTORY on a file ENOTDIR.
            Errno::ELOOP => SafeError::Refused("symlink"),
            Errno::ENOTDIR => SafeError::Refused("not a directory"),
            Errno::EISDIR => SafeError::Refused("a directory"),
            other => SafeError::Io(io::Error::from_raw_os_error(other as i32)),
        }
    }
}

type Result<T> = std::result::Result<T, SafeError>;

/// One path component: not empty, not `.` or `..`, no `/` and no NUL.
pub fn is_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\0')
}

fn component(name: &str) -> Result<&str> {
    if is_component(name) {
        Ok(name)
    } else {
        Err(SafeError::Refused("not a single path component"))
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use nix::fcntl::{OFlag, openat, renameat};
    use nix::sys::stat::{Mode, SFlag, fstat, mkdirat};
    use nix::unistd::{UnlinkatFlags, unlinkat};
    use std::os::fd::AsFd;

    const DIR: OFlag = OFlag::O_RDONLY
        .union(OFlag::O_DIRECTORY)
        .union(OFlag::O_NOFOLLOW)
        .union(OFlag::O_CLOEXEC);

    /// Open `path` as a directory; its last component must not be a symlink.
    pub fn open_dir(path: &Path) -> Result<OwnedFd> {
        Ok(nix::fcntl::open(path, DIR, Mode::empty())?)
    }

    /// Open the directory `name` inside `dir`, never through a symlink.
    pub fn open_dir_at(dir: &OwnedFd, name: &str) -> Result<OwnedFd> {
        Ok(openat(dir.as_fd(), component(name)?, DIR, Mode::empty())?)
    }

    /// Open the regular, singly linked file `name` inside `dir` for reading.
    pub fn open_file_at(dir: &OwnedFd, name: &str) -> Result<File> {
        let fd = openat(
            dir.as_fd(),
            component(name)?,
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?;
        let st = fstat(fd.as_fd())?;
        let kind = SFlag::from_bits_truncate(st.st_mode as _) & SFlag::S_IFMT;
        if kind != SFlag::S_IFREG {
            return Err(SafeError::Refused("not a regular file"));
        }
        if st.st_nlink != 1 {
            return Err(SafeError::Refused("hardlinked"));
        }
        Ok(File::from(fd))
    }

    /// The size the descriptor reports (the file, not whatever the path
    /// names a moment later).
    pub fn len_of(file: &File) -> Result<u64> {
        Ok(fstat(file.as_fd())?.st_size as u64)
    }

    /// Create `name` inside `dir`; it must not exist, and is never a symlink.
    pub fn create_new_at(dir: &OwnedFd, name: &str, mode: u32) -> Result<File> {
        let fd = openat(
            dir.as_fd(),
            component(name)?,
            OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::from_bits_truncate(mode as _),
        )?;
        Ok(File::from(fd))
    }

    /// Make the directory `name` inside `dir` and open it.
    pub fn mkdir_at(dir: &OwnedFd, name: &str, mode: u32) -> Result<OwnedFd> {
        mkdirat(
            dir.as_fd(),
            component(name)?,
            Mode::from_bits_truncate(mode as _),
        )?;
        open_dir_at(dir, name)
    }

    pub fn rename_at(dir: &OwnedFd, from: &str, to: &str) -> Result<()> {
        Ok(renameat(
            dir.as_fd(),
            component(from)?,
            dir.as_fd(),
            component(to)?,
        )?)
    }

    pub fn remove_at(dir: &OwnedFd, name: &str) -> Result<()> {
        Ok(unlinkat(
            dir.as_fd(),
            component(name)?,
            UnlinkatFlags::NoRemoveDir,
        )?)
    }
}

#[cfg(unix)]
pub use imp::{
    create_new_at, len_of, mkdir_at, open_dir, open_dir_at, open_file_at, remove_at, rename_at,
};

/// Read all of an open file, refusing more than `cap` bytes.
pub fn read_capped(mut file: File, cap: u64) -> Result<Vec<u8>> {
    if len_of(&file)? > cap {
        return Err(SafeError::TooLarge);
    }
    let mut buf = Vec::new();
    (&mut file)
        .take(cap + 1)
        .read_to_end(&mut buf)
        .map_err(SafeError::Io)?;
    if buf.len() as u64 > cap {
        return Err(SafeError::TooLarge);
    }
    Ok(buf)
}

/// Walk `root` / `dirs` / `name` and open the file, every step by descriptor.
pub fn open_under(root: &Path, dirs: &[&str], name: &str) -> Result<File> {
    let mut dir = open_dir(root)?;
    for d in dirs {
        dir = open_dir_at(&dir, d)?;
    }
    open_file_at(&dir, name)
}

/// The bytes of `root` / `dirs` / `name`, at most `cap`.
pub fn read_under(root: &Path, dirs: &[&str], name: &str, cap: u64) -> Result<Vec<u8>> {
    read_capped(open_under(root, dirs, name)?, cap)
}

/// The size of `root` / `dirs` / `name`, by the descriptor.
pub fn len_under(root: &Path, dirs: &[&str], name: &str) -> Result<u64> {
    len_of(&open_under(root, dirs, name)?)
}

/// Write `bytes` as `name` inside `dir` without ever following a link: to a
/// fresh temp name created exclusively, then `renameat` over `name`.
pub fn replace_at(dir: &OwnedFd, name: &str, bytes: &[u8], mode: u32) -> Result<()> {
    component(name)?;
    let tmp = format!(".{name}.{}.tmp", ulid::Ulid::generate());
    let mut f = create_new_at(dir, &tmp, mode)?;
    let written = f.write_all(bytes).and_then(|()| f.sync_all());
    drop(f);
    if let Err(e) = written {
        let _ = remove_at(dir, &tmp);
        return Err(SafeError::Io(e));
    }
    rename_at(dir, &tmp, name).inspect_err(|_| {
        let _ = remove_at(dir, &tmp);
    })
}

/// Create `name` inside `dir` with `bytes`; it must not exist already.
pub fn write_new_at(dir: &OwnedFd, name: &str, bytes: &[u8], mode: u32) -> Result<()> {
    let mut f = create_new_at(dir, name, mode)?;
    if let Err(e) = f.write_all(bytes) {
        drop(f);
        let _ = remove_at(dir, name);
        return Err(SafeError::Io(e));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn a_plain_file_is_read_by_descriptor_and_capped() {
        let home = TempHome::new();
        fs::create_dir_all(home.path().join("a/b")).unwrap();
        fs::write(home.path().join("a/b/f.txt"), b"hello").unwrap();
        assert_eq!(
            read_under(home.path(), &["a", "b"], "f.txt", 10).unwrap(),
            b"hello"
        );
        assert_eq!(len_under(home.path(), &["a", "b"], "f.txt").unwrap(), 5);
        assert!(matches!(
            read_under(home.path(), &["a", "b"], "f.txt", 3),
            Err(SafeError::TooLarge)
        ));
        assert!(matches!(
            read_under(home.path(), &["a", "b"], "missing", 10),
            Err(SafeError::NotFound)
        ));
        for bad in ["..", ".", "x/y", ""] {
            assert!(
                matches!(
                    read_under(home.path(), &["a"], bad, 10),
                    Err(SafeError::Refused(_))
                ),
                "{bad}"
            );
            assert!(
                matches!(
                    read_under(home.path(), &[bad], "f.txt", 10),
                    Err(SafeError::Refused(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn symlinks_hardlinks_fifos_and_directories_are_refused() {
        let home = TempHome::new();
        let outside = TempHome::new();
        fs::write(outside.path().join("secret.png"), b"OUTSIDE").unwrap();
        fs::create_dir_all(home.path().join("d")).unwrap();
        // A symlinked file.
        symlink(
            outside.path().join("secret.png"),
            home.path().join("d/link.png"),
        )
        .unwrap();
        assert!(matches!(
            read_under(home.path(), &["d"], "link.png", 99),
            Err(SafeError::Refused("symlink"))
        ));
        // A hardlink to a file elsewhere: nlink == 2.
        fs::hard_link(
            outside.path().join("secret.png"),
            home.path().join("d/hard.png"),
        )
        .unwrap();
        assert!(matches!(
            read_under(home.path(), &["d"], "hard.png", 99),
            Err(SafeError::Refused("hardlinked"))
        ));
        // A symlinked directory component.
        symlink(outside.path(), home.path().join("linked")).unwrap();
        assert!(matches!(
            read_under(home.path(), &["linked"], "secret.png", 99),
            Err(SafeError::Refused(_))
        ));
        // The root itself as a symlink.
        assert!(matches!(
            read_under(&home.path().join("linked"), &[], "secret.png", 99),
            Err(SafeError::Refused(_))
        ));
        // A directory where a file is expected, and a FIFO that would block.
        assert!(matches!(
            read_under(home.path(), &[], "d", 99),
            Err(SafeError::Refused(_))
        ));
        nix::unistd::mkfifo(
            &home.path().join("d/pipe.png"),
            nix::sys::stat::Mode::S_IRWXU,
        )
        .unwrap();
        assert!(matches!(
            read_under(home.path(), &["d"], "pipe.png", 99),
            Err(SafeError::Refused("not a regular file"))
        ));
    }

    /// The race the review found (H1): another process keeps swapping the
    /// checked name between a regular file and a symlink to a file outside.
    /// No read ever returns the outside bytes.
    #[test]
    fn a_symlink_swapped_in_while_reading_never_leaks_the_target() {
        let home = TempHome::new();
        let outside = TempHome::new();
        fs::write(outside.path().join("secret.png"), b"OUTSIDE-SECRET").unwrap();
        let dir = home.path().join("att");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.png"), b"inside").unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let swapper = {
            let (dir, secret, stop) =
                (dir.clone(), outside.path().join("secret.png"), stop.clone());
            std::thread::spawn(move || {
                let mut n = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let tmp_link = dir.join(format!("l{n}"));
                    let tmp_file = dir.join(format!("f{n}"));
                    let _ = symlink(&secret, &tmp_link);
                    let _ = fs::rename(&tmp_link, dir.join("a.png"));
                    let _ = fs::write(&tmp_file, b"inside");
                    let _ = fs::rename(&tmp_file, dir.join("a.png"));
                    n += 1;
                }
                n
            })
        };
        let (mut inside, mut refused) = (0, 0);
        for _ in 0..3_000 {
            match read_under(home.path(), &["att"], "a.png", 1 << 20) {
                Ok(bytes) => {
                    assert_ne!(bytes, b"OUTSIDE-SECRET", "a swap leaked the target");
                    inside += 1;
                }
                Err(_) => refused += 1,
            }
        }
        stop.store(true, Ordering::Relaxed);
        let swaps = swapper.join().unwrap();
        assert!(swaps > 0 && inside + refused == 3_000);
        // A directory swapped for a symlink mid-walk is refused too.
        let stop = Arc::new(AtomicBool::new(false));
        let swapper = {
            let (home_p, out, stop) = (
                home.path().to_path_buf(),
                outside.path().to_path_buf(),
                stop.clone(),
            );
            std::thread::spawn(move || {
                let mut n = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let l = home_p.join(format!("dl{n}"));
                    let _ = symlink(&out, &l);
                    let _ = fs::rename(home_p.join("att"), home_p.join(format!("real{n}")));
                    let _ = fs::rename(&l, home_p.join("att"));
                    let _ = fs::rename(home_p.join("att"), home_p.join(format!("gone{n}")));
                    let _ = fs::rename(home_p.join(format!("real{n}")), home_p.join("att"));
                    n += 1;
                }
            })
        };
        fs::copy(
            outside.path().join("secret.png"),
            outside.path().join("a.png"),
        )
        .unwrap();
        for _ in 0..3_000 {
            if let Ok(bytes) = read_under(home.path(), &["att"], "a.png", 1 << 20) {
                assert_ne!(
                    bytes, b"OUTSIDE-SECRET",
                    "a directory swap leaked the target"
                );
            }
        }
        stop.store(true, Ordering::Relaxed);
        swapper.join().unwrap();
    }

    #[test]
    fn writes_create_exclusively_and_replace_by_rename_never_through_a_link() {
        let home = TempHome::new();
        let outside = TempHome::new();
        let root = open_dir(home.path()).unwrap();
        let d = mkdir_at(&root, "x", 0o700).unwrap();
        write_new_at(&d, "a.md", b"# uno", 0o600).unwrap();
        assert!(
            write_new_at(&d, "a.md", b"# dos", 0o600).is_err(),
            "never overwrites"
        );
        // A symlink planted at the name is replaced, its target untouched.
        fs::write(outside.path().join("victim"), b"keep").unwrap();
        symlink(outside.path().join("victim"), home.path().join("x/m.json")).unwrap();
        assert!(write_new_at(&d, "m.json", b"{}", 0o600).is_err());
        replace_at(&d, "m.json", b"{\"v\":1}", 0o600).unwrap();
        assert_eq!(fs::read(outside.path().join("victim")).unwrap(), b"keep");
        assert_eq!(
            read_under(home.path(), &["x"], "m.json", 99).unwrap(),
            b"{\"v\":1}"
        );
        let left: Vec<String> = fs::read_dir(home.path().join("x"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(left.iter().all(|n| !n.ends_with(".tmp")), "{left:?}");
        remove_at(&d, "a.md").unwrap();
        assert!(!home.path().join("x/a.md").exists());
    }
}
