//! Read-only Git checkout and linked-worktree hygiene inspection.

use serde::Serialize;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt, OsStringExt};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditReport {
    requested_repo: String,
    common_dir: Option<String>,
    requested_target: Option<String>,
    include_worktrees: bool,
    worktrees: Vec<WorktreeReport>,
    errors: Vec<String>,
}

impl AuditReport {
    pub fn has_repository(&self) -> bool {
        self.common_dir.is_some()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorktreeReport {
    root: String,
    primary: bool,
    branch: Option<String>,
    detached: bool,
    head: Option<String>,
    upstream: Option<String>,
    upstream_gone: bool,
    target: Option<String>,
    target_head: Option<String>,
    ahead: Option<u64>,
    behind: Option<u64>,
    dirty: bool,
    /// What the lane's own lifecycle receipt claims, checked against evidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    lane: Option<LaneVerdict>,
    errors: Vec<String>,
}

/// A lane's state, derived rather than believed.
///
/// `.WORKTREE-OWNER.md` records `State: active` when the lane is opened and
/// nothing ever changes it back: closure is a declaration, and the session that
/// owed the declaration is usually the one that died.
///
/// So the receipt is treated as a claim and checked: is the supervisor alive,
/// and is there anything in there worth keeping?
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LaneVerdict {
    /// The session named on the receipt.
    session: Option<String>,
    /// Whether its supervisor process still exists.
    supervisor_alive: bool,
    /// `active` | `abandoned` | `stranded` | `closed` | `unmarked`
    state: &'static str,
    /// What to do about it, when there is something to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    advice: Option<String>,
}

pub fn inspect(
    repo: &Path,
    requested_target: Option<&str>,
    include_worktrees: bool,
) -> AuditReport {
    let requested_repo = absolute_display(repo);
    let requested_target = requested_target.map(str::to_owned);
    let mut report = AuditReport {
        requested_repo,
        common_dir: None,
        requested_target: requested_target.clone(),
        include_worktrees,
        worktrees: Vec::new(),
        errors: Vec::new(),
    };

    let root = match git_path(repo, ["rev-parse", "--show-toplevel"]) {
        Ok(root) => root,
        Err(error) => {
            report.errors.push(format!("repository: {error}"));
            return report;
        }
    };

    let common_dir = match git_path(&root, ["rev-parse", "--git-common-dir"]) {
        Ok(path) => absolutize_from(&root, &path),
        Err(error) => {
            report.errors.push(format!("common Git directory: {error}"));
            return report;
        }
    };
    report.common_dir = Some(display_path(&common_dir));

    let all_roots = match list_worktree_roots(&root, &common_dir) {
        Ok(roots) if !roots.is_empty() => roots,
        Ok(_) => {
            report
                .errors
                .push("worktree list: Git returned no worktrees".to_string());
            vec![root.clone()]
        }
        Err(error) => {
            report.errors.push(format!("worktree list: {error}"));
            vec![root.clone()]
        }
    };
    let primary_root = all_roots.first().cloned().unwrap_or_else(|| root.clone());
    let selected_roots = if include_worktrees {
        // The worktree you asked about leads, then its siblings. Listing them
        // in Git's own order would have moved the answer to position 1 the day
        // linked lanes stopped being opt-in, which silently changes what every
        // existing `--json` caller reads out of `worktrees[0]`.
        let requested = canonical_or(&root);
        let mut ordered: Vec<PathBuf> = vec![requested.clone()];
        ordered.extend(
            all_roots
                .into_iter()
                .filter(|candidate| !same_path(&canonical_or(candidate), &requested)),
        );
        ordered
    } else {
        vec![root]
    };

    report.worktrees = selected_roots
        .into_iter()
        .map(|worktree| inspect_worktree(&worktree, &primary_root, requested_target.as_deref()))
        .collect();
    report
}

pub fn render(report: &AuditReport, json: bool) -> String {
    if json {
        return serde_json::to_string(report).unwrap_or_else(|_| "{}".to_string());
    }

    let mut lines = Vec::new();
    if let Some(common_dir) = &report.common_dir {
        lines.push(format!("Git repository: {common_dir}"));
    } else {
        lines.push(format!("Not a Git repository: {}", report.requested_repo));
    }
    for error in &report.errors {
        lines.push(format!("  error: {error}"));
    }
    for worktree in &report.worktrees {
        let identity = worktree
            .branch
            .as_deref()
            .map(str::to_owned)
            .or_else(|| worktree.head.as_deref().map(short_sha))
            .unwrap_or_else(|| "unknown HEAD".to_string());
        let kind = if worktree.primary {
            "primary"
        } else {
            "linked"
        };
        lines.push(format!("{} [{kind}] {identity}", worktree.root));
        let mut state = Vec::new();
        if worktree.detached {
            state.push("detached".to_string());
        }
        if worktree.dirty {
            state.push("dirty".to_string());
        } else {
            state.push("clean".to_string());
        }
        if worktree.upstream_gone {
            state.push("upstream gone".to_string());
        }
        if let (Some(ahead), Some(behind), Some(target)) =
            (worktree.ahead, worktree.behind, worktree.target.as_deref())
        {
            state.push(format!("ahead {ahead}, behind {behind} vs {target}"));
        }
        lines.push(format!("  {}", state.join("; ")));
        if let Some(lane) = &worktree.lane {
            let owner = lane.session.as_deref().unwrap_or("unknown session");
            lines.push(format!(
                "  lane {} · {} {}",
                lane.state,
                &owner[..owner.len().min(11)],
                if lane.supervisor_alive {
                    "(supervisor alive)"
                } else {
                    "(supervisor gone)"
                }
            ));
            if let Some(advice) = &lane.advice {
                lines.push(format!("    → {advice}"));
            }
        }
        for error in &worktree.errors {
            lines.push(format!("  error: {error}"));
        }
    }
    lines.join("\n")
}

fn inspect_worktree(
    root: &Path,
    primary_root: &Path,
    requested_target: Option<&str>,
) -> WorktreeReport {
    let mut errors = Vec::new();
    let root = canonical_or(root);
    let primary = same_path(&root, primary_root);

    let head = match git_text(&root, ["rev-parse", "HEAD"]) {
        Ok(head) => Some(head),
        Err(error) => {
            errors.push(format!("HEAD: {error}"));
            None
        }
    };
    let branch = git_text(&root, ["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
    let detached = head.is_some() && branch.is_none();
    let upstream = branch
        .as_deref()
        .and_then(|branch| configured_upstream(&root, branch));
    let upstream_gone = upstream
        .as_deref()
        .is_some_and(|upstream| resolve_commit(&root, upstream).is_err());

    let dirty = match worktree_is_dirty(&root) {
        Ok(dirty) => dirty,
        Err(error) => {
            errors.push(format!("working tree state: {error}"));
            false
        }
    };

    let target = if let Some(target) = requested_target {
        Some(target.to_string())
    } else if let Some(upstream) = &upstream {
        Some(upstream.clone())
    } else if resolve_commit(&root, "origin/dev").is_ok() {
        Some("origin/dev".to_string())
    } else {
        None
    };

    let mut target_head = None;
    let mut ahead = None;
    let mut behind = None;
    if let Some(target) = target.as_deref() {
        match resolve_commit(&root, target) {
            Ok(commit) => {
                target_head = Some(commit);
                if head.is_some() {
                    match git_text(
                        &root,
                        [
                            "rev-list",
                            "--left-right",
                            "--count",
                            &format!("{target}...HEAD"),
                        ],
                    ) {
                        Ok(counts) => match parse_counts(&counts) {
                            Some((target_only, head_only)) => {
                                behind = Some(target_only);
                                ahead = Some(head_only);
                            }
                            None => errors.push(format!(
                                "comparison with {target}: unexpected rev-list output {counts:?}"
                            )),
                        },
                        Err(error) => errors.push(format!("comparison with {target}: {error}")),
                    }
                }
            }
            Err(error) => errors.push(format!("target {target}: {error}")),
        }
    }

    let lane = (!primary).then(|| lane_verdict(&root, dirty, ahead.unwrap_or(0)));

    WorktreeReport {
        root: display_path(&root),
        primary,
        branch,
        detached,
        head,
        upstream,
        upstream_gone,
        target,
        target_head,
        ahead,
        behind,
        dirty,
        lane,
        errors,
    }
}

/// Check a lane's receipt against what is actually true of it.
fn lane_verdict(root: &Path, dirty: bool, ahead: u64) -> LaneVerdict {
    let Ok(receipt) = std::fs::read_to_string(root.join(".WORKTREE-OWNER.md")) else {
        return LaneVerdict {
            session: None,
            supervisor_alive: false,
            state: "unmarked",
            advice: Some(
                "no lifecycle receipt: this lane was not opened through worktree-lease.sh create"
                    .into(),
            ),
        };
    };
    let field = |name: &str| -> Option<String> {
        receipt
            .lines()
            .find_map(|line| {
                let mut cells = line.split('|').map(str::trim);
                cells.next()?;
                (cells.next()? == name).then(|| cells.next())?
            })
            .map(|value| value.trim_matches('`').trim().to_string())
    };
    let session = field("Session").filter(|s| !s.is_empty());
    let declared = field("State").unwrap_or_default();
    let supervisor_alive = field("Supervisor PID")
        .and_then(|pid| pid.parse::<u32>().ok())
        .is_some_and(|pid| {
            use harbard_core::lease::PidChecker;
            harbard_core::SystemPidChecker.is_alive(pid)
        });

    if declared != "active" {
        return LaneVerdict {
            session,
            supervisor_alive,
            state: "closed",
            advice: (dirty || ahead > 0).then(|| {
                "closed but not empty: rescue the work before removing the worktree".into()
            }),
        };
    }
    if supervisor_alive {
        return LaneVerdict {
            session,
            supervisor_alive,
            state: "active",
            advice: None,
        };
    }
    // The receipt says active and the process that would have closed it is
    // gone. Whether that costs anything depends on what is inside.
    if dirty || ahead > 0 {
        LaneVerdict {
            session,
            supervisor_alive,
            state: "stranded",
            advice: Some(format!(
                "owner is gone with work still here ({}{}{}). Rescue it — commit and deliver the \
                 SHA — before this lane is closed",
                if ahead > 0 {
                    format!("{ahead} commit(s) ahead")
                } else {
                    String::new()
                },
                if ahead > 0 && dirty { ", " } else { "" },
                if dirty { "uncommitted changes" } else { "" },
            )),
        }
    } else {
        LaneVerdict {
            session,
            supervisor_alive,
            state: "abandoned",
            advice: Some(
                "owner is gone and the lane is empty — nothing was ever produced here. Close it \
                 with worktree-lease.sh release, then remove it"
                    .into(),
            ),
        }
    }
}

fn configured_upstream(root: &Path, branch: &str) -> Option<String> {
    git_text(
        root,
        [
            "for-each-ref",
            "--format=%(upstream:short)",
            &format!("refs/heads/{branch}"),
        ],
    )
    .ok()
    .filter(|upstream| !upstream.is_empty())
}

/// Determine dirtiness without `git status`, index refreshes, fsmonitor, or
/// user-configured content programs. Exit code 1 from the diff plumbing means
/// "different"; any other non-zero status is an inspection error.
fn worktree_is_dirty(root: &Path) -> Result<bool, String> {
    if git_differs(
        root,
        [
            "diff-index",
            "--cached",
            "--quiet",
            "--no-ext-diff",
            "HEAD",
            "--",
        ],
    )? {
        return Ok(true);
    }

    if tracked_worktree_differs(root)? {
        return Ok(true);
    }

    let output = git_raw(root, ["ls-files", "--others", "--exclude-standard", "-z"])?;
    Ok(!output.stdout.is_empty())
}

fn tracked_worktree_differs(root: &Path) -> Result<bool, String> {
    let output = git_raw(
        root,
        [
            "diff-files",
            "--raw",
            "-z",
            "--full-index",
            "--no-abbrev",
            "--no-ext-diff",
            "--",
        ],
    )?;
    let mut fields = output.stdout.split(|byte| *byte == 0);
    while let Some(header) = fields.next() {
        if header.is_empty() {
            break;
        }
        let path = fields
            .next()
            .ok_or_else(|| "diff-files returned a record without a path".to_string())?;
        let columns: Vec<&[u8]> = header.split(|byte| byte.is_ascii_whitespace()).collect();
        if columns.len() != 5 || !columns[0].starts_with(b":") {
            return Err(format!(
                "diff-files returned an unexpected record {:?}",
                String::from_utf8_lossy(header)
            ));
        }
        let old_mode = &columns[0][1..];
        let new_mode = columns[1];
        let old_oid = columns[2];
        let status = columns[4];
        if old_mode != new_mode || status != b"M" {
            return Ok(true);
        }

        let path = PathBuf::from(os_string_from_bytes(path));
        let metadata = match std::fs::symlink_metadata(root.join(&path)) {
            Ok(metadata) => metadata,
            Err(_) => return Ok(true),
        };
        if metadata.file_type().is_dir() {
            return Ok(true);
        }
        let actual = if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(root.join(&path)).map_err(|error| error.to_string())?;
            git_hash_bytes(root, &os_str_bytes(target.as_os_str()))?
        } else if metadata.file_type().is_file() {
            let args: Vec<OsString> = vec![
                OsString::from("hash-object"),
                OsString::from("--no-filters"),
                OsString::from("--"),
                path.into_os_string(),
            ];
            strip_line_ending(&git_raw(root, args)?.stdout).to_vec()
        } else {
            return Ok(true);
        };
        if actual != old_oid {
            return Ok(true);
        }
    }
    Ok(false)
}

fn git_hash_bytes(root: &Path, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut child = git_command(root, ["hash-object", "--no-filters", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    child
        .stdin
        .take()
        .ok_or_else(|| "git hash-object stdin was unavailable".to_string())?
        .write_all(bytes)
        .map_err(|error| error.to_string())?;
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(strip_line_ending(&output.stdout).to_vec())
    } else {
        Err(git_failure(&output))
    }
}

fn git_differs<I, S>(root: &Path, args: I) -> Result<bool, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_command(root, args)
        .output()
        .map_err(|error| error.to_string())?;
    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(git_failure(&output)),
    }
}

fn resolve_commit(root: &Path, reference: &str) -> Result<String, String> {
    git_text(
        root,
        ["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
    )
}

fn parse_counts(value: &str) -> Option<(u64, u64)> {
    let mut parts = value.split_whitespace();
    let left = parts.next()?.parse().ok()?;
    let right = parts.next()?.parse().ok()?;
    (parts.next().is_none()).then_some((left, right))
}

fn list_worktree_roots(root: &Path, common_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let output = git_raw(root, ["worktree", "list", "--porcelain", "-z"])?;
    let mut roots = Vec::new();
    for field in output.stdout.split(|byte| *byte == 0) {
        if let Some(path) = field.strip_prefix(b"worktree ") {
            roots.push(PathBuf::from(os_string_from_bytes(path)));
        }
    }
    // Git reports a submodule's internal `.git/modules/...` storage directory
    // as its primary worktree. `core.worktree` is the canonical mapping back
    // to the checkout visible to the operator.
    if roots
        .first()
        .is_some_and(|first| same_path(first, common_dir))
        && let Ok(configured) = git_path(root, ["config", "--path", "--get", "core.worktree"])
    {
        let checkout = absolutize_from(common_dir, &configured);
        if let Some(first) = roots.first_mut() {
            *first = checkout;
        }
    }
    Ok(roots)
}

fn git_text<I, S>(root: &Path, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_raw(root, args)?;
    Ok(String::from_utf8_lossy(strip_line_ending(&output.stdout)).into_owned())
}

fn git_path<I, S>(root: &Path, args: I) -> Result<PathBuf, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_raw(root, args)?;
    Ok(PathBuf::from(os_string_from_bytes(strip_line_ending(
        &output.stdout,
    ))))
}

fn git_raw<I, S>(root: &Path, args: I) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_command(root, args)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(output)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("git exited with {}", output.status)
        } else {
            stderr
        })
    }
}

