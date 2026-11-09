//! What an agent is doing right now, read from the same transcript records
//! the team view already folds, and the short label the console may show
//! beside it.
//!
//! The label is the only text that leaves here, and it is built to be safe
//! to show: a file's basename, a Grep pattern cut to 40 characters, a web
//! host, or a shell program with at most two plain subcommand words
//! (`cargo test`, `bun run build`, `git status`). Nothing else of a command
//! line survives: no arguments, no paths, no env assignments, no URL query,
//! and no word that looks like a token, a key or a password.
//!
//! The states are the robot faces of the console. `disconnected` is the
//! console's own (serve does not answer), so it is never produced here.

use crate::redact::invisible;
pub use crate::redact::{MASK, looks_secret, mask_secrets};
use serde::Serialize;
use serde_json::Value;

/// Every state serve may send, in the order the console documents them.
pub const STATES: [&str; 13] = [
    "normal",
    "thinking",
    "running",
    "testing",
    "editing",
    "reading",
    "web",
    "delegating",
    "waiting",
    "asleep",
    "blocked",
    "finished",
    "verified",
];

/// A quick tool (an edit, a read, a fetch) keeps its state this long after
/// its result, so the face does not flicker back to "thinking" between two
/// edits.
pub const LINGER_MS: u64 = 20_000;
/// The tail of an assistant reply the wait phrases are looked for in, as in
/// `~/.claude/hooks/teammate-idle-guard.py`.
pub const WAIT_TAIL_CHARS: usize = 1_500;
/// Grep patterns are cut to this many characters.
pub const PATTERN_CHARS: usize = 40;

/// `activity` on an agent: its state, since when, a safe label, and the
/// newest record the derivation read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Activity {
    pub state: &'static str,
    pub since_ms: u64,
    pub detail: Option<String>,
    pub source_ms: u64,
}

impl Activity {
    pub fn new(state: &'static str, since_ms: u64, detail: Option<String>, source_ms: u64) -> Self {
        Self {
            state,
            since_ms,
            detail,
            source_ms,
        }
    }

    /// The plain face of a team-view state, with no tool evidence: what a
    /// row built by hand (tests, older callers) carries.
    pub fn plain(state: &str, ms: u64) -> Self {
        let face = match state {
            "running" => "thinking",
            "done" => "finished",
            _ => "normal",
        };
        Self::new(face, ms, None, ms)
    }
}

/// The families of tool calls the faces tell apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Delegate,
    Test,
    Bash,
    Web,
    Edit,
    Read,
    Other,
}

impl ToolKind {
    /// Which call wins when several run at once.
    pub fn rank(self) -> u8 {
        match self {
            ToolKind::Delegate => 6,
            ToolKind::Test => 5,
            ToolKind::Bash => 4,
            ToolKind::Web => 3,
            ToolKind::Edit => 2,
            ToolKind::Read => 1,
            ToolKind::Other => 0,
        }
    }

    pub fn state(self) -> &'static str {
        match self {
            ToolKind::Delegate => "delegating",
            ToolKind::Test => "testing",
            ToolKind::Bash => "running",
            ToolKind::Web => "web",
            ToolKind::Edit => "editing",
            ToolKind::Read => "reading",
            ToolKind::Other => "thinking",
        }
    }

    /// Quick tools whose state outlives their result by [`LINGER_MS`].
    pub fn lingers(self) -> bool {
        matches!(self, ToolKind::Edit | ToolKind::Read | ToolKind::Web)
    }
}

/// One tool call, classified, with its safe label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub kind: ToolKind,
    pub detail: Option<String>,
    pub started_ms: u64,
}

/// The one input field a call's label is made from: the file, the pattern,
/// the URL or the command. Kept while the call is in flight, so labels are
/// built only for what is shown, never for a transcript's whole history.
pub fn key_arg(name: &str, input: &Value) -> Option<String> {
    let s = |k: &str| input.get(k).and_then(Value::as_str);
    let arg = match name {
        "Edit" | "Write" | "MultiEdit" | "Read" => s("file_path"),
        "NotebookEdit" => s("notebook_path").or_else(|| s("file_path")),
        "Glob" | "Grep" => s("pattern"),
        "WebFetch" => s("url"),
        "Bash" => s("command"),
        _ => None,
    }?;
    // A label never needs more; a pasted script is cut, not kept whole.
    Some(arg.chars().take(ARG_CHARS).collect())
}

/// Characters of a call's key field kept while it runs.
pub const ARG_CHARS: usize = 8_192;

/// Classify a `tool_use` block by name and input.
pub fn classify(name: &str, input: &Value, started_ms: u64) -> ToolCall {
    classify_arg(name, key_arg(name, input).as_deref(), started_ms)
}

/// Classify a call from its name and the field [`key_arg`] kept.
pub fn classify_arg(name: &str, arg: Option<&str>, started_ms: u64) -> ToolCall {
    let (kind, detail) = match name {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => (ToolKind::Edit, arg.and_then(basename)),
        "Read" | "Glob" => (ToolKind::Read, arg.and_then(basename)),
        "Grep" => (ToolKind::Read, arg.and_then(pattern)),
        "WebFetch" => (ToolKind::Web, arg.and_then(host)),
        "WebSearch" => (ToolKind::Web, None),
        "Agent" | "Task" => (ToolKind::Delegate, None),
        "Bash" => match arg.map(command_label) {
            Some(label) => (
                if label.test {
                    ToolKind::Test
                } else {
                    ToolKind::Bash
                },
                label.text,
            ),
            None => (ToolKind::Bash, None),
        },
        _ => (ToolKind::Other, None),
    };
    ToolCall {
        kind,
        detail,
        started_ms,
    }
}

