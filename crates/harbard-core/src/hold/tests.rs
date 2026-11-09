use super::*;
use crate::profile::Profile;
use crate::test_util::TempHome;
use serde_json::json;

const NOW: u64 = 1_793_000_000_000;
const MIN: u64 = 60_000;

/// A note at `ms`, from an agent session, to `to` (`harness:human` when None).
fn note(ms: u64, n: u128, kind: &str, to: Option<(&str, &str)>, text: &str) -> Event {
    let (k, v) = to.unwrap_or(("harness", "human"));
    serde_json::from_value(json!({
        "v": 1,
        "id": ulid::Ulid::from_parts(ms, n).to_string(),
        "ts": crate::clock::rfc3339_utc(ms),
        "event": "note",
        "session": "s-agent",
        "harness": "claude-code",
        "host": "mac",
        "cwd": "/w",
        "text": text,
        "to": {"kind": k, "value": v},
        "kind": kind,
    }))
    .unwrap()
}

fn in_state(id: &str, since: u64) -> Profile {
    let mut p = Profile::default();
    p.apply(&json!({"state": {"id": id}}), since).unwrap();
    p
}

fn traffic() -> Vec<Event> {
    vec![
        note(NOW + MIN, 1, "fyi", None, "Themes installed · checked"),
        note(NOW + 2 * MIN, 2, "ask", None, "Large or small size?"),
        note(NOW + 3 * MIN, 3, "warning", None, "The branch has drifted"),
        note(NOW + 4 * MIN, 4, "handoff", None, "Review PR 12"),
        // Not to the human: never held, never in a digest.
        note(
            NOW + 5 * MIN,
            5,
            "fyi",
            Some(("session", "s-other")),
            "for another session",
        ),
    ]
}

fn kinds(d: &Digest) -> Vec<&str> {
    d.items.iter().map(|i| i.kind.as_str()).collect()
}

#[test]
fn each_state_holds_what_it_says() {
    let t = traffic();
    let held = |state: &str| -> Vec<&str> {
        t.iter()
            .filter(|e| holds(state, e))
            .map(|e| e.kind.as_str())
            .collect()
    };
    assert_eq!(held("available"), Vec::<&str>::new(), "delivered normally");
    assert_eq!(
        held("focused"),
        vec!["fyi", "ask"],
        "non-blocking fyi and ask"
    );
    assert_eq!(
        held("swamped"),
        vec!["fyi", "ask"],
        "held as in Concentrado"
    );
    assert_eq!(
        held("away"),
        vec!["fyi", "ask", "warning", "handoff"],
        "queued"
    );
    assert_eq!(held("reviewing"), Vec::<&str>::new(), "the queue flushes");
    assert_eq!(
        held("done"),
        vec!["fyi", "ask", "warning", "handoff"],
        "queued"
    );
}

#[test]
fn leaving_a_holding_state_delivers_one_digest_of_what_it_held() {
    for (state, expected) in [
        ("focused", vec!["fyi", "ask"]),
        ("swamped", vec!["fyi", "ask"]),
        ("away", vec!["fyi", "ask", "warning", "handoff"]),
        ("done", vec!["fyi", "ask", "warning", "handoff"]),
    ] {
        let home = TempHome::new();
        let before = in_state(state, NOW);
        let after = Profile::default();
        let d = on_state_change(home.path(), &traffic(), &before, &after, NOW + 10 * MIN)
            .unwrap()
            .unwrap_or_else(|| panic!("{state}: a digest"));
        assert_eq!(kinds(&d), expected, "{state}");
        assert_eq!((d.state.as_str(), d.reason.as_str()), (state, "left"));
        assert_eq!((d.from_ms, d.to_ms), (NOW, NOW + 10 * MIN));
        assert_eq!(
            Digests::load(home.path()).items.len(),
            1,
            "{state}: stored once"
        );
    }
}

