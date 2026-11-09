//! Classify harness tool calls as read-only, write, or ambiguous.
//!
//! Pure reads never acquire a lease. Known mutators need the writer lease.
//! Ambiguous tools do **not** take the lease — if they did, the first Task or
//! opaque script became exclusive writer and bricked every other session.
//! Known mutators (including compound/`bash -c`/substitution Forms classified
//! as Write) still deny under a foreign holder.

use serde_json::Value;

/// Whether a tool call needs the writer lease before it may proceed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteClass {
    /// Safe without a lease (status, diff, search, non-mutating tests, …).
    Read,
    /// Known mutating tool / command — must hold the writer lease.
    Write,
    /// Not proven read-only — treated as write by the gate.
    Ambiguous,
}

impl WriteClass {
    /// Only proven checkout mutators contend for the writer lease.
    /// Ambiguous must not — acquiring on Task/`./mystery.sh` is what locked
    /// entire fleets out of a worktree.
    pub fn needs_lease(self) -> bool {
        matches!(self, Self::Write)
    }
}

/// Classify a hook tool payload (`tool_name` + `tool_input` / Cursor `command`).
pub fn classify_tool(tool_name: &str, tool_input: &Value) -> WriteClass {
    let name = tool_name.trim();
    if name.is_empty() {
        return WriteClass::Ambiguous;
    }
    let lower = name.to_ascii_lowercase();

    // Cursor / Claude / Codex read + gather tools (never need a lease).
    if matches!(
        lower.as_str(),
        "read"
            | "grep"
            | "glob"
            | "semanticsearch"
            | "semsearch"
            | "websearch"
            | "webfetch"
            | "askquestion"
            | "todoread"
            | "listmcpresources"
            | "fetchmcpresource"
            | "getmcptools"
            | "await"
            | "awaitshell"
            | "readlints"
            | "browser_snapshot"
            | "browser_tabs"
            | "browser_console_messages"
            | "browser_network_requests"
            | "browser_take_screenshot"
            // Claude Code / Codex fleet gather — reading task output must never
            // brick a session when another writer holds the checkout lease.
            | "taskoutput"
            | "task_output"
            | "gettaskoutput"
            | "monitor"
            | "monitoryevent"
            | "task_stop" // stop is control-plane, not a checkout write
            | "sendusermessage"
            | "switchmode"
            // The harness task list lives in the harness, not in the tree.
            // Measured: 84 unreadable admissions were these, and an unreadable
            // call is one nobody can rule out — which is how a session ends up
            // waiting on a checkout for a to-do item.
            | "taskcreate"
            | "taskupdate"
            | "tasklist"
            | "taskget"
            | "structuredoutput"
    ) {
        return WriteClass::Read;
    }

    // Known checkout / workspace mutators.
    if matches!(
        lower.as_str(),
        "write"
            | "edit"
            | "multiedit"
            | "delete"
            | "notebookedit"
            | "notebook_edit"
            | "apply_patch"
            | "applypatch"
            | "todowrite"
            | "editnotebook"
            | "generateimage"
            | "slashcommand"
    ) || lower.starts_with("edit")
        || lower == "write"
    {
        return WriteClass::Write;
    }

    // Agent orchestration: may spawn writers later, but must not itself take
    // the checkout lease (that bricked sibling sessions). Ambiguous ⇒ no lease.
    if matches!(
        lower.as_str(),
        "task" | "agent" | "workflow" | "bestofnrunner" | "best-of-n-runner"
    ) {
        return WriteClass::Ambiguous;
    }

    // Shell family — classify the command string.
    if matches!(
        lower.as_str(),
        "bash" | "shell" | "exec_command" | "powershell" | "terminal"
    ) {
        let command = tool_input
            .get("command")
            .and_then(|v| v.as_str())
            .or_else(|| tool_input.get("cmd").and_then(|v| v.as_str()))
            .unwrap_or("");
        return classify_shell(command);
    }

    // Cursor beforeShellExecution sends the command at the top level.
    if let Some(command) = tool_input.get("command").and_then(|v| v.as_str())
        && (lower.contains("shell") || tool_input.get("tool_name").is_none())
    {
        // Only when this looks like a shell hook payload without a clearer tool.
        if name.eq_ignore_ascii_case("beforeshellexecution") {
            return classify_shell(command);
        }
    }

    // MCP and unknown tools: fail closed (may write).
    if lower.starts_with("mcp") || lower.starts_with("mcp__") || lower.starts_with("mcp:") {
        return WriteClass::Ambiguous;
    }

    WriteClass::Ambiguous
}

/// Classify a shell command string. Pipelines and chains inherit the strictest
/// segment class (Read < Ambiguous < Write). Command substitutions (`$(…)` /
/// backticks) are classified too — otherwise `echo $(git push)` looks like Read.
pub fn classify_shell(command: &str) -> WriteClass {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return WriteClass::Ambiguous;
    }
    // File redirection mutates. Descriptor duplication and explicit discard
    // are control-flow only and must not make a read require the checkout
    // writer lease (`2>&1`, `>/dev/null`).
    if has_write_redirect(trimmed) {
        return WriteClass::Write;
    }

    let (stripped, inners) = peel_command_substitutions(trimmed);
    let mut worst = WriteClass::Read;
    for inner in &inners {
        worst = stricter(worst, classify_shell(inner));
        if worst == WriteClass::Write {
            return WriteClass::Write;
        }
    }
    for segment in split_shell_segments(&stripped) {
        let class = classify_shell_segment(segment);
        worst = stricter(worst, class);
        if worst == WriteClass::Write {
            return WriteClass::Write;
        }
    }
    worst
}