/// Does a call of this tool keep its face for [`LINGER_MS`] after its result?
pub fn lingers(name: &str) -> bool {
    matches!(
        name,
        "Edit"
            | "Write"
            | "MultiEdit"
            | "NotebookEdit"
            | "Read"
            | "Glob"
            | "Grep"
            | "WebFetch"
            | "WebSearch"
    )
}

/// A call as the transcript fold keeps it: name, key field, start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub name: String,
    pub arg: Option<String>,
    pub started_ms: u64,
}

impl Call {
    pub fn of(name: &str, input: &Value, started_ms: u64) -> Self {
        Call {
            name: name.chars().take(64).collect(),
            arg: key_arg(name, input),
            started_ms,
        }
    }

    pub fn classify(&self) -> ToolCall {
        classify_arg(&self.name, self.arg.as_deref(), self.started_ms)
    }
}

// ---------------------------------------------------------------------------
// Safe labels
// ---------------------------------------------------------------------------

/// No control or invisible format characters, no surrounding space, at most
/// `max` characters.
fn clean(text: &str, max: usize) -> Option<String> {
    let cleaned: String = text
        .chars()
        .filter(|c| !invisible(*c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.chars().count() <= max {
        return Some(cleaned);
    }
    let mut out: String = cleaned.chars().take(max).collect();
    out = out.trim_end().to_string();
    out.push('…');
    Some(out)
}

/// The last path component, when it names nothing secret.
pub fn basename(path: &str) -> Option<String> {
    let name = path.trim_end_matches('/').rsplit('/').next()?;
    let name = clean(name, 60)?;
    if looks_secret(&name)
        || name.to_ascii_lowercase().starts_with(".env")
        || mask_secrets(&name) != name
    {
        return None;
    }
    Some(name)
}

/// A Grep pattern, cut to [`PATTERN_CHARS`]; none when it looks secret.
pub fn pattern(p: &str) -> Option<String> {
    let p = clean(p, usize::MAX)?;
    // The prose rules too: `DB_PASS=hunter2`, `KEY=Sup3r-S3cret`, a UUID
    //.
    if p.split_whitespace().any(looks_secret) || looks_secret(&p) || mask_secrets(&p) != p {
        return None;
    }
    clean(&p, PATTERN_CHARS)
}

/// The host a WHATWG URL parser (Node, Chrome) would fetch, and nothing
/// else: no scheme, user, port, path or query. Only http and https; for
/// them `\\` ends the authority like `/`, and the userinfo is what comes
/// before the last `@` inside it. Anything doubtful is no label at all.
pub fn host(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = url.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let rest = rest.trim_start_matches(['/', '\\']);
    let authority = rest.split(['/', '\\', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if host.starts_with('[') {
        let (v6, _) = host.split_once(']')?;
        format!("{v6}]")
    } else {
        host.split(':').next()?.to_string()
    };
    let host = host.to_ascii_lowercase();
    let host = host.trim_end_matches('.');
    let ok = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'))
        && !host.contains("..");
    (ok && !looks_secret(host)).then(|| host.to_string())
}

/// A shell command's label and whether it runs tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLabel {
    pub text: Option<String>,
    pub test: bool,
}

/// Programs that set up or tidy around the real command.
const PLUMBING: [&str; 32] = [
    "cd", "pushd", "popd", "export", "set", "unset", "source", ".", "echo", "printf", "true",
    "false", "sleep", "tail", "head", "tee", "cat", "wc", "sort", "uniq", "mkdir", "test", "[",
    "read", "wait", "exit", "local", "trap", "ulimit", "alias", "ls", "pwd",
];
/// Prefixes that run the next word as the program.
const WRAPPERS: [&str; 8] = [
    "env", "sudo", "time", "nice", "nohup", "exec", "command", "builtin",
];
/// A first subcommand after which one more plain word still names the action.
const RUNNERS: [&str; 5] = ["run", "exec", "x", "dlx", "nextest"];

/// Programs a label may name without a look at PATH: the usual tools of a
/// coding agent. Anything else is named only when serve found it on PATH
/// at startup ([`set_path_programs`]); otherwise the label is empty and the
/// console says "Running a command".
const KNOWN: &[&str] = &[
    "ag",
    "awk",
    "base64",
    "basename",
    "bash",
    "biome",
    "brew",
    "bun",
    "bunx",
    "cargo",
    "cat",
    "cd",
    "chmod",
    "clang",
    "claude",
    "cmake",
    "codex",
    "cp",
    "curl",
    "cut",
    "cwebp",
    "date",
    "defaults",
    "deno",
    "df",
    "diff",
    "dirname",
    "docker",
    "du",
    "dwebp",
    "echo",
    "env",
    "eslint",
    "export",
    "fd",
    "ffmpeg",
    "file",
    "find",
    "gh",
    "git",
    "go",
    "gradle",
    "grep",
    "gzip",
    "harbard",
    "head",
    "jest",
    "jq",
    "just",
    "kill",
    "kubectl",
    "launchctl",
    "ln",
    "ls",
    "lsof",
    "magick",
    "make",
    "md5",
    "mkdir",
    "mv",
    "mysql",
    "node",
    "npm",
    "npx",
    "open",
    "openssl",
    "patch",
    "pbcopy",
    "pbpaste",
    "perl",
    "pg_dump",
    "pgrep",
    "pip",
    "pip3",
    "pkill",
    "playwright",
    "plutil",
    "pnpm",
    "pnpx",
    "poetry",
    "prettier",
    "printf",
    "ps",
    "psql",
    "pwd",
    "pytest",
    "python",
    "python3",
    "readlink",
    "realpath",
    "redis-cli",
    "rg",
    "rm",
    "rsvg-convert",
    "rsync",
    "ruby",
    "rustc",
    "rustup",
    "scp",
    "sed",
    "sh",
    "shasum",
    "sips",
    "sleep",
    "sort",
    "sqlite3",
    "ssh",
    "stat",
    "swift",
    "tail",
    "tar",
    "tee",
    "terraform",
    "test",
    "timeout",
    "touch",
    "tr",
    "true",
    "tsc",
    "uniq",
    "unzip",
    "uv",
    "vitest",
    "wc",
    "wget",
    "which",
    "xargs",
    "xcodebuild",
    "xcrun",
    "yarn",
    "yq",
    "zip",
    "zsh",
];

/// The only programs whose first plain word is shown, as a subcommand, and
/// the subcommands each may show. A word off its program's list is data
/// (a recipe, a script, a path) and is never shown: a list, not a shape,
/// is what keeps a random word out of a label.
const SUBCOMMANDS: &[(&str, &[&str])] = &[
    (
        "brew",
        &[
            "bundle",
            "cleanup",
            "doctor",
            "info",
            "install",
            "link",
            "list",
            "outdated",
            "search",
            "services",
            "tap",
            "uninstall",
            "unlink",
            "update",
            "upgrade",
        ],
    ),
    (
        "bun",
        &[
            "add", "build", "create", "init", "install", "link", "outdated", "pm", "remove", "run",
            "test", "update", "upgrade", "x",
        ],
    ),
    (
        "cargo",
        &[
            "add", "audit", "bench", "build", "check", "clean", "clippy", "deny", "doc", "fix",
            "fmt", "fuzz", "init", "install", "llvm-cov", "metadata", "new", "nextest", "publish",
            "remove", "run", "test", "tree", "update", "vet",
        ],
    ),
    (
        "docker",
        &[
            "build", "compose", "exec", "images", "inspect", "logs", "network", "ps", "pull",
            "push", "rm", "rmi", "run", "stop", "system", "tag", "volume",
        ],
    ),
    (
        "gh",
        &[
            "api", "auth", "browse", "gist", "issue", "label", "pr", "release", "repo", "run",
            "search", "status", "workflow",
        ],
    ),
    (
        "git",
        &[
            "add",
            "am",
            "apply",
            "archive",
            "bisect",
            "blame",
            "branch",
            "checkout",
            "cherry-pick",
            "clean",
            "clone",
            "commit",
            "config",
            "describe",
            "diff",
            "fetch",
            "format-patch",
            "gc",
            "grep",
            "init",
            "log",
            "ls-files",
            "merge",
            "mv",
            "notes",
            "pull",
            "push",
            "rebase",
            "reflog",
            "remote",
            "reset",
            "restore",
            "rev-parse",
            "revert",
            "rm",
            "shortlog",
            "show",
            "stash",
            "status",
            "submodule",
            "switch",
            "tag",
            "worktree",
        ],
    ),
    (
        "go",
        &[
            "build", "clean", "env", "fmt", "generate", "get", "install", "list", "mod", "run",
            "test", "tool", "version", "vet", "work",
        ],
    ),
    (
        "harbard",
        &[
            "audit",
            "deliver",
            "doctor",
            "gate",
            "hub",
            "install",
            "integrate",
            "lane",
            "protocol",
            "serve",
            "setup",
            "status",
            "team",
            "theme",
            "trace",
            "watch",
        ],
    ),
    ("just", &[]),
    (
        "npm",
        &[
            "add", "audit", "ci", "exec", "i", "init", "install", "list", "ls", "outdated",
            "publish", "rebuild", "remove", "run", "test", "update", "why",
        ],
    ),
    (
        "pnpm",
        &[
            "add", "audit", "dlx", "exec", "i", "install", "list", "ls", "outdated", "publish",
            "remove", "run", "test", "update", "why",
        ],
    ),
];

/// Package scripts a runner (`bun run`, `npm run`, `cargo nextest`) may name.
const SCRIPTS: &[&str] = &[
    "archive",
    "build",
    "check",
    "clean",
    "dev",
    "e2e",
    "format",
    "generate",
    "lint",
    "list",
    "mock",
    "preview",
    "run",
    "serve",
    "shot",
    "start",
    "test",
    "typecheck",
    "watch",
];

fn vocabulary(program: &str) -> Option<&'static [&'static str]> {
    SUBCOMMANDS
        .iter()
        .find(|(p, _)| *p == program)
        .map(|(_, v)| *v)
}

fn path_programs() -> &'static std::sync::RwLock<std::collections::BTreeSet<String>> {
    static SET: std::sync::OnceLock<std::sync::RwLock<std::collections::BTreeSet<String>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(Default::default)
}

/// The executable names serve found in its PATH directories at startup.
/// Labels name a program only if it is on [`KNOWN`] or in this set.
pub fn set_path_programs(names: impl IntoIterator<Item = String>) {
    let mut set = path_programs().write().unwrap_or_else(|e| e.into_inner());
    set.extend(names.into_iter().filter(|n| n.len() <= 64));
}

/// May a label name this program?
pub fn known_program(name: &str) -> bool {
    KNOWN.binary_search(&name).is_ok()
        || path_programs()
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains(name)
}

/// `[a-z][a-z0-9-]{1,24}`: what a subcommand looks like.
fn subcommand_word(w: &str) -> bool {
    let mut chars = w.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && (2..=25).contains(&w.len())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !looks_secret(w)
}

fn plain_program(token: &str) -> Option<String> {
    let name = token.rsplit('/').next()?;
    let ok = !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '.');
    (ok && !looks_secret(name) && known_program(name)).then(|| name.to_string())
}

