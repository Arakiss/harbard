//! What changed, in words: versions, the changelog and the commit list.
//!
//! Pure functions over text git already printed, so every rule here is
//! tested without a repository.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A `major.minor.patch[-pre]` version. Build metadata is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    pre: Vec<String>,
}

impl Version {
    /// `0.3.0-alpha.9`, with or without a leading `v`.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim().trim_start_matches('v');
        let raw = raw.split('+').next()?;
        let (core, pre) = match raw.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (raw, None),
        };
        let mut parts = core.split('.');
        let mut out = [0u64; 3];
        for slot in &mut out {
            *slot = parts.next()?.parse().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        let pre = match pre {
            Some("") => return None,
            Some(p) => p.split('.').map(str::to_string).collect(),
            None => Vec::new(),
        };
        if pre.iter().any(String::is_empty) {
            return None;
        }
        Some(Self { core: out, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            // A release outranks its pre-releases (semver §11).
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let ord = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(x), Ok(y)) => x.cmp(&y),
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if ord != Ordering::Equal {
                            return ord;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The newest `v*` tag strictly greater than `installed`, with its version.
pub fn newest_tag<'a>(tags: impl IntoIterator<Item = &'a str>, installed: &str) -> Option<String> {
    let floor = Version::parse(installed);
    tags.into_iter()
        .filter(|t| t.starts_with('v'))
        .filter_map(|t| Version::parse(t).map(|v| (v, t)))
        .filter(|(v, _)| floor.as_ref().is_none_or(|f| v > f))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, t)| t.to_string())
}

/// One commit, as the update dialog lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub commit: String,
    pub kind: String,
    #[serde(default)]
    pub scope: Option<String>,
    pub subject: String,
    pub date_ms: u64,
}

const KINDS: &[&str] = &[
    "feat", "fix", "perf", "refactor", "docs", "test", "build", "chore",
];

/// `feat(console): a rail tile` → kind `feat`, scope `console`, subject
/// `a rail tile`. Anything that is not a conventional subject is `other`
/// and keeps its whole line. `ci`/`style`/`revert` fold into `chore`.
pub fn classify(subject: &str) -> (String, Option<String>, String) {
    let other = || ("other".to_string(), None, subject.trim().to_string());
    let Some((head, rest)) = subject.split_once(':') else {
        return other();
    };
    let head = head.trim().trim_end_matches('!');
    let (kind, scope) = match head.split_once('(') {
        Some((kind, scope)) => match scope.strip_suffix(')') {
            Some(scope) if !scope.is_empty() && !scope.contains(['(', ')', ' ']) => {
                (kind, Some(scope.to_string()))
            }
            _ => return other(),
        },
        None => (head, None),
    };
    let kind = match kind {
        "ci" | "style" | "revert" => "chore",
        k if KINDS.contains(&k) => k,
        _ => return other(),
    };
    let rest = rest.trim();
    if rest.is_empty() {
        return other();
    }
    (kind.to_string(), scope, rest.to_string())
}

/// The record separator `git log` is asked for: `%H%x1f%s%x1f%ct%x1e`.
pub const LOG_FORMAT: &str = "%H%x1f%s%x1f%ct%x1e";

/// Parse `git log --format=LOG_FORMAT` output.
pub fn parse_log(out: &str) -> Vec<Change> {
    out.split('\x1e')
        .filter_map(|record| {
            let mut fields = record.trim_matches(['\n', '\r']).split('\x1f');
            let commit = fields.next()?.trim();
            let subject = fields.next()?;
            let secs: u64 = fields.next()?.trim().parse().ok()?;
            if commit.len() < 7 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let (kind, scope, subject) = classify(subject);
            Some(Change {
                commit: commit.to_string(),
                kind,
                scope,
                subject,
                date_ms: secs.saturating_mul(1000),
            })
        })
        .collect()
}

