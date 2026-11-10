//! Ported from the console's `lib/legible.ts`: a channel the operator can
//! read even when agents ignore the protocol.
//!
//! The protocol flags stay in the read for the Retro; a stream shows at most
//! one quiet grey marker, "written for machines", with the reasons in its
//! tooltip, and only on the first flagged message of a run of one author.
//! Runs of deliveries, reviews or blockers from one team fold into one line,
//! and agents' internal reports fold away in channels.
//!
//! The logic returns data; the words come from the small `*_text` / `label`
//! functions that take `&App`.

use crate::i18n::{serve_text, t, tv};
use crate::model::{Message, ReadablePart, WorkspaceView};
use gpui::{App, SharedString};
use std::collections::{HashMap, HashSet};

/// Why a message reads as written for machines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Unlabeled,
    TooLong,
    Repeat,
    Sha,
    ReviewCode,
    Word,
    /// A `jargon` flag from an older serve that sent no reasons.
    NotPlain,
}

impl Reason {
    fn key(self) -> &'static str {
        match self {
            Reason::Unlabeled => "activity.reasonUnlabeled",
            Reason::TooLong => "activity.reasonTooLong",
            Reason::Repeat => "activity.reasonRepeat",
            Reason::Sha => "activity.reasonSha",
            Reason::ReviewCode => "activity.reasonReviewCode",
            Reason::Word => "activity.reasonWord",
            Reason::NotPlain => "activity.reasonNotPlain",
        }
    }

    pub fn text(self, cx: &App) -> SharedString {
        t(cx, self.key())
    }
}

/// "written for machines".
pub fn machine_text(cx: &App) -> SharedString {
    t(cx, "activity.machineText")
}

/// One message's marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineMark {
    /// In the server's order.
    pub reasons: Vec<Reason>,
    /// Flagged messages of the same run after this one, which show no marker.
    pub more: usize,
}

impl MachineMark {
    /// The reasons in plain words.
    pub fn reason_texts(&self, cx: &App) -> Vec<SharedString> {
        self.reasons.iter().map(|r| r.text(cx)).collect()
    }

    /// The tooltip: the reasons, and how many more messages of the run share them.
    pub fn hint(&self, cx: &App) -> SharedString {
        let reasons = join_list(&self.reason_texts(cx), cx);
        let head = tv(cx, "activity.machineHint", &[("reasons", &reasons)]);
        match self.more {
            0 => head,
            1 => format!("{head} {}", t(cx, "activity.machineHintNextOne")).into(),
            n => format!(
                "{head} {}",
                tv(
                    cx,
                    "activity.machineHintNextMany",
                    &[("count", &n.to_string())]
                )
            )
            .into(),
        }
    }
}

/// "a, b and c".
fn join_list(items: &[SharedString], cx: &App) -> String {
    match items {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => {
            let head = rest
                .iter()
                .map(|s| s.as_ref())
                .collect::<Vec<&str>>()
                .join(", ");
            tv(cx, "activity.listJoin", &[("items", &head), ("last", last)]).to_string()
        }
    }
}

fn reasons_of(m: &Message) -> Vec<Reason> {
    let mut out = Vec::new();
    for f in &m.flags {
        match f.as_str() {
            "jargon" => {
                if m.jargon.is_empty() {
                    out.push(Reason::NotPlain);
                } else {
                    out.extend(m.jargon.iter().filter_map(|r| match r.as_str() {
                        "sha" => Some(Reason::Sha),
                        "review_code" => Some(Reason::ReviewCode),
                        "word" => Some(Reason::Word),
                        _ => None,
                    }));
                }
            }
            "unlabeled" => out.push(Reason::Unlabeled),
            "too_long" => out.push(Reason::TooLong),
            "repeat" => out.push(Reason::Repeat),
            _ => {}
        }
    }
    out
}

/// The marker of one message on its own (a thread pane, Later): None when clean.
pub fn machine_mark(m: &Message) -> Option<MachineMark> {
    if m.from == "operator" {
        return None;
    }
    let reasons = reasons_of(m);
    if reasons.is_empty() {
        return None;
    }
    Some(MachineMark { reasons, more: 0 })
}

