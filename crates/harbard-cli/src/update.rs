//! Updates: is there a newer harbard, what changed, and install it.
//!
//! The contract is `docs/SPEC-UPDATES.md`. In short: a check fetches the
//! source checkout and compares the update channel's tip with the commit
//! this binary was built from; a run fast-forwards the checkout and rebuilds
//! every installed surface in a detached process. serve checks in the
//! background; hooks only read the cached answer.

pub mod cmd;
pub mod notes;
pub mod run;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use harbard_core::{Store, clock};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use notes::Change;

/// The newest commits a check lists; the rest are counted, not listed.
pub const CHANGES_MAX: usize = 200;
/// How long a fetch may take before the check gives up.
const FETCH_TIMEOUT: Duration = Duration::from_secs(45);
/// Local git: a few seconds is already a stuck repository.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(20);
/// How often serve checks when `auto_check` is on.
pub const CHECK_EVERY_MS: u64 = 6 * 60 * 60 * 1000;
/// A cache older than this lets a hook start a background check.
pub const STALE_MS: u64 = 24 * 60 * 60 * 1000;
/// Hooks start at most one background check per this interval.
const ATTEMPT_EVERY_MS: u64 = 60 * 60 * 1000;
/// The branch every channel follows.
pub const BRANCH: &str = "main";

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Every commit on `origin/main`.
    #[default]
    Main,
    /// Only `v*` tags newer than the running version.
    Releases,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Main => "main",
            Channel::Releases => "releases",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "main" => Some(Channel::Main),
            "releases" | "release" => Some(Channel::Releases),
            _ => None,
        }
    }
}

/// `{home}/update.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default)]
    pub channel: Channel,
    #[serde(default = "yes")]
    pub auto_check: bool,
    /// Another checkout to update from (else the one this binary was built from).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
}

fn yes() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            channel: Channel::Main,
            auto_check: true,
            source: None,
        }
    }
}

impl Prefs {
    pub fn path(store: &Store) -> PathBuf {
        store.root().join("update.json")
    }

    /// The saved preferences, or the defaults when none (or unreadable).
    pub fn load(store: &Store) -> Self {
        std::fs::read_to_string(Self::path(store))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, store: &Store) -> std::io::Result<()> {
        write_json(&Self::path(store), self)
    }
}

// ---------------------------------------------------------------------------
// What runs, and where it comes from
// ---------------------------------------------------------------------------

/// The version and commit of this binary.
pub fn installed() -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "commit": installed_commit(),
    })
}

pub fn installed_commit() -> Option<String> {
    let c = env!("HARBARD_COMMIT");
    (c.len() == 40 && c.bytes().all(|b| b.is_ascii_hexdigit())).then(|| c.to_string())
}

/// A device card with the commit this build came from: Pairing compares it
/// with the hub's own to say who is behind.
pub fn stamp(mut card: harbard_core::DeviceCard) -> harbard_core::DeviceCard {
    card.commit = installed_commit().unwrap_or_default();
    card
}

/// The primary checkout updates fetch and fast-forward: `HARBARD_UPDATE_SOURCE`,
/// else the preferences' `source`, else the checkout this binary was built
/// from. A linked worktree resolves to its primary checkout, which is the
/// one the LaunchAgents run.
pub fn source(prefs: &Prefs) -> Option<PathBuf> {
    let raw = std::env::var_os("HARBARD_UPDATE_SOURCE")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| prefs.source.clone())
        .or_else(|| {
            let built = env!("HARBARD_SOURCE");
            (!built.is_empty()).then(|| PathBuf::from(built))
        })?;
    primary_checkout(&raw)
}

fn primary_checkout(path: &Path) -> Option<PathBuf> {
    let common = git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        LOCAL_TIMEOUT,
    )
    .ok()?;
    let common = PathBuf::from(common.trim());
    if common.file_name()? != ".git" {
        return None; // a bare repository has no checkout to build
    }
    let root = common.parent()?.to_path_buf();
    root.join("Cargo.toml").is_file().then_some(root)
}

/// Why the checkout cannot fast-forward to `target`, if it cannot.
pub fn source_problem(repo: &Path, target: Option<&str>) -> Option<&'static str> {
    let branch = git(
        repo,
        &["symbolic-ref", "--short", "-q", "HEAD"],
        LOCAL_TIMEOUT,
    )
    .map(|b| b.trim().to_string())
    .unwrap_or_default();
    if branch != BRANCH {
        return Some("not_on_main");
    }
    let dirty = git(
        repo,
        &["status", "--porcelain", "--untracked-files=no"],
        LOCAL_TIMEOUT,
    )
    .map(|s| !s.trim().is_empty())
    .unwrap_or(true);
    if dirty {
        return Some("dirty");
    }
    let target = target?;
    let ancestor =
        |a: &str, b: &str| git(repo, &["merge-base", "--is-ancestor", a, b], LOCAL_TIMEOUT).is_ok();
    if !ancestor("HEAD", target) && !ancestor(target, "HEAD") {
        return Some("diverged");
    }
    None
}