/// A changelog cut into what a reader compares: headings and items.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    /// `## 0.3.0-alpha.9 — 2026-10-20`, `## Unreleased`.
    Section(String),
    /// `### An idle desk`.
    Heading(String),
    /// A bullet with its continuation lines, or a paragraph.
    Item(String),
}

fn blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut item: Vec<&str> = Vec::new();
    let flush = |item: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !item.is_empty() {
            out.push(Block::Item(item.join("\n")));
            item.clear();
        }
    };
    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix("## ") {
            flush(&mut item, &mut out);
            out.push(Block::Section(rest.trim().to_string()));
        } else if trimmed.starts_with("### ") || trimmed.starts_with("#### ") {
            flush(&mut item, &mut out);
            out.push(Block::Heading(trimmed.to_string()));
        } else if trimmed.starts_with("# ") || trimmed.is_empty() {
            flush(&mut item, &mut out);
        } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            flush(&mut item, &mut out);
            item.push(trimmed);
        } else {
            item.push(trimmed);
        }
    }
    flush(&mut item, &mut out);
    out
}

/// The version a `## …` section names, if any (`Unreleased` names none).
fn section_version(title: &str) -> Option<Version> {
    Version::parse(title.split_whitespace().next()?)
}

/// What the changelog at the target says that the changelog at the
/// installed commit did not: every item of the Unreleased section and of
/// each version newer than `installed_version`, minus the items the
/// installed changelog already carried (a release moves Unreleased items
/// under its own heading word for word, so they stay out). Headings are
/// kept only above kept items; `## Unreleased` itself is dropped. `None`
/// when nothing is new.
pub fn new_notes(target: &str, installed: Option<&str>, installed_version: &str) -> Option<String> {
    let seen: BTreeSet<String> = installed
        .map(blocks)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|b| match b {
            Block::Item(text) => Some(normalize(&text)),
            _ => None,
        })
        .collect();
    let floor = Version::parse(installed_version);
    let mut out: Vec<String> = Vec::new();
    let mut section: Option<String> = None;
    let mut heading: Option<String> = None;
    let mut in_scope = false;
    let mut started = false;
    for block in blocks(target) {
        match block {
            Block::Section(title) => {
                started = true;
                let version = section_version(&title);
                in_scope = match (&version, &floor) {
                    (None, _) => title.eq_ignore_ascii_case("unreleased"),
                    (Some(v), Some(f)) => v > f,
                    (Some(_), None) => true,
                };
                // Sections run newest first: the first one at or below the
                // installed version ends what is new.
                if !in_scope && version.is_some() {
                    break;
                }
                section = version.map(|_| format!("## {title}"));
                heading = None;
            }
            Block::Heading(text) => heading = Some(text),
            Block::Item(text) => {
                if !started || !in_scope || seen.contains(&normalize(&text)) {
                    continue;
                }
                if let Some(s) = section.take() {
                    out.push(s);
                }
                if let Some(h) = heading.take() {
                    out.push(h);
                }
                out.push(text);
            }
        }
    }
    (!out.is_empty()).then(|| out.join("\n\n"))
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn versions_order_like_semver() {
        assert!(
            v("0.3.0-alpha.10") > v("0.3.0-alpha.9"),
            "numeric, not lexical"
        );
        assert!(
            v("0.3.0") > v("0.3.0-alpha.9"),
            "a release outranks its pre-releases"
        );
        assert!(v("0.3.1-alpha.1") > v("0.3.0"));
        assert!(v("0.3.0-beta") > v("0.3.0-alpha.9"));
        assert!(v("0.3.0-alpha.1.1") > v("0.3.0-alpha.1"));
        assert_eq!(v("v1.2.3+build.5"), v("1.2.3"));
        for bad in ["", "1.2", "1.2.3.4", "1.2.x", "1.2.3-", "1.2.3-a..b"] {
            assert!(Version::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn the_newest_tag_must_beat_the_installed_version() {
        let tags = [
            "v0.3.0-alpha.8",
            "v0.3.0-alpha.10",
            "v0.3.0-alpha.9",
            "nightly",
            "vnext",
        ];
        assert_eq!(
            newest_tag(tags, "0.3.0-alpha.9").as_deref(),
            Some("v0.3.0-alpha.10")
        );
        assert_eq!(newest_tag(tags, "0.3.0-alpha.10"), None);
        assert_eq!(newest_tag(tags, "0.3.0"), None);
    }

    #[test]
    fn conventional_subjects_are_classified() {
        assert_eq!(
            classify("feat(console): a rail tile per workspace"),
            (
                "feat".into(),
                Some("console".into()),
                "a rail tile per workspace".into()
            )
        );
        assert_eq!(
            classify("fix: a typo"),
            ("fix".into(), None, "a typo".into())
        );
        assert_eq!(classify("feat(core)!: break it").0, "feat");
        assert_eq!(classify("ci: speed").0, "chore");
        assert_eq!(
            classify("Merge branch 'x'"),
            ("other".into(), None, "Merge branch 'x'".into())
        );
        assert_eq!(classify("wip(a b): x").0, "other");
        assert_eq!(classify("feat(): x").0, "other");
        assert_eq!(classify("feat:   ").0, "other");
    }

    #[test]
    fn git_log_records_parse_and_junk_is_skipped() {
        let out = "a955a72aa\x1ffeat(hub): an invite\x1f1762600000\x1e\n\
                   5ddb5bbcc\x1fbuild(desktop): wait: longer\x1f1762500000\x1e\n\
                   nothex\x1fx\x1f1\x1e\n";
        let changes = parse_log(out);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].kind, "feat");
        assert_eq!(changes[0].date_ms, 1_762_600_000_000);
        assert_eq!(
            changes[1].subject, "wait: longer",
            "only the first colon splits"
        );
    }

    const OLD: &str = "# Changelog\n\nIntro.\n\n## Unreleased\n\n### Quiet hooks\n\n- brief prints nothing.\n- reminders are gone,\n  from every hook.\n\n## 0.3.0-alpha.9 — 2026-10-20\n\n- Strip reads.\n";

    #[test]
    fn notes_are_only_what_the_installed_changelog_did_not_say() {
        let new = "# Changelog\n\nIntro.\n\n## Unreleased\n\n### Workspaces\n\n- one per organisation.\n\n### Quiet hooks\n\n- brief prints nothing.\n- reminders are gone, from every hook.\n- protocol --hook prints one line.\n\n## 0.3.0-alpha.9 — 2026-10-20\n\n- Strip reads.\n";
        let notes = new_notes(new, Some(OLD), "0.3.0-alpha.9").unwrap();
        assert_eq!(
            notes,
            "### Workspaces\n\n- one per organisation.\n\n### Quiet hooks\n\n- protocol --hook prints one line."
        );
    }

    #[test]
    fn a_release_in_between_keeps_its_heading_and_drops_what_was_known() {
        let new = "# Changelog\n\n## Unreleased\n\n- later fix.\n\n## 0.3.0-alpha.10 — 2026-11-09\n\n### Quiet hooks\n\n- brief prints nothing.\n- a new line.\n\n## 0.3.0-alpha.9 — 2026-10-20\n\n- Strip reads.\n";
        let notes = new_notes(new, Some(OLD), "0.3.0-alpha.9").unwrap();
        assert_eq!(
            notes,
            "- later fix.\n\n## 0.3.0-alpha.10 — 2026-11-09\n\n### Quiet hooks\n\n- a new line."
        );
    }

    #[test]
    fn nothing_new_is_none_and_no_installed_changelog_shows_all() {
        assert_eq!(new_notes(OLD, Some(OLD), "0.3.0-alpha.9"), None);
        let all = new_notes(OLD, None, "0.3.0-alpha.9").unwrap();
        assert!(all.starts_with("### Quiet hooks"), "{all}");
        assert!(
            !all.contains("Strip"),
            "sections at or below the installed version stay out"
        );
        assert!(
            !all.contains("Intro"),
            "text above the first section is not a note"
        );
    }
}
