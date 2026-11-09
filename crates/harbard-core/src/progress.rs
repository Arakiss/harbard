//! What an agent did, step by step: the hover card's "Últimos pasos" and
//! "Progreso".
//!
//! [`Tally`] rides inside the team projection ([`crate::team`]) for every
//! agent: tool calls so far, commits seen, the last test run's counts and
//! the last few finished steps. It keeps labels and numbers only.
//!
//! What never leaves here: tool input beyond the safe label of
//! [`crate::activity`], and tool output. From a test run's output only the
//! counts are parsed ([`test_counts`]); the text is dropped.

use crate::activity::{self, Call, ToolKind};
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;

/// Finished steps the team projection keeps per agent.
pub const RECENT_STEPS: usize = 5;
/// Bytes of a tool result read for test counts; a summary is at the end.
const OUTPUT_TAIL_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Test counts
// ---------------------------------------------------------------------------

/// What a test runner's summary says, and nothing else of its output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TestCounts {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

impl TestCounts {
    fn add(&mut self, other: TestCounts) {
        self.passed = self.passed.saturating_add(other.passed);
        self.failed = self.failed.saturating_add(other.failed);
        self.skipped = self.skipped.saturating_add(other.skipped);
    }
}

#[derive(Clone, Copy)]
enum Bucket {
    Passed,
    Failed,
    Skipped,
}

fn bucket(word: &str) -> Option<Bucket> {
    let w = word.trim_end_matches(['.', ':', ',', ';']);
    Some(match w {
        "passed" | "pass" | "passing" | "xpassed" | "flaky" => Bucket::Passed,
        "failed" | "fail" | "failing" | "error" | "errors" | "timed" => Bucket::Failed,
        "skipped" | "skip" | "ignored" | "todo" | "xfailed" | "pending" => Bucket::Skipped,
        _ => return None,
    })
}

/// `<n> passed`, `<n> failed`, `<n> skipped` … pairs in one summary line.
fn pairs(line: &str) -> Option<TestCounts> {
    let words: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '|' | '(' | ')' | '[' | ']'))
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = TestCounts::default();
    let mut any = false;
    for w in words.windows(2) {
        let Ok(n) = w[0].parse::<u32>() else { continue };
        let Some(b) = bucket(&w[1].to_ascii_lowercase()) else {
            continue;
        };
        any = true;
        match b {
            Bucket::Passed => out.passed = out.passed.saturating_add(n),
            Bucket::Failed => out.failed = out.failed.saturating_add(n),
            Bucket::Skipped => out.skipped = out.skipped.saturating_add(n),
        }
    }
    any.then_some(out)
}