/// Replace `$(…)` / `` `…` `` (outside single quotes) with `true` and return
/// the substitution bodies for recursive classification.
fn peel_command_substitutions(command: &str) -> (String, Vec<String>) {
    let bytes = command.as_bytes();
    let mut out = String::with_capacity(command.len());
    let mut inners = Vec::new();
    let mut i = 0;
    let mut in_squote = false;
    let mut in_dquote = false;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && in_dquote && i + 1 < bytes.len() {
            out.push(bytes[i] as char);
            out.push(bytes[i + 1] as char);
            i += 2;
            continue;
        }
        if b == b'\'' && !in_dquote {
            in_squote = !in_squote;
            out.push('\'');
            i += 1;
            continue;
        }
        if b == b'"' && !in_squote {
            in_dquote = !in_dquote;
            out.push('"');
            i += 1;
            continue;
        }
        if !in_squote
            && b == b'$'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'('
            && let Some((inner, end)) = extract_dollar_paren(command, i + 2)
        {
            inners.push(inner);
            out.push_str("true");
            i = end;
            continue;
        }
        if !in_squote
            && b == b'`'
            && let Some((inner, end)) = extract_backticks(command, i + 1)
        {
            inners.push(inner);
            out.push_str("true");
            i = end;
            continue;
        }
        out.push(b as char);
        i += 1;
    }
    (out, inners)
}

fn extract_dollar_paren(command: &str, start: usize) -> Option<(String, usize)> {
    let bytes = command.as_bytes();
    let mut depth = 1usize;
    let mut i = start;
    let mut in_squote = false;
    let mut in_dquote = false;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && in_dquote && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if b == b'\'' && !in_dquote {
            in_squote = !in_squote;
            i += 1;
            continue;
        }
        if b == b'"' && !in_squote {
            in_dquote = !in_dquote;
            i += 1;
            continue;
        }
        if !in_squote && !in_dquote {
            if b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
                depth += 1;
                i += 2;
                continue;
            }
            if b == b'(' {
                depth += 1;
            } else if b == b')' {
                depth -= 1;
                if depth == 0 {
                    return Some((command[start..i].to_string(), i + 1));
                }
            }
        }
        i += 1;
    }
    None
}

fn extract_backticks(command: &str, start: usize) -> Option<(String, usize)> {
    let bytes = command.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            return Some((command[start..i].to_string(), i + 1));
        }
        i += 1;
    }
    None
}

fn stricter(a: WriteClass, b: WriteClass) -> WriteClass {
    use WriteClass::*;
    match (a, b) {
        (Write, _) | (_, Write) => Write,
        (Ambiguous, _) | (_, Ambiguous) => Ambiguous,
        (Read, Read) => Read,
    }
}

fn has_write_redirect(command: &str) -> bool {
    let bytes = command.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            let quote = b;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' && quote == b'"' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if b == b'>' {
            i += 1;
            // Append (`>>`) and noclobber override (`>|`) use the same target
            // rules as a normal redirect.
            if i < bytes.len() && (bytes[i] == b'>' || bytes[i] == b'|') {
                i += 1;
            }
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }

            // `2>&1` / `>&-` only duplicate or close descriptors.
            if i < bytes.len() && bytes[i] == b'&' {
                i += 1;
                let start = i;
                if i < bytes.len() && bytes[i] == b'-' {
                    i += 1;
                } else {
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i > start && redirect_boundary(bytes, i) {
                    continue;
                }
                return true;
            }

            // Discarding output is not a checkout mutation. Support the
            // unquoted and quoted forms emitted by shell wrappers.
            let (target, end) = redirect_target(command, i);
            if target.as_deref() == Some("/dev/null") {
                i = end;
                continue;
            }
            return true;
        }
        // `tee` is handled as a write command; `<<` alone is ambiguous input.
        i += 1;
    }
    false
}

fn redirect_target(command: &str, start: usize) -> (Option<String>, usize) {
    let bytes = command.as_bytes();
    if start >= bytes.len() {
        return (None, start);
    }
    if bytes[start] == b'\'' || bytes[start] == b'"' {
        let quote = bytes[start];
        let mut end = start + 1;
        while end < bytes.len() && bytes[end] != quote {
            end += 1;
        }
        let target = command.get(start + 1..end).map(str::to_string);
        return (target, end.saturating_add(1).min(bytes.len()));
    }
    let mut end = start;
    while end < bytes.len()
        && !bytes[end].is_ascii_whitespace()
        && !matches!(bytes[end], b';' | b'&' | b'|')
    {
        end += 1;
    }
    (command.get(start..end).map(str::to_string), end)
}