/// One shell word: its text with the quotes taken off, and whether any part
/// of it was quoted. A quoted word is data, never a program or a subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Word {
    text: String,
    quoted: bool,
}

/// Simple commands of a shell line, word by word, as a POSIX shell splits
/// them (the way shlex does): `'…'` and `"…"` keep spaces, `;`, `|`, `&`,
/// newlines and parentheses together inside one word, a backslash escapes
/// the next character outside single quotes, and `#` at the start of a word
/// comments out the rest of the line. Separators split only outside quotes.
fn simple_commands(cmd: &str) -> Vec<Vec<Word>> {
    let mut out: Vec<Vec<Word>> = Vec::new();
    let mut cur: Vec<Word> = Vec::new();
    let mut text = String::new();
    let mut quoted = false;
    let mut in_word = false;
    let mut chars = cmd.chars().peekable();
    let mut case = CaseState::default();
    let end_word =
        |cur: &mut Vec<Word>, text: &mut String, quoted: &mut bool, in_word: &mut bool| {
            if *in_word {
                cur.push(Word {
                    text: std::mem::take(text),
                    quoted: *quoted,
                });
            }
            *quoted = false;
            *in_word = false;
        };
    while let Some(c) = chars.next() {
        match c {
            // `$(( … ))`: arithmetic is part of its word, never a command.
            '$' if chars.clone().take(2).eq(['(', '(']) => {
                chars.next();
                chars.next();
                in_word = true;
                quoted = true;
                let mut depth = 2;
                for q in chars.by_ref() {
                    match q {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => text.push(q),
                    }
                }
            }
            // ANSI-C `$'…'`: backslash escapes hold, so `\'` does not end it.
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                quoted = true;
                while let Some(q) = chars.next() {
                    match q {
                        '\'' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                text.push(n);
                            }
                        }
                        _ => text.push(q),
                    }
                }
            }
            '\'' => {
                in_word = true;
                quoted = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    text.push(q);
                }
            }
            // `NAME=( … )`: an array's values belong to the assignment.
            '(' if in_word && is_assignment_start(&text) => {
                quoted = true;
                let mut depth = 1;
                let mut quote: Option<char> = None;
                while let Some(q) = chars.next() {
                    // A backslash keeps the next character, except inside '…'.
                    if q == '\\' && quote != Some('\'') {
                        text.push(q);
                        if let Some(n) = chars.next() {
                            text.push(n);
                        }
                        continue;
                    }
                    match (quote, q) {
                        (Some(open), q) if q == open => quote = None,
                        (Some(_), _) => {}
                        (None, '\'' | '"') => quote = Some(q),
                        (None, '(') => depth += 1,
                        (None, ')') => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    text.push(q);
                }
            }
            // `2>&1`, `>&f`, `&>f`: the ampersand belongs to the redirection.
            '&' if in_word && text.ends_with('>') => text.push('&'),
            '&' if !in_word && chars.peek() == Some(&'>') => {
                in_word = true;
                text.push('&');
            }
            '"' => {
                in_word = true;
                quoted = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                text.push(n);
                            }
                        }
                        _ => text.push(q),
                    }
                }
            }
            // A backslash-newline is a line continuation: no word of its own.
            '\\' => match chars.next() {
                Some('\n') | None => {}
                Some(n) => {
                    in_word = true;
                    text.push(n);
                }
            },
            '#' if !in_word => {
                for q in chars.by_ref() {
                    if q == '\n' {
                        break;
                    }
                }
                end_word(&mut cur, &mut text, &mut quoted, &mut in_word);
                case.flush(&mut out, &mut cur, '\n');
            }
            // `name() { … }`: a function definition names no run; its body
            // is parsed as commands of its own.
            '(' if {
                let mut ahead = chars.clone();
                while ahead.peek().is_some_and(|c| *c == ' ' || *c == '\t') {
                    ahead.next();
                }
                ahead.peek() == Some(&')')
            } && {
                end_word(&mut cur, &mut text, &mut quoted, &mut in_word);
                cur.len() == 1 && !cur[0].quoted
            } =>
            {
                while chars.next_if(|c| *c == ' ' || *c == '\t').is_some() {}
                chars.next();
                cur.clear();
            }
            ';' | '|' | '&' | '\n' | '(' | ')' => {
                end_word(&mut cur, &mut text, &mut quoted, &mut in_word);
                case.flush(&mut out, &mut cur, c);
                // `;;` ends a case clause: a pattern comes next.
                if c == ';' && chars.peek() == Some(&';') {
                    chars.next();
                    case.clause_ended();
                }
            }
            c if c.is_whitespace() => end_word(&mut cur, &mut text, &mut quoted, &mut in_word),
            c => {
                in_word = true;
                text.push(c);
            }
        }
    }
    end_word(&mut cur, &mut text, &mut quoted, &mut in_word);
    case.flush(&mut out, &mut cur, '\n');
    out
}

