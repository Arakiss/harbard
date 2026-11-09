//! Checkout-lifecycle hazards: shell shapes that break the one-writer rule in
//! ways a writer lease cannot see.
//!
//! [`WriteClass`](crate::write_class::WriteClass) answers one question — does
//! this call need the writer lease? That question has nothing to say about a
//! command that is perfectly entitled to the lease it holds and still breaks
//! the checkout: `git switch` moves the branch under every other process in
//! that tree, `git worktree remove` deletes a lane out from under its
//! supervisor, a Git alias can hide either one behind a name the gate never
//! reads, and `cd ../other-lane && …` writes to a checkout this session does
//! not own. All four are *legal writes*. The lease grants them.
//!
//! So this is a second, orthogonal classification: not "may you write here"
//! but "is this shape allowed at all". A command can be `Read` and still be a
//! hazard (`git worktree list` is not, but `cd ../lane > file` reads nothing
//! and still crosses), and it can be `Write` with no hazard at all (an
//! ordinary commit).
//!
//! This lived for months as a Python script the PreToolUse hook shelled out
//! to. It was correct, and it was invisible: no types, no tests, no way for
//! the rest of Harbard to know a denial had happened or why. Bringing it here
//! makes the categories part of the model — the gate records them, `friction`
//! ranks them, and the hook shrinks to what only it can decide (the operator's
//! one-shot override lives in the operator's own directory, not in a library).

use std::path::{Component, Path, PathBuf};

/// A shell shape that violates the checkout lifecycle regardless of who holds
/// the writer lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hazard {
    /// Moves HEAD in a live checkout: `git switch`, `git checkout <ref>`.
    BranchSwitch,
    /// Raw worktree creation or mutation, outside the governed lifecycle.
    Worktree,
    /// A Git alias, which can expand to either of the above under a name the
    /// classifier never sees.
    GitAlias,
    /// A literal path reaching into another registered worktree.
    CrossCheckout,
}

impl Hazard {
    /// The stable wire name. The hook matches on these, so they are API.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BranchSwitch => "branch-switch",
            Self::Worktree => "worktree",
            Self::GitAlias => "git-alias",
            Self::CrossCheckout => "cross-checkout",
        }
    }

    /// Why it was denied and what to do instead. A denial that does not name
    /// the way forward just gets retried in a slightly different shape.
    pub fn reason(self) -> &'static str {
        match self {
            Self::BranchSwitch => {
                "Branch switching inside an active checkout is forbidden: it moves the branch \
                 under every other process in that tree. Create or enter a separately governed \
                 worktree."
            }
            Self::Worktree => {
                "Raw worktree creation or mutation is forbidden. Use worktree-lease.sh create, \
                 release, and remove, which audit, claim, and record the lane as one operation."
            }
            Self::GitAlias => {
                "Git aliases are forbidden in governed agent commands because they can hide a \
                 branch switch or a worktree mutation behind a name. Use the canonical Git verb."
            }
            Self::CrossCheckout => {
                "A literal shell path targets another registered worktree. Start or hand off to \
                 a session rooted in that checkout."
            }
        }
    }
}

/// The command could not be read well enough to clear it.
///
/// Deliberately not a `Hazard`: an unterminated quote is not a branch switch,
/// it is an unreadable command, and the caller must fail closed rather than
/// guess which of the four it might have been.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unreadable;

fn on_disk(path: &Path) -> bool {
    path.exists()
}

static ON_DISK: fn(&Path) -> bool = on_disk;

/// What the classifier needs to know about the session it is protecting.
pub struct Policy<'a> {
    /// The session's own checkout root. Paths inside it are never crossings.
    pub checkout: Option<&'a Path>,
    /// Where the command starts, which is not always the checkout root.
    pub cwd: &'a Path,
    /// Every worktree registered for this repository, from `git worktree list`.
    pub worktrees: &'a [PathBuf],
    /// Alias names from `git --list-cmds=alias`.
    pub aliases: &'a [String],
    /// How to ask whether a path exists. Injected so the `git checkout <file>`
    /// disambiguation is testable without a fixture repository on disk.
    pub probe: &'a dyn Fn(&Path) -> bool,
}

impl<'a> Policy<'a> {
    pub fn new(
        checkout: Option<&'a Path>,
        cwd: &'a Path,
        worktrees: &'a [PathBuf],
        aliases: &'a [String],
    ) -> Self {
        Self {
            checkout,
            cwd,
            worktrees,
            aliases,
            probe: &ON_DISK,
        }
    }

    pub fn with_probe(mut self, probe: &'a dyn Fn(&Path) -> bool) -> Self {
        self.probe = probe;
        self
    }
}

