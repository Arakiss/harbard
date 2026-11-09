//! How the console names agents and sessions, and what an `@` reaches.
//!
//! The display name is `<team> · <role>`: "robots · builder",
//! "keychain · reviewer 2", "Claude · lead". It comes from the agent's own
//! name (the Agent tool's `name`, or `harbard name` for a bus session) split
//! on its last `-`, with the role normalised by [`ROLES`]. A name with no `-`
//! keeps its project as the team: "acme · cap136". Something with no name
//! at all is "<project> · session HH:MM", never the first words of a prompt,
//! which read as a sentence cut in half.
//!
//! Also here, because the same names are what an `@` matches: the mention
//! tokens of a text, and which paths are scratch space (a temporary session
//! never asks the operator for anything).

use crate::coded::{Params, params};

/// Role words agents are named with, and the one word each is shown as.
pub const ROLES: [(&str, &str); 22] = [
    ("builder", "builder"),
    ("build", "builder"),
    ("review", "reviewer"),
    ("reviewer", "reviewer"),
    ("integ", "integration"),
    ("integration", "integration"),
    ("lead", "lead"),
    ("research", "research"),
    ("researcher", "research"),
    ("study", "study"),
    ("test", "tests"),
    ("tests", "tests"),
    ("fix", "fix"),
    ("docs", "docs"),
    ("design", "design"),
    ("audit", "audit"),
    ("ops", "ops"),
    ("eval", "eval"),
    ("port", "migration"),
    ("merge", "integration"),
    ("upgrade", "upgrade"),
    ("holdout", "holdout"),
];

/// The role of a session with no team of its own.
pub const SESSION_ROLE: &str = "session";

/// Spanish role words, kept so names agents still type and names already in
/// the registry keep resolving to the same role.
const SPANISH_ROLES: [(&str, &str); 17] = [
    ("constructor", "builder"),
    ("revisor", "reviewer"),
    ("integración", "integration"),
    ("coordinador", "lead"),
    ("investigación", "research"),
    ("estudio", "study"),
    ("pruebas", "tests"),
    ("arreglo", "fix"),
    ("documentación", "docs"),
    ("diseño", "design"),
    ("auditoría", "audit"),
    ("operaciones", "ops"),
    ("evaluación", "eval"),
    ("migración", "migration"),
    ("actualización", "upgrade"),
    ("control", "holdout"),
    ("sesión", SESSION_ROLE),
];

/// The canonical role a word names, whichever word it was said with.
fn canonical_role(word: &str) -> Option<&'static str> {
    let lower = word.to_lowercase();
    ROLES
        .iter()
        .find(|(alias, shown)| *alias == lower || *shown == lower)
        .map(|(_, shown)| *shown)
        .or_else(|| {
            SPANISH_ROLES
                .iter()
                .find(|(alias, shown)| *alias == lower || *shown == lower)
                .map(|(_, shown)| *shown)
        })
}

/// The role word as it is shown: the canonical word when known, else as
/// written.
pub fn role_word(raw: &str) -> String {
    canonical_role(raw).map_or_else(|| raw.to_string(), str::to_string)
}