/// Where a `case … esac` is in its clauses, so its patterns (`a)`,
/// `a|b)`, `(x)`, `*)`) are never read as commands.
#[derive(Default)]
struct CaseState {
    depth: usize,
    expect_pattern: bool,
}

impl CaseState {
    /// End the simple command in `cur`, which `term` closed.
    fn flush(&mut self, out: &mut Vec<Vec<Word>>, cur: &mut Vec<Word>, term: char) {
        if cur.is_empty() {
            return;
        }
        let words = std::mem::take(cur);
        let first = words.first().filter(|w| !w.quoted).map(|w| w.text.as_str());
        if first == Some("case") {
            // The header, and the first pattern when it shares the line.
            self.depth += 1;
            self.expect_pattern = term != ')';
            return;
        }
        if self.depth > 0 {
            if first == Some("esac") {
                self.depth -= 1;
                self.expect_pattern = false;
                return;
            }
            if self.expect_pattern {
                if term == ')' {
                    self.expect_pattern = false;
                }
                return;
            }
        }
        out.push(words);
    }

    fn clause_ended(&mut self) {
        if self.depth > 0 {
            self.expect_pattern = true;
        }
    }
}

/// Is `text` exactly `NAME=`, where an array value may start?
fn is_assignment_start(text: &str) -> bool {
    text.strip_suffix('=').is_some_and(|name| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Shell reserved words that open or close a compound command: skipped
/// before the command they introduce.
const RESERVED: [&str; 14] = [
    "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "!", "{", "}", "esac", "in",
];
/// Reserved words whose own words are names and patterns, not a command:
/// the whole simple command they head is dropped.
const HEADERS: [&str; 4] = ["for", "select", "case", "function"];

/// Does `flag` of wrapper `prog` take the next word as its value?
fn takes_value(prog: &str, flag: &str) -> bool {
    match prog {
        "sudo" => matches!(
            flag,
            "-g" | "-h" | "-p" | "-C" | "-D" | "-r" | "-t" | "-U" | "-u" | "-T"
        ),
        "env" => matches!(flag, "-C" | "-S" | "-P" | "-u"),
        "nice" => matches!(flag, "-n"),
        _ => false,
    }
}

/// `NAME=value` with an unquoted name; its whole value, quoted or not, is
/// part of the same word, so it goes with it.
fn is_assignment(word: &Word) -> bool {
    match word.text.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit())
        }
        None => false,
    }
}