/// Strip ANSI escape sequences (`ESC [ … letter`).
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for q in chars.by_ref() {
                    if q.is_ascii_alphabetic() {
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

/// The counts in a test run's output, from the summary lines of a known
/// runner only: cargo test (every `test result:` line, summed), nextest's
/// `Summary`, jest's and vitest's `Tests`, pytest's last line, bun test's
/// `<n> pass` / `<n> fail`, Playwright's `<n> passed`, and go test's
/// `--- PASS` / `--- FAIL` lines (else its `ok` / `FAIL` packages). Nothing
/// of the output is returned but the numbers.
pub fn test_counts(output: &str) -> Option<TestCounts> {
    test_counts_for(None, output)
}

/// Above this, a "count" is not one a test run printed.
pub const COUNT_MAX: u32 = 1_000_000;

/// The runner families whose summaries are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Cargo,
    Js,
    Py,
    Go,
    Any,
}

/// The family of a test command's safe label (`cargo test`, `bun test`,
/// `npm run test`, `pytest`, `go test`); any when the label names none.
fn family(runner: Option<&str>) -> Family {
    match runner.and_then(|r| r.split_whitespace().next()) {
        Some("cargo") => Family::Cargo,
        Some("go") => Family::Go,
        Some("pytest" | "py.test" | "python" | "python3" | "uv" | "poetry") => Family::Py,
        Some(
            "bun" | "bunx" | "npm" | "npx" | "pnpm" | "pnpx" | "yarn" | "jest" | "vitest"
            | "playwright" | "deno" | "node",
        ) => Family::Js,
        _ => Family::Any,
    }
}

/// The counts in a test run's output, read only from the summaries of the
/// runner family its label names: a `cargo
/// test` run is never read as Go's `ok` lines. A count over [`COUNT_MAX`]
/// is none at all.
pub fn test_counts_for(runner: Option<&str>, output: &str) -> Option<TestCounts> {
    let c = counts_in(family(runner), output)?;
    (c.passed.max(c.failed).max(c.skipped) <= COUNT_MAX).then_some(c)
}

fn counts_in(f: Family, output: &str) -> Option<TestCounts> {
    let on = |x: Family| f == Family::Any || f == x;
    let text = plain(output);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    // cargo test / libtest: one line per test binary.
    let mut cargo = TestCounts::default();
    let mut seen = false;
    for l in lines.iter().filter(|_| on(Family::Cargo)) {
        if let Some(rest) = l.strip_prefix("test result:")
            && let Some(c) = pairs(rest)
        {
            cargo.add(c);
            seen = true;
        }
    }
    if seen {
        return Some(cargo);
    }
    // nextest: `Summary [ 1.234s] 50 tests run: 49 passed, 1 failed, 0 skipped`
    if let Some(c) = lines
        .iter()
        .rev()
        .filter(|_| on(Family::Cargo))
        .filter(|l| {
            l.starts_with("Summary [") && (l.contains("tests run:") || l.contains("test run:"))
        })
        .find_map(|l| l.split_once("run:").and_then(|(_, r)| pairs(r)))
    {
        return Some(c);
    }
    // jest: `Tests:       1 failed, 5 passed, 6 total`
    if let Some(c) = lines
        .iter()
        .rev()
        .filter(|_| on(Family::Js))
        .find_map(|l| l.strip_prefix("Tests:").and_then(pairs))
    {
        return Some(c);
    }
    // vitest: `Tests  1 failed | 5 passed (6)`
    if let Some(c) = lines
        .iter()
        .rev()
        .filter(|_| on(Family::Js))
        .find_map(|l| l.strip_prefix("Tests ").and_then(pairs))
    {
        return Some(c);
    }
    // pytest: `==== 5 passed, 1 failed in 0.31s ====`, or `-q`'s bare line.
    if let Some(c) = lines.iter().rev().filter(|_| on(Family::Py)).find_map(|l| {
        let inner = l.trim_matches('=').trim();
        let (head, time) = inner.rsplit_once(" in ")?;
        let timed = time.starts_with(|c: char| c.is_ascii_digit()) && time.contains('s');
        (timed && head.starts_with(|c: char| c.is_ascii_digit()))
            .then(|| pairs(head))
            .flatten()
    }) {
        return Some(c);
    }
    // bun test: ` 235 pass`, ` 0 fail`, ` 3 skip` on lines of their own.
    let mut bun = TestCounts::default();
    let mut bun_seen = false;
    for l in lines.iter().filter(|_| on(Family::Js)) {
        let mut it = l.split_whitespace();
        if let (Some(n), Some(word), None) = (it.next(), it.next(), it.next())
            && let Ok(n) = n.parse::<u32>()
        {
            match word {
                "pass" => bun.passed = n,
                "fail" => bun.failed = n,
                "skip" | "todo" => bun.skipped = bun.skipped.saturating_add(n),
                _ => continue,
            }
            bun_seen = true;
        }
    }
    if bun_seen {
        return Some(bun);
    }
    // Playwright: `  5 passed (3.2s)`, `  1 failed`, `  2 skipped`, `1 flaky`.
    let mut pw = TestCounts::default();
    let mut pw_seen = false;
    for l in lines.iter().filter(|_| on(Family::Js)) {
        let head = l.split(" (").next().unwrap_or(l);
        let mut it = head.split_whitespace();
        if let (Some(n), Some(word), None) = (it.next(), it.next(), it.next())
            && let Ok(n) = n.parse::<u32>()
            && let Some(b) = bucket(word)
            && matches!(word, "passed" | "failed" | "skipped" | "flaky")
        {
            match b {
                Bucket::Passed => pw.passed = pw.passed.saturating_add(n),
                Bucket::Failed => pw.failed = pw.failed.saturating_add(n),
                Bucket::Skipped => pw.skipped = pw.skipped.saturating_add(n),
            }
            pw_seen = true;
        }
    }
    if pw_seen {
        return Some(pw);
    }
    // go test -v: one line per test; else one per package (`ok  \tpkg`).
    if !on(Family::Go) {
        return None;
    }
    let count = |p: &str| lines.iter().filter(|l| l.starts_with(p)).count() as u32;
    let (gp, gf, gs) = (count("--- PASS:"), count("--- FAIL:"), count("--- SKIP:"));
    if gp + gf + gs > 0 {
        return Some(TestCounts {
            passed: gp,
            failed: gf,
            skipped: gs,
        });
    }
    let ok = lines
        .iter()
        .filter(|l| l.starts_with("ok") && l.contains('\t'))
        .count() as u32;
    let failed = lines.iter().filter(|l| l.starts_with("FAIL\t")).count() as u32;
    (ok + failed > 0).then_some(TestCounts {
        passed: ok,
        failed,
        skipped: 0,
    })
}

/// The text of a `tool_result` block (a string, or text parts), its last
/// [`OUTPUT_TAIL_BYTES`] only. Read for test counts and dropped.
fn result_text(block: &Value) -> Option<String> {
    let text = match block.get("content")? {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    if text.len() <= OUTPUT_TAIL_BYTES {
        return Some(text);
    }
    let mut start = text.len() - OUTPUT_TAIL_BYTES;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    Some(text[start..].to_string())
}

// ---------------------------------------------------------------------------
// Steps and the tally
// ---------------------------------------------------------------------------

/// Claude Code's own tools, by name. Any other name (an MCP tool, a plugin)
/// is never shown: its step reads "Used a tool".
const TOOLS: [&str; 28] = [
    "Agent",
    "AskUserQuestion",
    "Bash",
    "BashOutput",
    "Edit",
    "ExitPlanMode",
    "Glob",
    "Grep",
    "KillShell",
    "LSP",
    "Monitor",
    "MultiEdit",
    "NotebookEdit",
    "Read",
    "SendMessage",
    "Skill",
    "Task",
    "TaskCreate",
    "TaskList",
    "TaskOutput",
    "TaskStop",
    "TaskUpdate",
    "TodoWrite",
    "ToolSearch",
    "WebFetch",
    "WebSearch",
    "Write",
    "EnterWorktree",
];

fn tool_name(name: &str) -> Option<&'static str> {
    TOOLS.iter().copied().find(|t| *t == name)
}

/// One tool call as the card and the timeline show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepRow {
    /// The face family of the call: `running`, `testing`, `editing`,
    /// `reading`, `web`, `delegating`, or `thinking` for any other tool.
    pub state: &'static str,
    /// Claude Code's tool name, only for its own tools.
    pub tool: Option<&'static str>,
    /// The safe label of [`crate::activity`]: a basename, a pattern, a host,
    /// or a program and its subcommand.
    pub detail: Option<String>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    /// `running`, `ok`, `error`, or `stopped` (the turn ended with no result).
    pub status: &'static str,
    /// A test run's counts, when its output showed them.
    pub test: Option<TestCounts>,
    /// The command runs `git commit` ([`activity::runs_git_commit`]). The
    /// message is never kept.
    pub commit: bool,
}