/// The English sentence for a problem or error code (the console words its own).
pub fn problem_message(code: &str) -> &'static str {
    match code {
        "no_checkout" => "no source checkout to update from (set `source` in update.json)",
        "not_on_main" => "the source checkout is not on main",
        "dirty" => "the source checkout has uncommitted changes to tracked files",
        "diverged" => "main has commits origin does not; push or integrate them first",
        "fetch_failed" => "git could not fetch origin",
        "busy" => "an update is already running",
        "interrupted" => "the update stopped before it finished",
        _ => "the update failed",
    }
}

// ---------------------------------------------------------------------------
// The check
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

impl ErrorInfo {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub version: String,
    pub commit: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub date_ms: u64,
}

/// `{home}/update/check.json`, and `check` in `GET /api/v1/update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub checked_ms: u64,
    pub channel: Channel,
    /// `current` | `available` | `error`.
    pub status: String,
    #[serde(default)]
    pub error: Option<ErrorInfo>,
    #[serde(default)]
    pub target: Option<Target>,
    #[serde(default)]
    pub behind: u32,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub changes: Vec<Change>,
    #[serde(default)]
    pub changes_truncated: bool,
    /// The commit the check compared against: a newer install makes the
    /// cached answer stale even before the next check.
    #[serde(default)]
    pub installed_commit: Option<String>,
}

impl Check {
    fn failed(channel: Channel, now: u64, error: ErrorInfo) -> Self {
        Self {
            checked_ms: now,
            channel,
            status: "error".into(),
            error: Some(error),
            target: None,
            behind: 0,
            notes: None,
            changes: Vec::new(),
            changes_truncated: false,
            installed_commit: installed_commit(),
        }
    }

    pub fn available(&self) -> bool {
        self.status == "available"
    }
}

pub fn dir(store: &Store) -> PathBuf {
    store.root().join("update")
}

fn check_path(store: &Store) -> PathBuf {
    dir(store).join("check.json")
}

/// The last check, when it still describes this binary. A check made by an
/// older build says "available" about commits this one already has.
pub fn cached(store: &Store) -> Option<Check> {
    let check: Check = std::fs::read_to_string(check_path(store))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())?;
    let mine = installed_commit();
    if check.installed_commit.is_some() && mine.is_some() && check.installed_commit != mine {
        return None;
    }
    Some(check)
}

/// The channel's newest commit, fetched; `Err` when it cannot say.
fn resolve_target(repo: &Path, channel: Channel) -> Result<(String, Target), ErrorInfo> {
    let fetched = git(
        repo,
        &["fetch", "--quiet", "--tags", "origin", BRANCH],
        FETCH_TIMEOUT,
    );
    if let Err(e) = &fetched {
        return Err(ErrorInfo::new("fetch_failed", e.clone()));
    }
    let date = |rev: &str| {
        git(repo, &["log", "-1", "--format=%ct", rev], LOCAL_TIMEOUT)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map_or(0, |s| s.saturating_mul(1000))
    };
    let commit_of = |rev: &str| {
        git(
            repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{rev}^{{commit}}"),
            ],
            LOCAL_TIMEOUT,
        )
        .map(|s| s.trim().to_string())
        .map_err(|e| ErrorInfo::new("fetch_failed", e))
    };
    match channel {
        Channel::Main => {
            let rev = format!("origin/{BRANCH}");
            let commit = commit_of(&rev)?;
            let version = git(
                repo,
                &["show", &format!("{commit}:Cargo.toml")],
                LOCAL_TIMEOUT,
            )
            .ok()
            .and_then(|toml| workspace_version(&toml))
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
            Ok((
                rev,
                Target {
                    version,
                    date_ms: date(&commit),
                    commit,
                    tag: None,
                    title: None,
                },
            ))
        }
        Channel::Releases => {
            let tags = git(repo, &["tag", "--list", "v*"], LOCAL_TIMEOUT).unwrap_or_default();
            let tag = notes::newest_tag(tags.lines().map(str::trim), env!("CARGO_PKG_VERSION"))
                // Nothing newer: the target is the running version's own tag
                // (or none), and the check says `current`.
                .or_else(|| {
                    let own = format!("v{}", env!("CARGO_PKG_VERSION"));
                    tags.lines().any(|t| t.trim() == own).then_some(own)
                });
            let Some(tag) = tag else {
                return Err(ErrorInfo::new("no_release", "origin has no v* release tag"));
            };
            let commit = commit_of(&tag)?;
            Ok((
                tag.clone(),
                Target {
                    version: tag.trim_start_matches('v').to_string(),
                    date_ms: date(&commit),
                    commit,
                    title: None,
                    tag: Some(tag),
                },
            ))
        }
    }
}

