//! "Deliveries": every DONE an agent sent and where its commit is now.
//!
//! The states, in order:
//!
//! 1. `delivered`: the agent says it is done;
//! 2. `en_revision`: a review agent named the commit;
//! 3. `with_fixes`: the review found things (a FINDING, or a DONE that
//!    reports findings);
//! 4. `integrated`: the commit is on `main`;
//! 5. `installed`: `main` at or after it is what serve (and the console,
//!    when it says which commit it runs) was built from.
//!
//! Everything comes from data already on hand: the view's DONE messages and
//! the SHAs they name, review agents' messages that name the same SHA, and
//! what git answers about the commit ([`ShaFacts`]). Nothing here runs git;
//! the caller passes the answers in.

use crate::team::commit_tokens;
use crate::workspace::{Agent, Message, WorkspaceView};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// What git says about one commit in one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaFacts {
    /// The full id.
    pub commit: String,
    /// The main commit carrying the same change, when it reached main
    /// rebased (a new id): what "Installed" looks for.
    pub landed_as: Option<String>,
    /// An ancestor of (or equal to) the tip of `main`.
    pub on_main: bool,
    /// For the repository serve was built from: whether the running build
    /// contains it. `None` elsewhere: "Installed" does not apply.
    pub installed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReviewStep {
    pub agent: String,
    pub message: String,
    pub ts_ms: u64,
    /// `reviewing` (it named the commit), `fixes` (it found things) or
    /// `reviewed` (its DONE found nothing).
    pub kind: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Shipment {
    /// The DONE message.
    pub id: String,
    pub agent: String,
    pub ts_ms: u64,
    /// The DONE's human line.
    pub text: String,
    pub channel: Option<String>,
    /// The commit as the DONE named it.
    pub sha: Option<String>,
    /// The full id, when git knows it in the channel's repository.
    pub commit: Option<String>,
    /// The id it reached main as, when a rebase gave it a new one.
    pub landed_as: Option<String>,
    /// `delivered`, `in_review`, `with_fixes`, `integrated` or `installed`.
    pub state: &'static str,
    /// The furthest state this delivery can reach: `installed` for the
    /// repository serve runs from, `integrated` for any other.
    pub last_state: &'static str,
    /// Not yet at `last_state`: what the sidebar's count counts.
    pub pending: bool,
    /// Why it cannot be followed: `sin_commit` (the DONE names none),
    /// `no_repository` (no channel), `unknown_commit` (git does not
    /// know it there). `None` when it can.
    pub untracked: Option<&'static str>,
    pub review: Vec<ReviewStep>,
}

/// A review agent: its role, workshop line, name, handle or agent type
/// says so. Never its brief, where a builder may be asked to "revisar".
pub fn is_reviewer(a: &Agent) -> bool {
    let fields = [
        a.role.as_deref(),
        a.role_line.as_deref(),
        Some(a.name.as_str()),
        Some(a.handle.as_str()),
        Some(a.label.as_str()),
    ];
    fields.iter().flatten().any(|f| {
        let f = f.to_lowercase();
        f.contains("review") || f.contains("revis") || f.contains("audit")
    })
}

/// The SHAs a message names, the ones written after `SHA` or `commit`
/// first ("SHA 59ee93d, stacked on 3df5bf6" delivers 59ee93d).
pub fn named_shas(text: &str) -> Vec<String> {
    let all = commit_tokens(text);
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut first: Vec<String> = Vec::new();
    for pair in words.windows(2) {
        let w = pair[0]
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        if matches!(w.as_str(), "sha" | "commit" | "commits") {
            for t in commit_tokens(pair[1]) {
                if all.contains(&t) && !first.contains(&t) {
                    first.push(t);
                }
            }
        }
    }
    let rest: Vec<String> = all.into_iter().filter(|t| !first.contains(t)).collect();
    first.extend(rest);
    first
}

/// Two abbreviations of one commit: one is a prefix of the other.
fn same_commit(a: &str, b: &str) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Whether a reviewer's DONE reports something to fix. "No findings" and
/// the like say it does not, whatever else the line holds.
pub fn reports_findings(text: &str) -> bool {
    let t = text.to_lowercase();
    // Spanish too: reviewers write their verdicts in the operator's language.
    const CLEAN: [&str; 9] = [
        "sin hallazgos",
        "sin arreglos",
        "0 hallazgos",
        "ningún hallazgo",
        "ningun hallazgo",
        "no findings",
        "0 findings",
        "nada que arreglar",
        "sin bloqueantes",
    ];
    if CLEAN.iter().any(|c| t.contains(c)) {
        return false;
    }
    t.split(|c: char| !c.is_alphanumeric()).any(|w| {
        w.starts_with("finding")
            || w.starts_with("finding")
            || w.starts_with("arregl")
            || w.starts_with("bloqueante")
            || w == "bug"
            || w == "bugs"
            || w == "p0"
            || w == "p1"
    })
}

fn full_text(m: &Message) -> String {
    match m.body.as_deref() {
        Some(b) => format!("{}\n{b}", m.text),
        None => m.text.clone(),
    }
}

/// The SHAs git is asked about, by repository: those the view's DONEs
/// name, so the caller can ask git once per repository.
pub fn wanted(view: &WorkspaceView) -> BTreeMap<String, Vec<String>> {
    let repo_of: HashMap<&str, &str> = view
        .channels
        .iter()
        .map(|c| (c.id.as_str(), c.repo.as_str()))
        .collect();
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for m in view
        .messages
        .iter()
        .filter(|m| m.from == "agent" && m.label == Some("DONE") && m.structured.is_none())
    {
        let Some(repo) = m.channels.first().and_then(|c| repo_of.get(c.as_str())) else {
            continue;
        };
        let shas = out.entry((*repo).to_string()).or_default();
        for s in named_shas(&full_text(m)) {
            if !shas.contains(&s) {
                shas.push(s);
            }
        }
    }
    out
}

/// Every DONE of the view and where it stands, newest first. `facts`
/// answers for `(repository, sha)`; `None` when git does not know the
/// commit there.
pub fn shipments(
    view: &WorkspaceView,
    facts: &dyn Fn(&str, &str) -> Option<ShaFacts>,
) -> Vec<Shipment> {
    let reviewers: HashSet<&str> = view
        .agents
        .iter()
        .filter(|a| is_reviewer(a))
        .map(|a| a.key.as_str())
        .collect();
    let repo_of: HashMap<&str, &str> = view
        .channels
        .iter()
        .map(|c| (c.id.as_str(), c.repo.as_str()))
        .collect();
    let said: Vec<(&Message, Vec<String>)> = view
        .messages
        .iter()
        .filter(|m| m.from == "agent" && m.structured.is_none())
        .map(|m| (m, named_shas(&full_text(m))))
        .collect();
    let done = |m: &Message| m.label == Some("DONE");
    // Builders' DONEs first: a reviewer's DONE naming one of their commits
    // is a review of it, not a delivery of its own.
    let built: Vec<(&str, &str)> = said
        .iter()
        .filter(|(m, _)| done(m) && !reviewers.contains(m.agent.as_str()))
        .flat_map(|(m, shas)| shas.iter().map(|s| (m.agent.as_str(), s.as_str())))
        .collect();
    let reviews_other = |m: &Message, shas: &[String]| {
        shas.iter().any(|s| {
            built
                .iter()
                .any(|(who, b)| *who != m.agent && same_commit(b, s))
        })
    };

    let mut out: Vec<Shipment> = Vec::new();
    for (m, shas) in &said {
        if !done(m) || (reviewers.contains(m.agent.as_str()) && reviews_other(m, shas)) {
            continue;
        }
        let channel = m.channels.first().cloned();
        let repo = channel.as_deref().and_then(|c| repo_of.get(c).copied());
        // The first SHA git knows in this repository; else the first named.
        let known = repo.and_then(|r| shas.iter().find_map(|s| facts(r, s).map(|f| (s, f))));
        let sha = known
            .as_ref()
            .map(|(s, _)| (*s).clone())
            .or_else(|| shas.first().cloned());
        let review: Vec<ReviewStep> = match sha.as_deref() {
            None => Vec::new(),
            Some(s) => {
                let mut steps: Vec<ReviewStep> = said
                    .iter()
                    .filter(|(r, rs)| {
                        r.agent != m.agent
                            && reviewers.contains(r.agent.as_str())
                            && rs.iter().any(|x| same_commit(x, s))
                    })
                    .map(|(r, _)| ReviewStep {
                        agent: r.agent.clone(),
                        message: r.id.clone(),
                        ts_ms: r.ts_ms,
                        kind: match r.label {
                            Some("FINDING") => "fixes",
                            Some("DONE") if reports_findings(&full_text(r)) => "fixes",
                            Some("DONE") => "reviewed",
                            _ => "reviewing",
                        },
                    })
                    .collect();
                steps.sort_by_key(|s| s.ts_ms);
                steps
            }
        };
        let facts = known.map(|(_, f)| f);
        let last_state = match &facts {
            Some(f) if f.installed.is_some() => "installed",
            _ => "integrated",
        };
        let state = match (&facts, review.last()) {
            (Some(f), _) if f.on_main && f.installed == Some(true) => "installed",
            (Some(f), _) if f.on_main => "integrated",
            (_, Some(r)) if r.kind == "fixes" => "with_fixes",
            (_, Some(_)) => "in_review",
            _ => "delivered",
        };
        let untracked = if sha.is_none() {
            Some("no_commit")
        } else if repo.is_none() {
            Some("no_repository")
        } else if facts.is_none() {
            Some("unknown_commit")
        } else {
            None
        };
        out.push(Shipment {
            id: m.id.clone(),
            agent: m.agent.clone(),
            ts_ms: m.ts_ms,
            text: m.text.clone(),
            channel,
            sha,
            commit: facts.as_ref().map(|f| f.commit.clone()),
            landed_as: facts.as_ref().and_then(|f| f.landed_as.clone()),
            state,
            last_state,
            pending: untracked.is_none() && state != last_state,
            untracked,
            review,
        });
    }
    // The same commit delivered again: the newest DONE stands for it.
    out.sort_by_key(|s| std::cmp::Reverse(s.ts_ms));
    let mut seen: Vec<String> = Vec::new();
    out.retain(|s| match s.commit.as_ref().or(s.sha.as_ref()) {
        None => true,
        Some(c) if seen.iter().any(|x| same_commit(x, c)) => false,
        Some(c) => {
            seen.push(c.clone());
            true
        }
    });
    out
}