fn redirect_boundary(bytes: &[u8], index: usize) -> bool {
    index >= bytes.len()
        || bytes[index].is_ascii_whitespace()
        || matches!(bytes[index], b';' | b'&' | b'|')
}

fn split_shell_segments(command: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = command.as_bytes();
    let mut start = 0;
    let mut i = 0;
    let mut in_squote = false;
    let mut in_dquote = false;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\'' && !in_dquote {
            in_squote = !in_squote;
            i += 1;
            continue;
        }
        if b == b'"' && !in_squote {
            in_dquote = !in_dquote;
            i += 1;
            continue;
        }
        if !in_squote && !in_dquote {
            // && || ; | (pipeline) \n
            if b == b';' || b == b'\n' {
                push_seg(&mut out, command, start, i);
                start = i + 1;
                i += 1;
                continue;
            }
            if b == b'&' && i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                push_seg(&mut out, command, start, i);
                start = i + 2;
                i += 2;
                continue;
            }
            if b == b'|' {
                // || or |
                if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    push_seg(&mut out, command, start, i);
                    start = i + 2;
                    i += 2;
                } else {
                    push_seg(&mut out, command, start, i);
                    start = i + 1;
                    i += 1;
                }
                continue;
            }
        }
        i += 1;
    }
    push_seg(&mut out, command, start, bytes.len());
    out
}

fn push_seg<'a>(out: &mut Vec<&'a str>, command: &'a str, start: usize, end: usize) {
    let seg = command[start..end].trim();
    if !seg.is_empty() {
        out.push(seg);
    }
}

fn classify_shell_segment(segment: &str) -> WriteClass {
    let tokens = shell_tokens(segment);
    let mut idx = 0;
    // Strip leading env assignments: FOO=bar CMD
    while idx < tokens.len() {
        let t = tokens[idx];
        if t.contains('=') && !t.starts_with('-') && !t.starts_with('/') {
            let key = t.split('=').next().unwrap_or("");
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                idx += 1;
                continue;
            }
        }
        break;
    }
    // Skip wrappers that don't change the underlying class.
    while idx < tokens.len() {
        match tokens[idx] {
            "command" if tokens.get(idx + 1) == Some(&"-v") => {
                // `command -v foo` is a read probe.
                return WriteClass::Read;
            }
            "command" | "env" | "nice" | "nohup" | "time" | "timeout" => {
                idx += 1;
                // timeout N …
                if tokens[idx - 1] == "timeout"
                    && idx < tokens.len()
                    && tokens[idx].chars().all(|c| c.is_ascii_digit() || c == '.')
                {
                    idx += 1;
                }
                continue;
            }
            "sudo" | "doas" => {
                idx += 1;
                continue;
            }
            "bun" | "npm" | "pnpm" | "yarn" | "cargo" | "go" | "python" | "python3" | "node"
            | "git" | "harbard" => break,
            _ => break,
        }
    }
    if idx >= tokens.len() {
        return WriteClass::Ambiguous;
    }
    let bin = tokens[idx];
    let args = &tokens[idx + 1..];
    classify_bin(bin, args)
}

fn shell_tokens(segment: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = segment.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'\'' || bytes[i] == b'"' {
            let quote = bytes[i];
            let start = i + 1;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            out.push(&segment[start..i]);
            if i < bytes.len() {
                i += 1;
            }
            continue;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        out.push(&segment[start..i]);
    }
    out
}

fn classify_bin(bin: &str, args: &[&str]) -> WriteClass {
    let base = bin.rsplit('/').next().unwrap_or(bin);
    let base_l = base.to_ascii_lowercase();

    match base_l.as_str() {
        // Pure reads / probes.
        "ls" | "ll" | "cat" | "head" | "tail" | "less" | "more" | "wc" | "file" | "stat"
        | "test" | "[" | "true" | "false" | "echo" | "printf" | "pwd" | "which" | "type"
        | "printenv" | "date" | "hostname" | "uname" | "id" | "whoami" | "basename" | "dirname"
        | "realpath" | "readlink" | "md5" | "md5sum" | "sha256sum" | "shasum" | "diff" | "cmp"
        | "sort" | "uniq" | "cut" | "tr" | "jq" | "yq" | "rg" | "grep" | "egrep" | "fgrep"
        | "ag" | "ack" | "find" | "fd" | "tree" | "bat" | "hexdump" | "xxd" | "strings"
        | "curl" | "wget" | "http" | "xh" | "man" | "help" | "sleep"
        // Navigation changes where a command runs, not what is in the tree.
        // Leaving it unknown made `cd x && ls` unreadable — 113 admissions on
        // this machine, every one of them a read wearing a cd. Whether the
        // destination is somebody else's checkout is a different question, and
        // `lifecycle::classify` is the one that answers it.
        | "cd" | "pushd" | "popd" => WriteClass::Read,

        "git" => classify_git(args),
        "gh" => classify_gh(args),
        "harbard" => classify_harbard(args),
        "cargo" => classify_cargo(args),
        "bun" | "npm" | "pnpm" | "yarn" => classify_js_pm(args),
        "go" => classify_go(args),
        "python" | "python3" | "pytest" => classify_python(base_l.as_str(), args),
        "just" => classify_just(args),
        "make" => WriteClass::Ambiguous,


        // Browser binaries / Playwright list — not checkout mutators.
        "playwright" | "chromium" | "chrome" | "msedge" | "firefox" | "open" => WriteClass::Read,

        // `sed` only mutates its input with an in-place flag. Without it,
        // transformed content goes to stdout and is subject to the redirect
        // classification above.
        "sed" => classify_sed(args),

        // Known mutators.
        "rm" | "rmi" | "mv" | "cp" | "install" | "touch" | "mkdir" | "rmdir" | "chmod"
        | "chown" | "ln" | "tee" | "perl" | "ruby" | "truncate" | "dd" | "shred" | "rsync"
        | "tar" | "unzip" | "gzip" | "gunzip" | "patch" | "ed" | "ex" | "vim" | "nvim" | "code" => {
            WriteClass::Write
        }

        // Shells: recurse into `-c` / `-lc` scripts so
        // `bash -c 'git push'` cannot soft-allow under a foreign lease.
        "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" => classify_shell_interpreter(args),
        "eval" => {
            if args.is_empty() {
                WriteClass::Ambiguous
            } else {
                classify_shell(&args.join(" "))
            }
        }
        "source" | "." => WriteClass::Ambiguous,

        _ => WriteClass::Ambiguous,
    }
}