/// `version = "…"` under `[workspace.package]`.
fn workspace_version(toml: &str) -> Option<String> {
    let mut in_section = false;
    for line in toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line == "[workspace.package]";
            continue;
        }
        if in_section && let Some(rest) = line.strip_prefix("version") {
            let value = rest.trim_start().strip_prefix('=')?.trim();
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

/// Fetch the source and compare the channel's tip with this binary. Never
/// panics; every failure is a `status: error` check. Saves the result.
pub fn check(store: &Store, prefs: &Prefs) -> Check {
    let now = clock::now_ms();
    let out = check_uncached(prefs, now);
    let _ = std::fs::create_dir_all(dir(store));
    let _ = write_json(&check_path(store), &out);
    out
}

fn check_uncached(prefs: &Prefs, now: u64) -> Check {
    let channel = prefs.channel;
    let Some(repo) = source(prefs) else {
        return Check::failed(
            channel,
            now,
            ErrorInfo::new("no_checkout", problem_message("no_checkout")),
        );
    };
    let (rev, target) = match resolve_target(&repo, channel) {
        Ok(t) => t,
        Err(e) => return Check::failed(channel, now, e),
    };
    // What the running build is: its commit when the repository knows it,
    // else the checkout's HEAD (a build from elsewhere).
    let base = installed_commit()
        .filter(|c| {
            git(
                &repo,
                &["cat-file", "-e", &format!("{c}^{{commit}}")],
                LOCAL_TIMEOUT,
            )
            .is_ok()
        })
        .unwrap_or_else(|| "HEAD".to_string());
    let range = format!("{base}..{rev}");
    let behind: u32 = git(&repo, &["rev-list", "--count", &range], LOCAL_TIMEOUT)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let mut changes = git(
        &repo,
        &[
            "log",
            "--no-merges",
            &format!("--format={}", notes::LOG_FORMAT),
            &format!("--max-count={}", CHANGES_MAX + 1),
            &range,
        ],
        LOCAL_TIMEOUT,
    )
    .map(|out| notes::parse_log(&out))
    .unwrap_or_default();
    let changes_truncated = changes.len() > CHANGES_MAX;
    changes.truncate(CHANGES_MAX);
    let notes = if behind > 0 {
        let at = |rev: &str| {
            git(
                &repo,
                &["show", &format!("{rev}:CHANGELOG.md")],
                LOCAL_TIMEOUT,
            )
            .ok()
        };
        at(&target.commit)
            .and_then(|new| notes::new_notes(&new, at(&base).as_deref(), env!("CARGO_PKG_VERSION")))
    } else {
        None
    };
    Check {
        checked_ms: now,
        channel,
        status: if behind > 0 { "available" } else { "current" }.into(),
        error: None,
        target: Some(target),
        behind,
        notes,
        changes,
        changes_truncated,
        installed_commit: installed_commit(),
    }
}

// ---------------------------------------------------------------------------
// Reads for serve, hooks and doctor
// ---------------------------------------------------------------------------

/// `GET /api/v1/update`.
pub fn view(store: &Store) -> Value {
    let prefs = Prefs::load(store);
    let repo = source(&prefs);
    let check = cached(store);
    let problem = match &repo {
        None => Some("no_checkout"),
        Some(repo) => source_problem(
            repo,
            check
                .as_ref()
                .and_then(|c| c.target.as_ref())
                .map(|t| t.commit.as_str()),
        ),
    };
    json!({
        "installed": installed(),
        "prefs": { "channel": prefs.channel, "auto_check": prefs.auto_check },
        "source": {
            "path": repo.as_ref().map(|p| p.display().to_string()),
            "problem": problem,
        },
        "check": check,
        "run": run::view(store),
    })
}

/// The SessionStart line, when the cached check says a newer harbard exists.
pub fn brief_line(store: &Store) -> Option<String> {
    let check = cached(store)?;
    check
        .available()
        .then(|| harbard_core::protocol::correction_update_available(check.behind).line())
}

/// `harbard doctor`: the cached answer, never a fetch. Advisory only.
pub fn doctor_text(store: &Store) -> String {
    let prefs = Prefs::load(store);
    let what = match cached(store) {
        Some(c) if c.available() => format!(
            "{} newer commit(s) on {} · harbard update",
            c.behind,
            prefs.channel.as_str()
        ),
        Some(c) if c.status == "current" => format!("up to date on {}", prefs.channel.as_str()),
        Some(c) => format!(
            "last check failed: {}",
            c.error.map_or_else(|| "unknown".to_string(), |e| e.message)
        ),
        None => "not checked yet · harbard update --check".to_string(),
    };
    format!("{:<12} {what}", "update")
}

pub fn doctor_json(store: &Store) -> Value {
    json!({ "prefs": Prefs::load(store), "check": cached(store) })
}

/// From a hook: when nobody checked for a day and nobody tried in the last
/// hour, start `harbard update --check --quiet` detached and return at once.
/// Never blocks, never fails the hook.
pub fn nudge_background_check(store: &Store, now: u64) {
    let prefs = Prefs::load(store);
    if !prefs.auto_check {
        return;
    }
    let fresh = std::fs::read_to_string(check_path(store))
        .ok()
        .and_then(|s| serde_json::from_str::<Check>(&s).ok())
        .is_some_and(|c| now.saturating_sub(c.checked_ms) < STALE_MS);
    if fresh {
        return;
    }
    let attempt = dir(store).join("attempt");
    let tried = std::fs::metadata(&attempt)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .is_some_and(|t| now.saturating_sub(t.as_millis() as u64) < ATTEMPT_EVERY_MS);
    if tried || std::fs::create_dir_all(dir(store)).is_err() {
        return;
    }
    if std::fs::write(&attempt, now.to_string()).is_err() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut cmd = Command::new(exe);
    cmd.args(["update", "--check", "--quiet", "--home"])
        .arg(store.root())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut cmd);
    let _ = cmd.spawn();
}