/// The markers of a stream, by message id: one per run of one author, on its
/// first flagged message. A run ends when someone else writes.
pub fn machine_marks(messages: &[&Message]) -> HashMap<String, MachineMark> {
    let mut out = HashMap::new();
    let ids: HashSet<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    let mut run_author: Option<(&str, &str, &str)> = None;
    let mut run_mark: Option<(String, MachineMark)> = None;
    for m in messages {
        // A repeat is drawn under its first copy, not on its own.
        if m.repeat_of.as_deref().is_some_and(|r| ids.contains(r)) {
            continue;
        }
        let author = (m.from.as_str(), m.agent.as_str(), m.author.as_str());
        if run_author != Some(author) {
            if let Some((id, mark)) = run_mark.take() {
                out.insert(id, mark);
            }
            run_author = Some(author);
        }
        let Some(mark) = machine_mark(m) else {
            continue;
        };
        match &mut run_mark {
            Some((_, open)) => open.more += 1,
            None => run_mark = Some((m.id.clone(), mark)),
        }
    }
    if let Some((id, mark)) = run_mark {
        out.insert(id, mark);
    }
    out
}

/// One piece of a readable line: a harbard-worded one in this language.
pub fn readable_part_text(part: &ReadablePart, cx: &App) -> SharedString {
    serve_text(cx, part.code.as_deref(), part.params.as_ref(), &part.text)
}

/// A readable line as plain text, chips at their short form.
pub fn readable_text(parts: &[ReadablePart], cx: &App) -> String {
    readable_text_with(parts, |p| readable_part_text(p, cx).to_string())
}

fn readable_text_with(parts: &[ReadablePart], word: impl Fn(&ReadablePart) -> String) -> String {
    parts.iter().map(word).collect()
}

/// What "Technical details" holds: with a readable line shown in its place,
/// the original line first, then the body; otherwise the body alone.
pub fn tech_body(m: &Message) -> Option<String> {
    if m.readable.as_ref().is_some_and(|r| !r.is_empty()) {
        return Some(match &m.body {
            Some(body) if !body.is_empty() => format!("{}\n\n{body}", m.text),
            _ => m.text.clone(),
        });
    }
    m.body.clone().filter(|b| !b.is_empty())
}

/// An agent's report to the session that launched it, with no line written
/// for the operator: work chatter. What answers, names someone or carries a
/// file stays in the channel. A session's note full of jargon is
/// coordination between sessions, the same thing.
pub fn is_internal(m: &Message) -> bool {
    let to_lead = m.from == "agent" && m.flags.iter().any(|f| f == "unlabeled");
    let between_sessions = m.id.starts_with("bus:") && !m.jargon.is_empty();
    (to_lead || between_sessions)
        && m.replies.is_empty()
        && m.reply_to.is_none()
        && m.attachments.is_empty()
        && m.mentions.is_empty()
}

/// The fewest messages that make a run.
const RUN_MIN: usize = 3;
/// Older lines of a run shown before "Show all".
pub const RUN_SHOWN: usize = 3;

/// What a run is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunKind {
    Delivery,
    Review,
    Blocked,
}

impl RunKind {
    fn of(shape: &str) -> Option<RunKind> {
        match shape {
            "delivery" => Some(RunKind::Delivery),
            "review" => Some(RunKind::Review),
            "blocked" => Some(RunKind::Blocked),
            _ => None,
        }
    }
}

/// A stream as drawn: single messages, runs of one kind from one team, and
/// folded internal updates.
#[derive(Clone, Debug)]
pub enum StreamItem<'a> {
    Message(&'a Message),
    Run {
        id: String,
        kind: RunKind,
        /// Every message of the run, the newest included.
        count: usize,
        team: Option<String>,
        older: Vec<&'a Message>,
        newest: &'a Message,
    },
    Internal {
        id: String,
        messages: Vec<&'a Message>,
        /// Distinct author names, in order.
        names: Vec<String>,
        /// Up to four distinct agent keys, for the faces.
        faces: Vec<String>,
    },
}