#[test]
fn available_and_reviewing_hold_nothing_so_leaving_them_makes_no_digest() {
    for state in ["available", "reviewing"] {
        let home = TempHome::new();
        let before = in_state(state, NOW);
        let d = on_state_change(
            home.path(),
            &traffic(),
            &before,
            &in_state("away", NOW + 9 * MIN),
            NOW + 10 * MIN,
        )
        .unwrap();
        assert!(d.is_none(), "{state}");
        assert!(Digests::load(home.path()).items.is_empty());
    }
}

#[test]
fn reviewing_flushes_the_held_queue_at_once() {
    let home = TempHome::new();
    let d = on_state_change(
        home.path(),
        &traffic(),
        &in_state("focused", NOW),
        &in_state("reviewing", NOW + 10 * MIN),
        NOW + 10 * MIN,
    )
    .unwrap()
    .expect("switching to Revisando flushes");
    assert_eq!(d.reason, "reviewing");
    assert_eq!(kinds(&d), vec!["fyi", "ask"]);
}

#[test]
fn a_held_note_is_delivered_once_and_an_empty_hold_makes_no_digest() {
    let home = TempHome::new();
    let none = on_state_change(
        home.path(),
        &[],
        &in_state("focused", NOW),
        &Profile::default(),
        NOW + MIN,
    )
    .unwrap();
    assert!(none.is_none(), "nothing held: no empty digest");
    // An hourly digest, then leaving: the second carries only what was
    // appended after the first.
    let t = traffic();
    let p = {
        let mut p = in_state("focused", NOW);
        p.apply(&json!({"digest_hourly": true}), NOW).unwrap();
        p
    };
    let first = hourly(home.path(), &t, &p, NOW + 61 * MIN)
        .unwrap()
        .expect("an hour passed");
    assert_eq!(first.reason, "hourly");
    assert_eq!(kinds(&first), vec!["fyi", "ask"]);
    assert!(
        hourly(home.path(), &t, &p, NOW + 70 * MIN)
            .unwrap()
            .is_none(),
        "not again within the hour"
    );
    let mut later = t.clone();
    later.push(note(NOW + 65 * MIN, 9, "ask", None, "Continue?"));
    let second = on_state_change(home.path(), &later, &p, &Profile::default(), NOW + 80 * MIN)
        .unwrap()
        .unwrap();
    assert_eq!(second.from_ms, NOW + 61 * MIN);
    assert_eq!(
        second
            .items
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Continue?"]
    );
}

