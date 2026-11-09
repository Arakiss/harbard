//! The workspace boundary, proved on every delivery path.
//!
//! Two failures matter. A note crossing from one workspace into a session of
//! another is the one that must never happen; a session that stops receiving
//! notes of its own workspace is the other. Every test here checks both
//! directions: a note from the other workspace reaches nobody by any path,
//! and a note from the reader's own workspace (or from the operator) is
//! delivered exactly as it was before workspaces existed — the gated answer
//! equals the open-gate answer, path by path.
//!
//! The repositories are real directories with a `.git/config`, so placement
//! runs through the same file reads the hooks make; the registry is a real
//! `workspaces.json` in a throwaway home.

use super::{Gate, Placement, Workspaces};
use crate::event::{
    Event, EventKind, HARNESS_HUMAN, NoteKind, Reader, Recipient, RecipientKind, Role,
    SCHEMA_VERSION,
};
use crate::membership::{self, AnchorKind, Member};
use crate::store::Store;
use crate::test_util::TempHome;
use std::collections::HashSet;
use std::fs;
use ulid::Ulid;

const NOW: u64 = 1_800_000_000_000;
const T0: u64 = NOW - 60_000;
const HARNESS: &str = "claude-code";
const CHANNEL: &str = "ops";

/// Two workspaces: `acme` (repos owned by Acme) and `personal` (owned by me,
/// and the default). `app` is Acme's; `tool` and `tool2` are mine.
struct World {
    _home: TempHome,
    store: Store,
    app: String,
    app_wt: String,
    tool: String,
    tool2: String,
}

fn fake_repo(root: &std::path::Path, name: &str, url: &str) -> String {
    let dir = root.join(name);
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::write(
        dir.join(".git/config"),
        format!("[remote \"origin\"]\n\turl = {url}\n"),
    )
    .unwrap();
    fs::canonicalize(&dir)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn world_with(registry: Option<Workspaces>) -> World {
    let home = TempHome::new();
    let store = Store::at(home.path().join("hb"));
    fs::create_dir_all(store.root()).unwrap();
    let repos = home.path().join("repos");
    let app = fake_repo(&repos, "app", "git@github.com:Acme/app.git");
    let tool = fake_repo(&repos, "tool", "https://github.com/me/tool.git");
    let tool2 = fake_repo(&repos, "tool2", "git@github.com:me/tool2.git");
    // A linked worktree of `app`, as git lays one out.
    let wt_git = std::path::Path::new(&app).join(".git/worktrees/feature");
    fs::create_dir_all(&wt_git).unwrap();
    fs::write(wt_git.join("commondir"), "../..\n").unwrap();
    let wt = repos.join("app-feature");
    fs::create_dir_all(&wt).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
    let app_wt = fs::canonicalize(&wt)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    if let Some(set) = registry {
        fs::write(
            store.root().join(super::FILE),
            serde_json::to_string_pretty(&set).unwrap(),
        )
        .unwrap();
    }
    let mut m = membership::read(&store);
    for session in ["a2", "b1"] {
        membership::join(
            &mut m,
            CHANNEL,
            Member {
                kind: AnchorKind::Session,
                value: session.into(),
            },
        );
    }
    membership::write(&store, &m).unwrap();
    World {
        _home: home,
        store,
        app,
        app_wt,
        tool,
        tool2,
    }
}

fn two_workspaces() -> Workspaces {
    let mut set = Workspaces {
        v: super::VERSION,
        default: "personal".into(),
        workspaces: Vec::new(),
        repos: Default::default(),
    };
    set.add("personal", "Personal", &["me".into()]).unwrap();
    set.add("acme", "Trabajo", &["Acme".into()]).unwrap();
    set
}

fn world() -> World {
    world_with(Some(two_workspaces()))
}

fn mk(ms: u64, event: EventKind, session: &str, repo: Option<&str>) -> Event {
    Event {
        v: SCHEMA_VERSION,
        id: Ulid::from_parts(ms, u128::from(ms as u16)).to_string(),
        ts: crate::clock::rfc3339_utc(ms),
        event,
        session: session.into(),
        harness: HARNESS.into(),
        host: "h".into(),
        cwd: repo.unwrap_or("/home/me").into(),
        repo: repo.map(str::to_string),
        repo_key: None,
        branch: Some("main".into()),
        role: Role::Main,
        parent: None,
        text: String::new(),
        to: None,
        kind: NoteKind::Fyi,
        note_ref: None,
    }
}

fn operator_note(ms: u64) -> Event {
    let mut e = mk(ms, EventKind::Note, "operator@h", None);
    e.harness = HARNESS_HUMAN.into();
    e.role = Role::Operator;
    e.cwd = String::new();
    e
}

/// Who reads: a session, its checkout, its repository identity.
#[derive(Clone, Copy)]
struct Who<'a> {
    session: &'a str,
    repo: &'a str,
    repo_key: &'a str,
}

