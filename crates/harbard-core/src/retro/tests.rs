//! A synthetic day: three builders leaking
//! secrets the same way, two file-read races, a note from the operator nobody
//! answered, an agent stalled an hour on a permission prompt, and names
//! nobody would choose.

use super::*;
use crate::activity::Activity;
use crate::event::{HARNESS_HUMAN, NoteKind, Recipient, Role, SCHEMA_VERSION};
use crate::feedback::Sample;
use crate::progress::{Progress, Stall};
use crate::team::{FormatProbe, MessageRow};
use ulid::Ulid;

const MIN: u64 = 60_000;
/// 2026-11-07T18:00:00Z.
const NOW: u64 = 1_794_074_400_000;
const T0: u64 = NOW - 8 * 60 * MIN;
const LEAD: &str = "11111111-2222-3333-4444-555555555555";
const OTHER_LEAD: &str = "99999999-2222-3333-4444-555555555555";

fn out(at: u64, first: &str) -> MessageRow {
    let label = first
        .split(" · ")
        .next()
        .filter(|l| crate::team::LABELS.contains(l));
    MessageRow {
        dir: "out",
        peer: "team-lead".into(),
        ts_ms: at,
        first_line: first.into(),
        label: label.map(str::to_string),
        chars: first.chars().count(),
        report: None,
        repeat: false,
        undelivered: false,
        structured: None,
        digest: format!("{at:016x}"),
        shas: Vec::new(),
        body: None,
        paths: Vec::new(),
    }
}

fn agent(
    id: &str,
    name: Option<&str>,
    desc: &str,
    state: &'static str,
    msgs: Vec<MessageRow>,
) -> AgentRow {
    AgentRow {
        id: id.into(),
        name: name.map(str::to_string),
        kind: "teammate",
        agent_type: None,
        color: None,
        model: None,
        description: Some(desc.into()),
        brief: None,
        state,
        state_reason: "turn_ended",
        last_action_ms: Some(NOW - 5 * MIN),
        seen_ms: NOW - 5 * MIN,
        records: 10,
        checkouts: Vec::new(),
        unplaced_writes: 0,
        worktree: None,
        messages_total: msgs.len(),
        messages: msgs,
        last_label: None,
        first_ms: Some(T0),
        git_roots: Vec::new(),
        write_dirs: Vec::new(),
        git_dirs: Vec::new(),
        target_dirs: Vec::new(),
        activity: Activity::plain(state, NOW - 5 * MIN),
        progress: Progress::default(),
    }
}

fn lead(session: &str, agents: Vec<AgentRow>) -> LeadPanel {
    LeadPanel {
        session: session.into(),
        project: "-harbard".into(),
        cwd: None,
        last_write_ms: NOW,
        lead_last_ms: Some(NOW),
        lead_found: true,
        agents,
        warnings: Vec::new(),
        skipped_lines: 0,
        lead_write_dirs: Vec::new(),
        lead_cmd_dirs: Vec::new(),
        git_cwds: Vec::new(),
    }
}

