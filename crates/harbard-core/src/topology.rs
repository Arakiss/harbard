//! Execution topology for writer domains (M5a).
//!
//! Contested checkout is not "two live sessions on a path". It is "two or more
//! distinct live *writer domains* share a worktree". Parent + herd child are
//! one domain. Inferred Cursor Task ghosts (`harness:other` + host-pid session)
//! do not count as writers unless they hold a writer lease (caller may pass
//! holder session ids).

use crate::event::{HARNESS_HUMAN, Role};
use crate::query::{Liveness, SessionView};
use std::collections::{BTreeSet, HashMap, HashSet};

/// How sure we are about a session's identity on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentityConfidence {
    /// Hook-stable id (Claude/Codex/Cursor conversation) or SubagentStart link.
    Observed,
    /// Reconstructed (ancestry, Codex DB heal) — usable, not authoritative.
    Inferred,
    /// Fallback `{host}-{pid}`; must not invent rivals.
    #[default]
    Unknown,
}

impl IdentityConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Inferred => "inferred",
            Self::Unknown => "unknown",
        }
    }
}

/// Session id minted when no stable harness id exists: `{host}-{pid}`.
pub fn looks_like_host_pid_session(session: &str, host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return false;
    }
    let prefix = format!("{host}-");
    session
        .strip_prefix(&prefix)
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

/// Infer identity confidence from wire fields (no new event kinds required).
pub fn infer_identity_confidence(
    session: &str,
    host: &str,
    harness: &str,
    parent: Option<&str>,
) -> IdentityConfidence {
    if parent.is_some() {
        return IdentityConfidence::Observed;
    }
    if looks_like_host_pid_session(session, host) {
        return IdentityConfidence::Unknown;
    }
    if harness == "other" {
        return IdentityConfidence::Inferred;
    }
    IdentityConfidence::Observed
}

/// True when this live session may participate in a writer domain for `repo`.
///
/// Excludes humans/operators and unknown host-pid `other` ghosts that are not
/// in `lease_holders`. Herd children are included so they collapse under parent.
pub fn is_writer_domain_member(
    view: &SessionView,
    repo: &str,
    lease_holders: &HashSet<&str>,
) -> bool {
    if view.liveness != Liveness::Live {
        return false;
    }
    if view.repo.as_deref() != Some(repo) {
        return false;
    }
    if view.role == Role::Operator || view.harness == HARNESS_HUMAN {
        return false;
    }
    if lease_holders.contains(view.session.as_str()) {
        return true;
    }
    // Cursor Task / bare shell ghosts: other + unknown host-pid, no parent.
    if view.harness == "other"
        && view.parent.is_none()
        && looks_like_host_pid_session(&view.session, &view.host)
    {
        return false;
    }
    if view.identity == IdentityConfidence::Unknown
        && view.parent.is_none()
        && view.harness == "other"
    {
        return false;
    }
    true
}

/// Walk parent links to the lineage root (one domain key per execution tree).
pub fn writer_domain_root(view: &SessionView, by_id: &HashMap<&str, &SessionView>) -> String {
    let mut cur = view;
    let mut guard = HashSet::new();
    guard.insert(cur.session.as_str());
    while let Some(parent_id) = cur.parent.as_deref() {
        if !guard.insert(parent_id) {
            break;
        }
        match by_id.get(parent_id) {
            Some(parent) => cur = parent,
            None => return parent_id.to_string(),
        }
    }
    cur.session.clone()
}

/// Distinct live writer-domain ids on `repo` (sorted).
pub fn distinct_writer_domains(
    views: &[SessionView],
    repo: &str,
    lease_holders: &HashSet<&str>,
) -> Vec<String> {
    let by_id: HashMap<&str, &SessionView> =
        views.iter().map(|s| (s.session.as_str(), s)).collect();
    let mut domains = BTreeSet::new();
    for view in views {
        if !is_writer_domain_member(view, repo, lease_holders) {
            continue;
        }
        domains.insert(writer_domain_root(view, &by_id));
    }
    domains.into_iter().collect()
}

/// True when ≥2 distinct writer domains share `me_repo`.
pub fn checkout_contested(
    views: &[SessionView],
    me_session: &str,
    me_repo: Option<&str>,
    lease_holders: &HashSet<&str>,
) -> bool {
    let Some(repo) = me_repo else {
        return false;
    };
    let _ = me_session;
    distinct_writer_domains(views, repo, lease_holders).len() >= 2
}