/// Classify `bash -c '…'`, `bash -lc '…'`, `sh -c '…'`.
///
/// Without a parseable `-c` script, fail closed as [`WriteClass::Write`] for
/// bare interactive shells invoked as tool commands (rare and opaque).
fn classify_shell_interpreter(args: &[&str]) -> WriteClass {
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a == "-c" || a == "--command" {
            return match args.get(i + 1) {
                Some(script) => classify_shell(script),
                // `bash -c` with no script is nonsense — treat as write.
                None => WriteClass::Write,
            };
        }
        if let Some(script) = a.strip_prefix("--command=") {
            return classify_shell(script);
        }
        // Clustered short flags: -lc, -ic, -ec, -c, …
        if a.starts_with('-') && !a.starts_with("--") && a.contains('c') {
            return match args.get(i + 1) {
                Some(script) => classify_shell(script),
                None => WriteClass::Write,
            };
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        // `bash ./script.sh` — opaque file, may mutate.
        return WriteClass::Ambiguous;
    }
    // Bare `bash` / `sh` with no args as a tool call — fail closed.
    WriteClass::Write
}

fn classify_sed(args: &[&str]) -> WriteClass {
    for arg in args {
        if *arg == "--" {
            break;
        }
        if *arg == "--in-place" || arg.starts_with("--in-place=") {
            return WriteClass::Write;
        }
        if arg.starts_with('-')
            && !arg.starts_with("--")
            && arg
                .as_bytes()
                .get(1..)
                .is_some_and(|flags| flags.contains(&b'i'))
        {
            return WriteClass::Write;
        }
    }
    WriteClass::Read
}

/// GitHub CLI: read subcommands never need the checkout writer lease.
fn classify_gh(args: &[&str]) -> WriteClass {
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        i += 1;
    }
    let Some(group) = args.get(i).copied() else {
        return WriteClass::Ambiguous;
    };
    let sub = args.get(i + 1).copied().unwrap_or("");
    match group {
        "help" | "version" | "status" | "browse" | "search" => WriteClass::Read,
        "auth" => match sub {
            "status" | "token" | "" => WriteClass::Read,
            _ => WriteClass::Ambiguous,
        },
        "run" | "workflow" => match sub {
            "list" | "view" | "watch" | "" => WriteClass::Read,
            // Downloads write into the cwd.
            "download" | "rerun" | "cancel" | "delete" | "enable" | "disable" | "run" => {
                WriteClass::Write
            }
            _ => WriteClass::Ambiguous,
        },
        "pr" | "issue" => match sub {
            "list" | "view" | "status" | "checks" | "diff" => WriteClass::Read,
            // `gh pr checkout` mutates the worktree.
            "checkout" | "create" | "close" | "merge" | "ready" | "edit" | "lock" | "unlock"
            | "reopen" | "comment" | "review" | "mark" | "delete" | "develop" => WriteClass::Write,
            _ => WriteClass::Ambiguous,
        },
        "repo" => match sub {
            "view" | "list" | "clone" => {
                if sub == "clone" {
                    WriteClass::Write
                } else {
                    WriteClass::Read
                }
            }
            "create" | "delete" | "edit" | "rename" | "sync" | "fork" | "deploy-key"
            | "archive" | "unarchive" => WriteClass::Write,
            _ => WriteClass::Ambiguous,
        },
        // The HTTP method decides whether this writes to GITHUB. It has no
        // bearing on the working tree, which is the only thing a writer lease
        // protects — `gh api -X POST` does not touch one tracked file. Calling
        // it ambiguous meant a session could not so much as compare two
        // branches while a peer held the checkout, which is how a lock meant
        // for one tree became a lock on the whole machine. `curl` is already
        // Read for exactly this reason, and a redirect into a file is caught
        // upstream by `has_write_redirect`.
        "api" => WriteClass::Read,
        "release" | "label" | "project" | "gist" | "codespace" | "cache" | "secret"
        | "variable" | "extension" | "alias" | "config" | "attestation" => match sub {
            "list" | "view" | "download" | "" => WriteClass::Read,
            _ => WriteClass::Ambiguous,
        },
        _ => WriteClass::Ambiguous,
    }
}

