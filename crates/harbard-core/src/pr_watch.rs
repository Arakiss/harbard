//! One shared watcher of pull-request state, so sessions waiting on a PR stop
//! polling GitHub themselves.
//!
//! Why this exists: sessions that each poll `gh` for the same handful of PRs
//! exhaust the account's shared API quota, and then nobody can read anything.
//! Here every interested process registers the PR
//! in one book (`lanes/v1/prs.json`) and reads the answer from it. Whoever
//! comes by after the poll interval claims the *turn* under the book's lock,
//! asks the source about a bounded batch outside the lock, and writes the
//! answers back. One turn per interval for the whole machine, however many
//! sessions wait; an hourly request budget and an exponential backoff on
//! failure bound the worst case.

use crate::lane::{PrFacts, write_private_atomic};
use crate::lease::{
    ensure_private_directory, open_private_lock, open_private_read, validate_private_regular,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// GitHub's merge queue takes minutes per PR; checking more often than every
/// three minutes buys nothing and spends the quota the incident ran out of.
pub const MIN_POLL_INTERVAL_SECS: u64 = 180;
/// A turn answers at most this many PRs; the rest wait for the next turn.
pub const MAX_REQUESTS_PER_TURN: usize = 10;
/// A ceiling far under GitHub's 5,000 requests/hour per account, so the
/// watcher can never be the thing that exhausts it for the humans.
pub const HOURLY_REQUEST_BUDGET: u32 = 120;
pub const MAX_BACKOFF_SECS: u64 = 60 * 60;
/// Interest lapses unless renewed: an awaited PR nobody waits on any more
/// stops costing requests within a day.
pub const INTEREST_TTL_MS: u64 = 24 * 60 * 60 * 1000;
/// `gh pr view` answers in about a second; a hung call must not hold a turn.
pub const SOURCE_TIMEOUT: Duration = Duration::from_secs(20);
const BOOK_SCHEMA_VERSION: u8 = 1;
const MAX_BOOK_BYTES: u64 = 512 * 1024;
const MAX_ENTRIES: usize = 256;
const HOUR_MS: u64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

impl PrState {
    pub fn settled(self) -> bool {
        !matches!(self, Self::Open)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Merged => "merged",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrEntry {
    pub repo: String,
    pub number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PrState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_ms: Option<u64>,
    pub wanted_until_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
struct PrBook {
    v: u8,
    entries: Vec<PrEntry>,
    /// When the last turn was claimed, by any process.
    turn_ms: u64,
    backoff_until_ms: u64,
    failures: u32,
    window_start_ms: u64,
    window_requests: u32,
}

impl PrBook {
    fn entry_mut(&mut self, repo: &str, number: u64) -> Option<&mut PrEntry> {
        self.entries
            .iter_mut()
            .find(|entry| entry.repo == repo && entry.number == number)
    }

    fn want(&mut self, repo: &str, number: u64, now_ms: u64) -> Result<(), PrWatchError> {
        let until = now_ms.saturating_add(INTEREST_TTL_MS);
        if let Some(entry) = self.entry_mut(repo, number) {
            entry.wanted_until_ms = entry.wanted_until_ms.max(until);
            return Ok(());
        }
        if self.entries.len() >= MAX_ENTRIES {
            return Err(PrWatchError::TooManyPrs);
        }
        self.entries.push(PrEntry {
            repo: repo.to_string(),
            number,
            state: None,
            checked_ms: None,
            wanted_until_ms: until,
        });
        Ok(())
    }
}

/// Settled PRs as last recorded in the book. Lock-free: the book is written by
/// rename, so a reader sees a whole book, possibly one turn old.
#[derive(Debug, Clone, Default)]
pub struct PrSnapshot(BTreeMap<(String, u64), PrState>);

impl PrSnapshot {
    pub fn state(&self, repo: &str, number: u64) -> Option<PrState> {
        self.0.get(&(repo.to_string(), number)).copied()
    }
}

impl PrFacts for PrSnapshot {
    fn settled(&self, repo: &str, number: u64) -> bool {
        self.state(repo, number).is_some_and(PrState::settled)
    }
}

#[derive(Debug)]
pub enum PrWatchError {
    Io(std::io::Error),
    CorruptState,
    TooManyPrs,
}

impl fmt::Display for PrWatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "pr watch storage error: {error}"),
            Self::CorruptState => f.write_str("corrupt pr watch state; refusing to mutate"),
            Self::TooManyPrs => write!(f, "the pr watch already follows {MAX_ENTRIES} PRs"),
        }
    }
}

