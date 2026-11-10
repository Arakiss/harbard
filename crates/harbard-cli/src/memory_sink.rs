//! The memory sink: an approved rule handed to a program the operator chose.
//!
//! harbard keeps rules on its own bus; anything that wants them elsewhere
//! plugs in here, through the contract in `docs/memory-sink.md`. It is off
//! until the operator names an executable, and it runs only for a rule whose
//! save box was ticked.
//!
//! The sink is someone else's code, so it gets nothing it was not handed:
//! the record on stdin (closed straight after), a short environment and a
//! deadline. harbard never unlocks anything for it: a sink that needs an
//! unlock answers `locked` and the console says so.

use harbard_core::Store;
use harbard_core::rules::ActiveRule;
use harbard_core::workspace::DeskState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// How long one call may take.
pub const DEADLINE: Duration = Duration::from_secs(15);
/// The version of the record harbard sends and of the answers it accepts.
pub const CONTRACT_VERSION: u32 = 1;
/// Outranks the `config.memory_sink_command` setting, so one serve can be
/// pointed elsewhere without editing the stored file.
#[cfg_attr(test, allow(dead_code))]
pub const ENV_VAR: &str = "HARBARD_MEMORY_SINK";
/// Enough for an answer; a chatty sink must not grow serve's memory.
const OUTPUT_LIMIT: u64 = 64 * 1024;
/// The only variables a sink inherits.
const INHERITED_ENV: [&str; 4] = ["HOME", "PATH", "USER", "LANG"];

/// Why a rule stayed only in harbard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    Disabled,
    NotConfigured,
    Locked,
    TimedOut,
    Refused { reason: String },
    Failed { detail: String },
    BadAnswer,
}

impl Skip {
    fn code(&self) -> &'static str {
        match self {
            Self::Disabled => "memory_sink_disabled",
            Self::NotConfigured => "memory_sink_not_configured",
            Self::Locked => "memory_sink_locked",
            Self::TimedOut => "memory_sink_timed_out",
            Self::Refused { .. } => "memory_sink_refused",
            Self::Failed { .. } => "memory_sink_failed",
            Self::BadAnswer => "memory_sink_bad_answer",
        }
    }

    fn note(&self) -> String {
        const ONLY_HERE: &str = "the rule was saved only in harbard.";
        match self {
            Self::Disabled => {
                format!("Saving to external memory is off in Preferences: {ONLY_HERE}")
            }
            Self::NotConfigured => format!("No external memory is configured: {ONLY_HERE}"),
            Self::Locked => format!(
                "The external memory is locked and harbard does not unlock it: {ONLY_HERE} Unlock it yourself if you want the rule saved there."
            ),
            Self::TimedOut => format!("The external memory did not answer in time: {ONLY_HERE}"),
            Self::Refused { reason } => {
                format!("The external memory refused the rule ({reason}): {ONLY_HERE}")
            }
            Self::Failed { detail } => {
                format!("The external memory could not save the rule ({detail}): {ONLY_HERE}")
            }
            Self::BadAnswer => {
                format!("The external memory gave an answer harbard cannot read: {ONLY_HERE}")
            }
        }
    }

    fn params(&self) -> BTreeMap<&'static str, String> {
        match self {
            Self::Refused { reason } => BTreeMap::from([("reason", reason.clone())]),
            Self::Failed { detail } => BTreeMap::from([("detail", detail.clone())]),
            _ => BTreeMap::new(),
        }
    }
}

/// What the console is told. `code` and `params` drive its copy; `note` is
/// the English fallback for a code it does not know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub saved: bool,
    pub code: Option<&'static str>,
    pub note: Option<String>,
    pub params: BTreeMap<&'static str, String>,
    pub id: Option<String>,
}

impl Outcome {
    pub fn skipped(skip: Skip) -> Self {
        Self {
            saved: false,
            code: Some(skip.code()),
            note: Some(skip.note()),
            params: skip.params(),
            id: None,
        }
    }

    fn saved(id: Option<String>) -> Self {
        Self {
            saved: true,
            code: None,
            note: None,
            params: BTreeMap::new(),
            id,
        }
    }
}

#[derive(Serialize)]
struct Record<'a> {
    version: u32,
    kind: &'static str,
    rule_id: &'a str,
    key: &'a str,
    title: &'a str,
    rule: &'a str,
    evidence: &'a str,
}

