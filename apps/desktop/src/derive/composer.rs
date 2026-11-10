//! The composer's derivations from `lib/workspace.ts` and `lib/invisible.ts`:
//! mentions, the reply body, refusals and what the composer says before or
//! instead of its box.
//!
//! Wording goes through [`Words`], which the app (`App`, any `Context`)
//! implements with the dictionaries; the tests implement it with the English
//! dictionaries alone.

use crate::chat::ComposeTarget;
use crate::model::{Agent, Channel, WorkspaceView};
use crate::ui::format::duration;
use gpui::{App, Context};
use serde_json::{Value, json};
use std::cmp::Ordering;

/// Where the composer's words come from.
pub trait Words {
    /// `ns.key` with its holes filled.
    fn tv(&self, key: &str, vars: &[(&str, &str)]) -> String;
    /// An agent's name as shown (`agentName`).
    fn agent_name(&self, a: &Agent) -> String;
    /// An agent's role line as shown (`agentRoleLine`), empty when none.
    fn role_line(&self, a: &Agent) -> String;
}

impl Words for App {
    fn tv(&self, key: &str, vars: &[(&str, &str)]) -> String {
        crate::i18n::tv(self, key, vars).to_string()
    }

    fn agent_name(&self, a: &Agent) -> String {
        crate::derive::agent_name(a, self).to_string()
    }

    fn role_line(&self, a: &Agent) -> String {
        let fallback = a.role_line.clone().unwrap_or_default();
        crate::i18n::serve_text(
            self,
            a.role_line_code.as_deref(),
            a.role_line_params.as_ref(),
            &fallback,
        )
        .to_string()
    }
}

impl<V> Words for Context<'_, V> {
    fn tv(&self, key: &str, vars: &[(&str, &str)]) -> String {
        let app: &App = self;
        app.tv(key, vars)
    }

    fn agent_name(&self, a: &Agent) -> String {
        let app: &App = self;
        app.agent_name(a)
    }

    fn role_line(&self, a: &Agent) -> String {
        let app: &App = self;
        app.role_line(a)
    }
}

// ---------- mentions ----------

/// What an `@` can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MentionKind {
    Agent,
    Team,
    Channel,
}

/// One line of the `@` list.
#[derive(Clone, Debug, PartialEq)]
pub struct MentionOption {
    /// What goes after the `@`.
    pub handle: String,
    pub label: String,
    pub hint: String,
    pub kind: MentionKind,
}

/// The `@word` being typed: where its `@` is (a byte offset) and the word
/// so far, lowercased.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionAt {
    pub start: usize,
    pub query: String,
}

fn handle_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// The largest char boundary at or before `i`.
fn floor_boundary(text: &str, i: usize) -> usize {
    let mut i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The `@word` being typed at `caret` (a byte offset), if any: the text
/// before the caret ends in `@word` that starts the text or follows a space
/// or `(` (`mentionAt`).
pub fn mention_at(text: &str, caret: usize) -> Option<MentionAt> {
    let before = &text[..floor_boundary(text, caret)];
    let word_start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| handle_char(*c))
        .last()
        .map_or(before.len(), |(i, _)| i);
    let at = before[..word_start].strip_suffix('@')?;
    if let Some(prev) = at.chars().next_back()
        && !(prev.is_whitespace() || prev == '(')
    {
        return None;
    }
    Some(MentionAt {
        start: at.len(),
        query: before[word_start..].to_lowercase(),
    })
}

/// A collation close enough to `localeCompare(…, "es")`: case and accents
/// folded first, the raw text breaking ties.
fn collate(a: &str, b: &str) -> Ordering {
    let key = |s: &str| -> String {
        s.chars()
            .flat_map(char::to_lowercase)
            .map(|c| match c {
                'á' | 'à' | 'ä' | 'â' => 'a',
                'é' | 'è' | 'ë' | 'ê' => 'e',
                'í' | 'ì' | 'ï' | 'î' => 'i',
                'ó' | 'ò' | 'ö' | 'ô' => 'o',
                'ú' | 'ù' | 'ü' | 'û' => 'u',
                other => other,
            })
            .collect()
    };
    key(a).cmp(&key(b)).then_with(|| a.cmp(b))
}