fn git_command<I, S>(root: &Path, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new("git");
    // An audit must address only `root`, even when launched from a Git hook or
    // another repository. Clear every inherited Git control variable rather
    // than maintaining a porous denylist as Git adds new variables.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-c")
        .arg("core.untrackedCache=false")
        .arg("-C")
        .arg(root);
    command.args(args);
    command
}

fn git_failure(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("git exited with {}", output.status)
    } else {
        stderr
    }
}

fn strip_line_ending(bytes: &[u8]) -> &[u8] {
    match bytes.strip_suffix(b"\n") {
        Some(bytes) => bytes.strip_suffix(b"\r").unwrap_or(bytes),
        None => bytes,
    }
}

#[cfg(unix)]
fn os_str_bytes(value: &OsStr) -> Vec<u8> {
    value.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn os_str_bytes(value: &OsStr) -> Vec<u8> {
    value.to_string_lossy().as_bytes().to_vec()
}

#[cfg(unix)]
fn os_string_from_bytes(bytes: &[u8]) -> OsString {
    OsString::from_vec(bytes.to_vec())
}

#[cfg(not(unix))]
fn os_string_from_bytes(bytes: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(bytes).into_owned())
}

fn absolute_display(path: &Path) -> String {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    display_path(&canonical_or(&path))
}