fn classify_git(args: &[&str]) -> WriteClass {
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        // Options that take their value as the next word (`-c key=value`).
        if a == "-C" || a == "-c" || a == "--git-dir" || a == "--work-tree" {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        break;
    }
    let Some(sub) = args.get(i).copied() else {
        return WriteClass::Ambiguous;
    };
    match sub {
        "status" | "diff" | "log" | "show" | "blame" | "describe" | "rev-parse" | "rev-list"
        | "ls-files" | "ls-tree" | "cat-file" | "name-rev" | "symbolic-ref" | "remote"
        | "branch" | "tag" | "stash" | "config" | "shortlog" | "version" | "help" | "grep"
        | "range-diff" | "whatchanged" | "reflog" => {
            // Narrow known mutating forms of otherwise-read subcommands.
            if sub == "branch"
                && args[i + 1..]
                    .iter()
                    .any(|a| matches!(*a, "-d" | "-D" | "-m" | "-M" | "-c" | "-C"))
            {
                return WriteClass::Write;
            }
            if sub == "tag"
                && args[i + 1..]
                    .iter()
                    .any(|a| matches!(*a, "-d" | "-f" | "-a" | "-s" | "-u"))
            {
                // listing tags is read; creating/deleting is write. Bare `git tag` lists.
                if args[i + 1..].iter().any(|a| !a.starts_with('-')) {
                    return WriteClass::Write;
                }
                if args[i + 1..].iter().any(|a| matches!(*a, "-d" | "-f")) {
                    return WriteClass::Write;
                }
            }
            // Bare `git stash` (or `git stash -u`, `-m …`) is `git stash push`.
            if sub == "stash"
                && args.get(i + 1).is_none_or(|a| {
                    a.starts_with('-')
                        || matches!(
                            *a,
                            "push" | "pop" | "apply" | "drop" | "clear" | "create" | "store"
                        )
                })
            {
                return WriteClass::Write;
            }
            if sub == "remote"
                && args.get(i + 1).is_some_and(|a| {
                    matches!(*a, "add" | "remove" | "rm" | "set-url" | "rename" | "prune")
                })
            {
                return WriteClass::Write;
            }
            if sub == "config"
                && !args[i + 1..]
                    .iter()
                    .any(|a| *a == "--get" || *a == "--list" || *a == "-l")
            {
                // `git config key` reads; `git config key value` writes. Ambiguous without parse.
                if args[i + 1..].len() >= 2 && !args[i + 1].starts_with('-') {
                    return WriteClass::Ambiguous;
                }
            }
            WriteClass::Read
        }
        "worktree" => classify_git_worktree(&args[i + 1..]),
        "add" | "commit" | "checkout" | "switch" | "restore" | "merge" | "rebase"
        | "cherry-pick" | "revert" | "reset" | "clean" | "push" | "pull" | "fetch" | "clone"
        | "init" | "mv" | "rm" | "am" | "apply" | "submodule" | "bisect" | "notes" | "replace"
        | "update-index" | "update-ref" | "commit-tree" | "read-tree" | "write-tree" | "gc"
        | "prune" | "repack" | "filter-branch" | "filter-repo" => WriteClass::Write,
        _ => WriteClass::Ambiguous,
    }
}

fn classify_git_worktree(args: &[&str]) -> WriteClass {
    let sub = args.iter().copied().find(|arg| !arg.starts_with('-'));
    match sub {
        None | Some("list") => WriteClass::Read,
        // This is the prescribed escape from a contested checkout. Git
        // serializes its common-dir metadata; the new tree gets its own writer
        // resource before source edits begin.
        Some("add") => WriteClass::Ambiguous,
        Some("remove" | "move" | "prune" | "repair" | "lock" | "unlock") => WriteClass::Write,
        Some(_) => WriteClass::Ambiguous,
    }
}