/// Classify a shell command against the session's checkout lifecycle.
///
/// `Ok(None)` means no hazard — not that the command is safe to run, only that
/// it is not one of these four. The writer lease still has its own say.
pub fn classify(command: &str, policy: &Policy<'_>) -> Result<Option<Hazard>, Unreadable> {
    let lexed = tokenize(command)?;

    // A heredoc body is data, except when the thing reading it is a shell —
    // then it is a script, and `bash <<EOF … git switch main … EOF` would walk
    // straight through a classifier that skipped it.
    for body in &lexed.scripts {
        if let Some(hazard) = classify(body, policy)? {
            return Ok(Some(hazard));
        }
    }

    let tokens = substitute_literal_assignments(lexed.tokens);

    // Paths first: a crossing is decided by where the command points, not by
    // which binary it runs, so `cp x ../other-lane/y` must not need `cp` to be
    // in a list of known mutators.
    if let Some(hazard) = literal_path_policy(&tokens, policy) {
        return Ok(Some(hazard));
    }

    // `GIT_CONFIG_KEY_0=alias.x GIT_CONFIG_VALUE_0=…` defines an alias for the
    // duration of one command without touching any config file.
    for token in &tokens {
        let upper = token.to_ascii_uppercase();
        if upper.starts_with("GIT_CONFIG_COUNT=")
            || upper.starts_with("GIT_CONFIG_KEY_")
            || upper.starts_with("GIT_CONFIG_VALUE_")
        {
            return Ok(Some(Hazard::GitAlias));
        }
    }

    for index in 0..tokens.len() {
        if !is_git_token(&tokens[index]) {
            continue;
        }
        let (sub, sub_index) = match next_non_option(&tokens, index + 1) {
            Subcommand::Alias => return Ok(Some(Hazard::GitAlias)),
            Subcommand::None => continue,
            Subcommand::Word(word, at) => (word, at),
        };
        if policy.aliases.iter().any(|alias| alias == sub) {
            return Ok(Some(Hazard::GitAlias));
        }
        match sub {
            // `git switch` exists only to move HEAD between branches.
            "switch" => return Ok(Some(Hazard::BranchSwitch)),
            // `git checkout` is two commands wearing one name: it moves HEAD,
            // and it restores files. Treating both as a switch denied
            // `git checkout bun.lock` — discarding an accidental lockfile
            // change — with a message about branch switching, which was
            // neither true nor actionable.
            "checkout" => {
                if checkout_moves_head(&tokens[sub_index + 1..], policy) {
                    return Ok(Some(Hazard::BranchSwitch));
                }
            }
            "config" => {
                if config_touches_alias(&tokens, sub_index) {
                    return Ok(Some(Hazard::GitAlias));
                }
            }
            "worktree" => {
                if let Subcommand::Word(verb, _) = next_non_option(&tokens, sub_index + 1)
                    && matches!(
                        verb,
                        "add" | "lock" | "move" | "prune" | "remove" | "repair" | "unlock"
                    )
                {
                    return Ok(Some(Hazard::Worktree));
                }
            }
            _ => {}
        }
    }

    Ok(None)
}

// ---------------------------------------------------------------------------
// Tokenizing
// ---------------------------------------------------------------------------

const PUNCTUATION: &[u8] = b";&|()<>";

/// What the lexer found: the command's own tokens, plus any heredoc body that
/// is going to be executed rather than consumed as data.
struct Lexed {
    tokens: Vec<String>,
    scripts: Vec<String>,
}