/// True when `other` is a rival writer domain vs `me` on the same repo
/// (used by brief co-presence).
pub fn is_rival_writer(
    me: &SessionView,
    other: &SessionView,
    lease_holders: &HashSet<&str>,
) -> bool {
    let Some(repo) = me.repo.as_deref() else {
        return false;
    };
    if other.repo.as_deref() != Some(repo) {
        return false;
    }
    if !is_writer_domain_member(me, repo, lease_holders)
        || !is_writer_domain_member(other, repo, lease_holders)
    {
        return false;
    }
    let by_id: HashMap<&str, &SessionView> =
        [(me.session.as_str(), me), (other.session.as_str(), other)]
            .into_iter()
            .collect();
    // Prefer a fuller index when available — for the two-session case, also
    // collapse if other.parent == me or me.parent == other.
    if other.parent.as_deref() == Some(me.session.as_str())
        || me.parent.as_deref() == Some(other.session.as_str())
    {
        return false;
    }
    writer_domain_root(me, &by_id) != writer_domain_root(other, &by_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventKind;

    fn view(
        session: &str,
        harness: &str,
        host: &str,
        repo: &str,
        role: Role,
        parent: Option<&str>,
        identity: IdentityConfidence,
    ) -> SessionView {
        SessionView {
            session: session.into(),
            harness: harness.into(),
            host: host.into(),
            cwd: repo.into(),
            repo: Some(repo.into()),
            repo_key: None,
            branch: None,
            role,
            parent: parent.map(str::to_string),
            root: parent.unwrap_or(session).into(),
            depth: u32::from(parent.is_some()),
            descendants: 0,
            lifecycle: crate::query::LifecycleState::Active,
            event_count: 1,
            intent: None,
            liveness: Liveness::Live,
            idle: false,
            last_signal_ms: 1,
            last_ms: 1,
            first_ms: 1,
            last_kind: EventKind::Intent,
            ending: false,
            saw_hello: true,
            saw_bye: false,
            identity,
        }
    }

    #[test]
    fn parent_and_herd_are_one_domain() {
        let parent = view(
            "parent",
            "claude-code",
            "h",
            "/r",
            Role::Main,
            None,
            IdentityConfidence::Observed,
        );
        let child = view(
            "child",
            "claude-code",
            "h",
            "/r",
            Role::Herd,
            Some("parent"),
            IdentityConfidence::Observed,
        );
        let empty = HashSet::new();
        assert!(!checkout_contested(
            &[parent.clone(), child],
            "parent",
            Some("/r"),
            &empty
        ));
        assert_eq!(
            distinct_writer_domains(&[parent], "/r", &empty),
            vec!["parent".to_string()]
        );
    }

    #[test]
    fn two_mains_are_contested() {
        let a = view(
            "a",
            "cursor",
            "h",
            "/r",
            Role::Main,
            None,
            IdentityConfidence::Observed,
        );
        let mut b = a.clone();
        b.session = "b".into();
        let empty = HashSet::new();
        assert!(checkout_contested(&[a, b], "a", Some("/r"), &empty));
    }

    #[test]
    fn host_pid_other_does_not_inflate_contested() {
        let main = view(
            "conv-uuid",
            "cursor",
            "mac",
            "/r",
            Role::Main,
            None,
            IdentityConfidence::Observed,
        );
        let ghost = view(
            "mac-4242",
            "other",
            "mac",
            "/r",
            Role::Main,
            None,
            IdentityConfidence::Unknown,
        );
        let empty = HashSet::new();
        assert!(!checkout_contested(
            &[main.clone(), ghost.clone()],
            "conv-uuid",
            Some("/r"),
            &empty
        ));
        // Lease holder ghost counts — real concurrent writer.
        let holders: HashSet<&str> = ["mac-4242"].into_iter().collect();
        assert!(checkout_contested(
            &[main, ghost],
            "conv-uuid",
            Some("/r"),
            &holders
        ));
    }

    #[test]
    fn looks_like_host_pid() {
        assert!(looks_like_host_pid_session("macbook-123", "macbook"));
        assert!(!looks_like_host_pid_session("macbook-abc", "macbook"));
        assert!(!looks_like_host_pid_session("uuid-here", "macbook"));
    }
}
