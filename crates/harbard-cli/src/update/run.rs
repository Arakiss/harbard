//! The run: fast-forward the checkout, rebuild and restart every installed
//! surface, reopen the app (docs/SPEC-UPDATES.md, "The run").
//!
//! `start` writes `run.json` and spawns `harbard update --run <id>` in its
//! own process group, its output appended to `run.log`; `execute` is that
//! process. It holds `run.lock` for as long as it lives, so a run whose
//! process died is told apart from one still going.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use fs2::FileExt;
use harbard_core::{Store, clock};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Channel, ErrorInfo, Prefs};

/// The steps, in order. The console words each one.
pub const STEPS: [&str; 6] = ["fetch", "cli", "hooks", "serve", "console", "desktop"];
/// How many lines of `run.log` a read carries.
const LOG_TAIL: usize = 40;
/// A run that has not taken its lock this long after it was written is dead.
const SPAWN_GRACE_MS: u64 = 15_000;
const SERVE_LABEL: &str = "com.harbard.serve";
const CONSOLE_LABEL: &str = "com.harbard.console";

/// A LaunchAgent label, overridable for a sandboxed run
/// (`HARBARD_UPDATE_SERVE_LABEL`, `HARBARD_UPDATE_CONSOLE_LABEL`).
fn label(var: &str, default: &'static str) -> String {
    std::env::var(var)
        .ok()
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}
const APP_ID: &str = "com.harbard.desktop";
const APP_EXEC_SUFFIX: &str = "/harbard.app/Contents/MacOS/harbard-desktop";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    /// `pending` | `running` | `done` | `failed` | `skipped`.
    pub state: String,
    #[serde(default)]
    pub started_ms: Option<u64>,
    #[serde(default)]
    pub finished_ms: Option<u64>,
    /// Why it was skipped, or what failed.
    #[serde(default)]
    pub detail: Option<String>,
}

/// `{home}/update/run.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    /// `running` | `done` | `failed`.
    pub state: String,
    pub started_ms: u64,
    #[serde(default)]
    pub finished_ms: Option<u64>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub channel: Channel,
    #[serde(default)]
    pub error: Option<ErrorInfo>,
    pub steps: Vec<Step>,
    /// What this run brings, copied from the check it started from, so
    /// "See what's new" still has it once the new build's check says
    /// "current".
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub changes: Vec<super::notes::Change>,
    #[serde(default)]
    pub changes_truncated: bool,
}

impl Run {
    fn new(channel: Channel, now: u64) -> Self {
        Self {
            id: ulid::Ulid::generate().to_string(),
            state: "running".into(),
            started_ms: now,
            finished_ms: None,
            from: super::installed_commit(),
            to: None,
            channel,
            error: None,
            steps: STEPS
                .iter()
                .map(|id| Step {
                    id: (*id).to_string(),
                    state: "pending".into(),
                    started_ms: None,
                    finished_ms: None,
                    detail: None,
                })
                .collect(),
            notes: None,
            changes: Vec::new(),
            changes_truncated: false,
        }
    }

    fn step(&mut self, id: &str) -> &mut Step {
        let at = self.steps.iter().position(|s| s.id == id).unwrap_or(0);
        &mut self.steps[at]
    }

    pub fn finished(&self) -> bool {
        self.state != "running"
    }
}

fn run_path(store: &Store) -> PathBuf {
    super::dir(store).join("run.json")
}

pub fn log_path(store: &Store) -> PathBuf {
    super::dir(store).join("run.log")
}

fn lock_path(store: &Store) -> PathBuf {
    super::dir(store).join("run.lock")
}

pub fn load(store: &Store) -> Option<Run> {
    std::fs::read_to_string(run_path(store))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn save(store: &Store, run: &Run) {
    let _ = super::write_json(&run_path(store), run);
}

/// Whether a run process holds the lock now.
fn locked(store: &Store) -> bool {
    let Ok(file) = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path(store))
    else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = FileExt::unlock(&file);
            false
        }
        Err(_) => true,
    }
}