/// Its own process group: a restart of whoever started it (serve under
/// launchd, a hook's harness) does not take it down.
pub fn detach(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
}

// ---------------------------------------------------------------------------
// serve: the background check
// ---------------------------------------------------------------------------

/// Two minutes after serve starts, then every [`CHECK_EVERY_MS`], while
/// `auto_check` is on. A check made by someone else (the CLI, the console's
/// "Check now") resets the clock.
pub fn start_checker(store: Store, shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let wait = |secs: u64| {
            let until = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < until && !shutdown.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(1));
            }
        };
        wait(120);
        while !shutdown.load(Ordering::Relaxed) {
            let prefs = Prefs::load(&store);
            let last = std::fs::read_to_string(check_path(&store))
                .ok()
                .and_then(|s| serde_json::from_str::<Check>(&s).ok())
                .filter(|c| c.channel == prefs.channel)
                .map_or(0, |c| c.checked_ms);
            let due =
                clock::now_ms().saturating_sub(last) >= CHECK_EVERY_MS || cached(&store).is_none();
            if prefs.auto_check && due && !run::busy(&store) {
                let _ = check(&store, &prefs);
            }
            wait(60);
        }
    });
}

// ---------------------------------------------------------------------------
// git and files
// ---------------------------------------------------------------------------

/// git in `repo`, bounded by `timeout`, never prompting. `Ok(stdout)` on
/// success, `Err(the last line of stderr)` otherwise.
pub fn git(repo: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        // No passphrase or host-key prompt nobody can answer; an ssh config
        // or agent still applies.
        cmd.env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=15",
        );
    }
    run_bounded(&mut cmd, timeout)
}

/// Run `cmd` (stdout and stderr piped) for at most `timeout`.
pub fn run_bounded(cmd: &mut Command, timeout: Duration) -> Result<String, String> {
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = stdout.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let err = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => break None,
        }
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    match status {
        Some(s) if s.success() => Ok(stdout),
        Some(_) => Err(stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("git failed")
            .trim()
            .to_string()),
        None => Err(format!("timed out after {} s", timeout.as_secs())),
    }
}

/// Write `value` as JSON by rename, so a reader never sees half a file.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), clock::now_ms()));
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workspace_version_is_read_from_its_own_section() {
        let toml = "[package]\nversion = \"9.9.9\"\n\n[workspace.package]\nedition = \"2024\"\nversion = \"0.3.0-alpha.10\"\n";
        assert_eq!(workspace_version(toml).as_deref(), Some("0.3.0-alpha.10"));
        assert_eq!(workspace_version("[package]\nversion = \"1.0.0\"\n"), None);
    }

    #[test]
    fn preferences_default_to_main_and_auto_check() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p, Prefs::default());
        assert_eq!(p.channel, Channel::Main);
        assert!(p.auto_check);
        let p: Prefs =
            serde_json::from_str(r#"{"channel":"releases","auto_check":false}"#).unwrap();
        assert_eq!(p.channel, Channel::Releases);
        assert_eq!(Channel::parse("release"), Some(Channel::Releases));
        assert_eq!(Channel::parse("nightly"), None);
    }

    #[test]
    fn a_bounded_command_is_killed_at_its_deadline() {
        let started = Instant::now();
        let mut cmd = Command::new("sleep");
        cmd.arg("5").stdout(Stdio::piped()).stderr(Stdio::piped());
        let out = run_bounded(&mut cmd, Duration::from_millis(200));
        assert!(out.unwrap_err().contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