/// Every delivery decision the hooks make, for one note and one reader:
/// `brief` (turn boundary), the inline channel (between tool calls), `inbox`,
/// `watch` (whether the note calls for the reader, and the watcher's wake
/// cause), and `stop-check` (whether the stop is held for it).
fn paths(
    events: &[Event],
    note: &Event,
    who: Who<'_>,
    gate: &Gate<'_>,
) -> Vec<(&'static str, bool)> {
    let channels = vec![CHANNEL.to_string()];
    let reader = Reader {
        session: who.session,
        harness: HARNESS,
        repo: Some(who.repo),
        repo_key: Some(who.repo_key),
        channels: &channels,
        workspace: gate,
    };
    let none = HashSet::new();
    let brief = crate::brief::render_with_views(
        &Store::at("/nonexistent-harbard-home"),
        events,
        &[],
        who.session,
        HARNESS,
        Some(who.repo),
        Some(who.repo_key),
        &channels,
        None,
        NOW,
        &none,
        gate,
    );
    let inline = crate::brief::render_inline(
        events,
        who.session,
        HARNESS,
        Some(who.repo),
        Some(who.repo_key),
        &channels,
        None,
        None,
        NOW,
        gate,
    );
    let inbox = crate::query::inbox(
        events,
        who.session,
        HARNESS,
        Some(who.repo),
        Some(who.repo_key),
        None,
        &none,
        gate,
    );
    let mut ledger = crate::wake::Ledger::new(who.session);
    for e in events {
        ledger.observe(e);
    }
    vec![
        ("brief", brief.delivered.contains(&note.id)),
        ("inline", inline.delivered.contains(&note.id)),
        ("inbox", inbox.events.iter().any(|e| e.id == note.id)),
        ("watch:calls_for", crate::wake::calls_for(note, &reader)),
        ("watch:cause", ledger.cause(note, &reader).is_some()),
        (
            "stop-check",
            crate::status::owed_notes(events, &reader, NOW)
                .iter()
                .any(|e| e.id == note.id),
        ),
    ]
}

fn kinds() -> [NoteKind; 4] {
    [
        NoteKind::Fyi,
        NoteKind::Warning,
        NoteKind::Ask,
        NoteKind::Handoff,
    ]
}

/// Every address a note can carry that would reach `who` in an open hub.
fn addresses(who: Who<'_>) -> Vec<(&'static str, Option<Recipient>)> {
    let to = |kind, value: &str| {
        Some(Recipient {
            kind,
            value: value.to_string(),
        })
    };
    vec![
        ("session", to(RecipientKind::Session, who.session)),
        ("channel", to(RecipientKind::Channel, CHANNEL)),
        ("repo", to(RecipientKind::Repo, who.repo_key)),
        ("checkout", to(RecipientKind::Repo, who.repo)),
        ("harness", to(RecipientKind::Harness, HARNESS)),
        ("broadcast", None),
    ]
}

/// The history of one case: everybody says hello, then the note.
fn history(w: &World, note: &Event) -> Vec<Event> {
    vec![
        mk(T0, EventKind::Hello, "a1", Some(&w.app)),
        mk(T0 + 1, EventKind::Hello, "a2", Some(&w.app_wt)),
        mk(T0 + 2, EventKind::Hello, "b1", Some(&w.tool)),
        mk(T0 + 3, EventKind::Hello, "b2", Some(&w.tool2)),
        note.clone(),
    ]
}

/// The whole matrix for one reader: a writer from the other workspace never
/// reaches it; a writer from its own workspace and the operator reach it
/// exactly as an open hub would.
fn assert_matrix(w: &World, who: Who<'_>, same: (&str, &str), other: (&str, &str)) {
    let placement = Placement::load(&w.store);
    assert!(!placement.is_single());
    let gate = placement.gate(Some(who.repo_key));
    let mut delivered_somewhere = 0;
    for kind in kinds() {
        for (label, to) in addresses(who) {
            for (writer, repo, expect) in [
                (other.0, Some(other.1), "cross"),
                (same.0, Some(same.1), "same"),
                ("operator@h", None, "operator"),
            ] {
                let mut note = if expect == "operator" {
                    operator_note(T0 + 10)
                } else {
                    mk(T0 + 10, EventKind::Note, writer, repo)
                };
                note.kind = kind;
                note.to = to.clone();
                note.text = format!("{expect} {label} {}", kind.as_str());
                let events = history(w, &note);
                let gated = paths(&events, &note, who, &gate);
                let open = paths(&events, &note, who, &Gate::OPEN);
                for ((path, got), (_, was)) in gated.iter().zip(&open) {
                    let case = format!(
                        "{expect} writer {writer}, to {label}, kind {}, path {path}, reader {}",
                        kind.as_str(),
                        who.session
                    );
                    match expect {
                        "cross" => assert!(!got, "crossed the boundary: {case}"),
                        _ => assert_eq!(got, was, "silenced or changed: {case}"),
                    }
                    if *got {
                        delivered_somewhere += 1;
                    }
                }
            }
        }
    }
    // The matrix is not vacuous: the same workspace and the operator are
    // delivered on many paths.
    assert!(
        delivered_somewhere > 40,
        "only {delivered_somewhere} deliveries"
    );
}