/// A run is going: its process holds the lock, or it was written moments
/// ago and its process has not taken the lock yet.
pub fn busy(store: &Store) -> bool {
    if std::fs::create_dir_all(super::dir(store)).is_err() {
        return false;
    }
    if locked(store) {
        return true;
    }
    load(store).is_some_and(|r| {
        !r.finished() && clock::now_ms().saturating_sub(r.started_ms) < SPAWN_GRACE_MS
    })
}

/// The last run as the API shows it, with the tail of its log. A run that
/// says `running` but whose process is gone reads as `failed/interrupted`.
pub fn view(store: &Store) -> Option<Value> {
    let mut run = load(store)?;
    if !run.finished() && !busy(store) {
        run.state = "failed".into();
        run.error = Some(ErrorInfo::new(
            "interrupted",
            super::problem_message("interrupted"),
        ));
        for step in &mut run.steps {
            if step.state == "running" {
                step.state = "failed".into();
            } else if step.state == "pending" {
                step.state = "skipped".into();
            }
        }
    }
    let mut value = serde_json::to_value(&run).ok()?;
    value["log_tail"] = json!(tail(&log_path(store), LOG_TAIL));
    Some(value)
}

/// The last `n` lines of `path`, without terminal colour codes, each capped.
fn tail(path: &Path, n: usize) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(64 * 1024);
    if file.seek(SeekFrom::Start(from)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    let _ = file.read_to_end(&mut bytes);
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<String> = text
        .split(['\n', '\r'])
        .map(strip_ansi)
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take(300).collect())
        .collect();
    let skip = lines.len().saturating_sub(n);
    lines.into_iter().skip(skip).collect()
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Start a run: refuse when one is going or the checkout cannot be
/// updated, else write `run.json` and spawn the detached process.
pub fn start(store: &Store, prefs: &Prefs) -> Result<Run, ErrorInfo> {
    if busy(store) {
        return Err(ErrorInfo::new("busy", super::problem_message("busy")));
    }
    let Some(repo) = super::source(prefs) else {
        return Err(ErrorInfo::new(
            "no_checkout",
            super::problem_message("no_checkout"),
        ));
    };
    if let Some(code) = super::source_problem(&repo, None) {
        return Err(ErrorInfo::new(code, super::problem_message(code)));
    }
    let mut run = Run::new(prefs.channel, clock::now_ms());
    if let Some(check) =
        super::cached(store).filter(|c| c.available() && c.channel == prefs.channel)
    {
        run.notes = check.notes;
        run.changes = check.changes;
        run.changes_truncated = check.changes_truncated;
    }
    std::fs::create_dir_all(super::dir(store))
        .map_err(|e| ErrorInfo::new("step_failed", e.to_string()))?;
    let log =
        File::create(log_path(store)).map_err(|e| ErrorInfo::new("step_failed", e.to_string()))?;
    save(store, &run);
    let exe = std::env::current_exe().map_err(|e| ErrorInfo::new("step_failed", e.to_string()))?;
    let mut cmd = Command::new(exe);
    cmd.args(["update", "--run", &run.id, "--home"])
        .arg(store.root())
        .current_dir(&repo)
        .env("PATH", tool_path())
        .stdin(Stdio::null())
        .stdout(
            log.try_clone()
                .map_err(|e| ErrorInfo::new("step_failed", e.to_string()))?,
        )
        .stderr(log);
    super::detach(&mut cmd);
    if let Err(e) = cmd.spawn() {
        let mut failed = run.clone();
        failed.state = "failed".into();
        failed.finished_ms = Some(clock::now_ms());
        failed.error = Some(ErrorInfo::new("step_failed", e.to_string()));
        save(store, &failed);
        return Err(ErrorInfo::new("step_failed", e.to_string()));
    }
    Ok(run)
}