/// A known role and its trailing number: `reviewer 2` is (`reviewer`, `2`).
/// `None` for a role harbard did not generate, which is shown as written.
pub fn known_role(role: &str) -> Option<(&'static str, Option<&str>)> {
    let (word, number) = match role.rsplit_once(' ') {
        Some((w, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => (w, Some(n)),
        _ => (role, None),
    };
    canonical_role(word).map(|canonical| (canonical, number))
}

/// A role line's words as shown: a known role by its canonical word.
pub fn role_text(role: &str) -> String {
    match known_role(role) {
        Some((word, Some(number))) => format!("{word} {number}"),
        Some((word, None)) => word.to_string(),
        None => role.to_string(),
    }
}

/// `robots-builder` → ("robots", "builder"); `keychain-review-2` →
/// ("keychain", "reviewer 2"). `None` for a name with no `-` to split on.
pub fn team_role(name: &str) -> Option<(String, String)> {
    let name = name.trim();
    let (stem, number) = match name.rsplit_once('-') {
        Some((stem, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
            (stem, Some(n))
        }
        _ => (name, None),
    };
    let (team, role) = stem.rsplit_once('-')?;
    if team.is_empty() || role.is_empty() {
        return None;
    }
    let mut role = role_word(role);
    if let Some(n) = number {
        role.push(' ');
        role.push_str(n);
    }
    Some((team.to_string(), role))
}

/// A display name and its parts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Display {
    pub name: String,
    pub team: Option<String>,
    pub role: Option<String>,
    /// The console's code for a harbard-worded `name`, filled from `params`.
    /// `None` when the name is text an agent or the operator chose.
    pub code: Option<&'static str>,
    pub params: Params,
}

/// The params of a known role: the console holds its word in each language.
fn role_params(team: &str, role: &str) -> Option<Params> {
    let (canonical, number) = known_role(role)?;
    let mut values = params([("team", team.into()), ("role", canonical.into())]);
    if let Some(n) = number {
        values.insert("number".into(), n.into());
    }
    Some(values)
}

/// The display name of an agent or a session: from its own name, else
/// "<project> · session HH:MM" (the machine's own time of its first record).
pub fn display(name: Option<&str>, project: &str, first_ms: Option<u64>) -> Display {
    match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => match team_role(n) {
            Some((team, role)) => {
                let values = role_params(&team, &role);
                Display {
                    name: format!("{team} · {role}"),
                    code: values.is_some().then_some("name.team_role"),
                    params: values.unwrap_or_default(),
                    team: Some(team),
                    role: Some(role),
                }
            }
            // A workshop name (a lead launched "Owen") is a name, not a
            // role: it stands alone, and the project is only its team.
            None if crate::workshop::listed(n).is_some() => Display {
                name: n.to_string(),
                team: (!project.is_empty()).then(|| project.to_string()),
                role: None,
                ..Display::default()
            },
            // No project known: the name as it is.
            None if project.is_empty() => Display {
                name: n.to_string(),
                team: None,
                role: None,
                ..Display::default()
            },
            None => Display {
                name: format!("{project} · {n}"),
                team: Some(project.to_string()),
                role: Some(n.to_string()),
                ..Display::default()
            },
        },
        None => {
            let time = first_ms.map(hhmm_local);
            let (name, code, values) = match (project.is_empty(), time) {
                (true, Some(t)) => (
                    format!("{SESSION_ROLE} {t}"),
                    "name.session.time",
                    params([("time", t.into())]),
                ),
                (true, None) => (SESSION_ROLE.to_string(), "name.session", params([])),
                (false, Some(t)) => (
                    format!("{project} · {SESSION_ROLE} {t}"),
                    "name.session.project_time",
                    params([("project", project.into()), ("time", t.into())]),
                ),
                (false, None) => (
                    format!("{project} · {SESSION_ROLE}"),
                    "name.session.project",
                    params([("project", project.into())]),
                ),
            };
            Display {
                name,
                team: None,
                role: None,
                code: Some(code),
                params: values,
            }
        }
    }
}

/// The project a working directory names: its last component, a worktree
/// folded into its repository (`harbard-wt-x` → `harbard`).
pub fn project_of(cwd: Option<&str>) -> String {
    let Some(cwd) = cwd
        .map(|c| c.trim_end_matches('/'))
        .filter(|c| !c.is_empty())
    else {
        return String::new();
    };
    let repo = crate::workspace::by_convention(
        &crate::workspace::worktree_component(cwd).unwrap_or_else(|| cwd.to_string()),
    );
    let last = repo.rsplit('/').next().unwrap_or(&repo);
    // `~/.claude`, `~/.cache`: tool state, never a project.
    if last.starts_with('.') {
        return String::new();
    }
    last.to_string()
}

/// "17:26" on the operator's own clock, the machine's zone at that instant.
pub fn hhmm_local(ms: u64) -> String {
    hhmm_at_offset(ms, local_utc_offset_secs(ms))
}

/// "17:26" `offset_secs` east of UTC.
fn hhmm_at_offset(ms: u64, offset_secs: i64) -> String {
    let secs = (ms / 1000) as i64;
    let local = time::OffsetDateTime::from_unix_timestamp(secs + offset_secs)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    format!("{:02}:{:02}", local.hour(), local.minute())
}

/// `time::UtcOffset::local_offset_at` refuses in a process with more than one
/// thread, and serve has many, so the C library answers instead.
#[cfg(unix)]
fn local_utc_offset_secs(ms: u64) -> i64 {
    let secs = (ms / 1000) as libc::time_t;
    // SAFETY: `tm` is a valid out-parameter for `localtime_r`, which is
    // thread-safe; it is only read after the call reports success.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            0
        } else {
            tm.tm_gmtoff as i64
        }
    }
}

#[cfg(not(unix))]
fn local_utc_offset_secs(_ms: u64) -> i64 {
    0
}