/// The words of one simple command, with assignments, redirections and
/// wrappers removed.
fn command_words(mut words: Vec<Word>) -> Vec<Word> {
    // `2>&1`, `> f`, `< f`: a redirection and its target are not arguments.
    let mut kept: Vec<Word> = Vec::with_capacity(words.len());
    let mut skip_next = false;
    for w in words.drain(..) {
        if skip_next {
            skip_next = false;
            continue;
        }
        let op = w.text.trim_start_matches(char::is_numeric);
        if !w.quoted && (op.starts_with(['>', '<']) || op.starts_with("&>")) {
            // An operator alone (`>`, `2>`, `&>`, `>&`) takes the next word.
            skip_next = op.trim_start_matches(['>', '<', '&']).is_empty();
            continue;
        }
        kept.push(w);
    }
    let mut words = kept;
    // `for f in a b`, `case $x in`: loop variables and patterns, not a command.
    if words
        .first()
        .is_some_and(|w| !w.quoted && HEADERS.contains(&w.text.as_str()))
    {
        return Vec::new();
    }
    while let Some(first) = words.first() {
        if !first.quoted && RESERVED.contains(&first.text.as_str()) {
            words.remove(0);
            continue;
        }
        if is_assignment(first) {
            words.remove(0);
            continue;
        }
        if first.quoted {
            break;
        }
        let prog = first
            .text
            .rsplit('/')
            .next()
            .unwrap_or(&first.text)
            .to_string();
        if WRAPPERS.contains(&prog.as_str()) {
            words.remove(0);
            // `env -i`, `nice -n 5`, `sudo -u x`: their flags and the
            // values those flags take go too, so a value is never the program.
            while words.first().is_some_and(|w| w.text.starts_with('-')) {
                let flag = words.remove(0);
                if !flag.quoted && takes_value(&prog, &flag.text) && !words.is_empty() {
                    words.remove(0);
                }
            }
            continue;
        }
        if prog == "timeout" {
            words.remove(0);
            while words.first().is_some_and(|w| w.text.starts_with('-')) {
                words.remove(0);
            }
            if !words.is_empty() {
                words.remove(0);
            }
            continue;
        }
        break;
    }
    words
}

