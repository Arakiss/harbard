//! A copy of the harness transcripts harbard does not own.
//!
//! Everything on the bus is append-only and permanent, but the conversation
//! between an agent and its lead lives in transcripts the harness writes and
//! may delete: Claude Code under `~/.claude/projects`, Codex under
//! `~/.codex/sessions` and `~/.codex/archived_sessions` (the roots
//! [`TraceRoots`] names). Off by default. When `{home}/archive.json` names a
//! directory (`{"dir": "<absolute path>"}`), a sync copies them there,
//! mirroring the source:
//!
//! ```text
//! <dir>/claude/projects/<path under the Claude projects root>
//! <dir>/codex/sessions/<path under the Codex sessions root>
//! <dir>/codex/archived_sessions/<path under the Codex archived root>
//! ```
//!
//! A sync never deletes anything from the archive. Per regular file:
//!
//! - missing in the archive: copied;
//! - larger in the source, or the same size and newer: refreshed;
//! - smaller in the source (rewritten, not appended): the archive copy is
//!   kept as `<name>.rewritten-<ULID>` and the new one copied beside it;
//! - gone from the source: the archive copy stays;
//! - unchanged: one `lstat` on each side, nothing opened, nothing read.
//!
//! Every copy is written to a fresh temp name in its directory, given the
//! source's modification time, and renamed into place. On macOS it is an
//! APFS clone (`fclonefileat`): an archive on the same volume shares blocks
//! with the source until either changes. When cloning is unsupported or
//! crosses volumes it is an ordinary copy. Never a hardlink: [`crate::safe_fs`]
//! and the team projector refuse files with more than one link, so a
//! hardlinked transcript would vanish from the live views.
//!
//! Both trees are walked by descriptor (`openat` with `O_NOFOLLOW`), as in
//! `safe_fs`: a source root that is itself a symlink is followed, any symlink
//! below it is skipped and counted, and nothing in the archive is written
//! through a link. The archive dir may not sit inside a source root.

use crate::harness_trace::TraceRoots;
use crate::safe_fs::{self, SafeError};
use fs2::FileExt;
use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag, openat};
use nix::sys::stat::{FileStat, Mode, SFlag, fstat, fstatat};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, FileTimes, Permissions};
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// `{home}/archive.json`: `{"dir": "<absolute path>"}`. Absent means off.
pub const CONFIG_FILE: &str = "archive.json";
/// `{home}/archive.state.json`: what the last run did.
pub const STATE_FILE: &str = "archive.state.json";
/// `{home}/archive.lock`: held for the length of a run, so two never overlap.
pub const LOCK_FILE: &str = "archive.lock";
/// How many of the last errors the state keeps.
pub const ERRORS_KEPT: usize = 20;
const CONFIG_CAP: u64 = 64 * 1024;
const STATE_CAP: u64 = 1024 * 1024;
const MAX_DEPTH: usize = 32;
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
/// Same size and newer by more than this counts as changed. A copy carries
/// the source's time to the nanosecond on APFS; the slack keeps an archive on
/// a coarser file system (FAT keeps 2 s) from being refreshed on every run.
const MTIME_SLACK_NS: i128 = 2_000_000_000;

/// One transcript tree and where it lands in the archive.
#[derive(Debug, Clone)]
pub struct Source {
    /// The path under the archive dir, e.g. `["claude", "projects"]`.
    pub dest: [&'static str; 2],
    pub root: PathBuf,
}

impl Source {
    /// `claude/projects`, `codex/sessions`, `codex/archived_sessions`.
    pub fn label(&self) -> String {
        self.dest.join("/")
    }
}

/// The three trees, from the same roots the trace reader uses.
pub fn sources(roots: &TraceRoots) -> [Source; 3] {
    [
        Source {
            dest: ["claude", "projects"],
            root: roots.claude_projects.clone(),
        },
        Source {
            dest: ["codex", "sessions"],
            root: roots.codex_sessions.clone(),
        },
        Source {
            dest: ["codex", "archived_sessions"],
            root: roots.codex_archived.clone(),
        },
    ]
}

/// What `{home}/archive.json` holds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub dir: PathBuf,
}

/// The configuration, `None` when the archive is off.
pub fn load_config(home: &Path) -> Result<Option<Config>, String> {
    let path = home.join(CONFIG_FILE);
    let bytes = match safe_fs::read_under(home, &[], CONFIG_FILE, CONFIG_CAP) {
        Ok(bytes) => bytes,
        Err(SafeError::NotFound) => return Ok(None),
        Err(e) => {
            return Err(format!(
                "{} cannot be read ({e}); run `harbard archive disable`, then `harbard archive enable --dir <path>`",
                path.display()
            ));
        }
    };
    let config: Config = serde_json::from_slice(&bytes).map_err(|e| {
        format!(
            "{} is not {{\"dir\": \"<absolute path>\"}} ({e}); run `harbard archive enable --dir <path>` to rewrite it",
            path.display()
        )
    })?;
    if !config.dir.is_absolute() {
        return Err(format!(
            "{} names a relative dir ({}); run `harbard archive enable --dir <absolute path>`",
            path.display(),
            config.dir.display()
        ));
    }
    Ok(Some(config))
}