/// PATH with the toolchains the build needs, whatever the caller had: a
/// LaunchAgent's PATH carries neither cargo nor, sometimes, bun.
fn tool_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs: Vec<String> = vec![
        format!("{home}/.cargo/bin"),
        format!("{home}/.bun/bin"),
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
    ];
    for d in std::env::var("PATH").unwrap_or_default().split(':') {
        if !d.is_empty() && !dirs.iter().any(|x| x == d) {
            dirs.push(d.to_string());
        }
    }
    for d in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
        if !dirs.iter().any(|x| x == d) {
            dirs.push(d.to_string());
        }
    }
    dirs.join(":")
}

/// The detached process: `harbard update --run <id>`.
pub fn execute(store: &Store, id: &str) -> i32 {
    let _ = std::fs::create_dir_all(super::dir(store));
    let Ok(lock) = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path(store))
    else {
        eprintln!("harbard update: cannot open {}", lock_path(store).display());
        return 1;
    };
    if lock.try_lock_exclusive().is_err() {
        eprintln!("harbard update: another update is running");
        return 1;
    }
    let Some(mut run) = load(store).filter(|r| r.id == id && !r.finished()) else {
        eprintln!("harbard update: run {id} is not pending");
        return 1;
    };
    let prefs = Prefs::load(store);
    let app_was_open = app_running();
    let result = steps(store, &prefs, &mut run, app_was_open);
    let now = clock::now_ms();
    match result {
        Ok(()) => {
            run.state = "done".into();
            say("done", "harbard is up to date");
        }
        Err(error) => {
            for step in &mut run.steps {
                if step.state == "pending" {
                    step.state = "skipped".into();
                }
            }
            say("failed", &error.message);
            run.state = "failed".into();
            run.error = Some(error);
            // An app the run closed is opened again, whatever failed.
            if app_was_open && !app_running() {
                let _ = reopen_app();
            }
        }
    }
    run.finished_ms = Some(now);
    save(store, &run);
    // The new binary answers for itself: a check made by this (old)
    // process would call its own commit out of date.
    if run.state == "done"
        && let Ok(exe) = std::env::current_exe()
    {
        let mut cmd = Command::new(exe);
        cmd.args(["update", "--check", "--quiet", "--home"])
            .arg(store.root())
            .stdin(Stdio::null());
        let _ = super::run_bounded(
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped()),
            Duration::from_secs(90),
        );
    }
    drop(lock);
    i32::from(run.state != "done")
}

fn say(step: &str, text: &str) {
    println!("==> {step}: {text}");
}

/// Mark `id` running, run `body`, and record how it ended.
fn step(
    store: &Store,
    run: &mut Run,
    id: &str,
    body: impl FnOnce(&mut Run) -> Result<Option<String>, ErrorInfo>,
) -> Result<(), ErrorInfo> {
    {
        let s = run.step(id);
        s.state = "running".into();
        s.started_ms = Some(clock::now_ms());
    }
    save(store, run);
    let outcome = body(run);
    let s = run.step(id);
    s.finished_ms = Some(clock::now_ms());
    let out = match outcome {
        Ok(None) => {
            s.state = "done".into();
            Ok(())
        }
        Ok(Some(skipped)) => {
            s.state = "skipped".into();
            s.detail = Some(skipped);
            Ok(())
        }
        Err(error) => {
            s.state = "failed".into();
            s.detail = Some(error.message.clone());
            Err(error)
        }
    };
    save(store, run);
    out
}