fn today_team() -> TeamView {
    let at = |m: u64| T0 + m * MIN;
    let mut stalled = agent(
        "a6",
        Some("harbard-profile"),
        "Build the operator states and the profile",
        "done",
        vec![
            out(at(30), "START · harbard-profile · Empiezo por los estados."),
            out(
                at(200),
                "DONE · harbard-profile · Instalado en el Mac, listo para revisar.",
            ),
        ],
    );
    stalled.progress.stalls = vec![Stall {
        tool: "Bash",
        pattern: "rm_variable",
        started_ms: at(100),
        gap_ms: 60 * MIN,
    }];
    let mut waiting = agent(
        "a8",
        Some("keychain-review-2"),
        "Review the keychain branch",
        "done",
        vec![out(at(300), "DONE · keychain-review-2 · Sin hallazgos.")],
    );
    waiting.activity = Activity::new("waiting", NOW - 20 * MIN, None, NOW);
    let mut panel = lead(
        LEAD,
        vec![
            agent(
                "a1",
                Some("harbard-protocol"),
                "Build protocol messages, names, safe_fs and Host checks",
                "done",
                vec![
                    out(at(10), "START · harbard-protocol · Empiezo."),
                    out(
                        at(90),
                        "DONE · harbard-protocol · Instalado y verificado en la app.",
                    ),
                ],
            ),
            agent(
                "a2",
                Some("harbard-direct"),
                "Build the redact module and the live view",
                "stopped",
                vec![out(at(150), "DONE · harbard-direct · Instalado.")],
            ),
            agent(
                "a3",
                Some("protocol-review"),
                "Review the protocolo branch",
                "done",
                vec![
                    out(
                        at(120),
                        "FINDING · protocol-review · El token se filtra sin enmascarar en el log del servidor.",
                    ),
                    out(
                        at(125),
                        "FINDING · protocol-review · Carrera al leer el fichero: TOCTOU entre stat y open.",
                    ),
                    out(at(130), "DONE · protocol-review · Dos hallazgos."),
                ],
            ),
            agent(
                "a4",
                Some("direct-review"),
                "Review the directo branch",
                "done",
                vec![
                    out(
                        at(160),
                        "FINDING · direct-review · Fuga de secretos: la contraseña aparece en la vista en directo.",
                    ),
                    out(
                        at(165),
                        "FINDING · direct-review · File-read race: el symlink se sigue entre la comprobación y la lectura.",
                    ),
                    out(at(170), "DONE · direct-review · Primera pasada."),
                    out(at(240), "DONE · direct-review · Segunda pasada, corregido."),
                ],
            ),
            agent(
                "a5",
                Some("profile-review"),
                "Review the perfil branch",
                "done",
                vec![
                    out(
                        at(210),
                        "FINDING · profile-review · Leak: GITHUB_TOKEN visible sin redact en el perfil.",
                    ),
                    out(at(220), "DONE · profile-review · Un hallazgo."),
                ],
            ),
            stalled,
            agent("a7", None, "Synthetic subagent", "done", Vec::new()),
            waiting,
        ],
    );
    panel.warnings.push(crate::team::Warning {
        kind: "two_writers",
        agents: vec!["a1".into(), "a2".into()],
        checkout: Some("/nowhere/harbard".into()),
        since_ms: at(60),
        count: 2,
    });
    let other = lead(
        OTHER_LEAD,
        vec![agent(
            "b1",
            Some("acme"),
            "Construir la app",
            "running",
            Vec::new(),
        )],
    );
    TeamView {
        v: 1,
        now_ms: NOW,
        window_ms: WEEK_MS,
        quiet_running_ms: 15 * MIN,
        root_found: true,
        leads: vec![panel, other],
        format: FormatProbe::default(),
    }
}

fn ev(ms: u64, seq: u16, kind: EventKind, operator: bool, text: &str) -> Event {
    Event {
        v: SCHEMA_VERSION,
        id: Ulid::from_parts(ms, seq as u128).to_string(),
        ts: crate::clock::rfc3339_utc(ms),
        event: kind,
        session: if operator {
            "operator@mac".into()
        } else {
            LEAD.into()
        },
        harness: if operator {
            HARNESS_HUMAN.into()
        } else {
            "claude-code".into()
        },
        host: "mac".into(),
        cwd: String::new(),
        repo: None,
        repo_key: None,
        branch: None,
        role: if operator { Role::Operator } else { Role::Main },
        parent: None,
        text: text.into(),
        to: operator.then(|| Recipient {
            kind: RecipientKind::Session,
            value: LEAD.into(),
        }),
        kind: NoteKind::Fyi,
        note_ref: None,
    }
}

fn today_bus() -> Vec<Event> {
    let unanswered = ev(
        NOW - 3 * 60 * MIN,
        1,
        EventKind::Note,
        true,
        "¿Cómo va la revisión de directo?",
    );
    let asked = ev(
        NOW - 5 * 60 * MIN,
        2,
        EventKind::Note,
        true,
        "¿Qué quiere decir sluice en tu mensaje?",
    );
    let mut answer = ev(
        NOW - 5 * 60 * MIN + 12 * MIN,
        3,
        EventKind::Note,
        false,
        "sluice es la cola de entregas.",
    );
    answer.note_ref = Some(asked.id.clone());
    vec![asked, answer, unanswered]
}