fn current_version() -> u32 {
    CONTRACT_VERSION
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default = "current_version")]
    version: u32,
    saved: bool,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

// Unit tests must never reach a sink named in the developer's own shell.
fn env_command() -> Option<OsString> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        std::env::var_os(ENV_VAR)
    }
}

// Absolute only: a bare name would mean a PATH search and a relative path
// would depend on where serve happened to start.
fn resolve(env: Option<OsString>, setting: Option<&str>) -> Option<PathBuf> {
    let chosen = match env {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(setting?),
    };
    (chosen.is_absolute() && executable(&chosen)).then_some(chosen)
}

/// The configured sink, if any. Looking only stats a file; nothing runs.
pub fn command(store: &Store) -> Option<PathBuf> {
    let setting = DeskState::load(store).config.memory_sink_command;
    resolve(env_command(), setting.as_deref())
}

/// Hand an approved rule to the sink. `evidence` is what the retro itself
/// showed, already filtered by the caller.
pub fn record_rule(bin: &Path, rule: &ActiveRule, evidence: &str) -> Outcome {
    let record = Record {
        version: CONTRACT_VERSION,
        kind: "decision",
        rule_id: &rule.id,
        key: &rule.key,
        title: &rule.title,
        rule: &rule.text,
        evidence,
    };
    run_with_deadline(bin, &record, DEADLINE)
}

// A reader of its own per pipe: reading after the exit would let a sink that
// fills a pipe block itself until the deadline.
fn drain<R: Read + Send + 'static>(source: Option<R>) -> Receiver<String> {
    let (done, received) = mpsc::channel();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        if let Some(mut pipe) = source {
            let _ = pipe.by_ref().take(OUTPUT_LIMIT).read_to_end(&mut kept);
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
        }
        let _ = done.send(String::from_utf8_lossy(&kept).into_owned());
    });
    received
}

// The whole group, or a shell wrapper would die while the program it started
// keeps waiting on its prompt.
fn kill_group(child: &mut std::process::Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// On Linux, exec fails with ETXTBSY while any process still holds the
/// executable open for writing. A sink that was just installed or upgraded can
/// be in that state for a moment: another thread's fork inherits the writer's
/// descriptor until its own exec closes it. The window is short, so a few
/// brief retries turn a spurious failure into a delivered record.
fn spawn_retrying_busy(cmd: &mut Command) -> std::io::Result<std::process::Child> {
    let mut attempt = 0;
    loop {
        match cmd.spawn() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 5 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(20 * attempt));
            }
            other => return other,
        }
    }
}

fn run_with_deadline(bin: &Path, record: &Record<'_>, deadline: Duration) -> Outcome {
    let payload = serde_json::to_vec(record).unwrap_or_default();
    let mut cmd = Command::new(bin);
    cmd.env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    for key in INHERITED_ENV {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    let mut child = match spawn_retrying_busy(&mut cmd) {
        Ok(child) => child,
        Err(e) => {
            return Outcome::skipped(Skip::Failed {
                detail: e.kind().to_string(),
            });
        }
    };
    let stdin = child.stdin.take();
    // On its own thread for the same reason as `drain`; dropping the handle
    // is the EOF that lets a sink stop reading.
    std::thread::spawn(move || {
        if let Some(mut pipe) = stdin {
            let _ = pipe.write_all(&payload);
        }
    });
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                kill_group(&mut child);
                return Outcome::skipped(Skip::TimedOut);
            }
        }
    };
    // Bounded too: a daemon the sink left behind can hold the pipes open.
    let wait = deadline
        .saturating_sub(started.elapsed())
        .max(Duration::from_millis(100));
    let out = stdout.recv_timeout(wait).unwrap_or_default();
    let err = stderr.recv_timeout(wait).unwrap_or_default();
    read_answer(status.success(), &out, &err)
}