fn steps(store: &Store, prefs: &Prefs, run: &mut Run, app_was_open: bool) -> Result<(), ErrorInfo> {
    let repo = super::source(prefs)
        .ok_or_else(|| ErrorInfo::new("no_checkout", super::problem_message("no_checkout")))?;

    step(store, run, "fetch", |run| {
        say("fetch", &format!("fetching origin in {}", repo.display()));
        let (_, target) = super::resolve_target(&repo, prefs.channel)?;
        if let Some(code) = super::source_problem(&repo, Some(&target.commit)) {
            return Err(ErrorInfo::new(code, super::problem_message(code)));
        }
        let have = super::git(
            &repo,
            &["merge-base", "--is-ancestor", &target.commit, "HEAD"],
            Duration::from_secs(20),
        )
        .is_ok();
        if !have {
            say(
                "fetch",
                &format!("fast-forwarding main to {}", &target.commit[..12]),
            );
            super::git(
                &repo,
                &["merge", "--ff-only", "--quiet", &target.commit],
                Duration::from_secs(120),
            )
            .map_err(|e| ErrorInfo::new("fetch_failed", e))?;
        }
        run.to = super::git(&repo, &["rev-parse", "HEAD"], Duration::from_secs(20))
            .ok()
            .map(|s| s.trim().to_string());
        Ok(None)
    })?;

    step(store, run, "cli", |_| {
        say("cli", "building and installing the CLI");
        script(
            &repo,
            "scripts/install-cli.sh",
            &[],
            Duration::from_secs(30 * 60),
        )
        .map(|()| None)
    })?;

    // The new binary wires what it needs: a version that adds a hook must not
    // depend on someone remembering `harbard setup` on each machine. Never
    // fatal: the update is already installed, and doctor reports a stale wiring.
    step(store, run, "hooks", |_| {
        say("hooks", "connecting the harnesses");
        let Ok(bin) = std::env::current_exe() else {
            return Ok(Some("no_binary".into()));
        };
        let mut cmd = Command::new(bin);
        cmd.arg("setup")
            .env("PATH", tool_path())
            .stdin(Stdio::null());
        Ok(super::run_bounded(&mut cmd, Duration::from_secs(60))
            .err()
            .map(|_| "setup_failed".to_string()))
    })?;

    step(store, run, "serve", |_| {
        let serve = label("HARBARD_UPDATE_SERVE_LABEL", SERVE_LABEL);
        if !launchd_loaded(&serve) {
            return Ok(Some("not_installed".into()));
        }
        say("serve", "restarting the bus");
        launchctl_kickstart(&serve)?;
        if !wait_for_port(serve_port(), Duration::from_secs(90)) {
            return Err(ErrorInfo::new(
                "step_failed",
                "serve did not answer within 90 s after its restart",
            ));
        }
        Ok(None)
    })?;

    let console = launchd_loaded(&label("HARBARD_UPDATE_CONSOLE_LABEL", CONSOLE_LABEL));
    step(store, run, "console", |_| {
        if !console {
            return Ok(Some("not_installed".into()));
        }
        say(
            "console",
            "building the console and the app, then swapping them in",
        );
        script(
            &repo,
            "scripts/update-desktop.sh",
            &[("HARBARD_QUIT_APP", "1"), ("HARBARD_OPEN_APP", "0")],
            Duration::from_secs(40 * 60),
        )
        .map(|()| None)
    })?;

    step(store, run, "desktop", |_| {
        if !console {
            return Ok(Some("not_installed".into()));
        }
        if !app_was_open {
            return Ok(Some("not_open".into()));
        }
        if app_running() {
            return Ok(None);
        }
        say("desktop", "opening the app again");
        reopen_app()
    })?;
    Ok(())
}

/// `bash <repo>/<path>`, its output into the run log, bounded.
fn script(
    repo: &Path,
    path: &str,
    env: &[(&str, &str)],
    timeout: Duration,
) -> Result<(), ErrorInfo> {
    let mut cmd = Command::new("bash");
    cmd.arg(repo.join(path))
        .current_dir(repo)
        .env("PATH", tool_path())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| ErrorInfo::new("step_failed", format!("{path}: {e}")))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(ErrorInfo::new(
                    "step_failed",
                    format!("{path} exited with {status}; see the log"),
                ));
            }
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ErrorInfo::new(
                    "step_failed",
                    format!("{path} timed out after {} min", timeout.as_secs() / 60),
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(ErrorInfo::new("step_failed", e.to_string())),
        }
    }
}

fn launchd_domain(label: &str) -> String {
    let uid = nix::unistd::getuid();
    format!("gui/{uid}/{label}")
}