/// The `@` tokens of a text, lower-cased, in order, without duplicates:
/// `@robots`, `@harbard-view`, `@canal`, `@operator`. An `@` inside a word (an
/// email address) is not a mention.
pub fn mention_tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '.')) {
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].is_alphanumeric() || matches!(chars[j], '-' | '_' | '.'))
            {
                j += 1;
            }
            let token: String = chars[i + 1..j].iter().collect::<String>();
            let token = token.trim_end_matches(['.', '-', '_']).to_lowercase();
            if !token.is_empty() && !out.contains(&token) {
                out.push(token);
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Reaches the operator whatever their configured name is, so an agent that
/// does not know it can still ask (`@operator`).
pub const OPERATOR_MENTION: &str = "operator";

/// The words `@canal` is spelled with: Spanish ones stay, people type them.
pub const CHANNEL_MENTIONS: [&str; 3] = ["canal", "channel", "aqui"];

/// System temp space: a session there is a throwaway (an eval workspace, a
/// scratch clone), never a project.
pub const TEMP_DIRS: [&str; 5] = [
    "/tmp",
    "/private/tmp",
    "/private/var/folders",
    "/var/folders",
    "/private/var/tmp",
];

/// Under a system temp dir, or under `extra` (`$TMPDIR`).
pub fn is_temp(path: &str, extra: &[String]) -> bool {
    let under = |root: &str| {
        let root = root.trim_end_matches('/');
        !root.is_empty() && (path == root || path.starts_with(&format!("{root}/")))
    };
    TEMP_DIRS.iter().any(|r| under(r)) || extra.iter().any(|r| under(r))
}

/// A `~/.claude/projects` directory name encodes its path with `-` for `/`.
pub fn is_temp_project(dir: &str, extra: &[String]) -> bool {
    let encode = |p: &str| p.trim_end_matches('/').replace(['/', '.', '_'], "-");
    TEMP_DIRS
        .iter()
        .map(|r| encode(r))
        .chain(extra.iter().map(|r| encode(r)))
        .any(|enc| !enc.is_empty() && (dir == enc || dir.starts_with(&format!("{enc}-"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dot-folder (`~/.claude`, `~/.cache`) is never a project.
    #[test]
    fn a_dot_folder_is_never_a_project() {
        assert_eq!(project_of(Some("/Users/p/.claude")), "");
        assert_eq!(project_of(Some("/Users/p/.claude/")), "");
        assert_eq!(project_of(Some("/work/harbard-wt-cli")), "harbard");
        let d = display(Some("Owen"), &project_of(Some("/Users/p/.claude")), None);
        assert_eq!(d.name, "Owen");
        assert_eq!(d.team, None);
        // A workshop name is a name, not a role: "Owen", "constructor ·
        // harbard" never "Owen · harbard".
        let d = display(Some("Owen"), "harbard", None);
        assert_eq!(d.name, "Owen");
        assert_eq!(d.team.as_deref(), Some("harbard"));
        assert_eq!(d.role, None);
        // Any other bare name keeps the project as its team, as before.
        assert_eq!(display(Some("cap136"), "acme", None).name, "acme · cap136");
    }

    #[test]
    fn names_are_team_and_role_with_the_role_normalised() {
        let d = |n: &str| display(Some(n), "harbard", None).name;
        assert_eq!(d("robots-builder"), "robots · builder");
        assert_eq!(d("robots-review"), "robots · reviewer");
        assert_eq!(d("robots-integ"), "robots · integration");
        assert_eq!(d("keychain-review-2"), "keychain · reviewer 2");
        assert_eq!(d("harbard-dock"), "harbard · dock");
        assert_eq!(d("harbard-strip"), "harbard · strip");
        // A name with nothing to split keeps its project as the team.
        assert_eq!(d("cap136"), "harbard · cap136");
        assert_eq!(display(Some("cap136"), "", None).name, "cap136");
        let full = display(Some("robots-builder"), "harbard", None);
        assert_eq!(
            (full.team.as_deref(), full.role.as_deref()),
            (Some("robots"), Some("builder"))
        );
    }

    #[test]
    fn an_unnamed_session_is_its_project_and_start_never_a_prompt() {
        // 2026-11-07T16:26:00Z, an hour east and five hours west of Greenwich.
        let ms = 1_794_068_760_000;
        assert_eq!(hhmm_at_offset(ms, 3600), "17:26");
        assert_eq!(hhmm_at_offset(ms, -5 * 3600), "11:26");
        assert!(local_utc_offset_secs(ms).abs() <= 14 * 3600);
        // The session's name carries the machine's own clock, whatever it is.
        let shown = hhmm_local(ms);
        assert_eq!(
            display(None, "harbard", Some(ms)).name,
            format!("harbard · session {shown}")
        );
        assert_eq!(
            display(Some("  "), "contoso", Some(ms)).name,
            format!("contoso · session {shown}")
        );
        assert_eq!(project_of(Some("/w/harbard-wt-protocolo/")), "harbard");
        assert_eq!(project_of(Some("/w/contoso")), "contoso");
        assert_eq!(project_of(None), "");
    }

    #[test]
    fn mentions_are_at_tokens_not_email_addresses() {
        assert_eq!(
            mention_tokens(
                "@robots y @Harbard-View, mira esto; avisa a @canal. Correo: a@b.es @dana."
            ),
            vec!["robots", "harbard-view", "canal", "dana"]
        );
        assert!(mention_tokens("sin menciones, p@ra nada").is_empty());
    }

    #[test]
    fn temp_space_is_recognised_by_path_and_by_project_dir() {
        let tmp = vec!["/private/var/folders/xy/abc/T".to_string()];
        assert!(is_temp(
            "/private/var/folders/xy/abc/T/sandbox-l1-3-ws",
            &[]
        ));
        assert!(is_temp("/tmp/x", &[]));
        assert!(!is_temp("/Users/p/Projects/harbard", &tmp));
        assert!(is_temp_project(
            "-private-var-folders-xy-abc-T-sandbox-l1-3-ws",
            &[]
        ));
        assert!(is_temp_project("-tmp-scratch", &[]));
        assert!(!is_temp_project("-Users-p-Projects-harbard", &tmp));
    }
}