/// Is `words` a test runner? `cargo test`, `cargo nextest`, `bun test`,
/// `npm|pnpm|yarn [run] test`, `pytest`, `python -m pytest`, `go test`,
/// `vitest`, `jest`, `playwright test`, also behind `npx`/`bunx`/`uv run`.
/// Only unquoted words count: a commit message that names the tests is text.
fn runs_tests(words: &[Word]) -> bool {
    let lower: Vec<String> = words
        .iter()
        .map(|w| {
            if w.quoted {
                String::new()
            } else {
                w.text.to_ascii_lowercase()
            }
        })
        .collect();
    let prog = |i: usize| {
        lower
            .get(i)
            .map(|w| w.rsplit('/').next().unwrap_or(w).to_string())
            .unwrap_or_default()
    };
    let at = |i: usize| lower.get(i).map(String::as_str).unwrap_or("");
    let mut i = 0;
    // Launchers: `npx vitest`, `bunx playwright test`, `pnpm dlx jest`,
    // `uv run pytest`, `python -m pytest`.
    loop {
        match (prog(i).as_str(), at(i + 1)) {
            ("npx" | "bunx" | "pnpx", _) => i += 1,
            ("pnpm" | "yarn" | "uv" | "poetry", "dlx" | "exec" | "run")
                if !matches!(at(i + 2), "test" | "") =>
            {
                i += 2
            }
            ("python" | "python3", "-m") => i += 2,
            _ => break,
        }
        if i >= lower.len() {
            return false;
        }
    }
    let p = prog(i);
    let a = at(i + 1);
    let b = at(i + 2);
    match p.as_str() {
        "cargo" => matches!(a, "test" | "nextest") || (a.starts_with('+') && b == "test"),
        "bun" => a == "test" || (a == "run" && b == "test"),
        "npm" | "pnpm" | "yarn" => matches!(a, "test" | "t") || (a == "run" && b == "test"),
        "go" => a == "test",
        "pytest" | "py.test" | "vitest" | "jest" => true,
        "playwright" => a == "test",
        _ => false,
    }
}

/// Simple commands, with `bash -c '…'` and `sh -lc "…"` opened: the quoted
/// script is a command line of its own, parsed the same way. Depth-capped.
fn commands_of(cmd: &str, depth: u8) -> Vec<Vec<Word>> {
    let mut out = Vec::new();
    for words in simple_commands(cmd) {
        let words = command_words(words);
        let Some(first) = words.first() else { continue };
        let prog = first
            .text
            .rsplit('/')
            .next()
            .unwrap_or(&first.text)
            .to_string();
        let script = (!first.quoted
            && matches!(prog.as_str(), "bash" | "sh" | "zsh")
            && words
                .get(1)
                .is_some_and(|f| !f.quoted && f.text.starts_with('-') && f.text.ends_with('c')))
        .then(|| words.get(2))
        .flatten();
        match script {
            Some(s) if depth < 2 => out.extend(commands_of(&s.text, depth + 1)),
            _ => out.push(words),
        }
    }
    out
}

/// The label of a Bash command: the program and up to two plain
/// subcommand words of its main simple command, and whether that command
/// runs tests. `cd x && FOO=1 cargo test -p y 2>&1 | tail` is `cargo test`.
/// Quoting is a shell's: nothing inside quotes is ever a program, a
/// subcommand, or a command of its own.
pub fn command_label(cmd: &str) -> CommandLabel {
    let body = crate::team::without_heredocs(cmd);
    let commands = commands_of(&body, 0);
    let test = commands.iter().position(|w| runs_tests(w));
    let named = |w: &Vec<Word>| label_of(w).is_some();
    let plumbing =
        |w: &Vec<Word>| PLUMBING.contains(&w[0].text.rsplit('/').next().unwrap_or(&w[0].text));
    // A test run first; else the first named command that is not plumbing;
    // else the first named one; else nothing ("Running a command").
    let chosen = test
        .or_else(|| commands.iter().position(|w| named(w) && !plumbing(w)))
        .or_else(|| commands.iter().position(named));
    let Some(i) = chosen else {
        return CommandLabel {
            text: None,
            test: false,
        };
    };
    CommandLabel {
        text: label_of(&commands[i]),
        test: test.is_some(),
    }
}

fn label_of(words: &[Word]) -> Option<String> {
    let first = words.first().filter(|w| !w.quoted)?;
    let program = plain_program(&first.text)?;
    // Only a short list of programs has subcommands worth naming; for any
    // other the first argument is data (a host, a file, a pattern).
    let Some(vocab) = vocabulary(&program) else {
        return Some(program);
    };
    let mut out = vec![program.clone()];
    let mut rest = words[1..].iter().peekable();
    // `git -C <dir> status`, `cargo +nightly test`: skip what only places it.
    loop {
        match rest.peek() {
            Some(w) if program == "git" && !w.quoted && matches!(w.text.as_str(), "-C" | "-c") => {
                rest.next();
                rest.next();
            }
            Some(w) if program == "cargo" && !w.quoted && w.text.starts_with('+') => {
                rest.next();
            }
            _ => break,
        }
    }
    let shown = |w: &&Word, list: &[&str]| {
        !w.quoted
            && (subcommand_word(&w.text) || w.text == "i" || w.text == "x")
            && list.contains(&w.text.as_str())
    };
    if let Some(first) = rest.next().filter(|w| shown(w, vocab)) {
        out.push(first.text.clone());
        if RUNNERS.contains(&first.text.as_str())
            && let Some(second) = rest.next().filter(|w| shown(w, SCRIPTS))
        {
            out.push(second.text.clone());
        }
    }
    Some(out.join(" "))
}