fn classify_harbard(args: &[&str]) -> WriteClass {
    let mut i = 0;
    while i < args.len() {
        let a = args[i];
        if a.starts_with("--") {
            // flag with value
            if matches!(
                a,
                "--home"
                    | "--session"
                    | "--harness"
                    | "--hook"
                    | "--cwd"
                    | "--repo"
                    | "--branch"
                    | "--role"
                    | "--stale-secs"
                    | "--retention-days"
                    // Its value is a whole shell command; without this the
                    // loop would read the command itself as the verb.
                    | "--command"
            ) {
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        break;
    }
    let Some(verb) = args.get(i).copied() else {
        return WriteClass::Ambiguous;
    };
    match verb {
        // Bus / status verbs write only under ~/.harbard — never the git
        // checkout. Requiring the writer lease here bricked agents that only
        // needed to declare intent or ack under a foreign holder.
        // `friction` and `deliveries` only read the store; `deliver` appends a
        // queue entry under ~/.harbard and touches no checkout — you are handing
        // over a SHA, which is the whole point of it existing. `friction`
        // reported ITSELF as unreadable on its first run, which is the loop this
        // is meant to close: the log names what to teach, and this is the answer.
        "ls" | "brief" | "inbox" | "doctor" | "audit" | "check" | "protocol" | "channels"
        | "trace" | "export" | "help" | "hello" | "intent" | "note" | "ack" | "bye" | "join"
        | "leave" | "search" | "call" | "subscribe" | "friction" | "deliveries" | "deliver"
        | "checkout-policy" => WriteClass::Read,
        // Lanes live under ~/.harbard/lanes and touch no checkout. A session
        // waiting on `await lane` must never need the writer lease it is
        // queueing behind to ask for its turn.
        "lane" | "heartbeat" | "await" => WriteClass::Read,
        // Mutate harness configs, lease store authority, or long-running hosts.
        // `integrate` genuinely moves a branch, so it is the one sluice verb
        // that must hold something before it runs.
        "claim" | "release" | "setup" | "install" | "serve" | "hub" | "pair" | "device"
        | "satellite" | "gate" | "integrate" => WriteClass::Write,
        _ => WriteClass::Ambiguous,
    }
}

fn classify_cargo(args: &[&str]) -> WriteClass {
    let Some(sub) = args.first().copied() else {
        return WriteClass::Ambiguous;
    };
    match sub {
        "test" | "check" | "clippy" | "fmt" | "deny" | "tree" | "metadata" | "search"
        | "version" | "help" | "explain" => {
            if sub == "fmt" && !args.contains(&"--check") {
                return WriteClass::Write;
            }
            WriteClass::Read
        }
        "build" | "run" | "bench" | "doc" => WriteClass::Ambiguous, // target/ only, but opaque
        "add" | "remove" | "update" | "install" | "publish" | "yank" | "init" | "new" | "fix"
        | "clean" | "generate-lockfile" => WriteClass::Write,
        _ => WriteClass::Ambiguous,
    }
}

fn classify_js_pm(args: &[&str]) -> WriteClass {
    let Some(sub) = args.first().copied() else {
        return WriteClass::Ambiguous;
    };
    match sub {
        "test" | "run" => {
            // bun run typecheck / lint / build — build writes; typecheck/test read-ish.
            if args.get(1).is_some_and(|a| {
                matches!(
                    *a,
                    "test"
                        | "typecheck"
                        | "lint"
                        | "check"
                        | "vitest"
                        | "jest"
                        | "playwright"
                        | "e2e"
                )
            }) {
                return WriteClass::Read;
            }
            if sub == "test" {
                return WriteClass::Read;
            }
            WriteClass::Ambiguous
        }
        "x" | "dlx" | "exec" | "pm" => WriteClass::Ambiguous,
        "install" | "i" | "add" | "remove" | "uninstall" | "update" | "upgrade" | "link"
        | "publish" => WriteClass::Write,
        _ => WriteClass::Ambiguous,
    }
}

fn classify_go(args: &[&str]) -> WriteClass {
    match args.first().copied() {
        Some("test" | "vet" | "list" | "version" | "env" | "doc" | "help") => WriteClass::Read,
        Some("build" | "run") => WriteClass::Ambiguous,
        Some("get" | "install" | "mod" | "generate" | "fmt" | "fix") => WriteClass::Write,
        _ => WriteClass::Ambiguous,
    }
}

fn classify_python(bin: &str, args: &[&str]) -> WriteClass {
    if bin == "pytest" {
        return WriteClass::Read;
    }
    if args.contains(&"-m") {
        let mut iter = args.iter();
        while let Some(a) = iter.next() {
            if *a == "-m"
                && let Some(mod_name) = iter.next()
                && *mod_name == "pytest"
            {
                return WriteClass::Read;
            }
        }
    }
    WriteClass::Ambiguous
}

fn classify_just(args: &[&str]) -> WriteClass {
    match args.first().copied() {
        Some("gates" | "test" | "check" | "lint" | "--list" | "-l") => WriteClass::Read,
        Some(_) => WriteClass::Ambiguous,
        None => WriteClass::Read,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `git stash` with no subcommand is `git stash push`: it rewrites the
    /// working tree. Only `list` and `show` read.
    #[test]
    fn bare_git_stash_writes_and_stash_list_reads() {
        for write in [
            "git stash",
            "git stash -u",
            "git stash -m wip",
            "git stash push",
        ] {
            assert_eq!(classify_shell(write), WriteClass::Write, "{write}");
        }
        for read in [
            "git stash list",
            "git stash show -p",
            "git -C /w stash list",
        ] {
            assert_eq!(classify_shell(read), WriteClass::Read, "{read}");
        }
        // `-c` takes a value: the verb after it is still found.
        assert_eq!(classify_shell("git -c a=b commit -m x"), WriteClass::Write);
        assert_eq!(classify_shell("git -c a=b status"), WriteClass::Read);
    }

    /// Two shapes the classifier could not read, taken straight off the
    /// friction report: 113 admissions were a read wearing a `cd`, and 84 were
    /// the harness's own task list. Neither can touch a checkout.
    #[test]
    fn navigation_and_the_task_list_are_reads() {
        assert_eq!(classify_shell("cd apps/console && ls"), WriteClass::Read);
        assert_eq!(classify_shell("cd /tmp"), WriteClass::Read);
        assert_eq!(classify_shell("pushd x && cat y"), WriteClass::Read);
        for tool in ["TaskCreate", "TaskUpdate", "TaskList", "StructuredOutput"] {
            assert_eq!(
                classify_tool(tool, &serde_json::json!({})),
                WriteClass::Read,
                "{tool}"
            );
        }
        // And a cd in front of a real mutation is still a mutation.
        assert_eq!(
            classify_shell("cd apps/console && git commit -m x"),
            WriteClass::Write
        );
        assert_eq!(classify_shell("cd x && rm -f y"), WriteClass::Write);
    }
    use serde_json::json;

    #[test]
    fn read_tools_need_no_lease() {
        for name in ["Read", "Grep", "Glob", "WebSearch"] {
            assert_eq!(classify_tool(name, &json!({})), WriteClass::Read);
        }
    }

    #[test]
    fn write_tools_need_lease() {
        for name in ["Write", "Edit", "Delete", "apply_patch"] {
            assert_eq!(classify_tool(name, &json!({})), WriteClass::Write);
        }
    }

    #[test]
    fn gather_tools_need_no_lease() {
        for name in ["TaskOutput", "Monitor", "Await", "AwaitShell"] {
            assert_eq!(classify_tool(name, &json!({})), WriteClass::Read);
        }
        for cmd in ["playwright test --list", "chromium --version"] {
            assert_eq!(
                classify_shell(cmd),
                WriteClass::Read,
                "{cmd} must not require writer lease"
            );
        }
        for cmd in [] {
            assert_eq!(
                classify_shell(cmd),
                WriteClass::Write,
                "{cmd} must require writer lease"
            );
        }
    }

    #[test]
    fn command_substitution_cannot_hide_write() {
        for cmd in [
            r#"echo $(git push)"#,
            "echo $(git push origin main)",
            "echo `git push`",
            r#"printf '%s' "$(git push)"#,
            r#"echo $(echo $(git push))"#,
        ] {
            assert_eq!(
                classify_shell(cmd),
                WriteClass::Write,
                "substitution bypass: {cmd}"
            );
        }
        assert_eq!(classify_shell(r#"echo $(git log -1)"#), WriteClass::Read);
    }

    #[test]
    fn checkout_mutators_still_need_lease() {
        for cmd in [
            "git add -A",
            "git commit -m x",
            "rm -rf src",
            "cargo fmt --all",
            "bun install",
            "echo x > file.rs",
        ] {
            assert_eq!(
                classify_shell(cmd),
                WriteClass::Write,
                "{cmd} must remain Write"
            );
        }
    }

    /// P0 security: compound / wrapper forms must not strip a trailing write.
    #[test]
    fn adversarial_compound_and_shell_c_cannot_bypass_write() {
        for cmd in [
            r#"echo "x" && git push"#,
            "true && git push origin main",
            "false || git push",
            "true; git push",
            r#"bash -c "git push""#,
            r#"bash -c 'git push origin main'"#,
            r#"bash -lc "git push""#,
            r#"sh -c "git push""#,
            "eval git push",
            r#"echo hi && bash -c 'git push'"#,
        ] {
            assert_eq!(
                classify_shell(cmd),
                WriteClass::Write,
                "bypass must not work: {cmd}"
            );
        }
    }

    /// Absolute paths used to fall through to Ambiguous (soft-allow under a
    /// foreign lease) while the bare token was blanket-Write — so `/usr/bin/git
    /// log` slipped reads past a gate that blocked `git log`, and `/usr/bin/git
    /// push` would have slipped writes. Basename + subcommand close both.
    #[test]
    fn absolute_path_uses_basename_not_ambiguous() {
        for cmd in [
            "/usr/bin/git push",
            "/opt/homebrew/bin/git push origin main",
            "/bin/bash -c '/usr/bin/git push'",
            "env /usr/bin/git push",
            "command /usr/bin/git push",
            "/usr/local/bin/gh pr merge 1",
            "/opt/homebrew/bin/harbard claim --writer",
        ] {
            assert_eq!(
                classify_shell(cmd),
                WriteClass::Write,
                "absolute path must not bypass: {cmd}"
            );
        }
        for cmd in [
            "/usr/bin/git log -1",
            "/usr/bin/git show HEAD",
            "/usr/bin/git diff --stat",
            "/usr/bin/git grep -n foo",
            "/opt/homebrew/bin/gh run list",
            "/usr/local/bin/harbard ls",
        ] {
            assert_eq!(
                classify_shell(cmd),
                WriteClass::Read,
                "absolute path read must classify: {cmd}"
            );
        }
    }

    #[test]
    fn git_gh_harbard_read_subcommands() {
        for cmd in [
            "git log -1",
            "git blame HEAD -- crates/harbard-core/src/lib.rs",
            "git status",
            "gh run list",
            "gh pr view 1",
            "gh pr checks 1",
            "gh issue list",
            "harbard ls",
            "harbard doctor",
        ] {
            assert_eq!(classify_shell(cmd), WriteClass::Read, "{cmd}");
        }
        for cmd in [
            "gh pr merge 1",
            "gh pr create",
            "harbard claim --writer",
            "git push",
        ] {
            assert_eq!(classify_shell(cmd), WriteClass::Write, "{cmd}");
        }
        // Bus control plane must not require the checkout writer lease.
        for cmd in [
            r#"harbard intent "take the PRE password reset""#,
            r#"harbard note "ack handoff""#,
            "harbard ack 01KYT71QYMQ47RVCW38RG6Z8H7",
            "harbard hello",
            "harbard bye",
            r#"harbard lane take migrations --for "0234-0236" --release-on pr:2258"#,
            "harbard lane release dev-stack",
            "harbard heartbeat",
            "harbard await lane migrations --timeout-secs 600",
            "harbard await pr 2258 merged",
        ] {
            assert_eq!(
                classify_shell(cmd),
                WriteClass::Read,
                "bus verb must not need checkout lease: {cmd}"
            );
        }
    }

    #[test]
    fn orchestration_is_ambiguous_not_hard_write() {
        for name in ["Task", "Agent", "Workflow"] {
            assert_eq!(classify_tool(name, &json!({})), WriteClass::Ambiguous);
            assert!(
                !classify_tool(name, &json!({})).needs_lease(),
                "{name} must not take the writer lease"
            );
        }
    }

    #[test]
    fn shell_reads_vs_writes() {
        assert_eq!(classify_shell("git status"), WriteClass::Read);
        assert_eq!(classify_shell("git diff --stat HEAD"), WriteClass::Read);
        assert_eq!(classify_shell("git log -1 --oneline"), WriteClass::Read);
        assert_eq!(classify_shell("rg WriteClass crates"), WriteClass::Read);
        assert_eq!(classify_shell("cargo test --workspace"), WriteClass::Read);
        assert_eq!(classify_shell("cargo fmt --all --check"), WriteClass::Read);
        assert_eq!(classify_shell("bun run typecheck"), WriteClass::Read);
        assert_eq!(classify_shell("git add -A"), WriteClass::Write);
        assert_eq!(classify_shell("git commit -m 'x'"), WriteClass::Write);
        assert_eq!(classify_shell("rm -rf target"), WriteClass::Write);
        assert_eq!(classify_shell("echo hi > /tmp/out"), WriteClass::Write);
    }

    #[test]
    fn read_only_shell_forms_do_not_take_the_checkout_lease() {
        for cmd in [
            // A peer holding the checkout must never stop you reading GitHub:
            // this is the exact call a session could not make while blocked,
            // which left it unable to compare two branches.
            "gh api repos/o/r/compare/a...b",
            "gh api -X POST repos/o/r/issues/1/comments -f body=x",
            "sed -n '1,20p' README.md",
            "sed -e 's/x/y/' README.md",
            "git worktree list --porcelain",
            "git worktree add --help",
            "ls >/dev/null",
            "rg needle 2>/dev/null",
            "git status 2>&1",
            "open --help",
            "codex exec --help",
            "codex-rescue --help",
            "harbard subscribe --once",
        ] {
            assert!(
                !classify_shell(cmd).needs_lease(),
                "{cmd} must remain available under a foreign writer lease"
            );
        }
    }

    #[test]
    fn nearby_mutating_forms_still_take_the_checkout_lease() {
        for cmd in [
            "sed -i '' 's/x/y/' README.md",
            "sed -i.bak 's/x/y/' README.md",
            "sed 's/x/y/' -i README.md",
            "sed --in-place=.bak 's/x/y/' README.md",
            "git worktree remove other",
            "git worktree move old new",
            "git push",
            "echo changed > README.md",
            "rg needle 2>errors.log",
        ] {
            assert_eq!(
                classify_shell(cmd),
                WriteClass::Write,
                "{cmd} must remain protected by the writer lease"
            );
        }
    }

    #[test]
    fn ambiguous_does_not_take_lease() {
        assert_eq!(
            classify_shell("./scripts/mystery.sh"),
            WriteClass::Ambiguous
        );
        assert_eq!(classify_shell("make release"), WriteClass::Ambiguous);
        assert_eq!(
            classify_tool("mcp__fs__write", &json!({})),
            WriteClass::Ambiguous
        );
        assert!(!classify_shell("./scripts/mystery.sh").needs_lease());
        assert_eq!(
            classify_shell("./scripts/mystery.sh && git push"),
            WriteClass::Write
        );
        assert!(classify_shell("git push").needs_lease());
    }

    #[test]
    fn chain_inherits_strictest() {
        assert_eq!(classify_shell("git status && git diff"), WriteClass::Read);
        assert_eq!(
            classify_shell("git status && git commit -m x"),
            WriteClass::Write
        );
        assert_eq!(
            classify_shell("git status && ./mystery"),
            WriteClass::Ambiguous
        );
    }

    #[test]
    fn bash_tool_uses_command_field() {
        assert_eq!(
            classify_tool("Bash", &json!({"command": "git status"})),
            WriteClass::Read
        );
        assert_eq!(
            classify_tool("Shell", &json!({"command": "git push"})),
            WriteClass::Write
        );
    }
}