/// What an `@` can name here: `@channel` in a channel, teams of two or more
/// agents, then agents by display name (`mentionOptions`). Temporary and
/// closed agents, and agents without a handle, are not offered.
pub fn mention_options(
    view: &WorkspaceView,
    channel: Option<&str>,
    w: &impl Words,
) -> Vec<MentionOption> {
    mention_options_of(&view.agents, channel, w)
}

fn mention_options_of(
    agents: &[Agent],
    channel: Option<&str>,
    w: &impl Words,
) -> Vec<MentionOption> {
    let agents: Vec<&Agent> = agents
        .iter()
        .filter(|a| !a.temporary && !a.closed && a.handle.as_deref().is_some_and(|h| !h.is_empty()))
        .collect();
    let mut out = Vec::new();
    if let Some(channel) = channel {
        out.push(MentionOption {
            handle: "channel".into(),
            label: "@channel".into(),
            hint: w.tv("workspace.mentionChannelHint", &[("channel", channel)]),
            kind: MentionKind::Channel,
        });
    }
    let mut teams: std::collections::BTreeMap<String, usize> = Default::default();
    for a in &agents {
        if let Some(team) = &a.team
            && a.kind != "session"
        {
            *teams.entry(team.to_lowercase()).or_default() += 1;
        }
    }
    for (team, n) in teams {
        if n > 1 {
            out.push(MentionOption {
                label: format!("@{team}"),
                hint: w.tv("workspace.mentionTeamHint", &[("n", &n.to_string())]),
                handle: team,
                kind: MentionKind::Team,
            });
        }
    }
    let mut sorted = agents;
    sorted.sort_by(|x, y| collate(&x.name, &y.name));
    for a in sorted {
        let handle = a.handle.clone().unwrap_or_default();
        let role = if a.role_line.is_some() {
            w.role_line(a)
        } else {
            String::new()
        };
        out.push(MentionOption {
            label: w.agent_name(a),
            hint: if a.role_line.is_some() {
                format!("{role} · @{handle}")
            } else {
                format!("@{handle}")
            },
            handle,
            kind: MentionKind::Agent,
        });
    }
    out
}

/// The options that match what is typed after the `@`, at most eight
/// (`matchMentions`).
pub fn match_mentions(options: &[MentionOption], query: &str) -> Vec<MentionOption> {
    let q = query.to_lowercase();
    options
        .iter()
        .filter(|o| q.is_empty() || o.handle.contains(&q) || o.label.to_lowercase().contains(&q))
        .take(8)
        .cloned()
        .collect()
}

/// The text with the `@word` from `start` to `caret` replaced by
/// `@handle `, and the caret after it (`applyMention`).
pub fn apply_mention(text: &str, start: usize, caret: usize, handle: &str) -> (String, usize) {
    let start = floor_boundary(text, start);
    let caret = floor_boundary(text, caret).max(start);
    let insert = format!("@{handle} ");
    let out = format!("{}{insert}{}", &text[..start], &text[caret..]);
    (out, start + insert.len())
}

/// The + menu's "Mention": an `@` at `at`, after a space when the text
/// before it does not end in one. Returns the text and the caret.
pub fn insert_at_sign(text: &str, at: usize) -> (String, usize) {
    let at = floor_boundary(text, at);
    let lead = if at > 0 && !text[..at].ends_with(char::is_whitespace) {
        " @"
    } else {
        "@"
    };
    (
        format!("{}{lead}{}", &text[..at], &text[at..]),
        at + lead.len(),
    )
}

// ---------- sending ----------