/// Split a command into words and operator runs, quote-aware.
///
/// Operators stay in the stream because the path policy needs them: without
/// `&&` there is no way to know that `cd ../lane` ended and something else
/// began, and without `>` there is no redirection target to check.
///
/// Heredoc bodies are lifted out rather than lexed. They are not shell text —
/// a sentence with an apostrophe, or a Python string with a quote, is
/// perfectly ordinary inside one — and reading them as shell made this
/// classifier report "unreadable", which the hook turns into a hard denial.
fn tokenize(command: &str) -> Result<Lexed, Unreadable> {
    let mut tokens = Vec::new();
    let mut scripts = Vec::new();
    let mut word = String::new();
    let mut has_word = false;
    // Delimiters awaiting a body, and whether that body will be executed.
    let mut pending: Vec<(String, bool)> = Vec::new();
    let mut expect_delimiter = false;
    let mut segment_head: Option<String> = None;
    let bytes = command.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];

        if byte == b'\\' {
            // A backslash-newline is a line continuation and disappears; any
            // other escape contributes its literal character.
            if i + 1 < bytes.len() {
                if bytes[i + 1] != b'\n' {
                    word.push(bytes[i + 1] as char);
                    has_word = true;
                }
                i += 2;
                continue;
            }
            return Err(Unreadable);
        }

        if byte == b'\'' || byte == b'"' {
            let quote = byte;
            i += 1;
            has_word = true;
            loop {
                if i >= bytes.len() {
                    // An unterminated quote is not a command we can reason
                    // about. Fail closed rather than classify half of it.
                    return Err(Unreadable);
                }
                if bytes[i] == b'\\' && quote == b'"' && i + 1 < bytes.len() {
                    word.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                word.push(bytes[i] as char);
                i += 1;
            }
            continue;
        }

        if byte.is_ascii_whitespace() {
            flush(
                &mut tokens,
                &mut word,
                &mut has_word,
                &mut expect_delimiter,
                &mut pending,
                &mut segment_head,
            );
            if byte == b'\n' {
                // An unquoted newline separates commands exactly like `;` —
                // and, if a heredoc was opened on this line, its body starts
                // right here and must be skipped rather than lexed.
                tokens.push(";".to_string());
                segment_head = None;
                i += 1;
                if !pending.is_empty() {
                    i = consume_heredocs(command, i, &mut pending, &mut scripts);
                }
                continue;
            }
            i += 1;
            continue;
        }

        if PUNCTUATION.contains(&byte) {
            flush(
                &mut tokens,
                &mut word,
                &mut has_word,
                &mut expect_delimiter,
                &mut pending,
                &mut segment_head,
            );
            let start = i;
            while i < bytes.len() && PUNCTUATION.contains(&bytes[i]) {
                i += 1;
            }
            let operator = &command[start..i];
            // `<<` opens a heredoc; `<<<` is a here-string whose operand is an
            // ordinary word with no body to skip.
            if operator == "<<" {
                expect_delimiter = true;
            }
            if is_boundary(operator) {
                segment_head = None;
            }
            tokens.push(operator.to_string());
            continue;
        }

        word.push(byte as char);
        has_word = true;
        i += 1;
    }

    flush(
        &mut tokens,
        &mut word,
        &mut has_word,
        &mut expect_delimiter,
        &mut pending,
        &mut segment_head,
    );
    Ok(Lexed { tokens, scripts })
}

/// Close the current word, and let it play whichever role the lexer state says
/// it has: a heredoc delimiter, the name of the command being run, or a token.
fn flush(
    tokens: &mut Vec<String>,
    word: &mut String,
    has_word: &mut bool,
    expect_delimiter: &mut bool,
    pending: &mut Vec<(String, bool)>,
    segment_head: &mut Option<String>,
) {
    if !*has_word {
        return;
    }
    let word = std::mem::take(word);
    *has_word = false;
    if *expect_delimiter {
        *expect_delimiter = false;
        // `<<-DELIM` strips leading tabs from the body; the dash belongs to
        // the operator, not to the delimiter.
        let delimiter = word.strip_prefix('-').unwrap_or(&word).to_string();
        let executed = segment_head.as_deref().is_some_and(reads_a_script);
        pending.push((delimiter, executed));
        tokens.push(word);
        return;
    }
    if segment_head.is_none()
        && split_assignment(&word).is_none()
        && !matches!(
            word.as_str(),
            "builtin" | "command" | "env" | "sudo" | "doas" | "nohup" | "nice" | "time"
        )
    {
        *segment_head = Some(word.clone());
    }
    tokens.push(word);
}

/// Whether a command run with a heredoc will execute that body as shell.
fn reads_a_script(command: &str) -> bool {
    let base = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command);
    matches!(base, "bash" | "sh" | "zsh" | "dash" | "ksh" | "shell")
}