/// "5 deliveries from acme", "3 reviews".
pub fn run_label(kind: RunKind, count: usize, team: Option<&str>, cx: &App) -> SharedString {
    let base = match kind {
        RunKind::Delivery => "activity.runDelivery",
        RunKind::Review => "activity.runReview",
        RunKind::Blocked => "activity.runBlocker",
    };
    let counted = crate::i18n::tn(cx, base, count as u64, &[]);
    match team {
        Some(team) => tv(
            cx,
            "activity.runOfTeam",
            &[("run", &counted), ("team", team)],
        ),
        None => counted,
    }
}

/// "5 internal updates · Naomi, Celia, Jason and 2 more".
pub fn internal_label(count: usize, names: &[String], cx: &App) -> SharedString {
    let shown = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    let who = if names.len() > 3 {
        tv(
            cx,
            "activity.namesAndMore",
            &[("names", &shown), ("count", &(names.len() - 3).to_string())],
        )
        .to_string()
    } else {
        shown
    };
    crate::i18n::tn(cx, "activity.internal", count as u64, &[("names", &who)])
}

/// Consecutive messages of one kind (deliveries, reviews, blocks) from one
/// team's agents collapse into one run: the older ones as a compact list,
/// the newest whole. A message with a thread or attachments, an operator
/// note, the lead or anything else ends the run; a repeat drawn under its
/// first copy neither joins nor ends it. With `fold_internal` (channels),
/// internal updates fold into one line.
pub fn runs<'a>(
    view: &WorkspaceView,
    messages: &[&'a Message],
    fold_internal: bool,
) -> Vec<StreamItem<'a>> {
    let ids: HashSet<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    let team_of = |m: &Message| view.agent(&m.agent).and_then(|a| a.team.clone());
    let key_of = |m: &Message| -> Option<(RunKind, Option<String>)> {
        if !matches!(m.from.as_str(), "agent" | "session" | "peer") {
            return None;
        }
        if !m.replies.is_empty() || m.reply_to.is_some() || !m.attachments.is_empty() {
            return None;
        }
        let kind = RunKind::of(m.shape.as_deref()?)?;
        Some((kind, team_of(m)))
    };
    let mut out: Vec<StreamItem<'a>> = Vec::new();
    let mut run: Vec<&'a Message> = Vec::new();
    let mut run_key: Option<(RunKind, Option<String>)> = None;
    let mut internal: Vec<&'a Message> = Vec::new();

    let flush_internal = |internal: &mut Vec<&'a Message>, out: &mut Vec<StreamItem<'a>>| {
        if internal.is_empty() {
            return;
        }
        let mut names: Vec<String> = Vec::new();
        let mut faces: Vec<String> = Vec::new();
        for m in internal.iter() {
            let name = view
                .agent(&m.agent)
                .map_or_else(|| m.author.clone(), |a| a.name.clone());
            if !names.contains(&name) {
                names.push(name);
            }
            if faces.len() < 4 && !faces.contains(&m.agent) {
                faces.push(m.agent.clone());
            }
        }
        out.push(StreamItem::Internal {
            id: format!("internal:{}", internal[0].id),
            messages: std::mem::take(internal),
            names,
            faces,
        });
    };
    let flush = |run: &mut Vec<&'a Message>,
                 run_key: &mut Option<(RunKind, Option<String>)>,
                 out: &mut Vec<StreamItem<'a>>| {
        if run.len() >= RUN_MIN {
            let (kind, team) = run_key.take().expect("a run has a key");
            let mut older = std::mem::take(run);
            let newest = older.pop().expect("a run has messages");
            out.push(StreamItem::Run {
                id: format!("run:{}", older[0].id),
                kind,
                count: older.len() + 1,
                team,
                older,
                newest,
            });
        } else {
            out.extend(run.drain(..).map(StreamItem::Message));
        }
        *run_key = None;
    };

    for &m in messages {
        if fold_internal && is_internal(m) {
            flush(&mut run, &mut run_key, &mut out);
            internal.push(m);
            continue;
        }
        flush_internal(&mut internal, &mut out);
        if m.repeat_of.as_deref().is_some_and(|r| ids.contains(r)) {
            // Drawn under its first copy: kept where it was, outside any run.
            if run.is_empty() {
                out.push(StreamItem::Message(m));
            }
            continue;
        }
        let key = key_of(m);
        if key.is_some() && key == run_key {
            run.push(m);
            continue;
        }
        flush(&mut run, &mut run_key, &mut out);
        match key {
            Some(k) => {
                run.push(m);
                run_key = Some(k);
            }
            None => out.push(StreamItem::Message(m)),
        }
    }
    flush(&mut run, &mut run_key, &mut out);
    flush_internal(&mut internal, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Agent;

    fn message(id: &str, author: &str) -> Message {
        Message {
            id: id.into(),
            agent: author.into(),
            from: "agent".into(),
            author: author.into(),
            text: format!("text {id}"),
            channels: vec!["acme".into()],
            ..Default::default()
        }
    }

    /// Three loud chips on every message.
    fn loud(mut m: Message) -> Message {
        m.flags = vec!["unlabeled".into(), "too_long".into(), "jargon".into()];
        m.jargon = vec!["sha".into(), "word".into()];
        m
    }

    #[test]
    fn three_chips_become_one_marker_with_the_reasons() {
        let mark = machine_mark(&loud(message("a", "cap146"))).expect("a mark");
        assert_eq!(
            mark.reasons,
            vec![
                Reason::Unlabeled,
                Reason::TooLong,
                Reason::Sha,
                Reason::Word
            ]
        );
        assert_eq!(mark.more, 0);
    }

    #[test]
    fn a_clean_message_gets_no_marker_and_an_old_serve_still_reads() {
        assert!(machine_mark(&message("a", "x")).is_none());
        let mut old = message("a", "x");
        old.flags = vec!["jargon".into()];
        assert_eq!(machine_mark(&old).unwrap().reasons, vec![Reason::NotPlain]);
        let mut rep = message("a", "x");
        rep.flags = vec!["repeat".into()];
        assert_eq!(machine_mark(&rep).unwrap().reasons, vec![Reason::Repeat]);
    }

    #[test]
    fn a_run_of_one_author_shows_the_marker_once() {
        let mut lead = message("6", "lead");
        lead.from = "lead".into();
        lead.author = "Claude · lead".into();
        let ms = [
            loud(message("1", "cap146")),
            loud(message("2", "cap146")),
            message("3", "cap146"),
            loud(message("4", "cap146")),
            loud(message("5", "cap143")),
            lead,
            loud(message("7", "cap143")),
        ];
        let refs: Vec<&Message> = ms.iter().collect();
        let marks = machine_marks(&refs);
        let mut keys: Vec<&str> = marks.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, vec!["1", "5", "7"]);
        assert_eq!(marks["1"].more, 2);
        assert_eq!(marks["5"].more, 0);
    }

    #[test]
    fn an_operator_note_is_never_marked() {
        let mut n = message("n", "dana");
        n.from = "operator".into();
        n.flags = vec!["too_long".into()];
        assert!(machine_marks(&[&n]).is_empty());
    }

    #[test]
    fn a_repeat_under_its_first_copy_does_not_count_in_the_run() {
        let first = loud(message("1", "a"));
        let mut again = loud(message("2", "a"));
        again.repeat_of = Some("1".into());
        again.flags.push("repeat".into());
        assert_eq!(machine_marks(&[&first, &again])["1"].more, 0);
    }

    #[test]
    fn the_original_line_goes_under_the_technical_details() {
        let mut m = message("a", "cap146");
        m.text = "CAP-146 is done: head acb3816f, pushed.".into();
        m.body = Some("Gates green.".into());
        m.readable = Some(vec![ReadablePart {
            kind: "text".into(),
            text: "CAP-146 terminado y subido".into(),
            ..Default::default()
        }]);
        assert_eq!(
            tech_body(&m).as_deref(),
            Some("CAP-146 is done: head acb3816f, pushed.\n\nGates green.")
        );
        m.body = None;
        assert_eq!(
            tech_body(&m).as_deref(),
            Some("CAP-146 is done: head acb3816f, pushed.")
        );
    }

    #[test]
    fn without_a_readable_line_the_details_are_the_body() {
        let mut m = message("a", "x");
        m.body = Some("detail".into());
        assert_eq!(tech_body(&m).as_deref(), Some("detail"));
        assert_eq!(tech_body(&message("a", "x")), None);
    }

    #[test]
    fn the_readable_line_as_plain_text() {
        let parts = vec![
            ReadablePart {
                kind: "text".into(),
                text: "Rebased onto robots ".into(),
                ..Default::default()
            },
            ReadablePart {
                kind: "sha".into(),
                text: "fcf628c".into(),
                full: Some("fcf628c0ab".into()),
                ..Default::default()
            },
        ];
        assert_eq!(
            readable_text_with(&parts, |p| p.text.clone()),
            "Rebased onto robots fcf628c"
        );
    }

    fn raw(id: &str, who: &str) -> Message {
        let mut m = message(id, who);
        m.flags = vec!["unlabeled".into(), "jargon".into()];
        m
    }

    fn kinds(items: &[StreamItem]) -> Vec<&'static str> {
        items
            .iter()
            .map(|i| match i {
                StreamItem::Message(_) => "message",
                StreamItem::Run { .. } => "run",
                StreamItem::Internal { .. } => "internal",
            })
            .collect()
    }

    #[test]
    fn internal_reports_fold_together_across_authors() {
        let v = WorkspaceView::default();
        let ms = [raw("1", "Naomi"), raw("2", "Celia"), raw("3", "Naomi")];
        let refs: Vec<&Message> = ms.iter().collect();
        let items = runs(&v, &refs, true);
        assert_eq!(kinds(&items), vec!["internal"]);
        let StreamItem::Internal {
            messages,
            names,
            faces,
            id,
        } = &items[0]
        else {
            unreachable!()
        };
        assert_eq!(messages.len(), 3);
        assert_eq!(names, &vec!["Naomi".to_string(), "Celia".to_string()]);
        assert_eq!(faces, &vec!["Naomi".to_string(), "Celia".to_string()]);
        assert_eq!(id, "internal:1");
    }

    #[test]
    fn a_message_written_for_them_splits_internal_groups() {
        let v = WorkspaceView::default();
        let mut told = message("2", "Celia");
        told.label = Some("DONE".into());
        let ms = [raw("1", "Naomi"), told, raw("3", "Naomi")];
        let refs: Vec<&Message> = ms.iter().collect();
        assert_eq!(
            kinds(&runs(&v, &refs, true)),
            vec!["internal", "message", "internal"]
        );
    }

    #[test]
    fn what_names_someone_answers_or_carries_a_file_is_not_internal() {
        let mut a = raw("1", "a");
        a.mentions = vec!["dana".into()];
        assert!(!is_internal(&a));
        let mut b = raw("1", "a");
        b.replies = vec!["r".into()];
        assert!(!is_internal(&b));
        let mut c = raw("1", "a");
        c.from = "lead".into();
        assert!(!is_internal(&c));
        assert!(!is_internal(&message("1", "a")));
    }

    #[test]
    fn a_sessions_note_with_jargon_folds() {
        let mut a = message("bus:1", "a");
        a.from = "lead".into();
        a.jargon = vec!["word".into()];
        assert!(is_internal(&a));
        let mut b = message("bus:2", "a");
        b.from = "lead".into();
        assert!(!is_internal(&b));
    }

    #[test]
    fn a_direct_conversation_keeps_every_message() {
        let v = WorkspaceView::default();
        let ms = [raw("1", "a"), raw("2", "a")];
        let refs: Vec<&Message> = ms.iter().collect();
        assert_eq!(kinds(&runs(&v, &refs, false)), vec!["message", "message"]);
    }

    fn team_view() -> WorkspaceView {
        let agent = |key: &str, team: Option<&str>| Agent {
            key: key.into(),
            name: key.into(),
            team: team.map(Into::into),
            ..Default::default()
        };
        WorkspaceView {
            agents: vec![
                agent("cap82", Some("acme")),
                agent("cap150", Some("acme")),
                agent("cap149", Some("acme")),
                agent("cap52", Some("acme")),
                agent("cap86", Some("acme")),
                agent("rev", Some("acme")),
                agent("x", Some("harbard")),
                agent("a", None),
            ],
            ..Default::default()
        }
    }

    fn done(id: &str, who: &str) -> Message {
        let mut m = message(id, who);
        m.shape = Some("delivery".into());
        m
    }

    #[test]
    fn five_deliveries_of_one_team_fold_into_one_line() {
        let v = team_view();
        let ms = [
            message("0", "rev"),
            done("1", "cap82"),
            done("2", "cap150"),
            done("3", "cap149"),
            done("4", "cap52"),
            done("5", "cap86"),
            message("6", "rev"),
        ];
        let refs: Vec<&Message> = ms.iter().collect();
        let items = runs(&v, &refs, false);
        assert_eq!(kinds(&items), vec!["message", "run", "message"]);
        let StreamItem::Run {
            kind,
            count,
            team,
            older,
            newest,
            id,
        } = &items[1]
        else {
            unreachable!()
        };
        assert_eq!(
            (*kind, *count, team.as_deref()),
            (RunKind::Delivery, 5, Some("acme"))
        );
        assert_eq!(
            older.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["1", "2", "3", "4"]
        );
        assert_eq!(newest.id, "5");
        assert_eq!(id, "run:1");
    }

    #[test]
    fn a_run_needs_three_and_breaks_on_team_kind_or_thread() {
        let v = team_view();
        let two = [done("1", "cap82"), done("2", "cap150")];
        assert_eq!(
            kinds(&runs(&v, &two.iter().collect::<Vec<_>>(), false)),
            vec!["message", "message"]
        );
        let mut review = message("5", "cap52");
        review.shape = Some("review".into());
        let mixed = [
            done("1", "cap82"),
            done("2", "cap150"),
            done("3", "x"),
            done("4", "cap149"),
            review,
        ];
        assert!(
            runs(&v, &mixed.iter().collect::<Vec<_>>(), false)
                .iter()
                .all(|i| matches!(i, StreamItem::Message(_)))
        );
        let mut threaded = done("2", "cap150");
        threaded.replies = vec!["r".into()];
        let ms = [
            done("1", "cap82"),
            threaded,
            done("3", "cap149"),
            done("4", "cap52"),
        ];
        assert_eq!(
            kinds(&runs(&v, &ms.iter().collect::<Vec<_>>(), false)),
            vec!["message"; 4]
        );
    }

    #[test]
    fn reviews_name_their_team_and_an_operator_note_never_joins() {
        let v = team_view();
        let rv = |id: &str| {
            let mut m = message(id, "rev");
            m.shape = Some("review".into());
            m
        };
        let ms = [rv("1"), rv("2"), rv("3")];
        let items = runs(&v, &ms.iter().collect::<Vec<_>>(), false);
        let StreamItem::Run { kind, team, .. } = &items[0] else {
            unreachable!()
        };
        assert_eq!((*kind, team.as_deref()), (RunKind::Review, Some("acme")));
        let lone = [done("1", "a"), done("2", "a"), done("3", "a")];
        let items = runs(&v, &lone.iter().collect::<Vec<_>>(), false);
        let StreamItem::Run { team, .. } = &items[0] else {
            unreachable!()
        };
        assert_eq!(team, &None);
        let mut note = done("n", "cap82");
        note.from = "operator".into();
        let ms = [
            done("1", "cap82"),
            note,
            done("2", "cap150"),
            done("3", "cap149"),
        ];
        assert_eq!(
            kinds(&runs(&v, &ms.iter().collect::<Vec<_>>(), false)),
            vec!["message"; 4]
        );
    }

    #[test]
    fn a_repeat_neither_joins_nor_breaks_a_run() {
        let v = team_view();
        let mut again = done("1b", "cap82");
        again.repeat_of = Some("1".into());
        let ms = [
            done("1", "cap82"),
            again,
            done("2", "cap150"),
            done("3", "cap149"),
        ];
        let items = runs(&v, &ms.iter().collect::<Vec<_>>(), false);
        assert_eq!(kinds(&items), vec!["run"]);
        let StreamItem::Run { older, .. } = &items[0] else {
            unreachable!()
        };
        assert_eq!(
            older.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["1", "2"]
        );
    }
}