/// A step still in flight.
pub fn running_step(call: &Call) -> StepRow {
    let c = call.classify();
    StepRow {
        state: c.kind.state(),
        tool: tool_name(&call.name),
        detail: match c.kind {
            ToolKind::Delegate | ToolKind::Other => None,
            _ => c.detail,
        },
        started_ms: call.started_ms,
        ended_ms: None,
        status: "running",
        test: None,
        commit: call.name == "Bash" && call.arg.as_deref().is_some_and(activity::runs_git_commit),
    }
}

/// The last test run whose output showed counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TestRun {
    #[serde(flatten)]
    pub counts: TestCounts,
    /// No failure counted and the command did not fail.
    pub ok: bool,
    pub at_ms: u64,
}

/// What the hover card's "Progreso" and "Últimos pasos" read: only signals
/// seen in the transcript, never an estimate.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub tool_calls: usize,
    pub commits: usize,
    pub test: Option<TestRun>,
    /// The newest steps first, in flight or finished, at most
    /// [`RECENT_STEPS`].
    pub steps: Vec<StepRow>,
    /// Calls that waited [`STALL_MS`] or more for their result, oldest
    /// first, at most [`STALLS_KEPT`]: the shape of a permission prompt
    /// nobody answered. Never shown on the card; the retro reads them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stalls: Vec<Stall>,
}