impl std::error::Error for PrWatchError {}

impl From<std::io::Error> for PrWatchError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Where PR state comes from. Production asks `gh`; tests script the answers.
pub trait PrSource {
    fn state(&self, repo: &str, number: u64) -> Result<PrState, String>;
}

/// `gh pr view <n> --json state --jq .state`, run in the repository so `gh`
/// resolves the remote itself. `HARBARD_GH` swaps the program, which is how
/// the tests put a scripted double in its place.
#[derive(Debug, Clone)]
pub struct GhSource {
    program: OsString,
    timeout: Duration,
}

impl GhSource {
    pub fn new(program: impl Into<OsString>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            timeout,
        }
    }

    pub fn from_env() -> Self {
        Self::new(
            std::env::var_os("HARBARD_GH")
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "gh".into()),
            SOURCE_TIMEOUT,
        )
    }
}

impl PrSource for GhSource {
    fn state(&self, repo: &str, number: u64) -> Result<PrState, String> {
        let mut child = Command::new(&self.program)
            .args([
                "pr",
                "view",
                &number.to_string(),
                "--json",
                "state",
                "--jq",
                ".state",
            ])
            .current_dir(repo)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("cannot run gh: {error}"))?;
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() >= self.timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("gh pr view {number} timed out"));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(error) => return Err(format!("gh pr view {number}: {error}")),
            }
        };
        let mut output = String::new();
        if let Some(stdout) = child.stdout.take() {
            let _ = stdout.take(256).read_to_string(&mut output);
        }
        if !status.success() {
            return Err(format!("gh pr view {number} exited with {status}"));
        }
        match output.trim() {
            "OPEN" => Ok(PrState::Open),
            "MERGED" => Ok(PrState::Merged),
            "CLOSED" => Ok(PrState::Closed),
            other => Err(format!("gh pr view {number} answered {other:?}")),
        }
    }
}

/// What one call to [`PrWatch::tick`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    /// Another process polled less than an interval ago.
    NotDue,
    BackingOff {
        until_ms: u64,
    },
    BudgetSpent,
    /// Nothing unsettled is wanted.
    Idle,
    Polled {
        asked: usize,
        failed: bool,
    },
}

#[derive(Debug, Clone)]
pub struct PrWatch {
    home: PathBuf,
    root: PathBuf,
}

impl PrWatch {
    /// Shares the lanes directory: the PRs it follows are mostly lanes'.
    pub fn at(harbard_home: impl AsRef<Path>) -> Self {
        let home = harbard_home.as_ref().to_path_buf();
        Self {
            root: home.join("lanes").join("v1"),
            home,
        }
    }

    fn state_path(&self) -> PathBuf {
        self.root.join("prs.json")
    }

    /// True until something has ever been followed. A watcher loop checks this
    /// first so that merely running never creates state in a home nobody uses.
    pub fn is_unused(&self) -> bool {
        !self.state_path().exists()
    }

    /// Follow a PR (or keep following it for another day) and return what the
    /// book knows of it now.
    pub fn want(
        &self,
        repo: &str,
        number: u64,
        now_ms: u64,
    ) -> Result<Option<PrState>, PrWatchError> {
        self.with_book(|book| {
            book.want(repo, number, now_ms)?;
            Ok(book
                .entries
                .iter()
                .find(|entry| entry.repo == repo && entry.number == number)
                .and_then(|entry| entry.state))
        })
    }

    pub fn snapshot(&self) -> PrSnapshot {
        let Ok(Some(book)) = read_book(&self.state_path()) else {
            return PrSnapshot::default();
        };
        PrSnapshot(
            book.entries
                .into_iter()
                .filter_map(|entry| Some(((entry.repo, entry.number), entry.state?)))
                .collect(),
        )
    }