#[test]
fn hourly_digests_only_when_asked_for() {
    let home = TempHome::new();
    let p = in_state("focused", NOW);
    assert!(
        hourly(home.path(), &traffic(), &p, NOW + 120 * MIN)
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_digest_keeps_a_bounded_first_line_and_who_sent_it() {
    let long = format!("{}\nsecond line", "x".repeat(400));
    let t = vec![note(NOW + MIN, 1, "fyi", None, &long)];
    let home = TempHome::new();
    let d = on_state_change(
        home.path(),
        &t,
        &in_state("away", NOW),
        &Profile::default(),
        NOW + 2 * MIN,
    )
    .unwrap()
    .unwrap();
    let item = &d.items[0];
    assert!(item.text.chars().count() <= ITEM_TEXT_CHARS + 1 && !item.text.contains('\n'));
    assert_eq!(
        (item.session.as_str(), item.harness.as_str()),
        ("s-agent", "claude-code")
    );
}

#[test]
fn digests_are_read_once_and_capped() {
    let home = TempHome::new();
    for i in 0..(DIGESTS_MAX as u64 + 5) {
        on_state_change(
            home.path(),
            &[note(NOW + i * 10 * MIN + 1, 1, "fyi", None, "x")],
            &in_state("away", NOW + i * 10 * MIN),
            &Profile::default(),
            NOW + i * 10 * MIN + 2,
        )
        .unwrap();
    }
    let all = Digests::load(home.path());
    assert_eq!(all.items.len(), DIGESTS_MAX);
    let id = all.items.last().unwrap().id.clone();
    assert!(mark_read(home.path(), &id).unwrap());
    assert!(
        Digests::load(home.path())
            .items
            .iter()
            .any(|d| d.id == id && d.read)
    );
    assert!(!mark_read(home.path(), "nope").unwrap());
}

#[test]
fn what_a_note_to_the_human_is_told_when_it_is_held() {
    let fyi = note(NOW, 1, "fyi", None, "x");
    let to_other = note(NOW, 2, "fyi", Some(("session", "s2")), "x");
    assert_eq!(
        held_receipt("focused", &fyi),
        Some("held for the operator's digest: Focused")
    );
    assert_eq!(
        held_receipt("away", &fyi),
        Some("queued for the operator's return: Away")
    );
    assert_eq!(held_receipt("available", &fyi), None);
    assert_eq!(held_receipt("focused", &to_other), None);
    let warning = note(NOW, 3, "warning", None, "x");
    assert_eq!(
        held_receipt("focused", &warning),
        None,
        "a warning still reaches them in Focused"
    );
}

#[test]
fn what_is_held_so_far_can_be_read_without_cutting_a_digest() {
    let home = TempHome::new();
    let p = in_state("away", NOW);
    let so_far = pending(home.path(), &traffic(), &p, NOW + 10 * MIN);
    assert_eq!(
        so_far.iter().map(|i| i.kind.as_str()).collect::<Vec<_>>(),
        vec!["fyi", "ask", "warning", "handoff"]
    );
    assert!(
        Digests::load(home.path()).items.is_empty(),
        "reading is not delivering"
    );
    assert!(
        pending(home.path(), &traffic(), &Profile::default(), NOW + 10 * MIN).is_empty(),
        "Disponible holds nothing"
    );
    // After a cut, only what came later.
    on_state_change(
        home.path(),
        &traffic(),
        &p,
        &Profile::default(),
        NOW + 10 * MIN,
    )
    .unwrap();
    let again = in_state("away", NOW + 10 * MIN);
    assert!(pending(home.path(), &traffic(), &again, NOW + 20 * MIN).is_empty());
}

// A state chosen with an end ("Focused until
// 18:30") holds the notes while it lasts, then lapses on its own. No request
// carries `state`, so `on_state_change` never runs; after the end
// `state_at` is Available, so `hourly` and `pending` hold nothing and no
// digest is ever cut. The agents were told "held for the operator's digest".
// Expected: the notes it held are delivered in a digest (or still pending).
#[test]
fn a_state_that_lapses_delivers_what_it_held() {
    let home = TempHome::new();
    let mut p = Profile::default();
    p.apply(
        &json!({"state": {"id": "focused", "until_ms": NOW + 60 * MIN}}),
        NOW,
    )
    .unwrap();
    p.digest_hourly = true;
    let t = traffic();
    assert_eq!(
        pending(home.path(), &t, &p, NOW + 30 * MIN).len(),
        2,
        "fyi+ask held"
    );
    let after = NOW + 120 * MIN;
    // Whatever runs after the end: the hourly tick, the pending read, or the
    // next state change (to Away).
    let hourly_cut = hourly(home.path(), &t, &p, after).unwrap();
    let still = pending(home.path(), &t, &p, after);
    let next = in_state("away", after);
    let on_change = on_state_change(home.path(), &t, &p, &next, after).unwrap();
    let delivered = hourly_cut.map_or(0, |d| d.items.len())
        + on_change.map_or(0, |d| d.items.len())
        + still.len();
    assert_eq!(
        delivered,
        2,
        "the two notes Concentrado held were never delivered: {:?}",
        Digests::load(home.path())
    );
}

// The cut is by timestamp, not by what was read. A note
// written with ts T-1min but appended after serve read the log at T (a slow
// writer, or a satellite's event arriving late through POST /events) falls
// before `cut_ms` for every later digest.
// Expected: delivered in the next digest. Actual: never.
#[test]
fn a_note_appended_after_the_cut_with_an_earlier_ts_is_delivered() {
    let home = TempHome::new();
    let away = in_state("away", NOW);
    let cut_at = NOW + 10 * MIN;
    let late = note(cut_at - MIN, 9, "fyi", None, "Reached the log late");
    // The first cut read the log before the late note landed.
    let before_append = traffic();
    let conc = in_state("focused", cut_at);
    let first = on_state_change(home.path(), &before_append, &away, &conc, cut_at).unwrap();
    assert!(first.is_some());
    let mut after_append = traffic();
    after_append.push(late.clone());
    let back = Profile::default();
    let second =
        on_state_change(home.path(), &after_append, &conc, &back, cut_at + 10 * MIN).unwrap();
    let seen = [first, second]
        .iter()
        .flatten()
        .flat_map(|d| d.items.iter())
        .any(|i| i.id == late.id);
    assert!(
        seen,
        "the late note is in no digest: {:?}",
        Digests::load(home.path())
    );
}

#[test]
fn a_lapse_is_cut_once_and_only_with_what_came_before_its_end() {
    let home = TempHome::new();
    let mut p = Profile::default();
    p.apply(
        &json!({"state": {"id": "away", "until_ms": NOW + 10 * MIN}}),
        NOW,
    )
    .unwrap();
    let mut t = traffic();
    // After the end they are Available: this one rang normally.
    t.push(note(NOW + 20 * MIN, 8, "fyi", None, "ya disponible"));
    assert!(!lapse_due(home.path(), &p, NOW + 5 * MIN), "not yet");
    assert!(lapse_due(home.path(), &p, NOW + 30 * MIN));
    let d = settle_lapse(home.path(), &t, &p, NOW + 30 * MIN)
        .unwrap()
        .expect("the end is leaving it");
    assert_eq!((d.state.as_str(), d.reason.as_str()), ("away", "left"));
    assert_eq!(kinds(&d), vec!["fyi", "ask", "warning", "handoff"]);
    assert_eq!(d.to_ms, NOW + 10 * MIN, "it ended at its end");
    assert!(!lapse_due(home.path(), &p, NOW + 31 * MIN));
    assert!(
        settle_lapse(home.path(), &t, &p, NOW + 31 * MIN)
            .unwrap()
            .is_none(),
        "once"
    );
    // The next state change does not cut it again.
    let next = in_state("focused", NOW + 40 * MIN);
    assert!(
        on_state_change(home.path(), &t, &p, &next, NOW + 40 * MIN)
            .unwrap()
            .is_none()
    );
    assert_eq!(Digests::load(home.path()).items.len(), 1);
}

#[test]
fn a_state_change_after_a_lapse_cuts_the_lapse_first() {
    let home = TempHome::new();
    let mut p = Profile::default();
    p.apply(
        &json!({"state": {"id": "focused", "until_ms": NOW + 10 * MIN}}),
        NOW,
    )
    .unwrap();
    let next = in_state("away", NOW + 60 * MIN);
    let d = on_state_change(home.path(), &traffic(), &p, &next, NOW + 60 * MIN)
        .unwrap()
        .expect("the lapse, delivered at the next click");
    assert_eq!(kinds(&d), vec!["fyi", "ask"]);
    // Away starts where the log was: nothing it holds yet.
    assert!(pending(home.path(), &traffic(), &next, NOW + 61 * MIN).is_empty());
}

#[test]
fn entering_a_state_from_available_starts_where_the_log_is() {
    let home = TempHome::new();
    // Available: these reached them normally.
    let t = traffic();
    let conc = in_state("focused", NOW + 10 * MIN);
    on_state_change(home.path(), &t, &Profile::default(), &conc, NOW + 10 * MIN).unwrap();
    let mut later = t.clone();
    later.push(note(NOW + 15 * MIN, 9, "fyi", None, "en Concentrado"));
    let d = on_state_change(
        home.path(),
        &later,
        &conc,
        &Profile::default(),
        NOW + 20 * MIN,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        d.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
        vec!["en Concentrado"]
    );
}