/// A call that waited long for its result. Only the tool, a pattern of the
/// command (never the command), when and how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Stall {
    pub tool: &'static str,
    /// `rm_variable` (an `rm` on an unguarded `$VAR`), `shell_wrapper`
    /// (`bash -c`, `sh -c`, `eval`), or `other`.
    pub pattern: &'static str,
    pub started_ms: u64,
    pub gap_ms: u64,
}

/// A wait this long between a call and its result is a stall.
pub const STALL_MS: u64 = 10 * 60_000;
/// Stalls kept per transcript.
pub const STALLS_KEPT: usize = 32;

/// The pattern of a shell command that sat waiting, from its words only.
pub fn stall_pattern(cmd: &str) -> &'static str {
    let segments = cmd.split(['\n', ';', '|', '&']);
    let mut wrapper = false;
    for seg in segments {
        let words: Vec<&str> = seg.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            let prog = w.rsplit('/').next().unwrap_or(w);
            if matches!(prog, "bash" | "sh" | "zsh") && words.get(i + 1) == Some(&"-c")
                || prog == "eval"
            {
                wrapper = true;
            }
            if prog == "rm"
                && words[i + 1..]
                    .iter()
                    .any(|a| a.contains('$') && !a.contains(":?"))
            {
                return "rm_variable";
            }
        }
    }
    if wrapper { "shell_wrapper" } else { "other" }
}

/// Is a long wait for this call expected (tests, builds-by-sleep,
/// delegation), rather than a prompt?
fn long_by_nature(call: &Call) -> bool {
    let c = call.classify();
    if matches!(c.kind, ToolKind::Test | ToolKind::Delegate) {
        return true;
    }
    call.name == "Bash"
        && call.arg.as_deref().is_some_and(|cmd| {
            cmd.split_whitespace()
                .any(|w| matches!(w, "sleep" | "wait" | "timeout" | "gtimeout"))
        })
}

/// Counters and the last finished steps, folded call by call.
#[derive(Debug, Clone, Default)]
pub struct Tally {
    pub tool_calls: usize,
    pub commits: usize,
    pub test: Option<TestRun>,
    recent: VecDeque<StepRow>,
    stalls: VecDeque<Stall>,
}

impl Tally {
    /// A `tool_use` block was seen.
    pub fn called(&mut self) {
        self.tool_calls += 1;
    }

    /// `call` got its `tool_result` (`block`) at `ended_ms`.
    pub fn finish(&mut self, call: &Call, ended_ms: u64, block: &Value) -> StepRow {
        let is_error = block.get("is_error").and_then(Value::as_bool) == Some(true);
        let gap = ended_ms.saturating_sub(call.started_ms);
        if gap >= STALL_MS && !long_by_nature(call) {
            if self.stalls.len() >= STALLS_KEPT {
                self.stalls.pop_front();
            }
            self.stalls.push_back(Stall {
                tool: tool_name(&call.name).unwrap_or("other"),
                pattern: match (call.name.as_str(), call.arg.as_deref()) {
                    ("Bash", Some(cmd)) => stall_pattern(cmd),
                    _ => "other",
                },
                started_ms: call.started_ms,
                gap_ms: gap,
            });
        }
        let mut step = running_step(call);
        step.ended_ms = Some(ended_ms);
        step.status = if is_error { "error" } else { "ok" };
        if step.commit && !is_error {
            self.commits += 1;
        }
        if step.state == "testing"
            && let Some(counts) = result_text(block)
                .as_deref()
                .and_then(|out| test_counts_for(step.detail.as_deref(), out))
        {
            step.test = Some(counts);
            self.test = Some(TestRun {
                counts,
                ok: counts.failed == 0 && !is_error,
                at_ms: ended_ms,
            });
        }
        if self.recent.len() >= RECENT_STEPS {
            self.recent.pop_front();
        }
        self.recent.push_back(step.clone());
        step
    }