/// Does a Bash command run `git commit`? Read the way the label is: only
/// an unquoted `git` (behind `-C dir` or `-c k=v`) with `commit` as its
/// subcommand counts; a commit message or an `echo "git commit"` does not.
pub fn runs_git_commit(cmd: &str) -> bool {
    if !cmd.contains("commit") {
        return false;
    }
    let body = crate::team::without_heredocs(cmd);
    commands_of(&body, 0)
        .iter()
        .any(|w| label_of(w).as_deref() == Some("git commit"))
}

// ---------------------------------------------------------------------------
// The wait phrases
// ---------------------------------------------------------------------------

/// Every literal the idle guard's `WAIT` regex accepts, expanded: each must
/// start a word, and may continue (`finish` also reads `finishes`). Built
/// once per process.
fn wait_phrases() -> &'static [String] {
    static PHRASES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    PHRASES.get_or_init(build_wait_phrases)
}

fn build_wait_phrases() -> Vec<String> {
    let mut p: Vec<String> = [
        "waiting for",
        "waiting on",
        "wait for",
        "i'll wait",
        "ill wait",
        "in the background",
        "background job",
        "background task",
        "background run",
        "background shell",
        "notification",
        "notified",
        "esperando",
        "espero a",
        "espero que",
        "cuando termine",
        "cuando acabe",
        "en segundo plano",
        "me avisar",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    for verb in ["report", "check", "continue"] {
        for back in ["", "back "] {
            for when in ["when", "once", "after"] {
                p.push(format!("will {verb} {back}{when}"));
            }
        }
    }
    let things = ["job", "build", "run", "test", "task"];
    for end in ["finish", "complete"] {
        p.push(format!("once it {end}"));
        p.push(format!("when it {end}"));
        for t in things {
            p.push(format!("once the {t} {end}"));
            p.push(format!("once the {t}s {end}"));
            p.push(format!("when the {t} {end}"));
        }
        p.push(format!("when the tests {end}"));
    }
    p
}

/// The wait phrases by first byte, so a reply is read once, word by word.
fn phrases_by_first() -> &'static [Vec<&'static str>; 256] {
    static BY: std::sync::OnceLock<[Vec<&'static str>; 256]> = std::sync::OnceLock::new();
    BY.get_or_init(|| {
        let mut by: [Vec<&'static str>; 256] = std::array::from_fn(|_| Vec::new());
        for p in wait_phrases() {
            by[p.as_bytes()[0] as usize].push(p.as_str());
        }
        by
    })
}

/// Does the tail of a reply say it waits for a background job or a
/// wake-up? The same phrases as the idle guard, English and Spanish, each
/// starting a word (the guard's `\b`).
pub fn says_wait(text: &str) -> bool {
    // Only the last WAIT_TAIL_CHARS characters, found from the end.
    let start = text
        .char_indices()
        .rev()
        .nth(WAIT_TAIL_CHARS - 1)
        .map_or(0, |(i, _)| i);
    let tail = text[start..].to_lowercase().replace('\u{2019}', "'");
    let by = phrases_by_first();
    let mut prev_word = false;
    for (at, c) in tail.char_indices() {
        let word = c.is_alphanumeric() || c == '_';
        if !prev_word && c.is_ascii() {
            let rest = &tail[at..];
            if by[c as usize].iter().any(|p| rest.starts_with(p)) {
                return true;
            }
        }
        prev_word = word;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_command_label_is_its_program_and_plain_subcommands_only() {
        let l = |c: &str| command_label(c).text;
        assert_eq!(l("git status").as_deref(), Some("git status"));
        assert_eq!(l("bun run build").as_deref(), Some("bun run build"));
        assert_eq!(
            l("cd /x && CARGO_TARGET_DIR=/t cargo test -p harbard-core 2>&1 | tail -5").as_deref(),
            Some("cargo test")
        );
        assert_eq!(
            l("git -C /repo log --oneline -3").as_deref(),
            Some("git log")
        );
        assert_eq!(l("ls -la /tmp").as_deref(), Some("ls"));
        assert_eq!(
            l("python3 /Users/x/script.py --token abc").as_deref(),
            Some("python3")
        );
        assert_eq!(
            l("/opt/homebrew/bin/rsvg-convert -w 72 a.svg").as_deref(),
            Some("rsvg-convert")
        );
        assert_eq!(
            l("bash -c 'cd x && bun test src'").as_deref(),
            Some("bun test")
        );
        assert_eq!(
            l("timeout 30 cargo build --release").as_deref(),
            Some("cargo build")
        );
        assert_eq!(l("").as_deref(), None);
    }

    #[test]
    fn test_runners_are_told_apart_from_other_commands() {
        let t = |c: &str| command_label(c).test;
        for cmd in [
            "cargo test --workspace",
            "cargo nextest run",
            "cargo +nightly test",
            "bun test src",
            "bun run test",
            "npm test",
            "npm run test",
            "pnpm test",
            "yarn test",
            "pytest -q",
            "python3 -m pytest tests",
            "uv run pytest",
            "go test ./...",
            "npx vitest run",
            "bunx jest",
            "bunx playwright test e2e/x.spec.ts",
            "cd crates && set -o pipefail; cargo test 2>&1 | tee /tmp/t.log",
        ] {
            assert!(t(cmd), "{cmd} runs tests");
        }
        for cmd in [
            "cargo build",
            "bun run build",
            "git status",
            "npm install",
            "go build",
            "echo test",
        ] {
            assert!(!t(cmd), "{cmd} does not run tests");
        }
    }

    #[test]
    fn secrets_never_reach_a_label() {
        let (gh, bearer, aws) = (
            format!("ghp_FAKE{}", "x".repeat(26)),
            format!("sk-fake{}", "x".repeat(17)),
            format!("AKIAFAKE{}", "x".repeat(10)),
        );
        let cmd = format!(
            "GH_TOKEN={gh} mysql --password=hunter2 -u root \
                   && curl -H 'Authorization: Bearer {bearer}' 'https://api.example.com/v1?key={aws}'"
        );
        let label = command_label(&cmd).text.unwrap_or_default();
        for secret in [
            "ghp_",
            "hunter2",
            "Bearer",
            "sk-fake",
            "key=",
            "AKIA",
            "GH_TOKEN",
            "api.example.com",
        ] {
            assert!(!label.contains(secret), "{label} leaks {secret}");
        }
        assert_eq!(label, "mysql");
        // A token where a subcommand would be is not shown, and an unknown
        // program is not named at all.
        assert_eq!(
            command_label(&format!("deploy sk_live_{}", "x".repeat(18))).text,
            None
        );
        assert_eq!(
            command_label(&format!("git sk_live_{}", "x".repeat(18)))
                .text
                .as_deref(),
            Some("git")
        );
    }

    /// `known_program` binary-searches the allowlist.
    #[test]
    fn the_known_programs_are_sorted() {
        let mut sorted = KNOWN.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, KNOWN);
        for (p, vocab) in SUBCOMMANDS {
            assert!(known_program(p), "{p} has subcommands but is not known");
            assert!(
                vocab
                    .iter()
                    .all(|w| subcommand_word(w) || *w == "i" || *w == "x"),
                "{p}"
            );
        }
    }

    #[test]
    fn file_grep_and_web_labels_are_short_and_safe() {
        let c = |name: &str, input: Value| classify(name, &input, 1);
        assert_eq!(
            c("Read", json!({"file_path":"/a/b/team.rs"}))
                .detail
                .as_deref(),
            Some("team.rs")
        );
        assert_eq!(
            c("Edit", json!({"file_path":"/a/b/workspace.css"})).kind,
            ToolKind::Edit
        );
        assert_eq!(
            c("Read", json!({"file_path":"/Users/p/.harbard/hub.token"})).detail,
            None
        );
        assert_eq!(c("Read", json!({"file_path":"/x/.env.local"})).detail, None);
        let long = "a".repeat(80);
        let g = c("Grep", json!({"pattern": long})).detail.unwrap();
        assert_eq!(g.chars().count(), PATTERN_CHARS + 1, "{g}");
        assert!(g.ends_with('…'));
        assert_eq!(
            c("Grep", json!({"pattern":"fn state_of"}))
                .detail
                .as_deref(),
            Some("fn state_of")
        );
        assert_eq!(
            c(
                "Grep",
                json!({"pattern":concat!("ghp_abcd", "efghij0123456789")})
            )
            .detail,
            None
        );
        assert_eq!(
            c(
                "WebFetch",
                json!({"url":"https://user:pw@docs.rs:443/serde/latest?q=1#x"})
            )
            .detail
            .as_deref(),
            Some("docs.rs")
        );
        assert_eq!(c("WebSearch", json!({"query":"how to"})).detail, None);
        assert_eq!(c("Agent", json!({"prompt":"x"})).kind, ToolKind::Delegate);
        assert_eq!(
            c("Bash", json!({"command":"cargo test"})).kind,
            ToolKind::Test
        );
        assert_eq!(
            c("Bash", json!({"command":"cargo build"})).kind,
            ToolKind::Bash
        );
        assert_eq!(c("TodoWrite", json!({})).kind, ToolKind::Other);
    }

    #[test]
    fn the_wait_phrases_match_the_idle_guard_in_english_and_spanish() {
        for yes in [
            "I'm waiting for the build to finish.",
            "I'll wait for the notification.",
            "The tests run in the background; I will report back when they finish.",
            "Will check once the job completes.",
            "Once the tests finish I will send the report.",
            "Lo dejo en segundo plano y espero a que termine.",
            "Quedo esperando la notificación.",
            "Me avisarán cuando acabe.",
            "I\u{2019}ll wait.",
        ] {
            assert!(says_wait(yes), "{yes}");
        }
        for no in [
            "DONE · all tests pass.",
            "The waitress arrived.",
            "Report written to /tmp/x.md.",
            "Terminado.",
        ] {
            assert!(!says_wait(no), "{no}");
        }
        // Only the tail counts, as in the guard.
        let old = format!("waiting for x. {}", "z".repeat(WAIT_TAIL_CHARS));
        assert!(!says_wait(&old));
    }
}