#[test]
fn no_path_carries_a_note_from_acme_into_personal_and_none_is_silenced() {
    let w = world();
    let who = Who {
        session: "b1",
        repo: &w.tool,
        repo_key: &w.tool,
    };
    assert_matrix(&w, who, ("b2", &w.tool2.clone()), ("a1", &w.app.clone()));
}

#[test]
fn no_path_carries_a_note_from_personal_into_acme_and_none_is_silenced() {
    let w = world();
    // The Acme reader sits in a linked worktree: its repository is `app`.
    let who = Who {
        session: "a2",
        repo: &w.app_wt,
        repo_key: &w.app,
    };
    assert_matrix(&w, who, ("a1", &w.app.clone()), ("b2", &w.tool2.clone()));
}

#[test]
fn a_note_written_outside_any_repository_belongs_to_the_default_workspace() {
    let w = world();
    let placement = Placement::load(&w.store);
    let mut note = mk(T0 + 10, EventKind::Note, "loose", None);
    note.kind = NoteKind::Warning;
    note.to = Some(Recipient {
        kind: RecipientKind::Harness,
        value: HARNESS.into(),
    });
    let events = history(&w, &note);
    let in_default = Who {
        session: "b1",
        repo: &w.tool,
        repo_key: &w.tool,
    };
    let in_acme = Who {
        session: "a1",
        repo: &w.app,
        repo_key: &w.app,
    };
    let reached = |who: Who<'_>| {
        paths(&events, &note, who, &placement.gate(Some(who.repo_key)))
            .iter()
            .any(|(_, hit)| *hit)
    };
    assert!(reached(in_default), "the default workspace hears it");
    assert!(!reached(in_acme), "Acme never does");
}

#[test]
fn replies_and_dependency_results_do_not_wake_across_the_boundary() {
    let w = world();
    let placement = Placement::load(&w.store);
    let who = Who {
        session: "b1",
        repo: &w.tool,
        repo_key: &w.tool,
    };
    let gate = placement.gate(Some(who.repo_key));
    let mine = mk(T0 + 5, EventKind::Note, "b1", Some(&w.tool));
    for (writer, repo, should) in [("a1", &w.app, false), ("b2", &w.tool2, true)] {
        let mut reply = mk(T0 + 10, EventKind::Note, writer, Some(repo));
        reply.note_ref = Some(mine.id.clone());
        let mut dep = mk(T0 + 11, EventKind::Dependency, writer, Some(repo));
        dep.to = Some(Recipient {
            kind: RecipientKind::Session,
            value: "b1".into(),
        });
        dep.text = r#"{"action":"resolve","id":"d1","outcome":"satisfied","evidence":"x"}"#.into();
        let mut events = history(&w, &mine);
        events.push(reply.clone());
        events.push(dep.clone());
        let mut ledger = crate::wake::Ledger::new("b1");
        for e in &events {
            ledger.observe(e);
        }
        let channels: Vec<String> = Vec::new();
        let reader = Reader {
            session: "b1",
            harness: HARNESS,
            repo: Some(who.repo),
            repo_key: Some(who.repo_key),
            channels: &channels,
            workspace: &gate,
        };
        assert_eq!(
            ledger.cause(&reply, &reader).is_some(),
            should,
            "reply from {writer}"
        );
        let open = Reader {
            workspace: &Gate::OPEN,
            ..reader
        };
        assert!(
            ledger.cause(&dep, &open).is_some(),
            "an open hub wakes for it"
        );
        assert_eq!(
            ledger.cause(&dep, &reader).is_some(),
            should,
            "dependency result from {writer}"
        );
    }
}