    /// `call` never got a result: its turn ended first.
    pub fn stopped(&mut self, call: &Call, ended_ms: u64) {
        let mut step = running_step(call);
        step.ended_ms = Some(ended_ms);
        step.status = "stopped";
        if self.recent.len() >= RECENT_STEPS {
            self.recent.pop_front();
        }
        self.recent.push_back(step);
    }

    /// The card's view: the calls still in flight and the finished ones,
    /// newest first.
    pub fn progress<'a>(&self, pending: impl Iterator<Item = &'a Call>) -> Progress {
        let mut steps: Vec<StepRow> = pending.map(running_step).collect();
        steps.extend(self.recent.iter().cloned());
        steps.sort_by_key(|s| std::cmp::Reverse((s.started_ms, s.ended_ms.is_none())));
        steps.truncate(RECENT_STEPS);
        Progress {
            tool_calls: self.tool_calls,
            commits: self.commits,
            test: self.test,
            steps,
            stalls: self.stalls.iter().copied().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_800_000_000_000;

    fn counts(passed: u32, failed: u32, skipped: u32) -> Option<TestCounts> {
        Some(TestCounts {
            passed,
            failed,
            skipped,
        })
    }

    #[test]
    fn test_counts_come_from_each_runners_summary_only() {
        let cases: &[(&str, &str, Option<TestCounts>)] = &[
            (
                "cargo test, every binary summed",
                "running 3 tests\ntest a ... ok\n\ntest result: ok. 800 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 4.20s\n\ntest result: FAILED. 9 passed; 1 failed; 1 ignored; 0 measured\n",
                counts(809, 1, 3),
            ),
            (
                "nextest",
                "    Starting 50 tests\n------------\n     Summary [   1.234s] 50 tests run: 49 passed, 1 failed, 2 skipped\n",
                counts(49, 1, 2),
            ),
            (
                "jest",
                "PASS src/a.test.ts\nTests:       1 failed, 2 skipped, 5 passed, 8 total\nSnapshots:   0 total\n",
                counts(5, 1, 2),
            ),
            (
                "vitest",
                " Test Files  2 passed (2)\n      Tests  1 failed | 41 passed | 3 skipped (45)\n   Start at  10:00:00\n",
                counts(41, 1, 3),
            ),
            (
                "pytest",
                "collected 7 items\n\ntests/test_a.py ..F.s..\n=========== 5 passed, 1 failed, 1 skipped in 0.31s ===========\n",
                counts(5, 1, 1),
            ),
            ("pytest -q", "....\n4 passed in 0.02s\n", counts(4, 0, 0)),
            (
                "bun test",
                "src/a.test.ts:\n(pass) one [0.1ms]\n\n 235 pass\n 2 skip\n 1 fail\n 812 expect() calls\nRan 238 tests across 20 files. [1.2s]\n",
                counts(235, 1, 2),
            ),
            (
                "playwright",
                "Running 7 tests using 1 worker\n  ✓  1 a.spec.ts:3:1 › opens (1.2s)\n\n  1 failed\n    a.spec.ts:9:1 › closes\n  5 passed (3.2s)\n  1 skipped\n",
                counts(5, 1, 1),
            ),
            (
                "go test -v",
                "=== RUN   TestA\n--- PASS: TestA (0.00s)\n=== RUN   TestB\n--- FAIL: TestB (0.00s)\nFAIL\nFAIL\texample.com/x\t0.01s\n",
                counts(1, 1, 0),
            ),
            (
                "go test, packages",
                "ok  \texample.com/a\t0.012s\nok  \texample.com/b\t(cached)\nFAIL\texample.com/c\t0.3s\n",
                counts(2, 1, 0),
            ),
            (
                "ANSI colours",
                "\u{1b}[32m 12 pass\u{1b}[0m\n\u{1b}[31m 0 fail\u{1b}[0m\n",
                counts(12, 0, 0),
            ),
            (
                "a build error: no counts",
                "error[E0425]: cannot find value `x` in this scope\n  --> src/lib.rs:3:5\nerror: could not compile `x` (lib test) due to 1 previous error\n",
                None,
            ),
            (
                "prose that names numbers",
                "We have 3 apples and the tests passed yesterday.\nThe 2 failed attempts were mine.\n",
                None,
            ),
        ];
        for (name, out, want) in cases {
            assert_eq!(test_counts(out), *want, "{name}");
        }
    }

    /// A summary is read only from the runner the label names,
    /// and an absurd count is none.
    #[test]
    fn counts_come_from_the_runner_the_label_names_and_stay_plausible() {
        let tap = "TAP version 13\nok 1 - a\nok 2 - b\nnot ok 3 - c\n1..3\n";
        assert_eq!(test_counts_for(Some("bun test"), tap), None);
        assert_eq!(
            test_counts_for(None, tap),
            None,
            "TAP lines are not go packages"
        );
        assert_eq!(
            test_counts_for(Some("cargo test"), "ok so the thing works\nok cool\n"),
            None
        );
        assert_eq!(
            test_counts_for(
                Some("cargo test"),
                "test result: ok. 4294967295 passed; 0 failed\n"
            ),
            None
        );
        assert_eq!(
            test_counts_for(Some("cargo test"), "PIN\n 482913 pass\n"),
            None
        );
        assert_eq!(
            test_counts_for(Some("go test"), "test result: ok. 3 passed; 0 failed\n"),
            None
        );
        assert_eq!(
            test_counts_for(
                Some("go test"),
                "ok  \texample.com/a\t0.01s\nFAIL\texample.com/b\t0.2s\n"
            ),
            counts(1, 1, 0)
        );
        assert_eq!(
            test_counts_for(Some("bun test"), " 12 pass\n 1 fail\n"),
            counts(12, 1, 0)
        );
        assert_eq!(
            test_counts_for(
                Some("npm test"),
                "Tests:       1 failed, 5 passed, 6 total\n"
            ),
            counts(5, 1, 0)
        );
        assert_eq!(
            test_counts_for(Some("pytest"), "== 3 passed in 0.2s ==\n"),
            counts(3, 0, 0)
        );
        assert_eq!(
            test_counts_for(
                Some("cargo test"),
                "test result: ok. 809 passed; 0 failed; 3 ignored\n"
            ),
            counts(809, 0, 3)
        );
    }

    /// A Grep pattern or a file name that holds a secret by the
    /// prose rules is no label.
    #[test]
    fn a_label_never_shows_what_the_prose_filter_would_mask() {
        let label = |name: &str, input: Value| call(name, input, NOW).classify().detail;
        assert_eq!(label("Grep", json!({"pattern":"DB_PASS=hunter2"})), None);
        assert_eq!(label("Grep", json!({"pattern":"KEY=Sup3r-S3cret"})), None);
        assert_eq!(
            label(
                "Grep",
                json!({"pattern":"550e8400-e29b-41d4-a716-446655440000"})
            ),
            None
        );
        assert_eq!(label("Read", json!({"file_path":"/w/x=hunter2abc"})), None);
        assert_eq!(label("Edit", json!({"file_path":"/w/TOKEN=hunter2"})), None);
        assert_eq!(
            label("Grep", json!({"pattern":"fn state_of"})).as_deref(),
            Some("fn state_of")
        );
        assert_eq!(
            label("Read", json!({"file_path":"/w/src/team.rs"})).as_deref(),
            Some("team.rs")
        );
    }

    fn call(name: &str, input: Value, ms: u64) -> Call {
        Call::of(name, &input, ms)
    }

    fn result_block(id: &str, output: &str, is_error: bool) -> Value {
        json!({"type":"tool_result","tool_use_id":id,"content":output,"is_error":is_error})
    }

    #[test]
    fn a_test_run_keeps_its_counts_and_none_of_its_output() {
        let mut t = Tally::default();
        let c = call(
            "Bash",
            json!({"command":concat!("GITHUB_TOKEN=ghp_abcd", "efghijklmnopqrstuvwxyz0123456789 cargo test -p x --password hunter2 2>&1 | tail -5")}),
            NOW - 9_000,
        );
        t.called();
        let out = concat!(
            "OUTPUT-SENTINEL thread 'a' panicked at src/lib.rs\nsk-ant-",
            "api03-LEAKEDLEAKEDLEAKED0123456789\ntest result: FAILED. 41 passed; 2 failed; 0 ignored\n"
        );
        let step = t.finish(&c, NOW - 1_000, &result_block("t", out, true));
        assert_eq!(step.state, "testing");
        assert_eq!(step.detail.as_deref(), Some("cargo test"));
        assert_eq!(step.status, "error");
        assert_eq!(step.test, counts(41, 2, 0));
        let run = t.test.unwrap();
        assert_eq!(
            (run.counts, run.ok, run.at_ms),
            (counts(41, 2, 0).unwrap(), false, NOW - 1_000)
        );
        let wire = serde_json::to_string(&t.progress(std::iter::empty())).unwrap();
        for leak in [
            "OUTPUT-SENTINEL",
            "panicked",
            "LEAKED",
            "ghp_",
            "hunter2",
            "GITHUB_TOKEN",
            "tail -5",
        ] {
            assert!(!wire.contains(leak), "{leak} reached the progress: {wire}");
        }
        // A passing run with no counts in its output is no test result.
        let quiet = call("Bash", json!({"command":"bun test src"}), NOW - 500);
        t.finish(&quiet, NOW - 400, &result_block("u", "Done.", false));
        assert_eq!(
            t.test.unwrap().at_ms,
            NOW - 1_000,
            "the last run with counts stays"
        );
        // Output as text parts, too.
        let parts = json!({"type":"tool_result","tool_use_id":"v","content":[{"type":"text","text":" 7 pass\n 0 fail\n"}]});
        t.finish(
            &call("Bash", json!({"command":"bun test"}), NOW - 300),
            NOW - 200,
            &parts,
        );
        let run = t.test.unwrap();
        assert_eq!((run.counts, run.ok), (counts(7, 0, 0).unwrap(), true));
    }

    #[test]
    fn commits_count_successful_git_commit_runs_only() {
        let mut t = Tally::default();
        let ok = |cmd: &str| (cmd.to_string(), false);
        let runs = [
            ok("git add -A && git commit -m 'feat: x'"),
            ok("git -C /w/repo commit --amend --no-edit"),
            ok("cd /w && git commit -qm \"$(cat msg.txt)\""),
            ("git commit -m 'refused by a hook'".to_string(), true),
            ok("echo 'git commit -m x'"),
            ok("git log --grep commit"),
            ok("grep -rn 'git commit' docs"),
            ok("git status"),
        ];
        for (i, (cmd, err)) in runs.iter().enumerate() {
            let c = call("Bash", json!({"command":cmd}), NOW + i as u64);
            t.finish(&c, NOW + i as u64, &result_block("x", "", *err));
        }
        assert_eq!(t.commits, 3);
        // The step says it is a commit, refused or not, and keeps no message.
        let refused = call(
            "Bash",
            json!({"command":"git commit -m 'SECRET-MESSAGE'"}),
            NOW,
        );
        let step = t.finish(&refused, NOW, &result_block("y", "", true));
        assert!(step.commit);
        assert_eq!(t.commits, 3);
        let wire = serde_json::to_string(&t.progress(std::iter::empty())).unwrap();
        assert!(!wire.contains("SECRET-MESSAGE"));
        assert!(!running_step(&call("Bash", json!({"command":"git status"}), NOW)).commit);
        assert!(
            running_step(&call(
                "Bash",
                json!({"command":"git -C /w commit -qm x"}),
                NOW
            ))
            .commit
        );
    }

    #[test]
    fn the_last_steps_are_the_newest_five_in_flight_or_finished() {
        let mut t = Tally::default();
        let inputs = [
            ("Read", json!({"file_path":"/w/a.rs"})),
            ("Grep", json!({"pattern":"fn main"})),
            ("Edit", json!({"file_path":"/w/src/live.rs"})),
            ("WebFetch", json!({"url":"https://docs.rs/serde?token=abc"})),
            ("Bash", json!({"command":"git status"})),
            ("mcp__secret_server__do_thing", json!({"x":"y"})),
        ];
        for (i, (name, input)) in inputs.iter().enumerate() {
            let at = NOW + i as u64 * 1_000;
            t.called();
            let error = i == 4;
            t.finish(
                &call(name, input.clone(), at),
                at + 500,
                &result_block("x", "", error),
            );
        }
        let running = call(
            "Bash",
            json!({"command":"cargo test --workspace"}),
            NOW + 10_000,
        );
        t.called();
        let p = t.progress(std::iter::once(&running));
        assert_eq!(p.tool_calls, 7);
        let rows: Vec<(&str, Option<&str>, Option<&str>, &str)> = p
            .steps
            .iter()
            .map(|s| (s.state, s.tool, s.detail.as_deref(), s.status))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("testing", Some("Bash"), Some("cargo test"), "running"),
                ("thinking", None, None, "ok"),
                ("running", Some("Bash"), Some("git status"), "error"),
                ("web", Some("WebFetch"), Some("docs.rs"), "ok"),
                ("editing", Some("Edit"), Some("live.rs"), "ok"),
            ]
        );
        assert!(p.steps[0].ended_ms.is_none());
        assert_eq!(p.steps[1].ended_ms, Some(NOW + 5_500));
        let wire = serde_json::to_string(&p).unwrap();
        assert!(
            !wire.contains("secret_server"),
            "an MCP tool name is never shown"
        );
        assert!(!wire.contains("token=abc"));
    }

    #[test]
    fn a_long_wait_for_a_result_is_a_stall_with_a_pattern_and_no_command() {
        let mut t = Tally::default();
        let call = |name: &str, input: serde_json::Value, at: u64| Call::of(name, &input, at);
        let ok = json!({"type": "tool_result", "content": "ok"});
        let min = 60_000;
        t.finish(
            &call("Bash", json!({"command": "rm -rf $BUILD_DIR/out"}), NOW),
            NOW + 60 * min,
            &ok,
        );
        t.finish(
            &call("Bash", json!({"command": "bash -c 'git status'"}), NOW),
            NOW + 12 * min,
            &ok,
        );
        t.finish(
            &call("Bash", json!({"command": "rm -rf \"${OUT:?}\""}), NOW),
            NOW + 11 * min,
            &ok,
        );
        // Quick, or long by nature: no stall.
        t.finish(
            &call("Read", json!({"file_path": "/a/b.rs"}), NOW),
            NOW + min,
            &ok,
        );
        t.finish(
            &call("Bash", json!({"command": "cargo test --workspace"}), NOW),
            NOW + 40 * min,
            &ok,
        );
        t.finish(
            &call("Agent", json!({"prompt": "x"}), NOW),
            NOW + 90 * min,
            &ok,
        );
        t.finish(
            &call("Bash", json!({"command": "sleep 900"}), NOW),
            NOW + 15 * min,
            &ok,
        );
        let p = t.progress(std::iter::empty());
        let got: Vec<(&str, &str, u64)> = p
            .stalls
            .iter()
            .map(|s| (s.tool, s.pattern, s.gap_ms / min))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Bash", "rm_variable", 60),
                ("Bash", "shell_wrapper", 12),
                ("Bash", "other", 11)
            ]
        );
        let wire = serde_json::to_string(&p).unwrap();
        assert!(!wire.contains("BUILD_DIR"), "the command is never kept");
    }
}