    /// Take the shared turn if it is due and answer a bounded batch. `also`
    /// are PRs the caller needs followed (the lanes' `release_on`), registered
    /// whether or not the turn is due.
    pub fn tick(
        &self,
        source: &dyn PrSource,
        also: &[(String, u64)],
        interval_secs: u64,
        now_ms: u64,
    ) -> Result<TickOutcome, PrWatchError> {
        let interval_ms = interval_secs.max(MIN_POLL_INTERVAL_SECS) * 1000;
        let claimed = self.with_book(|book| {
            for (repo, number) in also {
                book.want(repo, *number, now_ms)?;
            }
            book.entries.retain(|entry| entry.wanted_until_ms > now_ms);
            if now_ms < book.backoff_until_ms {
                return Ok(Err(TickOutcome::BackingOff {
                    until_ms: book.backoff_until_ms,
                }));
            }
            if book.turn_ms != 0 && now_ms < book.turn_ms.saturating_add(interval_ms) {
                return Ok(Err(TickOutcome::NotDue));
            }
            if now_ms.saturating_sub(book.window_start_ms) >= HOUR_MS {
                book.window_start_ms = now_ms;
                book.window_requests = 0;
            }
            let budget = HOURLY_REQUEST_BUDGET.saturating_sub(book.window_requests) as usize;
            let mut due: Vec<&PrEntry> = book
                .entries
                .iter()
                .filter(|entry| !entry.state.is_some_and(PrState::settled))
                .collect();
            if due.is_empty() {
                return Ok(Err(TickOutcome::Idle));
            }
            if budget == 0 {
                return Ok(Err(TickOutcome::BudgetSpent));
            }
            // Least recently answered first, so a long list rotates fairly.
            due.sort_by_key(|entry| entry.checked_ms.unwrap_or(0));
            let batch: Vec<(String, u64)> = due
                .into_iter()
                .take(budget.min(MAX_REQUESTS_PER_TURN))
                .map(|entry| (entry.repo.clone(), entry.number))
                .collect();
            book.turn_ms = now_ms;
            book.window_requests = book.window_requests.saturating_add(batch.len() as u32);
            Ok(Ok(batch))
        })?;
        let batch = match claimed {
            Ok(batch) => batch,
            Err(outcome) => return Ok(outcome),
        };
        // Outside the lock: a slow `gh` must not block anyone registering
        // interest or reading. The turn is already ours, so nobody else asks.
        let mut answers = Vec::new();
        let mut failed = false;
        for (repo, number) in &batch {
            match source.state(repo, *number) {
                Ok(state) => answers.push((repo.clone(), *number, state)),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        self.with_book(|book| {
            for (repo, number, state) in &answers {
                if let Some(entry) = book.entry_mut(repo, *number) {
                    entry.state = Some(*state);
                    entry.checked_ms = Some(now_ms);
                }
            }
            if failed {
                book.failures = book.failures.saturating_add(1);
                let backoff = interval_ms
                    .saturating_mul(1 << book.failures.min(5))
                    .min(MAX_BACKOFF_SECS * 1000);
                book.backoff_until_ms = now_ms.saturating_add(backoff);
            } else {
                book.failures = 0;
                book.backoff_until_ms = 0;
            }
            Ok(())
        })?;
        Ok(TickOutcome::Polled {
            asked: answers.len() + usize::from(failed),
            failed,
        })
    }

    fn with_book<T>(
        &self,
        operation: impl FnOnce(&mut PrBook) -> Result<T, PrWatchError>,
    ) -> Result<T, PrWatchError> {
        ensure_private_directory(&self.home)?;
        ensure_private_directory(self.root.parent().expect("pr root has parent"))?;
        ensure_private_directory(&self.root)?;
        let lock = open_private_lock(&self.root.join("prs.lock"))?;
        lock.lock_exclusive()?;
        let result = (|| {
            let path = self.state_path();
            let before = read_book(&path)?.unwrap_or(PrBook {
                v: BOOK_SCHEMA_VERSION,
                ..PrBook::default()
            });
            let mut book = before.clone();
            let value = operation(&mut book)?;
            if book != before {
                let bytes = serde_json::to_vec(&book).map_err(|_| PrWatchError::CorruptState)?;
                write_private_atomic(&self.root, "prs", &path, &bytes)?;
            }
            Ok(value)
        })();
        let unlock = FileExt::unlock(&lock);
        match (result, unlock) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(PrWatchError::Io(error)),
        }
    }
}

fn read_book(path: &Path) -> Result<Option<PrBook>, PrWatchError> {
    let mut file = match open_private_read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    validate_private_regular(&metadata)?;
    if metadata.len() > MAX_BOOK_BYTES {
        return Err(PrWatchError::CorruptState);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_BOOK_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let book: PrBook = serde_json::from_slice(&bytes).map_err(|_| PrWatchError::CorruptState)?;
    if book.v != BOOK_SCHEMA_VERSION || book.entries.len() > MAX_ENTRIES {
        return Err(PrWatchError::CorruptState);
    }
    Ok(Some(book))
}

#[cfg(test)]
#[path = "pr_watch/tests.rs"]
mod tests;