#[cfg(unix)]
fn display_path(path: &Path) -> String {
    if let Some(path) = path.to_str() {
        return path.to_string();
    }
    let mut rendered = String::new();
    for byte in path.as_os_str().as_bytes() {
        match byte {
            b'\\' => rendered.push_str("\\\\"),
            0x20..=0x7e => rendered.push(char::from(*byte)),
            _ => rendered.push_str(&format!("\\x{byte:02X}")),
        }
    }
    rendered
}

#[cfg(not(unix))]
fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn absolutize_from(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        canonical_or(path)
    } else {
        canonical_or(&root.join(path))
    }
}

fn canonical_or(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn same_path(left: &Path, right: &Path) -> bool {
    canonical_or(left) == canonical_or(right)
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A receipt is a claim, and these are the four ways it can be wrong or
    /// right. The two that matter: a lane whose
    /// owner had been dead for hours and had produced nothing, and one whose
    /// owner was still working.
    fn lane_with(state: &str, pid: u32) -> tempdir::Dir {
        let dir = tempdir::Dir::new();
        std::fs::write(
            dir.path().join(".WORKTREE-OWNER.md"),
            format!(
                "| Field | Value |\n|---|---|\n| State | `{state}` |\n\
                 | Session | `abc-123` |\n| Supervisor PID | `{pid}` |\n"
            ),
        )
        .expect("receipt");
        dir
    }

    fn dead_pid() -> u32 {
        // Reserved by convention and never a live process id on any unix.
        0x7FFF_FFFE
    }

    #[test]
    fn an_active_receipt_with_a_dead_owner_and_nothing_inside_is_abandoned() {
        let lane = lane_with("active", dead_pid());
        let verdict = lane_verdict(lane.path(), false, 0);
        assert_eq!(verdict.state, "abandoned");
        assert!(!verdict.supervisor_alive);
        assert_eq!(verdict.session.as_deref(), Some("abc-123"));
        assert!(
            verdict.advice.is_some(),
            "an abandoned lane must say what to do"
        );
    }

    #[test]
    fn an_active_receipt_with_a_dead_owner_and_work_inside_is_stranded() {
        let lane = lane_with("active", dead_pid());
        for (dirty, ahead) in [(true, 0), (false, 3), (true, 3)] {
            let verdict = lane_verdict(lane.path(), dirty, ahead);
            assert_eq!(verdict.state, "stranded", "dirty={dirty} ahead={ahead}");
            // Never suggest removing a lane that still holds work.
            let advice = verdict.advice.unwrap_or_default();
            assert!(advice.contains("Rescue"), "{advice}");
        }
    }

    #[test]
    fn a_live_owner_is_simply_working() {
        let lane = lane_with("active", std::process::id());
        let verdict = lane_verdict(lane.path(), true, 2);
        assert_eq!(verdict.state, "active");
        assert!(verdict.supervisor_alive);
        assert!(verdict.advice.is_none(), "a working lane needs no advice");
    }

    #[test]
    fn a_released_lane_is_closed_but_still_flags_work_left_behind() {
        let lane = lane_with("released", dead_pid());
        assert_eq!(lane_verdict(lane.path(), false, 0).state, "closed");
        let dirty = lane_verdict(lane.path(), true, 0);
        assert_eq!(dirty.state, "closed");
        assert!(
            dirty.advice.is_some(),
            "a closed lane with uncommitted work must not look finished"
        );
    }

    #[test]
    fn a_worktree_with_no_receipt_is_unmarked_not_active() {
        let dir = tempdir::Dir::new();
        let verdict = lane_verdict(dir.path(), false, 0);
        assert_eq!(verdict.state, "unmarked");
        assert!(verdict.session.is_none());
    }

    /// Throwaway directory, so the crate keeps a `tempfile` dependency out of
    /// its graph for five tests.
    mod tempdir {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};

        static N: AtomicU64 = AtomicU64::new(0);

        pub struct Dir(PathBuf);

        impl Dir {
            pub fn new() -> Self {
                let path = std::env::temp_dir().join(format!(
                    "harbard-audit-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                let _ = std::fs::remove_dir_all(&path);
                std::fs::create_dir_all(&path).expect("temp lane");
                Dir(path)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