fn launchd_loaded(label: &str) -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    Command::new("launchctl")
        .args(["print", &launchd_domain(label)])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn launchctl_kickstart(label: &str) -> Result<(), ErrorInfo> {
    let mut cmd = Command::new("launchctl");
    cmd.args(["kickstart", "-k", &launchd_domain(label)])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::run_bounded(&mut cmd, Duration::from_secs(30))
        .map(|_| ())
        .map_err(|e| ErrorInfo::new("step_failed", format!("launchctl kickstart {label}: {e}")))
}

fn serve_port() -> u16 {
    std::env::var("HARBARD_API_PORT")
        .ok()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(7419)
}

fn wait_for_port(port: u16, within: Duration) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let started = Instant::now();
    // The old process may still be closing: give the restart a beat first.
    std::thread::sleep(Duration::from_millis(500));
    while started.elapsed() < within {
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

/// An installed copy of the app is running (never the dock demo, never a
/// build under `target/`).
fn app_running() -> bool {
    let Ok(out) = Command::new("ps")
        .args(["-axo", "comm="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .any(|l| l.ends_with(APP_EXEC_SUFFIX) && l.contains("Applications/"))
}

fn reopen_app() -> Result<Option<String>, ErrorInfo> {
    let mut cmd = Command::new("open");
    cmd.args(["-b", APP_ID])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::run_bounded(&mut cmd, Duration::from_secs(30))
        .map_err(|e| ErrorInfo::new("step_failed", format!("open -b {APP_ID}: {e}")))?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(30) {
        if app_running() {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(ErrorInfo::new(
        "step_failed",
        "the app did not start again within 30 s",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Home(PathBuf);

    impl Home {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store() -> (Home, Store) {
        let path = std::env::temp_dir().join(format!(
            "harbard-update-{}-{}",
            std::process::id(),
            ulid::Ulid::generate()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let store = Store::at(&path);
        (Home(path), store)
    }

    #[test]
    fn colour_codes_are_stripped_from_the_log_tail() {
        assert_eq!(
            strip_ansi("\u{1b}[1;32m   Compiling\u{1b}[0m x"),
            "   Compiling x"
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn the_tail_keeps_the_last_lines_and_splits_progress_returns() {
        let (dir, _store) = store();
        let path = dir.path().join("log");
        let text: String = (1..=60).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, format!("{text}a\rb\n\n")).unwrap();
        let t = tail(&path, 5);
        assert_eq!(t, vec!["line 58", "line 59", "line 60", "a", "b"]);
    }

    #[test]
    fn a_running_run_without_its_process_reads_as_interrupted() {
        let (_dir, store) = store();
        let mut run = Run::new(Channel::Main, clock::now_ms() - SPAWN_GRACE_MS - 1);
        run.steps[0].state = "done".into();
        run.steps[1].state = "running".into();
        save(&store, &run);
        assert!(!busy(&store), "nobody holds the lock and the grace is over");
        let v = view(&store).unwrap();
        assert_eq!(v["state"], "failed");
        assert_eq!(v["error"]["code"], "interrupted");
        let states: Vec<&str> = v["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["state"].as_str().unwrap())
            .collect();
        assert_eq!(
            states,
            ["done", "failed", "skipped", "skipped", "skipped", "skipped"]
        );
    }

    #[test]
    fn a_fresh_run_is_busy_before_its_process_takes_the_lock() {
        let (_dir, store) = store();
        save(&store, &Run::new(Channel::Main, clock::now_ms()));
        assert!(busy(&store));
        assert_eq!(view(&store).unwrap()["state"], "running");
    }

    #[test]
    fn a_held_lock_is_busy_whatever_run_json_says() {
        let (_dir, store) = store();
        std::fs::create_dir_all(super::super::dir(&store)).unwrap();
        let f = File::create(lock_path(&store)).unwrap();
        f.lock_exclusive().unwrap();
        assert!(busy(&store));
        FileExt::unlock(&f).unwrap();
        assert!(!busy(&store));
    }
}