fn read_answer(exited_ok: bool, out: &str, err: &str) -> Outcome {
    let answer = serde_json::from_str::<Answer>(out.trim())
        .ok()
        .filter(|a| a.version == CONTRACT_VERSION);
    match answer {
        Some(a) if a.saved && exited_ok => {
            Outcome::saved(a.id.and_then(|id| harbard_core::feedback::one_line(&id, 120)))
        }
        // A refusal counts whatever the exit code: sinks may exit non-zero
        // to say no.
        Some(a) if !a.saved => Outcome::skipped(match a.reason.as_deref() {
            Some("locked") => Skip::Locked,
            reason => Skip::Refused {
                reason: reason
                    .and_then(|r| harbard_core::feedback::one_line(r, 64))
                    .unwrap_or_else(|| "unspecified".to_string()),
            },
        }),
        _ if !exited_ok => {
            let first = err.lines().next().unwrap_or("");
            Outcome::skipped(Skip::Failed {
                detail: harbard_core::feedback::one_line(first, 120)
                    .unwrap_or_else(|| "no detail".to_string()),
            })
        }
        _ => Outcome::skipped(Skip::BadAnswer),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A scripted stand-in for a real sink: it records its stdin and its
    /// environment, then answers as `mode` says.
    pub(crate) struct FakeSink {
        dir: PathBuf,
    }

    impl FakeSink {
        pub(crate) fn new(mode: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "harbard-fake-sink-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let body = match mode {
                "ok" => r#"echo '{"version":1,"saved":true,"id":"record-1"}'"#.to_string(),
                "unversioned" => r#"echo '{"saved":true,"id":"record-2"}'"#.to_string(),
                "locked" => {
                    r#"echo '{"version":1,"saved":false,"reason":"locked"}'; exit 3"#.to_string()
                }
                "refused" => {
                    r#"echo '{"version":1,"saved":false,"reason":"duplicate"}'"#.to_string()
                }
                "future" => r#"echo '{"version":2,"saved":true}'"#.to_string(),
                "lying" => r#"echo '{"version":1,"saved":true}'; exit 1"#.to_string(),
                "garbage" => "echo 'not json'".to_string(),
                "failing" => "echo 'disk full' >&2; exit 1".to_string(),
                "flood" => "head -c 200000 /dev/zero".to_string(),
                "orphan" => format!("sleep 60 & echo $! > '{}'; wait", dir.join("pid").display()),
                _ => "sleep 60".to_string(),
            };
            let script = format!(
                "#!/bin/sh\ncat > '{}'\nenv > '{}'\n{body}\n",
                dir.join("record.json").display(),
                dir.join("env.log").display(),
            );
            let bin = dir.join("sink");
            std::fs::write(&bin, script).unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self { dir }
        }

        pub(crate) fn bin(&self) -> PathBuf {
            self.dir.join("sink")
        }

        pub(crate) fn record(&self) -> String {
            std::fs::read_to_string(self.dir.join("record.json")).unwrap_or_default()
        }

        fn env(&self) -> String {
            std::fs::read_to_string(self.dir.join("env.log")).unwrap_or_default()
        }

        fn pid(&self) -> Option<i32> {
            std::fs::read_to_string(self.dir.join("pid"))
                .ok()?
                .trim()
                .parse()
                .ok()
        }
    }

    impl Drop for FakeSink {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn rule() -> ActiveRule {
        ActiveRule {
            id: "01RULE".into(),
            key: "one_owner".into(),
            title: "One owner".into(),
            text: "One owner per primitive".into(),
            approved_ms: 1,
            edited: false,
            memory_sink: false,
        }
    }

    fn run(sink: &FakeSink) -> Outcome {
        record_rule(&sink.bin(), &rule(), "3 leaks in 3 branches")
    }

    fn short_record() -> Record<'static> {
        Record {
            version: CONTRACT_VERSION,
            kind: "decision",
            rule_id: "r",
            key: "k",
            title: "t",
            rule: "r",
            evidence: "e",
        }
    }

    #[test]
    fn nothing_configured_means_no_sink_and_no_search() {
        assert_eq!(resolve(None, None), None);
        assert_eq!(resolve(Some(OsString::new()), None), None);
    }

    #[test]
    fn the_setting_names_the_sink_and_the_environment_outranks_it() {
        let setting = FakeSink::new("ok");
        let env = FakeSink::new("ok");
        let setting_path = setting.bin().display().to_string();
        assert_eq!(resolve(None, Some(&setting_path)), Some(setting.bin()));
        assert_eq!(
            resolve(Some(env.bin().into_os_string()), Some(&setting_path)),
            Some(env.bin())
        );
    }

    #[test]
    fn a_relative_or_non_executable_command_counts_as_not_configured() {
        let sink = FakeSink::new("ok");
        assert_eq!(resolve(None, Some("sink")), None, "no PATH search");
        std::fs::set_permissions(sink.bin(), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(resolve(None, Some(&sink.bin().display().to_string())), None);
    }

    #[test]
    fn an_approved_rule_reaches_the_sink_as_versioned_json_on_stdin() {
        let sink = FakeSink::new("ok");
        let out = run(&sink);
        assert_eq!(out, Outcome::saved(Some("record-1".into())));
        let got: serde_json::Value = serde_json::from_str(&sink.record()).unwrap();
        assert_eq!(
            got,
            serde_json::json!({
                "version": 1,
                "kind": "decision",
                "rule_id": "01RULE",
                "key": "one_owner",
                "title": "One owner",
                "rule": "One owner per primitive",
                "evidence": "3 leaks in 3 branches",
            })
        );
    }

    #[test]
    fn an_answer_without_a_version_is_read_as_the_current_one() {
        let sink = FakeSink::new("unversioned");
        assert_eq!(run(&sink), Outcome::saved(Some("record-2".into())));
    }

    #[test]
    fn a_sink_runs_with_a_short_environment_only() {
        let sink = FakeSink::new("ok");
        run(&sink);
        let env = sink.env();
        assert!(env.contains("PATH="), "{env}");
        assert!(
            !env.contains("CARGO"),
            "the test's own variables leaked: {env}"
        );
    }

    #[test]
    fn a_locked_sink_is_skipped_with_a_machine_code() {
        let sink = FakeSink::new("locked");
        let out = run(&sink);
        assert!(!out.saved);
        assert_eq!(out.code, Some("memory_sink_locked"));
        assert!(out.note.unwrap().contains("does not unlock"));
    }

    #[test]
    fn a_refusal_carries_its_reason_as_a_param() {
        let sink = FakeSink::new("refused");
        let out = run(&sink);
        assert_eq!(out.code, Some("memory_sink_refused"));
        assert_eq!(
            out.params.get("reason").map(String::as_str),
            Some("duplicate")
        );
    }

    #[test]
    fn a_failure_carries_the_first_line_of_stderr() {
        let sink = FakeSink::new("failing");
        let out = run(&sink);
        assert_eq!(out.code, Some("memory_sink_failed"));
        assert_eq!(
            out.params.get("detail").map(String::as_str),
            Some("disk full")
        );
    }

    #[test]
    fn only_a_clean_exit_and_an_explicit_yes_count_as_saved() {
        for (mode, code) in [
            ("lying", "memory_sink_failed"),
            ("garbage", "memory_sink_bad_answer"),
            ("future", "memory_sink_bad_answer"),
        ] {
            let sink = FakeSink::new(mode);
            let out = run(&sink);
            assert!(!out.saved, "{mode}");
            assert_eq!(out.code, Some(code), "{mode}");
        }
    }

    #[test]
    fn a_sink_that_hangs_is_cut_at_the_deadline() {
        let sink = FakeSink::new("hang");
        let started = Instant::now();
        let out = run_with_deadline(&sink.bin(), &short_record(), Duration::from_millis(300));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(out.code, Some("memory_sink_timed_out"));
    }

    #[test]
    fn whatever_a_sink_started_dies_with_it_at_the_deadline() {
        let sink = FakeSink::new("orphan");
        let out = run_with_deadline(&sink.bin(), &short_record(), Duration::from_secs(3));
        assert_eq!(out.code, Some("memory_sink_timed_out"));
        let pid = nix::unistd::Pid::from_raw(sink.pid().expect("the orphan's pid"));
        let until = Instant::now() + Duration::from_secs(5);
        while nix::sys::signal::kill(pid, None).is_ok() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            nix::sys::signal::kill(pid, None).is_err(),
            "the process the sink started outlived it"
        );
    }

    #[test]
    fn a_sink_that_floods_its_output_cannot_hang_the_call() {
        let sink = FakeSink::new("flood");
        let started = Instant::now();
        let out = run(&sink);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(out.code, Some("memory_sink_bad_answer"));
    }
}