/// What is sent: the trimmed text, then each uploaded file's path on its
/// own line; empty when there is nothing to send.
pub fn full_body(text: &str, uploads: &[String]) -> String {
    std::iter::once(text.trim())
        .chain(uploads.iter().map(String::as_str))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body of `POST workspace/reply` (`replyBody`): to a channel, to an
/// agent, or in a thread (`reply_to`); `anyway` after "Send anyway".
pub fn reply_body(target: &ComposeTarget, text: &str, anyway: bool) -> Value {
    let mut body = match target {
        ComposeTarget::Channel(channel) => json!({ "channel": channel, "text": text }),
        ComposeTarget::Agent(agent) => json!({ "agent": agent, "text": text }),
        ComposeTarget::Thread {
            parent,
            channel,
            agent,
        } => {
            let mut b = json!({ "reply_to": parent, "text": text });
            if let Some(agent) = agent {
                b["agent"] = json!(agent);
            } else if let Some(channel) = channel {
                b["channel"] = json!(channel);
            }
            b
        }
    };
    if anyway {
        body["anyway"] = json!(true);
    }
    body
}

/// "Ask for a review from…": the ask, then the uploaded paths.
pub fn review_text(text: &str, uploads: &[String], w: &impl Words) -> String {
    let text = text.trim();
    let ask = if text.is_empty() {
        w.tv("messages.reviewRequestLatest", &[])
    } else {
        w.tv("messages.reviewRequest", &[("text", text)])
    };
    std::iter::once(ask)
        .chain(uploads.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Why serve refused a reply, in the composer's words (`replyError`).
/// `status` is `None` when serve could not be reached.
pub fn reply_error(status: Option<u16>, reason: Option<&str>, w: &impl Words) -> String {
    match status {
        Some(409) if reason == Some("no_reachable_member") => {
            w.tv("workspace.replyNoChannelReader", &[])
        }
        Some(409) => w.tv("workspace.noDelivery", &[]),
        Some(422) => w.tv("workspace.replyTooLong", &[]),
        _ => w.tv("workspace.replySaveFailed", &[]),
    }
}

/// serve's refusal of characters nobody sees, in this language with the
/// code points it named, or `None` for any other message
/// (`invisibleRefusal`).
pub fn invisible_refusal(message: &str, w: &impl Words) -> Option<String> {
    if !message.contains("characters you cannot see") {
        return None;
    }
    let bytes = message.as_bytes();
    let mut found: Vec<&str> = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'U' && bytes[i + 1] == b'+' {
            let hex = bytes[i + 2..]
                .iter()
                .take(6)
                .take_while(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(*b))
                .count();
            if hex >= 4 {
                found.push(&message[i..i + 2 + hex]);
                i += 2 + hex;
                continue;
            }
        }
        i += 1;
    }
    let list = if found.is_empty() {
        String::new()
    } else {
        format!(" ({})", found.join(", "))
    };
    Some(w.tv("messages.invisibleRefusal", &[("list", &list)]))
}

// ---------- what the composer says ----------

/// What a composer says before or instead of its box.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposerGate {
    /// Shown instead of the box ("… ."), no input.
    pub blocked: Option<String>,
    /// Shown above the box: the note is stored and waits.
    pub notice: Option<String>,
    /// Asked before sending ("send anyway?").
    pub confirm: Option<String>,
}

/// When every lead the composer would reach is stale: the note is stored
/// and waits (`composerNotice`).
pub fn composer_notice(agents: &[&Agent], w: &impl Words) -> Option<String> {
    if agents.is_empty() || agents.iter().any(|a| a.delivery == "ready") {
        return None;
    }
    let quiet = agents
        .iter()
        .filter(|a| a.delivery == "stale")
        .map(|a| a.lead_quiet_ms.unwrap_or(0))
        .min()?;
    Some(w.tv("workspace.composerStale", &[("time", &duration(quiet))]))
}

/// Instead of a box, only when there is no session at all to leave the
/// note for (`composerBlock`).
pub fn composer_block(agents: &[&Agent], w: &impl Words) -> Option<String> {
    agents.is_empty().then(|| w.tv("workspace.noReader", &[]))
}

/// The question before sending when no session would read it, now or when
/// it comes back (`composerConfirm`).
pub fn composer_confirm(agents: &[&Agent], w: &impl Words) -> Option<String> {
    if agents.is_empty()
        || agents
            .iter()
            .any(|a| a.delivery == "ready" || a.delivery == "stale")
    {
        return None;
    }
    Some(w.tv("workspace.nobodyLive", &[]))
}

/// A channel's composer, from serve's `reach` (`channelComposer`).
pub fn channel_composer(channel: &Channel, w: &impl Words) -> ComposerGate {
    match channel.reach.as_deref() {
        Some("stale") => ComposerGate {
            notice: Some(w.tv("workspace.channelStale", &[])),
            ..Default::default()
        },
        Some("none") => ComposerGate {
            confirm: Some(w.tv("workspace.nobodyLive", &[])),
            ..Default::default()
        },
        Some("nobody") => ComposerGate {
            blocked: Some(w.tv("workspace.noReader", &[])),
            ..Default::default()
        },
        // "ready", or a serve from before `reach`: serve answers itself.
        _ => ComposerGate::default(),
    }
}

/// Nobody reads this agent now: lost, or its lead is off the bus or ended
/// (`sessionGone`).
pub fn session_gone(a: &Agent) -> bool {
    a.lost || a.delivery == "not_on_bus" || a.delivery == "ended"
}

/// What the direct-message composer says about the way a message goes
/// (`dmHint`).
pub fn dm_hint(agent: &Agent, reader: Option<&str>, w: &impl Words) -> String {
    if session_gone(agent) {
        return format!("{}.", w.tv("workspace.sessionGone", &[]));
    }
    let name = w.agent_name(agent);
    if agent.kind == "session" {
        return w.tv("workspace.dmSession", &[("name", &name)]);
    }
    match reader {
        Some(reader) => w.tv("workspace.dmVia", &[("name", &name), ("reader", reader)]),
        None => w.tv("workspace.dmLead", &[("name", &name)]),
    }
}

/// A notice as the wait line shows it: first letter upper-cased, a period.
pub fn notice_line(notice: &str) -> String {
    let mut chars = notice.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The English dictionaries, as the app loads them.
    struct En(HashMap<String, String>);

    impl En {
        fn new() -> En {
            let mut map = HashMap::new();
            for ns in ["messages", "workspace"] {
                let bytes = crate::assets::Assets::local(&format!("locales/en/{ns}.json"))
                    .expect("dictionary");
                let v: serde_json::Map<String, Value> =
                    serde_json::from_slice(&bytes).expect("json");
                for (k, v) in v {
                    if let Value::String(s) = v {
                        map.insert(format!("{ns}.{k}"), s);
                    }
                }
            }
            En(map)
        }
    }

    impl Words for En {
        fn tv(&self, key: &str, vars: &[(&str, &str)]) -> String {
            crate::i18n::fill(self.0.get(key).map_or(key, String::as_str), vars)
        }
        fn agent_name(&self, a: &Agent) -> String {
            a.name.clone()
        }
        fn role_line(&self, a: &Agent) -> String {
            a.role_line.clone().unwrap_or_default()
        }
    }

    const MIN: u64 = 60_000;

    fn agent(key: &str, name: &str) -> Agent {
        Agent {
            key: key.into(),
            name: name.into(),
            kind: "teammate".into(),
            delivery: "ready".into(),
            presence: "working".into(),
            ..Default::default()
        }
    }

    fn with(mut a: Agent, f: impl FnOnce(&mut Agent)) -> Agent {
        f(&mut a);
        a
    }

    fn agents() -> Vec<Agent> {
        vec![
            with(agent("a1", "robots · builder"), |a| {
                a.handle = Some("robots-builder".into());
                a.team = Some("robots".into());
            }),
            with(agent("a2", "robots · reviewer"), |a| {
                a.handle = Some("robots-review".into());
                a.team = Some("robots".into());
            }),
            with(agent("a3", "harbard · dock"), |a| {
                a.handle = Some("harbard-dock".into());
                a.team = Some("harbard".into());
            }),
            with(agent("s1", "Claude · lead"), |a| {
                a.handle = Some("coordinator".into());
                a.team = Some("Claude".into());
                a.kind = "session".into();
            }),
            with(agent("t1", "sandbox · session 16:40"), |a| {
                a.handle = Some("t1".into());
                a.temporary = true;
            }),
        ]
    }

    #[test]
    fn at_offers_the_channel_teams_of_two_or_more_and_agents_by_name() {
        let w = En::new();
        let options = mention_options_of(&agents(), Some("harbard"), &w);
        let handles: Vec<&str> = options.iter().map(|o| o.handle.as_str()).collect();
        assert_eq!(
            handles,
            [
                "channel",
                "robots",
                "coordinator",
                "harbard-dock",
                "robots-builder",
                "robots-review"
            ]
        );
        assert_eq!(options[1].hint, "team, 2 agents");
        assert_eq!(options[0].hint, "all sessions in #harbard");
        assert!(
            !mention_options_of(&agents(), None, &w)
                .iter()
                .any(|o| o.handle == "channel")
        );
    }

    #[test]
    fn closed_agents_and_agents_without_a_handle_are_not_offered() {
        let w = En::new();
        let list = vec![
            with(agent("c", "closed one"), |a| {
                a.handle = Some("closed".into());
                a.closed = true;
            }),
            agent("n", "no handle"),
            with(agent("r", "with role"), |a| {
                a.handle = Some("roled".into());
                a.role_line = Some("reviewer".into());
            }),
        ];
        let options = mention_options_of(&list, None, &w);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].hint, "reviewer · @roled");
    }

    #[test]
    fn typing_at_rev_finds_the_reviewer_and_choosing_it_completes_the_word() {
        let w = En::new();
        let text = "Pass it to @rev";
        let at = mention_at(text, text.len());
        assert_eq!(
            at,
            Some(MentionAt {
                start: 11,
                query: "rev".into()
            })
        );
        let found = match_mentions(&mention_options_of(&agents(), None, &w), "rev");
        assert_eq!(
            found.iter().map(|o| o.handle.as_str()).collect::<Vec<_>>(),
            ["robots-review"]
        );
        assert_eq!(
            apply_mention(text, 11, text.len(), "robots-review"),
            ("Pass it to @robots-review ".into(), 26)
        );
        assert_eq!(mention_at("correo a@b.es", 13), None);
    }

    #[test]
    fn a_mention_starts_the_text_follows_a_space_or_a_parenthesis() {
        assert_eq!(
            mention_at("@", 1),
            Some(MentionAt {
                start: 0,
                query: String::new()
            })
        );
        assert_eq!(
            mention_at("hi (@Ro", 7),
            Some(MentionAt {
                start: 4,
                query: "ro".into()
            })
        );
        assert_eq!(mention_at("hi @ro there", 12), None);
        assert_eq!(
            mention_at("hi @ro there", 6),
            Some(MentionAt {
                start: 3,
                query: "ro".into()
            })
        );
        assert_eq!(
            mention_at("dí @José", "dí @José".len()),
            Some(MentionAt {
                start: 4,
                query: "josé".into()
            })
        );
    }

    #[test]
    fn the_list_shows_at_most_eight() {
        let options: Vec<MentionOption> = (0..12)
            .map(|i| MentionOption {
                handle: format!("a{i}"),
                label: format!("A{i}"),
                hint: String::new(),
                kind: MentionKind::Agent,
            })
            .collect();
        assert_eq!(match_mentions(&options, "").len(), 8);
        assert_eq!(match_mentions(&options, "a1").len(), 3);
    }

    #[test]
    fn mention_from_the_plus_menu_adds_a_space_when_needed() {
        assert_eq!(insert_at_sign("", 0), ("@".into(), 1));
        assert_eq!(insert_at_sign("hello", 5), ("hello @".into(), 7));
        assert_eq!(insert_at_sign("hello ", 6), ("hello @".into(), 7));
        assert_eq!(insert_at_sign("ab", 1), ("a @b".into(), 3));
    }

    #[test]
    fn a_reply_names_the_agent_or_the_channel() {
        let agent = ComposeTarget::Agent("k".into());
        assert_eq!(
            reply_body(&agent, "hello", false),
            json!({"agent": "k", "text": "hello"})
        );
        let thread = ComposeTarget::Thread {
            parent: "m1".into(),
            channel: None,
            agent: Some("k".into()),
        };
        assert_eq!(
            reply_body(&thread, "hello", false),
            json!({"agent": "k", "reply_to": "m1", "text": "hello"})
        );
        let channel = ComposeTarget::Channel("orchard".into());
        assert_eq!(
            reply_body(&channel, "hello", false),
            json!({"channel": "orchard", "text": "hello"})
        );
        let harbard = ComposeTarget::Channel("harbard".into());
        assert_eq!(
            reply_body(&harbard, "hello", true),
            json!({"channel": "harbard", "text": "hello", "anyway": true})
        );
    }

    #[test]
    fn the_body_is_the_trimmed_text_then_the_uploaded_paths() {
        assert_eq!(full_body("  hi \n", &[]), "hi");
        assert_eq!(full_body(" ", &["/a/b.png".into()]), "/a/b.png");
        assert_eq!(
            full_body("look", &["/a".into(), "/b".into()]),
            "look\n/a\n/b"
        );
        assert_eq!(full_body("   ", &[]), "");
        let w = En::new();
        assert_eq!(
            review_text("", &[], &w),
            "Can you review the latest work here?"
        );
        assert_eq!(
            review_text(" this ", &["/p".into()], &w),
            "Can you review this? this\n/p"
        );
    }

    #[test]
    fn a_stale_lead_the_note_is_stored_and_the_composer_says_it_will_wait() {
        let w = En::new();
        let stale = with(agent("k", "keychain"), |a| {
            a.delivery = "stale".into();
            a.lead_quiet_ms = Some(5 * 60 * MIN);
        });
        let ready = agent("r", "ready");
        assert_eq!(composer_block(&[&stale], &w), None);
        assert_eq!(
            composer_notice(&[&stale], &w).as_deref(),
            Some(
                "its main session has been silent for 5 h; the note will wait for it to come back"
            )
        );
        assert_eq!(composer_notice(&[&ready], &w), None);
        assert_eq!(composer_notice(&[&stale, &ready], &w), None);
        let short = with(agent("s", "s"), |a| {
            a.delivery = "stale".into();
            a.lead_quiet_ms = Some(20 * MIN);
        });
        assert_eq!(
            composer_notice(&[&short], &w).as_deref(),
            Some(
                "its main session has been silent for 20 min; the note will wait for it to come back"
            )
        );
    }

    #[test]
    fn with_no_live_session_the_composer_asks_and_blocks_only_with_no_session() {
        let w = En::new();
        let nobody = "Nobody will read it until a session opens; send anyway?";
        let off = with(agent("k", "keychain"), |a| a.delivery = "not_on_bus".into());
        let stale = with(agent("k", "keychain"), |a| a.delivery = "stale".into());
        let ended = with(agent("a", "a"), |a| a.delivery = "ended".into());
        let ready = agent("b", "b");
        assert_eq!(composer_block(&[&ready], &w), None);
        assert_eq!(composer_block(&[&off], &w), None);
        assert_eq!(composer_confirm(&[&off], &w).as_deref(), Some(nobody));
        assert_eq!(composer_confirm(&[&stale], &w), None);
        assert_eq!(composer_confirm(&[&ended, &ready], &w), None);
        assert_eq!(
            composer_block(&[], &w).as_deref(),
            Some("This channel has no session to leave a message for")
        );
        assert_eq!(
            reply_error(Some(409), Some("not_on_bus"), &w),
            w.tv("workspace.noDelivery", &[])
        );
        assert!(
            reply_error(Some(409), Some("no_reachable_member"), &w)
                .contains("Nobody in this channel")
        );
        assert_eq!(
            reply_error(Some(422), None, &w),
            "The message is too long for the bus; shorten it or link a report."
        );
        assert_eq!(
            reply_error(None, None, &w),
            "Couldn't save the message: harbard isn't responding."
        );
        assert_eq!(
            reply_error(Some(500), None, &w),
            "Couldn't save the message: harbard isn't responding."
        );
        for text in [
            w.tv("workspace.noDelivery", &[]),
            w.tv("workspace.noReader", &[]),
            reply_error(Some(409), Some("no_reachable_member"), &w),
        ] {
            assert!(!text.contains("yet"));
        }
    }

    #[test]
    fn a_channel_composer_follows_the_servers_reach() {
        let w = En::new();
        let ch = |reach: Option<&str>| {
            let c = Channel {
                reach: reach.map(str::to_string),
                ..Default::default()
            };
            channel_composer(&c, &w)
        };
        assert_eq!(ch(Some("ready")), ComposerGate::default());
        assert!(ch(Some("stale")).notice.unwrap().contains("will wait"));
        assert_eq!(
            ch(Some("none")).confirm.as_deref(),
            Some("Nobody will read it until a session opens; send anyway?")
        );
        assert_eq!(ch(Some("none")).blocked, None);
        assert_eq!(
            ch(Some("nobody")).blocked,
            Some(w.tv("workspace.noReader", &[]))
        );
        assert_eq!(ch(None), ComposerGate::default());
    }

    #[test]
    fn the_dm_composer_says_which_way_a_message_goes() {
        let w = En::new();
        let list = agents();
        assert_eq!(
            dm_hint(&list[0], None, &w),
            "Goes to robots · builder through the lead."
        );
        assert_eq!(
            dm_hint(&list[3], None, &w),
            "Goes straight to Claude · lead's session."
        );
        let gone = with(agent("g", "Ethan"), |a| a.lost = true);
        assert_eq!(
            dm_hint(&gone, None, &w),
            "Its session is gone; nobody will read this."
        );
        assert_eq!(
            dm_hint(&agent("e", "Ethan"), None, &w),
            "Goes to Ethan through the lead."
        );
        assert!(dm_hint(&agent("e", "Ethan"), Some("Claude"), &w).contains("it goes to Claude"));
    }

    #[test]
    fn invisible_characters_are_named_and_other_refusals_left_alone() {
        let w = En::new();
        let text = invisible_refusal(
            "`context` carries characters you cannot see (U+200B, U+E0049); they would reach the agents as text. Remove them, or retype it, and save again",
            &w,
        )
        .unwrap();
        assert!(text.contains("invisible characters (U+200B, U+E0049)"));
        assert_eq!(
            invisible_refusal("a status holds at most 100 characters", &w),
            None
        );
        let bare = invisible_refusal("characters you cannot see", &w).unwrap();
        assert!(bare.starts_with("Your text has invisible characters. "));
    }

    #[test]
    fn a_notice_is_capitalised_and_ends_in_a_period() {
        assert_eq!(notice_line("the note will wait"), "The note will wait.");
        assert_eq!(notice_line(""), "");
    }
}