fn tap(kind: TargetKind, id: &str, value: &str, ms: u64) -> Record {
    Record {
        id: Ulid::from_parts(ms, 9).to_string(),
        ms,
        target_kind: kind,
        target_id: id.into(),
        value: value.into(),
        reason: None,
        sample: (kind == TargetKind::RobotState).then(|| Sample {
            state: "thinking".into(),
            label: None,
        }),
        undone: false,
    }
}

fn today_feedback() -> Vec<Record> {
    vec![tap(
        TargetKind::Message,
        "a1:1:out:0",
        "unclear",
        NOW - 4 * 60 * MIN,
    )]
}

/// The operator's votes on names, as the workshop keeps them in names.json.
fn today_name_votes() -> Vec<crate::workshop::Vote> {
    let vote =
        |agent: &str, name: &str, vote: &str, ago: u64, to: Option<&str>| crate::workshop::Vote {
            ts_ms: NOW - ago,
            agent: agent.into(),
            name: name.into(),
            vote: vote.into(),
            to: to.map(str::to_string),
        };
    vec![
        vote("a7", "Agent-7", "down", 2 * 60 * MIN, None),
        vote("a8", "keychain-review-2", "down", 2 * 60 * MIN - 1, None),
        vote("a1", "Protocolo", "rename", 60 * MIN, Some("Owen")),
        vote("old", "Viejo", "down", WEEK_MS + 60 * MIN, None),
    ]
}

fn retro_of(scope: Scope) -> Retro {
    let team = today_team();
    let bus = today_bus();
    let fb = today_feedback();
    let votes = today_name_votes();
    compute(&RetroInput {
        now_ms: NOW,
        since_ms: NOW - WEEK_MS,
        scope,
        events: &bus,
        team: &team,
        view: None,
        feedback: &fb,
        name_votes: &votes,
    })
}

#[test]
fn classifiers_read_this_fleets_first_lines() {
    assert_eq!(
        finding_class("FINDING · x · El token se filtra sin enmascarar"),
        "secret_leak"
    );
    assert_eq!(
        finding_class("FINDING · x · Carrera al leer el fichero"),
        "file_race"
    );
    assert_eq!(
        finding_class("FINDING · x · Falta la comprobación de la cabecera Host"),
        "host_check"
    );
    assert_eq!(
        finding_class("FINDING · x · Estado incorrecto: dice pensando"),
        "wrong_state"
    );
    assert_eq!(
        finding_class("FINDING · x · El foco se pierde en la UI"),
        "ui"
    );
    assert_eq!(finding_class("FINDING · x · Un test inestable"), "other");
    assert_eq!(branch_of("direct-review"), "direct");
    assert_eq!(branch_of("keychain-review-2"), "keychain");
    assert_eq!(branch_of("harbard-profile"), "profile");
    assert!(looks_generated(None));
    assert!(looks_generated(Some("keychain-review-2")));
    assert!(looks_generated(Some("a3f9c2e1")));
    assert!(!looks_generated(Some("Hazel")));
    assert!(reads_english("Review the directo branch"));
    assert!(!reads_english("Revisar la rama directo"));
    assert!(asks_what_it_means("¿Qué quiere decir sluice?"));
    assert!(asks_what_it_means("que significa ack_pending"));
    assert!(!asks_what_it_means("¿Cómo va?"));
}

