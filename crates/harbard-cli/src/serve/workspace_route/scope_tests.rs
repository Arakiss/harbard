//! The console's read and writes, inside one workspace.
//!
//! Two lead sessions, one per workspace, each with an agent: `alba` works in
//! Acme's `app`, `bruno` in my `tool` and also wrote once in `app`. The read
//! of a workspace shows its own channels, agents and notes and nothing of
//! the other; a write from a workspace (a reply, a channel note, an `@`, a
//! "Resend to the lead") only ever targets sessions inside it.

use super::*;
use harbard_core::team::{AgentRow, CheckoutUse, FormatProbe, LeadPanel, TeamView};
use harbard_core::workspaces::{Placement, Workspaces};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

const LA: &str = "aaaaaaaa-0000-0000-0000-00000000000a";
const LB: &str = "bbbbbbbb-0000-0000-0000-00000000000b";

struct Fixture {
    root: PathBuf,
    store: Store,
    app: String,
    tool: String,
    now: u64,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Real checkouts outside temp space (serve never makes a channel of a temp
/// dir) and outside `/Volumes` (nor of a protected folder): under the
/// workspace's `target`, spelled through the repository so a `target` that
/// is a link to another volume is not followed.
fn fixture() -> Fixture {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let root = repo_root.join("target").join(format!(
        "ws-scope-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    let repo = |name: &str, url: &str| {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join(".git/config"),
            format!("[remote \"origin\"]\n\turl = {url}\n"),
        )
        .unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        dir.display().to_string()
    };
    let app = repo("app", "git@github.com:Acme/app.git");
    let tool = repo("tool", "git@github.com:me/tool.git");
    let store = Store::at(root.join("home"));
    std::fs::create_dir_all(store.root()).unwrap();
    let mut set = Workspaces::single();
    set.workspaces.clear();
    set.add("personal", "Personal", &["me".into()]).unwrap();
    set.add("acme", "Trabajo", &["Acme".into()]).unwrap();
    set.default = "personal".into();
    std::fs::write(
        Workspaces::path(&store),
        serde_json::to_string(&set).unwrap(),
    )
    .unwrap();
    let now = clock::now_ms();
    for (n, (session, cwd)) in [(LA, &app), (LB, &tool)].into_iter().enumerate() {
        store
            .append(&Event {
                v: harbard_core::SCHEMA_VERSION,
                id: ulid::Ulid::from_parts(now - 60_000, n as u128).to_string(),
                ts: clock::rfc3339_utc(now - 60_000),
                event: EventKind::Hello,
                session: session.into(),
                harness: "claude-code".into(),
                host: "h".into(),
                cwd: cwd.clone(),
                repo: Some(cwd.clone()),
                repo_key: None,
                branch: Some("main".into()),
                role: Role::Main,
                parent: None,
                text: String::new(),
                to: None,
                kind: NoteKind::Fyi,
                note_ref: None,
            })
            .unwrap();
    }
    Fixture {
        root,
        store,
        app,
        tool,
        now,
    }
}

fn agent(id: &str, name: &str, now: u64, wrote: &[&str]) -> AgentRow {
    AgentRow {
        id: id.into(),
        name: Some(name.into()),
        kind: "teammate",
        agent_type: None,
        color: None,
        model: None,
        description: None,
        brief: None,
        state: "working",
        state_reason: "tool_running",
        last_action_ms: Some(now - 30_000),
        seen_ms: now - 30_000,
        records: 3,
        checkouts: wrote
            .iter()
            .map(|p| CheckoutUse {
                path: p.to_string(),
                writes: 1,
                first_ms: now - 600_000,
                last_ms: now - 60_000,
            })
            .collect(),
        unplaced_writes: 0,
        worktree: None,
        messages: Vec::new(),
        messages_total: 0,
        last_label: None,
        first_ms: Some(now - 900_000),
        git_roots: Vec::new(),
        write_dirs: Vec::new(),
        git_dirs: Vec::new(),
        target_dirs: Vec::new(),
        activity: harbard_core::activity::Activity::plain("working", now - 30_000),
        progress: Default::default(),
    }
}

fn lead(session: &str, cwd: &str, now: u64, agents: Vec<AgentRow>) -> LeadPanel {
    LeadPanel {
        session: session.into(),
        project: "-p".into(),
        cwd: Some(cwd.into()),
        last_write_ms: now,
        lead_last_ms: Some(now),
        lead_found: true,
        agents,
        warnings: Vec::new(),
        skipped_lines: 0,
        lead_write_dirs: Vec::new(),
        lead_cmd_dirs: Vec::new(),
        git_cwds: vec![cwd.into()],
    }
}

fn team(f: &Fixture) -> TeamView {
    TeamView {
        v: 1,
        now_ms: f.now,
        window_ms: harbard_core::team::WINDOW_DEFAULT_MS,
        quiet_running_ms: 900_000,
        root_found: true,
        leads: vec![
            lead(
                LA,
                &f.app,
                f.now,
                vec![agent("ag-alba", "alba", f.now, &[&f.app])],
            ),
            lead(
                LB,
                &f.tool,
                f.now,
                vec![agent("ag-bruno", "bruno", f.now, &[&f.tool, &f.app])],
            ),
        ],
        format: FormatProbe::default(),
    }
}

fn events(f: &Fixture) -> Vec<Event> {
    f.store.read(harbard_core::Window::All).events
}

fn read(f: &Fixture, gate: &Gate<'_>) -> WorkspaceView {
    view_in(&f.store, &events(f), 3_600, &team(f), f.now, gate)
}

/// A path as the disk spells it: a `target` linked to another volume
/// resolves there.
fn real(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.to_string())
}

fn channel_repos(v: &WorkspaceView) -> Vec<String> {
    let mut r: Vec<String> = v.channels.iter().map(|c| real(&c.repo)).collect();
    r.sort();
    r
}

fn agent_keys(v: &WorkspaceView) -> Vec<String> {
    let mut k: Vec<String> = v.agents.iter().map(|a| a.key.clone()).collect();
    k.sort();
    k
}

#[test]
fn each_workspace_reads_its_own_channels_and_agents_only() {
    let f = fixture();
    let placement = Placement::load(&f.store);
    let acme = read(&f, &placement.gate_of("acme"));
    let mine = read(&f, &placement.gate_of("personal"));
    let open = read(&f, &Gate::OPEN);

    assert_eq!(channel_repos(&acme), vec![real(&f.app)]);
    assert_eq!(
        channel_repos(&mine),
        vec![real(&f.tool)],
        "bruno wrote in app, and app is still not a channel of Personal"
    );
    let mut both = vec![real(&f.app), real(&f.tool)];
    both.sort();
    assert_eq!(channel_repos(&open), both, "the whole hub is unchanged");

    assert!(agent_keys(&acme).iter().any(|k| k == "ag-alba"));
    assert!(!agent_keys(&acme).iter().any(|k| k == "ag-bruno"));
    assert!(agent_keys(&mine).iter().any(|k| k == "ag-bruno"));
    assert!(!agent_keys(&mine).iter().any(|k| k == "ag-alba"));
    for a in &mine.agents {
        assert!(
            a.channels
                .iter()
                .all(|c| mine.channels.iter().any(|ch| &ch.id == c)),
            "{} lists a channel Personal does not have: {:?}",
            a.key,
            a.channels
        );
    }
    // Nothing of the other workspace anywhere in the read.
    let text = serde_json::to_string(&mine).unwrap();
    assert!(!text.contains("ag-alba") && !text.contains(LA), "{text}");
    let text = serde_json::to_string(&acme).unwrap();
    assert!(!text.contains("ag-bruno") && !text.contains(LB), "{text}");
}

fn ctx_reply(f: &Fixture, gate: &Gate<'_>, body: Value) -> Result<Value, ApiError> {
    let evs = events(f);
    let t = || team(f);
    let mut open = |_: &Path, _: &[&str]| Ok(());
    let mut ctx = WriteCtx {
        store: &f.store,
        events: &evs,
        stale_secs: 3_600,
        now: f.now,
        team: &t,
        open: &mut open,
        gate,
    };
    write(&mut ctx, "reply", &body)
}

fn stored_sessions(out: &Value) -> Vec<String> {
    out["stored"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["session"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn an_at_mention_and_a_channel_note_never_reach_past_the_workspace() {
    let f = fixture();
    let placement = Placement::load(&f.store);
    let mine = placement.gate_of("personal");
    let acme = placement.gate_of("acme");

    // In Personal's channel, `@alba` names nobody: alba is Acme's.
    let channel = read(&f, &mine).channels[0].id.clone();
    let out = ctx_reply(
        &f,
        &mine,
        json!({ "channel": channel, "text": "revisad esto @alba @bruno" }),
    )
    .unwrap();
    assert_eq!(stored_sessions(&out), vec![LB.to_string()]);

    // The same `@alba` from Acme reaches alba's lead.
    let channel = read(&f, &acme).channels[0].id.clone();
    let out = ctx_reply(
        &f,
        &acme,
        json!({ "channel": channel, "text": "@alba mira" }),
    )
    .unwrap();
    assert_eq!(stored_sessions(&out), vec![LA.to_string()]);

    // A reply to an agent of the other workspace is refused: not in the view.
    let e = ctx_reply(&f, &mine, json!({ "agent": "ag-alba", "text": "hello" })).unwrap_err();
    assert_eq!(e.code, "no_delivery_path");
    // A channel of the other workspace has no readers here.
    let app_channel = read(&f, &acme).channels[0].id.clone();
    let e = ctx_reply(
        &f,
        &mine,
        json!({ "channel": app_channel, "text": "hello" }),
    )
    .unwrap_err();
    assert_eq!(e.code, "no_delivery_path");

    // Every note stored was addressed inside its workspace.
    for e in events(&f).iter().filter(|e| e.is_operator()) {
        let to = &e.to.as_ref().unwrap().value;
        assert!(to == LA || to == LB);
        if e.text.contains("revisad") {
            assert_eq!(to, LB, "{}", e.text);
        }
    }
}

#[test]
fn the_operators_notes_show_only_in_the_workspace_of_their_session() {
    let f = fixture();
    let placement = Placement::load(&f.store);
    let mine = placement.gate_of("personal");
    let acme = placement.gate_of("acme");
    ctx_reply(
        &f,
        &acme,
        json!({ "agent": "ag-alba", "text": "ACME-ONLY" }),
    )
    .unwrap();
    let in_acme = serde_json::to_string(&read(&f, &acme)).unwrap();
    let in_mine = serde_json::to_string(&read(&f, &mine)).unwrap();
    assert!(in_acme.contains("ACME-ONLY"));
    assert!(!in_mine.contains("ACME-ONLY"), "{in_mine}");
}

#[test]
fn the_rail_carries_one_tile_per_workspace_and_a_device_sees_its_grants() {
    use super::super::access::Caller;
    let f = fixture();
    let placement = Placement::load(&f.store);
    assert_eq!(
        select(&placement, &Caller::Local, None).unwrap(),
        Some("personal".into())
    );
    assert_eq!(
        select(&placement, &Caller::Local, Some("acme")).unwrap(),
        Some("acme".into())
    );
    assert_eq!(
        select(&placement, &Caller::Local, Some("ghost"))
            .unwrap_err()
            .code,
        "unknown_workspace"
    );
    let mut card = harbard_core::LocalDevice {
        id: "d1".into(),
        name: "laptop".into(),
        role: harbard_core::DeviceRole::Satellite,
    }
    .to_card(0, vec![]);
    card.access = harbard_core::Access::Display;
    card.workspaces = vec!["acme".into()];
    let device = Caller::Device(card.clone());
    assert_eq!(allowed(&placement, &device), vec!["acme".to_string()]);
    assert_eq!(
        select(&placement, &device, None).unwrap(),
        Some("acme".into())
    );
    let e = select(&placement, &device, Some("personal")).unwrap_err();
    assert_eq!((e.status, e.code), (403, "workspace_forbidden"));
    // No grant: the default workspace only. Operator: everything.
    card.workspaces.clear();
    assert_eq!(
        allowed(&placement, &Caller::Device(card.clone())),
        vec!["personal".to_string()]
    );
    card.access = harbard_core::Access::Operator;
    assert_eq!(allowed(&placement, &Caller::Device(card)).len(), 2);

    // The body names the current workspace and every tile with its badges.
    let v = read(&f, &placement.gate_of("personal"));
    let t = tiles(
        &f.store,
        &events(&f),
        3_600,
        &team(&f),
        f.now,
        &placement,
        &allowed(&placement, &Caller::Local),
        "personal",
        &v,
    );
    let ids: Vec<&str> = t.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, vec!["personal", "acme"]);
    assert!(t[0].current && t[0].default && !t[1].current);
    assert_eq!(t[1].name, "Trabajo");
    assert_eq!(t[1].tile, "T");
}

fn ctx_write(
    f: &Fixture,
    gate: &Gate<'_>,
    now: u64,
    action: &str,
    body: Value,
) -> Result<Value, ApiError> {
    let evs = events(f);
    let t = || team(f);
    let mut open = |_: &Path, _: &[&str]| Ok(());
    let mut ctx = WriteCtx {
        store: &f.store,
        events: &evs,
        stale_secs: 3_600,
        now,
        team: &t,
        open: &mut open,
        gate,
    };
    write(&mut ctx, action, &body)
}

/// "Resend to the lead" re-sends an unanswered note to the sessions it
/// was stored for, from the workspace whose card it is; from another
/// workspace the card does not exist.
#[test]
fn a_relay_to_the_coordinator_stays_in_the_workspace_of_its_card() {
    let f = fixture();
    let placement = Placement::load(&f.store);
    let mine = placement.gate_of("personal");
    let acme = placement.gate_of("acme");
    ctx_write(
        &f,
        &acme,
        f.now,
        "reply",
        json!({ "agent": "ag-alba", "text": "how is it?" }),
    )
    .unwrap();
    // Half an hour later nobody has answered.
    let later = f.now + 30 * 60_000;
    let at = |gate: &Gate<'_>| view_in(&f.store, &events(&f), 3_600, &team(&f), later, gate);
    let card = at(&acme)
        .cards
        .into_iter()
        .find(|c| c.kind == "unanswered")
        .expect("an unanswered card in Acme")
        .id;
    assert!(
        !at(&mine).cards.iter().any(|c| c.id == card),
        "Personal has no such card"
    );
    let e = ctx_write(&f, &mine, later, "nudge", json!({ "card": card })).unwrap_err();
    assert_eq!((e.status, e.code), (404, "unknown_item"));
    let out = ctx_write(&f, &acme, later, "nudge", json!({ "card": card })).unwrap();
    let to: Vec<&str> = out["stored"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["session"].as_str().unwrap())
        .collect();
    assert_eq!(to, vec![LA]);
}