/// Skip past the bodies of every heredoc opened on the line just ended,
/// keeping the ones a shell is going to execute.
fn consume_heredocs(
    command: &str,
    mut index: usize,
    pending: &mut Vec<(String, bool)>,
    scripts: &mut Vec<String>,
) -> usize {
    let bytes = command.as_bytes();
    for (delimiter, executed) in std::mem::take(pending) {
        let mut body = String::new();
        loop {
            if index >= bytes.len() {
                break;
            }
            let end = command[index..]
                .find('\n')
                .map_or(bytes.len(), |offset| index + offset);
            let line = &command[index..end];
            index = (end + 1).min(bytes.len());
            if line.trim() == delimiter {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        if executed {
            scripts.push(body);
        }
    }
    index
}

/// Resolve `$name` tokens backed by a literal assignment earlier in the same
/// command, so `LANE=../other-lane; cd $LANE` is seen for what it is.
///
/// This is deliberately not a shell: no embedded expansion, no substitution,
/// no environment. An assignment whose value is itself dynamic un-defines the
/// name rather than resolving it to something stale.
fn substitute_literal_assignments(tokens: Vec<String>) -> Vec<String> {
    let mut literals: Vec<(String, String)> = Vec::new();
    let mut resolved = Vec::with_capacity(tokens.len());

    for token in tokens {
        if let Some((name, value)) = split_assignment(&token) {
            literals.retain(|(known, _)| known != name);
            if !value.contains('$') && !value.contains('`') && !value.contains('\n') {
                literals.push((name.to_string(), value.to_string()));
            }
            resolved.push(token);
            continue;
        }
        if let Some(name) = variable_name(&token)
            && let Some((_, value)) = literals.iter().find(|(known, _)| known == name)
        {
            resolved.push(value.clone());
            continue;
        }
        resolved.push(token);
    }
    resolved
}

fn split_assignment(token: &str) -> Option<(&str, &str)> {
    let (name, value) = token.split_once('=')?;
    if name.is_empty() {
        return None;
    }
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((name, value))
}

/// `$name` or `${name}` as an entire token — nothing embedded, nothing partial.
fn variable_name(token: &str) -> Option<&str> {
    let rest = token.strip_prefix('$')?;
    let name = match rest.strip_prefix('{') {
        Some(braced) => braced.strip_suffix('}')?,
        None => rest,
    };
    if name.is_empty() {
        return None;
    }
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    chars
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        .then_some(name)
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const BOUNDARIES: &[&str] = &[";", ";;", "&", "&&", "|", "||", "(", ")"];
const WRITE_REDIRECTIONS: &[&str] = &[">", ">>", ">|", "<>", "&>", "&>>"];

fn is_boundary(token: &str) -> bool {
    BOUNDARIES.contains(&token)
}

/// Walk the command tracking where it is standing, and flag any literal path
/// that lands inside a worktree this session does not own.
fn literal_path_policy(tokens: &[String], policy: &Policy<'_>) -> Option<Hazard> {
    if policy.worktrees.is_empty() {
        return None;
    }
    let mut cwd = policy.cwd.to_path_buf();

    for segment in command_segments(tokens) {
        if let Some(start) = command_start(segment) {
            let verb = segment[start].as_str();
            if verb == "cd" || verb == "pushd" {
                let mut index = start + 1;
                while index < segment.len() {
                    let option = segment[index].as_str();
                    if option == "--" {
                        index += 1;
                        break;
                    }
                    let skippable = match verb {
                        "cd" => matches!(option, "-L" | "-P" | "-e" | "-@"),
                        _ => option == "-n",
                    };
                    if !skippable {
                        break;
                    }
                    index += 1;
                }
                if let Some(raw) = segment.get(index)
                    && let Some(target) = canonical_path(raw, &cwd)
                {
                    if targets_other_worktree(&target, policy) {
                        return Some(Hazard::CrossCheckout);
                    }
                    cwd = target;
                }
            }
        }

        for (index, token) in segment.iter().enumerate() {
            if !WRITE_REDIRECTIONS.contains(&token.as_str()) {
                continue;
            }
            if let Some(raw) = segment.get(index + 1)
                && let Some(target) = canonical_path(raw, &cwd)
                && targets_other_worktree(&target, policy)
            {
                return Some(Hazard::CrossCheckout);
            }
        }
    }
    None
}

fn command_segments(tokens: &[String]) -> Vec<&[String]> {
    let mut segments = Vec::new();
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        if is_boundary(token) {
            if index > start {
                segments.push(&tokens[start..index]);
            }
            start = index + 1;
        }
    }
    if start < tokens.len() {
        segments.push(&tokens[start..]);
    }
    segments
}

/// Where the actual command begins, past keywords, assignments and wrappers.
fn command_start(segment: &[String]) -> Option<usize> {
    let mut index = 0;
    while index < segment.len()
        && matches!(
            segment[index].as_str(),
            "!" | "do" | "elif" | "else" | "if" | "then" | "until" | "while"
        )
    {
        index += 1;
    }
    while index < segment.len() && split_assignment(&segment[index]).is_some() {
        index += 1;
    }
    while index < segment.len() && matches!(segment[index].as_str(), "builtin" | "command") {
        index += 1;
    }
    if index < segment.len() && segment[index] == "env" {
        index += 1;
        while index < segment.len()
            && (segment[index].starts_with('-') || split_assignment(&segment[index]).is_some())
        {
            index += 1;
        }
    }
    (index < segment.len()).then_some(index)
}

/// Best-effort absolute path. Real canonicalization when the path exists (so a
/// symlinked temp dir compares equal to what Git reported), lexical otherwise.
fn canonical_path(value: &str, cwd: &Path) -> Option<PathBuf> {
    if value.is_empty() || value.contains('\0') || variable_name(value).is_some() {
        return None;
    }
    let expanded = match value.strip_prefix("~/") {
        Some(rest) => home_dir()?.join(rest),
        None if value == "~" => home_dir()?,
        None => PathBuf::from(value),
    };
    let joined = if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    };
    Some(resolve(&joined))
}

/// Resolve as far as the filesystem allows, then re-attach what is left.
///
/// Plain `canonicalize` fails on a path that does not exist yet — which is
/// exactly the dangerous case, `echo x > ../other-lane/new-file`. Falling back
/// to a lexical answer let that through whenever any ancestor was a symlink,
/// and on macOS `/tmp` and `/var` both are. So: find the deepest ancestor that
/// does exist, canonicalize that, and put the remaining names back.
fn resolve(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    let lexical = lexical_normalize(path);
    let mut trailing = Vec::new();
    let mut cursor = lexical.as_path();
    while let Some(parent) = cursor.parent() {
        if let Some(name) = cursor.file_name() {
            trailing.push(name.to_owned());
        }
        if let Ok(real) = std::fs::canonicalize(parent) {
            let mut out = real;
            out.extend(trailing.iter().rev());
            return out;
        }
        cursor = parent;
    }
    lexical
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn targets_other_worktree(path: &Path, policy: &Policy<'_>) -> bool {
    // Worktrees nest (`.worktrees/x` lives inside the primary checkout), so a
    // path can be under several roots; it is in the deepest one. The session's
    // own checkout counts even when it was filtered out as an open lane.
    policy
        .worktrees
        .iter()
        .map(PathBuf::as_path)
        .chain(policy.checkout)
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.components().count())
        .is_some_and(|owner| Some(owner) != policy.checkout)
}

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

fn is_git_token(token: &str) -> bool {
    Path::new(token)
        .file_name()
        .is_some_and(|name| name == "git")
}

fn is_alias_config(value: &str) -> bool {
    value.to_ascii_lowercase().starts_with("alias.")
}

enum Subcommand<'a> {
    /// An alias was configured inline; nothing after it needs reading.
    Alias,
    Word(&'a str, usize),
    None,
}

/// Git's own options that take a separate value, so the subcommand search does
/// not mistake `main` in `git -C main status` for a subcommand.
const OPTIONS_WITH_VALUE: &[&str] = &[
    "-C",
    "-c",
    "--config-env",
    "--exec-path",
    "--git-dir",
    "--namespace",
    "--super-prefix",
    "--work-tree",
];

fn next_non_option(tokens: &[String], start: usize) -> Subcommand<'_> {
    let mut index = start;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if is_boundary(token) {
            return Subcommand::None;
        }
        if OPTIONS_WITH_VALUE.contains(&token) {
            let Some(value) = tokens.get(index + 1) else {
                return Subcommand::None;
            };
            if token == "-c" && is_alias_config(value) {
                return Subcommand::Alias;
            }
            index += 2;
            continue;
        }
        if let Some(inline) = token.strip_prefix("-c")
            && !inline.is_empty()
        {
            if is_alias_config(inline) {
                return Subcommand::Alias;
            }
            index += 1;
            continue;
        }
        if let Some(inline) = token.strip_prefix("--config-env=") {
            if is_alias_config(inline) {
                return Subcommand::Alias;
            }
            index += 1;
            continue;
        }
        if token.starts_with('-') {
            index += 1;
            continue;
        }
        return Subcommand::Word(token, index);
    }
    Subcommand::None
}