#[test]
fn the_digest_never_names_another_workspaces_sessions_or_intents() {
    let w = world();
    let placement = Placement::load(&w.store);
    let gate = placement.gate(Some(&w.tool));
    let mut intent_a = mk(T0 + 4, EventKind::Intent, "a1", Some(&w.app));
    intent_a.text = "secret employer migration".into();
    let mut intent_b = mk(T0 + 5, EventKind::Intent, "b2", Some(&w.tool2));
    intent_b.text = "polishing my own tool".into();
    // The operator writes, so the digest speaks; b2 and a1 are live others.
    let mut note = operator_note(T0 + 10);
    note.to = Some(Recipient {
        kind: RecipientKind::Session,
        value: "b1".into(),
    });
    note.text = "a note so the digest speaks".into();
    let mut events = history(&w, &note);
    events.insert(4, intent_a);
    events.insert(5, intent_b);
    for e in &events {
        w.store.append(e).unwrap();
    }
    let views = crate::query::sessions(&w.store, &events, NOW, 3_600);
    let render = |gate: &Gate<'_>| {
        crate::brief::render_with_views(
            &w.store,
            &events,
            &views,
            "b1",
            HARNESS,
            Some(&w.tool),
            Some(&w.tool),
            &[],
            None,
            NOW,
            &HashSet::new(),
            gate,
        )
        .text
        .unwrap_or_default()
    };
    let gated = render(&gate);
    assert!(!gated.contains("secret employer"), "{gated}");
    assert!(gated.contains("polishing my own tool"), "{gated}");
    // b2 and the operator's own session; never a1 or a2.
    assert!(gated.contains("2 live"), "{gated}");
    let open = render(&Gate::OPEN);
    assert!(
        open.contains("secret employer"),
        "the open hub did show it: {open}"
    );
}

#[test]
fn the_fleet_and_the_note_trace_owe_answers_only_inside_the_writers_workspace() {
    let w = world();
    // An ask to the channel both a2 (Acme) and b1 (Personal) belong to,
    // written by b2 (Personal): only b1 owes it.
    let mut ask = mk(T0 + 10, EventKind::Note, "b2", Some(&w.tool2));
    ask.kind = NoteKind::Ask;
    ask.to = Some(Recipient {
        kind: RecipientKind::Channel,
        value: CHANNEL.into(),
    });
    ask.text = "who owns the release?".into();
    let events = history(&w, &ask);
    for e in &events {
        w.store.append(e).unwrap();
    }
    let view = crate::fleet::build(&crate::fleet::FleetInputs {
        store: &w.store,
        events: &events,
        now_ms: NOW,
        stale_secs: 3_600,
        leases: &[],
        hub_enabled: false,
        devices: 0,
    });
    let item = view
        .pending
        .iter()
        .find(|p| p.note == ask.id)
        .expect("the ask is pending");
    let owed: Vec<&str> = item.owed_by.iter().map(|o| o.session.as_str()).collect();
    assert_eq!(owed, vec!["b1"], "a2 sits in Acme and never received it");

    let report = crate::note_trace::report(&w.store, &events, &[], &ask, NOW, 3_600);
    let pending: Vec<&str> = report.pending.iter().map(|p| p.session.as_str()).collect();
    assert!(pending.contains(&"b1"), "{pending:?}");
    assert!(!pending.contains(&"a2"), "{pending:?}");
    assert!(!pending.contains(&"a1"), "{pending:?}");
}

#[test]
fn with_one_workspace_or_none_on_disk_every_delivery_is_unchanged() {
    for registry in [None, Some(Workspaces::single())] {
        let w = world_with(registry);
        let placement = Placement::load(&w.store);
        assert!(placement.is_single());
        let gate = placement.gate(Some(&w.tool));
        assert!(matches!(gate, Gate::Open));
        let who = Who {
            session: "b1",
            repo: &w.tool,
            repo_key: &w.tool,
        };
        let mut note = mk(T0 + 10, EventKind::Note, "a1", Some(&w.app));
        note.kind = NoteKind::Ask;
        note.to = Some(Recipient {
            kind: RecipientKind::Session,
            value: "b1".into(),
        });
        let events = history(&w, &note);
        let gated = paths(&events, &note, who, &gate);
        assert!(gated.iter().all(|(_, hit)| *hit), "{gated:?}");
    }
}

#[test]
fn moving_a_repository_moves_its_history_and_its_sessions() {
    let w = world();
    let mut set = two_workspaces();
    set.pin(&w.app, "personal").unwrap();
    fs::write(
        w.store.root().join(super::FILE),
        serde_json::to_string_pretty(&set).unwrap(),
    )
    .unwrap();
    let placement = Placement::load(&w.store);
    let who = Who {
        session: "b1",
        repo: &w.tool,
        repo_key: &w.tool,
    };
    // A note written before the move, from the repository now moved.
    let mut note = mk(T0 + 10, EventKind::Note, "a1", Some(&w.app));
    note.kind = NoteKind::Ask;
    note.to = Some(Recipient {
        kind: RecipientKind::Session,
        value: "b1".into(),
    });
    let events = history(&w, &note);
    let gated = paths(&events, &note, who, &placement.gate(Some(who.repo_key)));
    assert!(gated.iter().all(|(_, hit)| *hit), "{gated:?}");
    // Its worktree moved with it.
    assert_eq!(placement.of_repo(Some(&w.app_wt)), "personal");
}