/// Refuse an archive dir that is relative, inside a source root, or that a
/// source root sits inside of (it would copy into itself either way).
/// Compared both as written and with symlinks resolved.
pub fn check_dir(dir: &Path, sources: &[Source]) -> Result<(), String> {
    if !dir.is_absolute() {
        return Err(format!(
            "the archive dir must be an absolute path, not {}",
            dir.display()
        ));
    }
    let dir_lex = lexical(dir);
    let dir_real = resolved(&dir_lex);
    for source in sources {
        let root_lex = lexical(&source.root);
        let root_real = resolved(&root_lex);
        for d in [&dir_lex, &dir_real] {
            for r in [&root_lex, &root_real] {
                if d.starts_with(r) {
                    return Err(format!(
                        "{} is inside {}, a folder it copies from; choose a directory outside the transcript folders",
                        dir.display(),
                        source.root.display()
                    ));
                }
                if r.starts_with(d.join(source.dest[0])) {
                    return Err(format!(
                        "{} would hold {} inside its own copy; choose another directory",
                        dir.display(),
                        source.root.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// `.` dropped and `..` applied, without touching the disk.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The longest existing ancestor with symlinks resolved, plus the rest.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(real) = fs::canonicalize(&cur) {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (cur.file_name().map(|n| n.to_os_string()), cur.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                cur = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Validate `dir`, create it (0700), and write `{home}/archive.json`.
pub fn enable(home: &Path, dir: &Path, roots: &TraceRoots) -> Result<Config, String> {
    let srcs = sources(roots);
    check_dir(dir, &srcs)?;
    let dir = lexical(dir);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(&dir)
        .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    if !fs::metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return Err(format!("{} is not a directory", dir.display()));
    }
    // Once it exists its real path is known; check that one too.
    check_dir(&dir, &srcs)?;
    fs::create_dir_all(home).map_err(|e| format!("cannot create {}: {e}", home.display()))?;
    let config = Config { dir };
    let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    let home_fd = safe_fs::open_dir(home).map_err(|e| format!("{}: {e}", home.display()))?;
    safe_fs::replace_at(&home_fd, CONFIG_FILE, &bytes, FILE_MODE)
        .map_err(|e| format!("cannot write {}: {e}", home.join(CONFIG_FILE).display()))?;
    Ok(config)
}

/// Remove `{home}/archive.json`. The archive itself is left as it is.
/// `Ok(false)` when it was already off.
pub fn disable(home: &Path) -> Result<bool, String> {
    let home_fd = match safe_fs::open_dir(home) {
        Ok(fd) => fd,
        Err(SafeError::NotFound) => return Ok(false),
        Err(e) => return Err(format!("{}: {e}", home.display())),
    };
    match safe_fs::remove_at(&home_fd, CONFIG_FILE) {
        Ok(()) => Ok(true),
        Err(SafeError::NotFound) => Ok(false),
        Err(e) => Err(format!(
            "cannot remove {}: {e}",
            home.join(CONFIG_FILE).display()
        )),
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// What the last run did (`{home}/archive.state.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct State {
    pub v: u32,
    pub dir: String,
    pub last_start_ms: Option<u64>,
    /// Absent or older than `last_start_ms` while a run is going, or when one
    /// was interrupted.
    pub last_end_ms: Option<u64>,
    /// Regular files found in the sources.
    pub seen: u64,
    /// Missing in the archive, copied.
    pub copied: u64,
    /// Grown or newer in the source, copied again.
    pub refreshed: u64,
    /// Shrunk in the source: the older copy kept beside the new one.
    pub versioned: u64,
    pub unchanged: u64,
    /// Symlinks and other non-regular entries below a root, not followed.
    pub skipped: u64,
    /// Of the copies written, how many were copy-on-write clones.
    pub cloned: u64,
    /// Logical bytes written (a clone costs none on disk).
    pub bytes: u64,
    pub sources: Vec<SourceState>,
    pub error_count: u64,
    /// The last [`ERRORS_KEPT`] errors.
    pub errors: Vec<RunError>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SourceState {
    pub label: String,
    pub root: String,
    pub found: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RunError {
    /// Relative to the archive dir, e.g. `claude/projects/<p>/<s>.jsonl`.
    pub path: String,
    pub error: String,
}

impl State {
    pub fn running_or_interrupted(&self) -> bool {
        match (self.last_start_ms, self.last_end_ms) {
            (Some(start), Some(end)) => end < start,
            (Some(_), None) => true,
            _ => false,
        }
    }

    fn error(&mut self, path: &str, error: impl std::fmt::Display) {
        self.error_count += 1;
        if self.errors.len() >= ERRORS_KEPT {
            self.errors.remove(0);
        }
        self.errors.push(RunError {
            path: path.to_string(),
            error: error.to_string(),
        });
    }
}

pub fn load_state(home: &Path) -> Option<State> {
    let bytes = safe_fs::read_under(home, &[], STATE_FILE, STATE_CAP).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_state(home: &Path, state: &State) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    let home_fd = safe_fs::open_dir(home).map_err(|e| format!("{}: {e}", home.display()))?;
    safe_fs::replace_at(&home_fd, STATE_FILE, &bytes, FILE_MODE)
        .map_err(|e| format!("cannot write {}: {e}", home.join(STATE_FILE).display()))
}

/// One line about a finished run.
pub fn summary(state: &State, took: Option<Duration>) -> String {
    let mut s = format!(
        "{} file(s) seen: {} copied, {} refreshed, {} rewritten (older copy kept), {} unchanged",
        state.seen, state.copied, state.refreshed, state.versioned, state.unchanged
    );
    if state.skipped > 0 {
        s.push_str(&format!(", {} symlink(s) skipped", state.skipped));
    }
    let written = state.copied + state.refreshed + state.versioned;
    if written > 0 {
        s.push_str(&format!(
            "; {} written, {} of {} as clones",
            human_bytes(state.bytes),
            state.cloned,
            written
        ));
    }
    s.push_str(&format!("; {} error(s)", state.error_count));
    if let Some(took) = took {
        s.push_str(&format!(" in {:.1} s", took.as_secs_f64()));
    }
    s
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

// ---------------------------------------------------------------------------
// Run
// ---------------------------------------------------------------------------

/// How a call to [`run`] ended.
#[derive(Debug)]
pub enum Outcome {
    /// No `archive.json`: nothing was read or written.
    Disabled,
    /// Another run holds the lock.
    Busy,
    Ran {
        state: State,
        took: Duration,
    },
}

/// Whether a run holds the lock right now.
pub fn busy(home: &Path) -> bool {
    let Ok(file) = open_lock(home, false) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = FileExt::unlock(&file);
            false
        }
        Err(e) => e.kind() == fs2::lock_contended_error().kind(),
    }
}

fn open_lock(home: &Path, create: bool) -> io::Result<File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .mode(FILE_MODE)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(home.join(LOCK_FILE))
}

/// Sync every source into the archive, once, in the calling thread.
pub fn run(home: &Path, roots: &TraceRoots) -> Result<Outcome, String> {
    run_with(home, roots, true)
}

fn run_with(home: &Path, roots: &TraceRoots, allow_clone: bool) -> Result<Outcome, String> {
    let Some(config) = load_config(home)? else {
        return Ok(Outcome::Disabled);
    };
    let lock = open_lock(home, true)
        .map_err(|e| format!("cannot open {}: {e}", home.join(LOCK_FILE).display()))?;
    if let Err(e) = lock.try_lock_exclusive() {
        if e.kind() == fs2::lock_contended_error().kind() {
            return Ok(Outcome::Busy);
        }
        return Err(format!(
            "cannot lock {}: {e}",
            home.join(LOCK_FILE).display()
        ));
    }
    let srcs = sources(roots);
    check_dir(&config.dir, &srcs).map_err(|e| {
        format!(
            "{e}; run `harbard archive enable --dir <another path>` or `harbard archive disable`"
        )
    })?;
    // Never recreated here: a missing dir on an unmounted volume would
    // otherwise fill the startup disk under /Volumes.
    if !fs::metadata(&config.dir).is_ok_and(|m| m.is_dir()) {
        return Err(format!(
            "the archive dir {} is missing (an unmounted volume?); mount it, or run `harbard archive enable --dir {}` to create it",
            config.dir.display(),
            config.dir.display()
        ));
    }

    let started = Instant::now();
    let start_ms = crate::clock::now_ms();
    let mut marker = load_state(home).unwrap_or_default();
    marker.last_start_ms = Some(start_ms);
    save_state(home, &marker)?;

    let mut state = State {
        v: 1,
        dir: config.dir.display().to_string(),
        last_start_ms: Some(start_ms),
        ..State::default()
    };
    let mut walker = Walker {
        state: &mut state,
        allow_clone,
    };
    match open_followed_dir(&config.dir) {
        Ok(archive) => {
            for source in &srcs {
                walker.source(&archive, source);
            }
        }
        Err(e) => walker
            .state
            .error("", format!("{}: {e}", config.dir.display())),
    }
    state.last_end_ms = Some(crate::clock::now_ms());
    save_state(home, &state)?;
    drop(lock);
    Ok(Outcome::Ran {
        state,
        took: started.elapsed(),
    })
}

/// Open a directory following a symlink at its last component (a source
/// root or the archive dir may be one; nothing below them may).
fn open_followed_dir(path: &Path) -> Result<OwnedFd, SafeError> {
    Ok(nix::fcntl::open(
        path,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?)
}

fn kind(st: &FileStat) -> SFlag {
    SFlag::from_bits_truncate(st.st_mode as _) & SFlag::S_IFMT
}

fn mtime_ns(st: &FileStat) -> i128 {
    i128::from(st.st_mtime) * 1_000_000_000 + i128::from(st.st_mtime_nsec)
}

fn mtime_of(st: &FileStat) -> SystemTime {
    match (u64::try_from(st.st_mtime), u32::try_from(st.st_mtime_nsec)) {
        (Ok(secs), Ok(nanos)) => UNIX_EPOCH + Duration::new(secs, nanos),
        _ => UNIX_EPOCH,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Copy,
    Refresh,
    Version,
}

/// Compare by metadata alone.
fn decide(source: &FileStat, archived: &FileStat) -> Option<Action> {
    if source.st_size > archived.st_size {
        Some(Action::Refresh)
    } else if source.st_size < archived.st_size {
        Some(Action::Version)
    } else if mtime_ns(source) > mtime_ns(archived) + MTIME_SLACK_NS {
        Some(Action::Refresh)
    } else {
        None
    }
}

struct Walker<'a> {
    state: &'a mut State,
    allow_clone: bool,
}

impl Walker<'_> {
    fn source(&mut self, archive: &OwnedFd, source: &Source) {
        let label = source.label();
        let src = match open_followed_dir(&source.root) {
            Ok(fd) => Some(fd),
            Err(SafeError::NotFound) => None,
            Err(e) => {
                self.state
                    .error(&label, format!("{}: {e}", source.root.display()));
                None
            }
        };
        self.state.sources.push(SourceState {
            label: label.clone(),
            root: source.root.display().to_string(),
            found: src.is_some(),
        });
        let Some(src) = src else { return };
        let dst = ensure_dir_at(archive, source.dest[0])
            .and_then(|top| ensure_dir_at(&top, source.dest[1]));
        match dst {
            Ok(dst) => self.walk(&src, &dst, &label, 0),
            Err(e) => self.state.error(&label, e),
        }
    }

    fn walk(&mut self, src: &OwnedFd, dst: &OwnedFd, rel: &str, depth: usize) {
        let mut names = Vec::new();
        match Dir::openat(
            src.as_fd(),
            ".",
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
            Mode::empty(),
        ) {
            Ok(mut dir) => {
                for entry in dir.iter() {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(e) => {
                            self.state.error(rel, e);
                            break;
                        }
                    };
                    let bytes = entry.file_name().to_bytes();
                    if bytes == b"." || bytes == b".." {
                        continue;
                    }
                    match std::str::from_utf8(bytes) {
                        Ok(name) => names.push(name.to_string()),
                        Err(_) => self.state.error(
                            rel,
                            format!("skipped a name that is not UTF-8: {}", bytes.escape_ascii()),
                        ),
                    }
                }
            }
            Err(e) => return self.state.error(rel, e),
        }
        names.sort_unstable();
        for name in names {
            let path = format!("{rel}/{name}");
            let st = match fstatat(src.as_fd(), name.as_str(), AtFlags::AT_SYMLINK_NOFOLLOW) {
                Ok(st) => st,
                // Gone between the listing and now: the next run decides.
                Err(Errno::ENOENT) => continue,
                Err(e) => {
                    self.state.error(&path, e);
                    continue;
                }
            };
            let k = kind(&st);
            if k == SFlag::S_IFREG {
                self.file(src, dst, &name, &st, &path);
            } else if k != SFlag::S_IFDIR {
                // A symlink, FIFO, socket or device: never followed or read.
                self.state.skipped += 1;
            } else if depth >= MAX_DEPTH {
                self.state
                    .error(&path, format!("deeper than {MAX_DEPTH} levels; not walked"));
            } else {
                let sub_src = match safe_fs::open_dir_at(src, &name) {
                    Ok(fd) => fd,
                    Err(SafeError::NotFound) => continue,
                    // Swapped for a symlink since the stat: skipped.
                    Err(SafeError::Refused(_)) => {
                        self.state.skipped += 1;
                        continue;
                    }
                    Err(e) => {
                        self.state.error(&path, e);
                        continue;
                    }
                };
                match ensure_dir_at(dst, &name) {
                    Ok(sub_dst) => self.walk(&sub_src, &sub_dst, &path, depth + 1),
                    Err(e) => self.state.error(&path, e),
                }
            }
        }
    }

    fn file(&mut self, src: &OwnedFd, dst: &OwnedFd, name: &str, st: &FileStat, path: &str) {
        self.state.seen += 1;
        let action = match fstatat(dst.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW) {
            Err(Errno::ENOENT) => Action::Copy,
            Err(e) => return self.state.error(path, e),
            Ok(have) if kind(&have) != SFlag::S_IFREG => {
                return self.state.error(
                    path,
                    "the archive holds something other than a regular file under this name; left as it is",
                );
            }
            Ok(have) => match decide(st, &have) {
                Some(action) => action,
                None => {
                    self.state.unchanged += 1;
                    return;
                }
            },
        };
        match place(src, dst, name, action, self.allow_clone) {
            Ok(placed) => {
                match action {
                    Action::Copy => self.state.copied += 1,
                    Action::Refresh => self.state.refreshed += 1,
                    Action::Version => self.state.versioned += 1,
                }
                self.state.bytes += placed.bytes;
                if placed.cloned {
                    self.state.cloned += 1;
                }
            }
            Err(Placing::Vanished) => {}
            Err(Placing::Failed(e)) => self.state.error(path, e),
        }
    }
}

/// Open the directory `name` in `dir`, creating it (0700) when missing.
fn ensure_dir_at(dir: &OwnedFd, name: &str) -> Result<OwnedFd, SafeError> {
    match safe_fs::open_dir_at(dir, name) {
        Err(SafeError::NotFound) => match safe_fs::mkdir_at(dir, name, DIR_MODE) {
            Err(SafeError::Io(e)) if e.raw_os_error() == Some(Errno::EEXIST as i32) => {
                safe_fs::open_dir_at(dir, name)
            }
            other => other,
        },
        other => other,
    }
}

struct Placed {
    bytes: u64,
    cloned: bool,
}

enum Placing {
    /// The source went away before it could be opened.
    Vanished,
    Failed(String),
}

impl From<SafeError> for Placing {
    fn from(e: SafeError) -> Self {
        Placing::Failed(e.to_string())
    }
}

/// Copy `name` from `src` into `dst` under a temp name, then rename it into
/// place; for [`Action::Version`] the current archive copy is first renamed
/// aside. A failure leaves the archive as it was, minus nothing.
fn place(
    src: &OwnedFd,
    dst: &OwnedFd,
    name: &str,
    action: Action,
    allow_clone: bool,
) -> Result<Placed, Placing> {
    let fd = match openat(
        src.as_fd(),
        name,
        OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::ENOENT) => return Err(Placing::Vanished),
        Err(e) => return Err(SafeError::from(e).into()),
    };
    let st = fstat(fd.as_fd()).map_err(SafeError::from)?;
    if kind(&st) != SFlag::S_IFREG {
        return Err(Placing::Failed("no longer a regular file".into()));
    }
    let mut source = File::from(fd);
    let tmp = format!(".{name}.{}.tmp", ulid::Ulid::generate());
    let cloned = allow_clone && clone_into(&source, dst, &tmp);
    let copied = if cloned {
        Ok(())
    } else {
        copy_into(&mut source, dst, &tmp)
    };
    let renamed = copied.and_then(|()| {
        finish(dst, &tmp, mtime_of(&st))?;
        if action == Action::Version {
            let kept = kept_name(dst, name)?;
            safe_fs::rename_at(dst, name, &kept)?;
        }
        safe_fs::rename_at(dst, &tmp, name)
    });
    if let Err(e) = renamed {
        let _ = safe_fs::remove_at(dst, &tmp);
        return Err(e.into());
    }
    Ok(Placed {
        bytes: st.st_size as u64,
        cloned,
    })
}

/// A copy-on-write clone of the open file as `tmp` in `dst`. False when the
/// file system cannot clone it (another volume, not APFS): the caller copies.
#[cfg(target_os = "macos")]
fn clone_into(source: &File, dst: &OwnedFd, tmp: &str) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(name) = std::ffi::CString::new(tmp) else {
        return false;
    };
    // SAFETY: both descriptors are open for the duration of the call and
    // `name` is a NUL-terminated single component; flags 0 clones the file
    // the descriptor holds, never a path.
    unsafe { libc::fclonefileat(source.as_raw_fd(), dst.as_raw_fd(), name.as_ptr(), 0) == 0 }
}

#[cfg(not(target_os = "macos"))]
fn clone_into(_source: &File, _dst: &OwnedFd, _tmp: &str) -> bool {
    false
}

fn copy_into(source: &mut File, dst: &OwnedFd, tmp: &str) -> Result<(), SafeError> {
    let mut out = safe_fs::create_new_at(dst, tmp, FILE_MODE)?;
    io::copy(source, &mut out)
        .and_then(|_| out.sync_all())
        .map_err(SafeError::Io)
}

/// The source's modification time and 0600, on the copy itself. Opening it
/// through `safe_fs` also proves it is a regular file with one link.
fn finish(dst: &OwnedFd, tmp: &str, modified: SystemTime) -> Result<(), SafeError> {
    let copy = safe_fs::open_file_at(dst, tmp)?;
    copy.set_times(FileTimes::new().set_modified(modified))
        .and_then(|()| copy.set_permissions(Permissions::from_mode(FILE_MODE)))
        .map_err(SafeError::Io)
}

/// A name for the older copy that nothing in the archive holds yet.
fn kept_name(dst: &OwnedFd, name: &str) -> Result<String, SafeError> {
    for _ in 0..4 {
        let kept = format!("{name}.rewritten-{}", ulid::Ulid::generate());
        match fstatat(dst.as_fd(), kept.as_str(), AtFlags::AT_SYMLINK_NOFOLLOW) {
            Err(Errno::ENOENT) => return Ok(kept),
            Err(e) => return Err(e.into()),
            Ok(_) => continue,
        }
    }
    Err(SafeError::Refused("no free name for the older copy"))
}

/// Ask the scheduler to treat the calling thread as background work (macOS:
/// throttled CPU and I/O). Best effort; a no-op elsewhere.
pub fn lower_thread_priority() {
    #[cfg(target_os = "macos")]
    // SAFETY: plain syscall on the calling thread; failure only means the
    // thread keeps its normal priority.
    unsafe {
        libc::setpriority(libc::PRIO_DARWIN_THREAD, 0, libc::PRIO_DARWIN_BG);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;
    use std::os::unix::fs::{MetadataExt, symlink};

    struct Lab {
        _tmp: TempHome,
        home: PathBuf,
        archive: PathBuf,
        roots: TraceRoots,
    }

    impl Lab {
        fn new() -> Self {
            let tmp = TempHome::new();
            let base = tmp.path().to_path_buf();
            let user = base.join("user");
            let roots = TraceRoots {
                home: user.clone(),
                claude_projects: user.join(".claude/projects"),
                codex_sessions: user.join(".codex/sessions"),
                codex_archived: user.join(".codex/archived_sessions"),
                cursor_projects: user.join(".cursor/projects"),
            };
            fs::create_dir_all(&roots.claude_projects).unwrap();
            fs::create_dir_all(&roots.codex_sessions).unwrap();
            Lab {
                home: base.join("harbard"),
                archive: base.join("archive"),
                roots,
                _tmp: tmp,
            }
        }

        fn enable(&self) {
            enable(&self.home, &self.archive, &self.roots).unwrap();
        }

        fn write(&self, path: &Path, bytes: &[u8]) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }

        fn claude(&self, rel: &str) -> PathBuf {
            self.roots.claude_projects.join(rel)
        }

        fn copy_of(&self, rel: &str) -> PathBuf {
            self.archive.join("claude/projects").join(rel)
        }

        fn run(&self) -> State {
            self.run_with(true)
        }

        fn run_with(&self, clone: bool) -> State {
            match run_with(&self.home, &self.roots, clone).unwrap() {
                Outcome::Ran { state, .. } => state,
                other => panic!("did not run: {other:?}"),
            }
        }
    }

    /// Bump a file's mtime by `secs` so a same-size change is visible.
    fn touch_later(path: &Path, secs: u64) {
        let f = File::options().write(true).open(path).unwrap();
        let t = fs::metadata(path).unwrap().modified().unwrap() + Duration::from_secs(secs);
        f.set_times(FileTimes::new().set_modified(t)).unwrap();
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn the_first_run_copies_every_tree_and_mirrors_its_layout() {
        let lab = Lab::new();
        lab.enable();
        lab.write(&lab.claude("-p/s1.jsonl"), b"lead\n");
        lab.write(&lab.claude("-p/s1/subagents/agent-a.jsonl"), b"agent\n");
        lab.write(
            &lab.roots.codex_sessions.join("2026/11/10/rollout-x.jsonl"),
            b"codex\n",
        );
        // The archived Codex root does not exist: noted, not an error.
        let state = lab.run();
        assert_eq!(
            (state.seen, state.copied, state.error_count),
            (3, 3, 0),
            "{state:?}"
        );
        assert_eq!(fs::read(lab.copy_of("-p/s1.jsonl")).unwrap(), b"lead\n");
        assert_eq!(
            fs::read(lab.copy_of("-p/s1/subagents/agent-a.jsonl")).unwrap(),
            b"agent\n"
        );
        assert_eq!(
            fs::read(
                lab.archive
                    .join("codex/sessions/2026/11/10/rollout-x.jsonl")
            )
            .unwrap(),
            b"codex\n"
        );
        assert_eq!(
            state.sources.iter().map(|s| s.found).collect::<Vec<_>>(),
            [true, true, false]
        );
        assert_eq!(state.bytes, 17);
        // No temp file is left behind.
        assert_eq!(names_in(&lab.copy_of("-p")), ["s1", "s1.jsonl"]);
        // The state on disk is the run.
        assert_eq!(load_state(&lab.home).unwrap(), state);
        assert!(!state.running_or_interrupted());

        // A second run touches nothing.
        let again = lab.run();
        assert_eq!(
            (again.seen, again.unchanged, again.copied + again.refreshed),
            (3, 3, 0)
        );
    }

    #[test]
    fn growth_and_a_newer_same_size_file_are_refreshed() {
        let lab = Lab::new();
        lab.enable();
        let path = lab.claude("-p/s1.jsonl");
        lab.write(&path, b"one\n");
        lab.run();
        lab.write(&path, b"one\ntwo\n");
        let state = lab.run();
        assert_eq!((state.refreshed, state.copied), (1, 0));
        assert_eq!(fs::read(lab.copy_of("-p/s1.jsonl")).unwrap(), b"one\ntwo\n");

        lab.write(&path, b"ONE\nTWO\n");
        touch_later(&path, 10);
        let state = lab.run();
        assert_eq!(state.refreshed, 1);
        assert_eq!(fs::read(lab.copy_of("-p/s1.jsonl")).unwrap(), b"ONE\nTWO\n");
        // The copy carries the source's time, so the next run is a stat.
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            fs::metadata(lab.copy_of("-p/s1.jsonl"))
                .unwrap()
                .modified()
                .unwrap()
        );
        assert_eq!(lab.run().unchanged, 1);
    }

    #[test]
    fn a_rewritten_file_keeps_both_versions() {
        let lab = Lab::new();
        lab.enable();
        let path = lab.claude("-p/s1.jsonl");
        lab.write(&path, b"a long first history\n");
        lab.run();
        lab.write(&path, b"short\n");
        let state = lab.run();
        assert_eq!(state.versioned, 1, "{state:?}");
        let names = names_in(&lab.copy_of("-p"));
        assert_eq!(names.len(), 2, "{names:?}");
        assert_eq!(names[0], "s1.jsonl");
        assert!(names[1].starts_with("s1.jsonl.rewritten-"), "{names:?}");
        assert_eq!(fs::read(lab.copy_of("-p/s1.jsonl")).unwrap(), b"short\n");
        assert_eq!(
            fs::read(lab.copy_of("-p").join(&names[1])).unwrap(),
            b"a long first history\n"
        );
        // Rewritten twice: three versions, none lost.
        lab.write(&path, b"s\n");
        lab.run();
        assert_eq!(names_in(&lab.copy_of("-p")).len(), 3);
    }

    #[test]
    fn a_deleted_source_keeps_its_archive_copy() {
        let lab = Lab::new();
        lab.enable();
        let path = lab.claude("-p/s1.jsonl");
        lab.write(&path, b"keep me\n");
        lab.run();
        fs::remove_dir_all(lab.claude("-p")).unwrap();
        let state = lab.run();
        assert_eq!((state.seen, state.error_count), (0, 0));
        assert_eq!(fs::read(lab.copy_of("-p/s1.jsonl")).unwrap(), b"keep me\n");
    }

    #[test]
    fn symlinks_below_a_root_are_skipped_and_a_symlinked_root_is_followed() {
        let lab = Lab::new();
        let outside = TempHome::new();
        fs::write(outside.path().join("secret.jsonl"), b"OUTSIDE").unwrap();
        fs::create_dir_all(outside.path().join("dir")).unwrap();
        fs::write(outside.path().join("dir/x.jsonl"), b"OUTSIDE").unwrap();
        lab.write(&lab.claude("-p/real.jsonl"), b"in\n");
        symlink(
            outside.path().join("secret.jsonl"),
            lab.claude("-p/link.jsonl"),
        )
        .unwrap();
        symlink(outside.path().join("dir"), lab.claude("-p/linked-dir")).unwrap();
        // The Codex sessions root is itself a symlink to a real folder.
        let real_sessions = lab.roots.home.join("elsewhere/sessions");
        lab.write(&real_sessions.join("r.jsonl"), b"codex\n");
        fs::remove_dir(&lab.roots.codex_sessions).unwrap();
        symlink(&real_sessions, &lab.roots.codex_sessions).unwrap();
        lab.enable();
        let state = lab.run();
        assert_eq!(
            (state.seen, state.copied, state.skipped),
            (2, 2, 2),
            "{state:?}"
        );
        assert_eq!(names_in(&lab.copy_of("-p")), ["real.jsonl"]);
        assert_eq!(
            fs::read(lab.archive.join("codex/sessions/r.jsonl")).unwrap(),
            b"codex\n"
        );
    }

    #[test]
    fn an_archive_inside_a_source_is_refused() {
        let lab = Lab::new();
        let inside = lab.roots.claude_projects.join("archive");
        let err = enable(&lab.home, &inside, &lab.roots).unwrap_err();
        assert!(err.contains("inside"), "{err}");
        assert!(!inside.exists(), "refused before creating anything");
        // Through `..` and through a symlink, too.
        let dotted = lab.roots.home.join(".codex/x/../sessions/a");
        assert!(enable(&lab.home, &dotted, &lab.roots).is_err());
        let alias = lab.roots.home.join("alias");
        symlink(&lab.roots.claude_projects, &alias).unwrap();
        assert!(enable(&lab.home, &alias.join("a"), &lab.roots).is_err());
        assert!(enable(&lab.home, Path::new("relative/dir"), &lab.roots).is_err());
        // A config written by hand is refused at run time as well.
        fs::create_dir_all(&lab.home).unwrap();
        fs::write(
            lab.home.join(CONFIG_FILE),
            format!("{{\"dir\": \"{}\"}}", inside.display()),
        )
        .unwrap();
        let err = run(&lab.home, &lab.roots).unwrap_err();
        assert!(err.contains("archive disable"), "{err}");
        assert!(!inside.exists());
        assert!(load_state(&lab.home).is_none());
    }

    #[test]
    fn disabled_does_nothing() {
        let lab = Lab::new();
        lab.write(&lab.claude("-p/s1.jsonl"), b"x\n");
        assert!(matches!(
            run(&lab.home, &lab.roots).unwrap(),
            Outcome::Disabled
        ));
        assert!(!lab.home.exists() || names_in(&lab.home).is_empty());
        assert!(!lab.archive.exists());
        // Enabled, then disabled: the archive stays, runs stop.
        lab.enable();
        lab.run();
        assert!(disable(&lab.home).unwrap());
        assert!(!disable(&lab.home).unwrap());
        lab.write(&lab.claude("-p/s2.jsonl"), b"y\n");
        assert!(matches!(
            run(&lab.home, &lab.roots).unwrap(),
            Outcome::Disabled
        ));
        assert!(lab.copy_of("-p/s1.jsonl").exists());
        assert!(!lab.copy_of("-p/s2.jsonl").exists());
    }

    #[test]
    fn an_archived_file_has_one_link_cloned_or_copied() {
        for clone in [true, false] {
            let lab = Lab::new();
            lab.enable();
            let path = lab.claude("-p/s1.jsonl");
            lab.write(&path, b"one\n");
            let state = lab.run_with(clone);
            assert_eq!(state.copied, 1);
            if !clone {
                assert_eq!(state.cloned, 0);
            }
            // The temp dir is on APFS on macOS: the clone path is the one taken.
            #[cfg(target_os = "macos")]
            if clone {
                assert_eq!(state.cloned, 1);
            }
            let copy = lab.copy_of("-p/s1.jsonl");
            let (src, dst) = (fs::metadata(&path).unwrap(), fs::metadata(&copy).unwrap());
            assert_eq!(dst.nlink(), 1, "clone={clone}");
            assert_eq!(src.nlink(), 1, "clone={clone}");
            assert_ne!(src.ino(), dst.ino(), "clone={clone}");
            assert_eq!(dst.mode() & 0o777, 0o600);
            // safe_fs (and the projector behind it) accepts the copy.
            assert_eq!(
                safe_fs::read_under(&lab.archive, &["claude", "projects", "-p"], "s1.jsonl", 99)
                    .unwrap(),
                b"one\n"
            );
            // Changing the source never changes the copy.
            lab.write(&path, b"two\n");
            assert_eq!(fs::read(&copy).unwrap(), b"one\n");
        }
    }

    #[test]
    fn a_held_lock_makes_a_second_run_busy() {
        let lab = Lab::new();
        lab.enable();
        let held = open_lock(&lab.home, true).unwrap();
        held.try_lock_exclusive().unwrap();
        assert!(busy(&lab.home));
        assert!(matches!(run(&lab.home, &lab.roots).unwrap(), Outcome::Busy));
        drop(held);
        assert!(!busy(&lab.home));
    }

    #[test]
    fn a_missing_archive_dir_is_not_recreated() {
        let lab = Lab::new();
        lab.enable();
        fs::remove_dir(&lab.archive).unwrap();
        let err = run(&lab.home, &lab.roots).unwrap_err();
        assert!(err.contains("missing"), "{err}");
        assert!(!lab.archive.exists());
    }

    #[test]
    fn errors_are_counted_and_the_last_ones_kept() {
        let mut state = State::default();
        for i in 0..(ERRORS_KEPT + 5) {
            state.error(&format!("p{i}"), "boom");
        }
        assert_eq!(state.error_count as usize, ERRORS_KEPT + 5);
        assert_eq!(state.errors.len(), ERRORS_KEPT);
        assert_eq!(state.errors[0].path, "p5");
    }

    #[test]
    fn a_non_file_in_the_archive_is_left_alone_and_reported() {
        let lab = Lab::new();
        lab.enable();
        lab.write(&lab.claude("-p/s1.jsonl"), b"x\n");
        fs::create_dir_all(lab.copy_of("-p/s1.jsonl")).unwrap();
        let state = lab.run();
        assert_eq!(state.error_count, 1, "{state:?}");
        assert!(lab.copy_of("-p/s1.jsonl").is_dir());
    }

    #[test]
    fn the_summary_reads_as_one_line() {
        let state = State {
            seen: 10,
            copied: 2,
            unchanged: 8,
            bytes: 2_500_000,
            cloned: 2,
            ..State::default()
        };
        let line = summary(&state, Some(Duration::from_millis(1500)));
        assert_eq!(
            line,
            "10 file(s) seen: 2 copied, 0 refreshed, 0 rewritten (older copy kept), 8 unchanged; 2.5 MB written, 2 of 2 as clones; 0 error(s) in 1.5 s"
        );
    }
}