fn config_touches_alias(tokens: &[String], config_index: usize) -> bool {
    match next_non_option(tokens, config_index + 1) {
        Subcommand::Alias => true,
        Subcommand::Word(key, _) => is_alias_config(key),
        Subcommand::None => tokens[config_index + 1..]
            .iter()
            .take_while(|token| !is_boundary(token))
            .any(|token| is_alias_config(token)),
    }
}

/// Options that make `git checkout` a branch operation whatever follows.
const CHECKOUT_BRANCH_FLAGS: &[&str] = &[
    "-b",
    "-B",
    "--orphan",
    "--detach",
    "--track",
    "-t",
    "--no-track",
    "--guess",
    "--no-guess",
];

/// True when this `git checkout` changes what HEAD points at.
///
/// Conservative on purpose: anything unreadable as a file operation counts as
/// a branch switch, because a false allow moves a branch under a live session
/// while a false deny is only an annoying message.
fn checkout_moves_head(rest: &[String], policy: &Policy<'_>) -> bool {
    let rest: Vec<&str> = rest
        .iter()
        .map(String::as_str)
        .take_while(|token| !is_boundary(token))
        .collect();
    if rest.is_empty() {
        // Bare `git checkout` moves nothing, but there is nothing to allow
        // either; keep the strict answer.
        return true;
    }
    // Everything after `--` is a pathspec. `git checkout -- file` and
    // `git checkout <ref> -- file` both restore files; neither moves HEAD.
    if rest.contains(&"--") {
        return false;
    }
    if rest
        .iter()
        .any(|token| CHECKOUT_BRANCH_FLAGS.contains(token))
    {
        return true;
    }
    // An argument naming something on disk is a pathspec. A branch name that
    // is also a path is ambiguous to Git itself, which errors rather than
    // guessing — so deferring to the filesystem matches what Git does.
    !rest
        .iter()
        .filter(|token| !token.starts_with('-'))
        .any(|token| (policy.probe)(&policy.cwd.join(token)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lanes() -> Vec<PathBuf> {
        vec![
            PathBuf::from("/repo"),
            PathBuf::from("/repo-lanes/feature-a"),
        ]
    }

    fn no_files(_: &Path) -> bool {
        false
    }

    fn policy<'a>(worktrees: &'a [PathBuf], aliases: &'a [String]) -> Policy<'a> {
        Policy::new(
            Some(Path::new("/repo")),
            Path::new("/repo"),
            worktrees,
            aliases,
        )
        .with_probe(&no_files)
    }

    fn hazard(command: &str) -> Option<Hazard> {
        let worktrees = lanes();
        let aliases = Vec::new();
        classify(command, &policy(&worktrees, &aliases)).expect("readable")
    }

    #[test]
    fn ordinary_work_is_not_a_hazard() {
        for command in [
            "git status --short",
            "git commit -m 'work'",
            "cargo test --workspace",
            "git worktree list",
            "git log --oneline -5 | head",
            "echo hello > notes.txt",
        ] {
            assert_eq!(hazard(command), None, "{command}");
        }
    }

    #[test]
    fn switch_always_moves_head() {
        assert_eq!(hazard("git switch main"), Some(Hazard::BranchSwitch));
        assert_eq!(hazard("git switch -c topic"), Some(Hazard::BranchSwitch));
    }

    #[test]
    fn checkout_restoring_a_file_is_not_a_branch_switch() {
        let worktrees = lanes();
        let aliases = Vec::new();
        let present = |path: &Path| path == Path::new("/repo/bun.lock");
        let policy = Policy::new(
            Some(Path::new("/repo")),
            Path::new("/repo"),
            &worktrees,
            &aliases,
        )
        .with_probe(&present);

        assert_eq!(classify("git checkout bun.lock", &policy), Ok(None));
        assert_eq!(classify("git checkout -- bun.lock", &policy), Ok(None));
        assert_eq!(classify("git checkout main -- bun.lock", &policy), Ok(None));
        assert_eq!(
            classify("git checkout main", &policy),
            Ok(Some(Hazard::BranchSwitch))
        );
        assert_eq!(
            classify("git checkout -b topic", &policy),
            Ok(Some(Hazard::BranchSwitch))
        );
    }

    #[test]
    fn bare_checkout_stays_strict() {
        assert_eq!(hazard("git checkout"), Some(Hazard::BranchSwitch));
    }

    /// The retiring Python classifier read `git checkout`'s arguments to the
    /// end of the line instead of to the end of the command, so anything after
    /// `&&` was mistaken for this checkout's pathspec. Two of the commonest
    /// tokens an agent writes were enough to walk through the denial: an
    /// absolute path (`/dev/null` "exists", so the checkout looked like a file
    /// restore) and a bare `--` belonging to a later `git diff`.
    #[test]
    fn a_later_command_cannot_disguise_a_branch_switch() {
        for command in [
            "git checkout main && echo hi 2>/dev/null",
            "git checkout main && cat /etc/hosts",
            "git checkout main && git diff -- file.txt",
            "git checkout main; ls -- .",
            "git checkout main | tee /dev/null",
        ] {
            assert_eq!(hazard(command), Some(Hazard::BranchSwitch), "{command}");
        }
    }

    #[test]
    fn worktree_mutations_are_denied_but_listing_is_not() {
        assert_eq!(
            hazard("git worktree add ../lane topic"),
            Some(Hazard::Worktree)
        );
        assert_eq!(
            hazard("git worktree remove ../lane"),
            Some(Hazard::Worktree)
        );
        assert_eq!(hazard("git worktree prune"), Some(Hazard::Worktree));
        assert_eq!(hazard("git worktree list --porcelain"), None);
    }

    #[test]
    fn aliases_are_denied_however_they_are_spelled() {
        assert_eq!(
            hazard("git -c alias.co=checkout co main"),
            Some(Hazard::GitAlias)
        );
        assert_eq!(hazard("git -calias.co=checkout co"), Some(Hazard::GitAlias));
        assert_eq!(
            hazard("git --config-env=alias.co=CO co"),
            Some(Hazard::GitAlias)
        );
        assert_eq!(
            hazard("git config alias.co checkout"),
            Some(Hazard::GitAlias)
        );
        assert_eq!(
            hazard("git config --unset alias.co"),
            Some(Hazard::GitAlias)
        );
        assert_eq!(
            hazard("GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=alias.co git co"),
            Some(Hazard::GitAlias)
        );
    }

    #[test]
    fn a_configured_alias_name_is_denied() {
        let worktrees = lanes();
        let aliases = vec!["sw".to_string(), "co".to_string()];
        assert_eq!(
            classify("git sw main", &policy(&worktrees, &aliases)),
            Ok(Some(Hazard::GitAlias))
        );
        assert_eq!(
            classify("git status", &policy(&worktrees, &aliases)),
            Ok(None)
        );
    }

    #[test]
    fn options_before_the_subcommand_do_not_hide_it() {
        assert_eq!(
            hazard("git -C /repo switch main"),
            Some(Hazard::BranchSwitch)
        );
        assert_eq!(
            hazard("git --no-pager switch main"),
            Some(Hazard::BranchSwitch)
        );
    }

    #[test]
    fn a_path_into_another_lane_is_a_crossing() {
        assert_eq!(
            hazard("cd /repo-lanes/feature-a && git commit -m x"),
            Some(Hazard::CrossCheckout)
        );
        assert_eq!(
            hazard("echo x > /repo-lanes/feature-a/notes.txt"),
            Some(Hazard::CrossCheckout)
        );
        assert_eq!(hazard("cd /repo/crates && cargo test"), None);
    }

    #[test]
    fn a_relative_crossing_resolves_against_the_current_directory() {
        assert_eq!(
            hazard("cd ../repo-lanes/feature-a && ls"),
            Some(Hazard::CrossCheckout)
        );
    }

    #[test]
    fn a_literal_variable_does_not_hide_the_destination() {
        assert_eq!(
            hazard("LANE=/repo-lanes/feature-a; cd $LANE"),
            Some(Hazard::CrossCheckout)
        );
        assert_eq!(
            hazard("LANE=/repo-lanes/feature-a; cd ${LANE}"),
            Some(Hazard::CrossCheckout)
        );
    }

    #[test]
    fn a_dynamic_variable_is_left_alone_rather_than_guessed() {
        // Nothing to resolve, so nothing to flag here; the writer lease and
        // the gate's own target resolution still apply.
        assert_eq!(hazard("LANE=$(cat which); cd $LANE"), None);
    }

    #[test]
    fn quoting_does_not_smuggle_a_switch_past_the_lexer() {
        assert_eq!(hazard("git 'switch' main"), Some(Hazard::BranchSwitch));
        assert_eq!(hazard("git \"switch\" main"), Some(Hazard::BranchSwitch));
    }

    #[test]
    fn a_newline_separates_commands_like_a_semicolon() {
        assert_eq!(
            hazard("cd /repo\ngit worktree add ../lane"),
            Some(Hazard::Worktree)
        );
    }

    /// A heredoc body is data. Reading it as shell made an apostrophe in
    /// prose — or a quote in a Python string — report the whole
    /// command as unclassifiable, which the hook turns into a denial.
    #[test]
    fn a_heredoc_body_is_data_not_shell() {
        for command in [
            "cat > notes.md <<'EOF'\ncannot: the file is missing\nEOF",
            "pbcopy <<'EOF'\nThe commit is ready; I couldn't test it\nEOF",
            "python3 - <<'PY'\nprint(\"it's fine\")\nPY",
            "cat <<-EOF > out\n\tindented body's text\n\tEOF",
        ] {
            let worktrees = lanes();
            let aliases = Vec::new();
            assert_eq!(
                classify(command, &policy(&worktrees, &aliases)),
                Ok(None),
                "{command}"
            );
        }
    }

    /// The exception that makes skipping bodies safe to do: when the reader is
    /// a shell, the body is not data, it is the script.
    #[test]
    fn a_heredoc_fed_to_a_shell_is_still_classified() {
        assert_eq!(
            hazard("bash <<'EOF'\ngit switch main\nEOF"),
            Some(Hazard::BranchSwitch)
        );
        assert_eq!(
            hazard("sh <<EOF\ngit worktree add ../lane\nEOF"),
            Some(Hazard::Worktree)
        );
        assert_eq!(
            hazard("/bin/bash <<'EOF'\ncd /repo-lanes/feature-a\nEOF"),
            Some(Hazard::CrossCheckout)
        );
        // …and the same body handed to something that only stores it is not.
        assert_eq!(hazard("cat <<'EOF' > x.sh\ngit switch main\nEOF"), None);
    }

    #[test]
    fn a_here_string_is_not_a_heredoc() {
        // `<<<` has no body to skip; what follows it is an ordinary word.
        assert_eq!(hazard("grep foo <<< 'bar'"), None);
        assert_eq!(hazard("git switch main <<< x"), Some(Hazard::BranchSwitch));
    }

    #[test]
    fn work_after_a_heredoc_is_still_read() {
        assert_eq!(
            hazard("cat > x <<'EOF'\nbody\nEOF\ngit switch main"),
            Some(Hazard::BranchSwitch)
        );
    }

    #[test]
    fn an_unterminated_quote_is_unreadable_not_safe() {
        let worktrees = lanes();
        let aliases = Vec::new();
        assert_eq!(
            classify("git switch 'main", &policy(&worktrees, &aliases)),
            Err(Unreadable)
        );
    }

    #[test]
    fn a_descriptor_redirection_is_not_a_path() {
        // `2>&1` tokenizes as `2`, `>&`, `1`: not a write redirection, and `1`
        // is not a path to resolve.
        assert_eq!(hazard("git status 2>&1"), None);
    }

    /// Creating a NEW file in another lane is the case that matters most, and
    /// it is the one plain canonicalization cannot see: the path does not
    /// exist yet, so a lexical answer wins, and a lexical answer misses every
    /// symlinked ancestor. On macOS that is `/tmp` and `/var` — where every
    /// throwaway checkout lives.
    #[test]
    fn a_new_file_in_another_lane_is_still_a_crossing() {
        let home = crate::test_util::TempHome::new();
        let lane = home.path().join("lane");
        std::fs::create_dir_all(&lane).expect("lane");
        // What `git worktree list` would report: the resolved path.
        let registered = std::fs::canonicalize(&lane).expect("canonical lane");
        // What the command says: the same lane, reached through whatever
        // symlinks the temp directory happens to sit behind.
        let literal = lane.join("new-file.txt");
        assert!(!literal.exists());

        let worktrees = vec![registered];
        let aliases = Vec::new();
        let checkout = home.path().join("repo");
        let policy =
            Policy::new(Some(&checkout), &checkout, &worktrees, &aliases).with_probe(&no_files);

        assert_eq!(
            classify(&format!("echo x > {}", literal.display()), &policy),
            Ok(Some(Hazard::CrossCheckout))
        );
    }

    #[test]
    fn the_session_own_checkout_is_never_a_crossing() {
        let worktrees = lanes();
        let aliases = Vec::new();
        assert_eq!(
            classify("cd /repo && ls", &policy(&worktrees, &aliases)),
            Ok(None)
        );
    }

    #[test]
    fn every_hazard_names_a_way_forward() {
        for hazard in [
            Hazard::BranchSwitch,
            Hazard::Worktree,
            Hazard::GitAlias,
            Hazard::CrossCheckout,
        ] {
            assert!(!hazard.as_str().is_empty());
            assert!(hazard.reason().len() > 40, "{}", hazard.as_str());
        }
    }
    /// `.worktrees/x` lives inside the primary checkout, so a path in it is
    /// also under the primary: the deepest registered root is the one it is in.
    #[test]
    fn a_nested_worktree_is_its_own_checkout_and_not_the_primarys() {
        let probe = |_: &Path| true;
        let primary = PathBuf::from("/r");
        let mine = PathBuf::from("/r/.worktrees/mine");
        let sibling = PathBuf::from("/r/.worktrees/other");
        let worktrees = vec![primary.clone(), mine.clone(), sibling.clone()];
        let aliases: Vec<String> = Vec::new();
        let from = |checkout: &'static str| {
            let checkout = Path::new(checkout);
            Policy::new(Some(checkout), checkout, &worktrees, &aliases).with_probe(&probe)
        };
        let classify_at =
            |checkout: &'static str, command: &str| classify(command, &from(checkout)).unwrap();

        let own = "cd /r/.worktrees/mine && git status";
        assert_eq!(classify_at("/r/.worktrees/mine", own), None);
        assert_eq!(
            classify_at("/r/.worktrees/mine", "cd /r/.worktrees/other && git status"),
            Some(Hazard::CrossCheckout)
        );
        assert_eq!(
            classify_at("/r/.worktrees/mine", "cd /r/crates && git status"),
            Some(Hazard::CrossCheckout)
        );
        assert_eq!(classify_at("/r", own), Some(Hazard::CrossCheckout));
        assert_eq!(classify_at("/r", "cd /r/crates && git status"), None);
    }
}