#[test]
fn todays_patterns_become_metrics() {
    let r = retro_of(Scope::Week);
    let leak = r
        .delivery
        .findings
        .iter()
        .find(|f| f.class == "secret_leak")
        .unwrap();
    assert_eq!(leak.count, 3);
    assert_eq!(leak.branches, ["direct", "profile", "protocol"]);
    assert!(leak.repeats);
    assert_eq!(leak.days, ["2026-11-07"]);
    let race = r
        .delivery
        .findings
        .iter()
        .find(|f| f.class == "file_race")
        .unwrap();
    assert_eq!((race.count, race.branches.len()), (2, 2));
    assert_eq!(r.delivery.review_rounds.get("direct"), Some(&2));
    assert_eq!(r.delivery.review_rounds.get("protocol"), Some(&1));
    assert_eq!(r.delivery.brief_to_install_p50_min, Some(150));

    assert_eq!(r.waiting.unanswered.len(), 1);
    assert_eq!(r.waiting.unanswered[0].open_min, 180);
    assert_eq!(
        (r.waiting.answered, r.waiting.answer_p50_min),
        (1, Some(12))
    );
    assert_eq!(r.waiting.stragglers, ["keychain-review-2"]);

    assert_eq!(r.harness.stalls.len(), 1);
    let s = &r.harness.stalls[0];
    assert_eq!(
        (s.agent.as_str(), s.pattern, s.minutes),
        ("harbard-profile", "rm_variable", 60)
    );
    assert_eq!(r.harness.two_writers, 1);

    assert_eq!(r.messages.not_understood, 1);
    assert_eq!(r.messages.what_does_it_mean, 1);
    assert_eq!(r.names.names_down, 2);
    assert!(r.names.generated.contains(&"keychain-review-2".to_string()));
    assert!(r.names.english_descriptions.len() >= 5);
    assert_eq!(r.names.renamed, 1);
    assert!(
        !r.feedback.contains_key("name"),
        "name votes are the workshop's, not taps"
    );
    assert_eq!(r.daily.len(), 7);
    assert_eq!(r.daily.last().unwrap().day, "2026-11-07");
    assert_eq!(r.daily.last().unwrap().findings, 5);
}

#[test]
fn todays_patterns_suggest_the_three_rules() {
    let r = retro_of(Scope::Week);
    let rule = |key: &str| {
        r.lessons
            .iter()
            .find(|l| l.key == key)
            .map(|l| l.rule.clone())
    };
    assert_eq!(
        rule("shared_security_primitive").as_deref(),
        Some(
            "When two or more builders need the same security primitive, one owns it and the others reuse it"
        )
    );
    assert_eq!(
        rule("rm_variable").as_deref(),
        Some("A rm on a variable stalls on a prompt; use \"${VAR:?}\"")
    );
    for key in ["unanswered_operator", "readable_names", "background_wait"] {
        assert!(rule(key).is_some(), "{key}");
    }
    let shared = r
        .lessons
        .iter()
        .find(|l| l.key == "shared_security_primitive")
        .unwrap();
    assert!(
        shared.evidence.contains("3 secret leaks on 3 branches"),
        "{}",
        shared.evidence
    );
    assert!(
        shared.evidence.contains("2 file-read races on 2 branches"),
        "{}",
        shared.evidence
    );
    assert_eq!(shared.dates, ["2026-11-07"]);
    let rm = r.lessons.iter().find(|l| l.key == "rm_variable").unwrap();
    assert!(rm.evidence.contains("60 min"), "{}", rm.evidence);
    // Decided keys are not proposed again.
    let decided: BTreeSet<String> = ["rm_variable".to_string()].into_iter().collect();
    assert!(
        candidates(&r, &decided)
            .iter()
            .all(|l| l.key != "rm_variable")
    );
}

#[test]
fn a_retro_carries_no_message_text() {
    let wire = serde_json::to_string(&retro_of(Scope::Week)).unwrap();
    assert!(
        wire.contains("secret_leak") && wire.contains("rm_variable"),
        "a retro with its samples"
    );
    for secret in [
        "GITHUB_TOKEN",
        "contraseña aparece",
        "sluice es la cola",
        "¿Cómo va la revisión",
        "TOCTOU",
    ] {
        assert!(!wire.contains(secret), "{secret} leaked into the retro");
    }
}

#[test]
fn a_team_retro_covers_one_finished_team() {
    let team = today_team();
    assert_eq!(finished_teams(&team), [LEAD]);
    let r = retro_of(Scope::Team(OTHER_LEAD.into()));
    assert_eq!(r.agents, 1);
    assert!(
        r.delivery
            .findings
            .iter()
            .all(|f| f.count == 0 || f.class == "other")
    );
    assert!(r.harness.stalls.is_empty());
    let r = retro_of(Scope::Team(LEAD.into()));
    assert_eq!(
        (r.scope, r.lead.as_deref(), r.agents),
        ("team", Some(LEAD), 8)
    );
}
