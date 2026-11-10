//! Ported from the console's `lib/rooms.ts`: rooms and `#feedback`.
//!
//! A room is a named channel the operator can create, archive and delete; a
//! project channel belongs to a repository and can only be archived. serve
//! decides what exists and refuses what the contract forbids; this holds the
//! small pure work the views share.

use super::workspace::{Words, collate};
use crate::model::{Channel, Message, WorkspaceView};
use gpui::SharedString;
use serde::Deserialize;
use std::borrow::Cow;

pub const LOBBY: &str = "lobby";
pub const COMMONS: &str = "commons";
pub const FEEDBACK: &str = "feedback";

/// The built-in rooms, in the order the sidebar lists them.
pub const BUILT_IN_ROOMS: [&str; 3] = [LOBBY, COMMONS, FEEDBACK];

pub fn is_built_in(name: &str) -> bool {
    BUILT_IN_ROOMS.contains(&name)
}

pub fn is_room(channel: &Channel) -> bool {
    channel.is_room()
}

/// A built-in room serve did not list (an older serve, an empty workspace).
pub fn built_in_placeholder(id: &str) -> Option<Channel> {
    is_built_in(id).then(|| Channel {
        id: id.to_string(),
        kind: Some("room".into()),
        built_in: true,
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// Serve's shapes
// ---------------------------------------------------------------------------

/// `GET /api/v1/channels/<room>/history?limit=50[&before=]`: oldest first.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct RoomHistoryPage {
    pub messages: Vec<Message>,
    /// The cursor for the page before this one; None at the very start.
    pub next_before: Option<String>,
}

/// One line of `GET /api/v1/channels/deleted`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeletedRoom {
    pub name: String,
    pub deleted_ms: u64,
    pub messages: u64,
}

// ---------------------------------------------------------------------------
// The sidebar
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct SidebarRooms<'a> {
    /// The built-in rooms first and always, then the operator's by name.
    pub rooms: Vec<Cow<'a, Channel>>,
    /// Anything archived that is not built in: rooms and project channels.
    pub archived: Vec<&'a Channel>,
}

/// `sidebarRooms`.
pub fn sidebar_rooms(view: &WorkspaceView) -> SidebarRooms<'_> {
    let mut rooms: Vec<Cow<Channel>> = BUILT_IN_ROOMS
        .iter()
        .filter_map(|id| match view.channel(id) {
            Some(c) => Some(Cow::Borrowed(c)),
            None => built_in_placeholder(id).map(Cow::Owned),
        })
        .collect();
    let mut own: Vec<&Channel> = view
        .channels
        .iter()
        .filter(|c| c.is_room() && !is_built_in(&c.id) && !c.archived)
        .collect();
    own.sort_by(|a, b| collate(&a.id, &b.id));
    rooms.extend(own.into_iter().map(Cow::Borrowed));
    let archived = view
        .channels
        .iter()
        .filter(|c| c.archived && !is_built_in(&c.id))
        .collect();
    SidebarRooms { rooms, archived }
}

// ---------------------------------------------------------------------------
// The channel menu
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelAction {
    Archive,
    Unarchive,
    Delete,
}

impl ChannelAction {
    /// The `action` of `POST /api/v1/channels {action, name}`. Delete opens
    /// the confirm dialog first.
    pub fn as_str(self) -> &'static str {
        match self {
            ChannelAction::Archive => "archive",
            ChannelAction::Unarchive => "unarchive",
            ChannelAction::Delete => "delete",
        }
    }

    /// "Archive", "Unarchive", "Delete room".
    pub fn label(self, w: &impl Words) -> SharedString {
        match self {
            ChannelAction::Archive => w.text("rooms.actionArchive", &[]),
            ChannelAction::Unarchive => w.text("rooms.actionUnarchive", &[]),
            ChannelAction::Delete => w.text("rooms.actionDelete", &[]),
        }
    }
}

/// What the channel menu offers (`channelActions`): a built-in room nothing
/// (the menu hides); archive and back; delete only once a room is archived.
pub fn channel_actions(channel: &Channel) -> Vec<ChannelAction> {
    if channel.built_in || is_built_in(&channel.id) {
        return Vec::new();
    }
    if !channel.archived {
        return vec![ChannelAction::Archive];
    }
    if channel.is_room() {
        vec![ChannelAction::Unarchive, ChannelAction::Delete]
    } else {
        vec![ChannelAction::Unarchive]
    }
}

/// serve's refusal of a channel action in the operator's words; a code this
/// build does not know keeps serve's message.
pub fn channel_refusal(code: Option<&str>, message: &str, w: &impl Words) -> SharedString {
    match code {
        Some("room_built_in") => w.text("rooms.refusedBuiltIn", &[]),
        Some("project_channel") => w.text("rooms.refusedProject", &[]),
        _ => message.to_string().into(),
    }
}

/// The toast: "That did not work: {reason}".
pub fn action_failed(reason: &str, w: &impl Words) -> SharedString {
    w.text("rooms.actionFailed", &[("reason", reason)])
}

/// "#{channel} is archived." and the line after it, for the archived strip.
pub fn archived_strip(channel: &Channel, w: &impl Words) -> (SharedString, SharedString) {
    let lead = w.text("rooms.archivedLead", &[("channel", &channel.id)]);
    let body = if channel.is_room() {
        w.text("rooms.archivedRoomBody", &[])
    } else {
        w.text("rooms.archivedProjectBody", &[])
    };
    (lead, body)
}

// ---------------------------------------------------------------------------
// Naming a room
// ---------------------------------------------------------------------------

/// 48 bytes is the bus's limit.
pub const NAME_MAX: usize = 48;

/// One name, one channel, however it was typed: trimmed, no leading `#`,
/// lowercase, spaces as dashes.
pub fn normalize_room_name(input: &str) -> String {
    input
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomNameProblem {
    Empty,
    TooLong,
    Invalid,
    BuiltIn,
    Taken,
}

impl RoomNameProblem {
    pub fn message(self, w: &impl Words) -> SharedString {
        let key = match self {
            RoomNameProblem::Empty => "rooms.nameEmpty",
            RoomNameProblem::TooLong => "rooms.nameTooLong",
            RoomNameProblem::Invalid => "rooms.nameInvalid",
            RoomNameProblem::BuiltIn => "rooms.nameBuiltIn",
            RoomNameProblem::Taken => "rooms.nameTaken",
        };
        w.text(key, &[("max", &NAME_MAX.to_string())])
    }
}

/// Why a normalized name cannot be created, given the ids in use.
pub fn name_problem(name: &str, taken: &[&str]) -> Option<RoomNameProblem> {
    if name.is_empty() {
        return Some(RoomNameProblem::Empty);
    }
    if name.chars().count() > NAME_MAX {
        return Some(RoomNameProblem::TooLong);
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Some(RoomNameProblem::Invalid);
    }
    if is_built_in(name) {
        return Some(RoomNameProblem::BuiltIn);
    }
    taken.contains(&name).then_some(RoomNameProblem::Taken)
}

/// The problem the create dialog shows for a normalized `name`: an empty
/// name only after a submit was tried.
pub fn room_name_problem(view: &WorkspaceView, name: &str, tried: bool) -> Option<RoomNameProblem> {
    let taken: Vec<&str> = view.channels.iter().map(|c| c.id.as_str()).collect();
    name_problem(name, &taken).filter(|p| tried || *p != RoomNameProblem::Empty)
}

/// Whether the create dialog may send: any problem blocks, shown or not.
pub fn room_name_ok(view: &WorkspaceView, raw: &str) -> bool {
    room_name_problem(view, &normalize_room_name(raw), true).is_none()
}

/// The line under the name field.
#[derive(Debug, Clone, PartialEq)]
pub struct RoomNameNote {
    pub text: SharedString,
    /// A problem: red, `role=alert`, the input `aria-invalid`.
    pub problem: bool,
    /// What will be created.
    pub name: String,
}

/// The problem; else "It will be created as #{name}" when typing changed
/// it; else the hint.
pub fn room_name_note(
    view: &WorkspaceView,
    raw: &str,
    tried: bool,
    w: &impl Words,
) -> RoomNameNote {
    let name = normalize_room_name(raw);
    if let Some(p) = room_name_problem(view, &name, tried) {
        return RoomNameNote {
            text: p.message(w),
            problem: true,
            name,
        };
    }
    let text = if !name.is_empty() && name != raw.trim() {
        w.text("rooms.willBeCreatedAs", &[("name", &name)])
    } else {
        w.text("rooms.fieldNameHint", &[])
    };
    RoomNameNote {
        text,
        problem: false,
        name,
    }
}

// ---------------------------------------------------------------------------
// A room's header, composer and messages
// ---------------------------------------------------------------------------

/// The header's line: what serve holds, or what a built-in room is for.
pub fn room_purpose(channel: &Channel, w: &impl Words) -> Option<SharedString> {
    if let Some(p) = channel.purpose.as_deref().filter(|p| !p.is_empty()) {
        return Some(p.to_string().into());
    }
    match channel.id.as_str() {
        LOBBY => Some(w.text("rooms.purposeLobby", &[])),
        COMMONS => Some(w.text("rooms.purposeCommons", &[])),
        FEEDBACK => Some(w.text("rooms.purposeFeedback", &[])),
        _ => None,
    }
}

/// What the composer says above the box (the composer capitalises it and
/// adds the full stop): a post in `#lobby`/`#commons` wakes nobody; a room
/// nobody reads still keeps the post.
pub fn room_composer_notice(channel: &Channel, w: &impl Words) -> Option<SharedString> {
    if channel.id == LOBBY || channel.id == COMMONS {
        return Some(w.text("rooms.noticeReachesNoWork", &[("room", &channel.id)]));
    }
    if channel.reach.as_deref() == Some("ready") {
        return None;
    }
    Some(w.text("rooms.noticeNoReaders", &[("room", &channel.id)]))
}

/// A room is flat: unlike a channel it keeps replies inline; protocol
/// records stay out.
pub fn room_stream<'a>(view: &'a WorkspaceView, room: &str) -> Vec<&'a Message> {
    view.messages
        .iter()
        .filter(|m| m.channels.iter().any(|c| c == room) && m.structured.is_none())
        .collect()
}

/// History pages and the live view, each message once, oldest first (then
/// by id). The live copy wins; protocol records from history are dropped.
pub fn merge_room_messages<'a>(history: &'a [Message], live: &[&'a Message]) -> Vec<&'a Message> {
    let mut by_id: std::collections::HashMap<&str, &'a Message> = std::collections::HashMap::new();
    for m in history.iter().filter(|m| m.structured.is_none()) {
        by_id.insert(m.id.as_str(), m);
    }
    for &m in live {
        by_id.insert(m.id.as_str(), m);
    }
    let mut out: Vec<&Message> = by_id.into_values().collect();
    out.sort_by(|a, b| a.ts_ms.cmp(&b.ts_ms).then_with(|| a.id.cmp(&b.id)));
    out
}

// ---------------------------------------------------------------------------
// #feedback
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackKind {
    Bug,
    Friction,
    Idea,
    Praise,
}

impl FeedbackKind {
    /// The filter buttons' order.
    pub const ALL: [FeedbackKind; 4] = [
        FeedbackKind::Bug,
        FeedbackKind::Friction,
        FeedbackKind::Idea,
        FeedbackKind::Praise,
    ];

    fn parse(word: &str) -> Option<FeedbackKind> {
        match word {
            "bug" => Some(FeedbackKind::Bug),
            "friction" => Some(FeedbackKind::Friction),
            "idea" => Some(FeedbackKind::Idea),
            "praise" => Some(FeedbackKind::Praise),
            _ => None,
        }
    }

    /// `data-kind` / the filter's id.
    pub fn as_str(self) -> &'static str {
        match self {
            FeedbackKind::Bug => "bug",
            FeedbackKind::Friction => "friction",
            FeedbackKind::Idea => "idea",
            FeedbackKind::Praise => "praise",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackNote {
    /// None for a note that does not open with a kind this build knows.
    pub kind: Option<FeedbackKind>,
    pub text: String,
    /// From the `harbard <version>` line `harbard feedback` adds.
    pub version: Option<String>,
}

/// `[kind] ` at the start of the first line: the word, and the rest.
fn kind_line(first: &str) -> Option<(&str, &str)> {
    let rest = first.strip_prefix('[')?;
    let end = rest.find(']')?;
    let word = &rest[..end];
    if word.is_empty() || !word.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    Some((word, rest[end + 1..].trim_start()))
}

/// `harbard <version>` alone on a line.
fn version_line(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("harbard")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let v = rest.trim();
    (!v.is_empty() && !v.contains(char::is_whitespace)).then_some(v)
}

/// `[<kind>] <text>` on the first line and `harbard <version>` on a later
/// one (`parseFeedback`).
pub fn parse_feedback(raw: &str) -> FeedbackNote {
    let mut lines = raw.split('\n');
    let first = lines.next().unwrap_or("");
    let rest: Vec<&str> = lines.collect();
    let (kind, first_text) =
        match kind_line(first).and_then(|(w, t)| FeedbackKind::parse(w).map(|k| (k, t))) {
            Some((k, t)) => (Some(k), t),
            None => (None, first),
        };
    let version_at = rest.iter().position(|l| version_line(l).is_some());
    let version = version_at
        .and_then(|i| version_line(rest[i]))
        .map(String::from);
    let mut body = vec![first_text];
    body.extend(
        rest.iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != version_at)
            .map(|(_, l)| *l),
    );
    FeedbackNote {
        kind,
        text: body.join("\n").trim().to_string(),
        version,
    }
}

/// "Bug", "Friction", "Idea", "Praise", or "Note" for no kind.
pub fn feedback_kind_label(kind: Option<FeedbackKind>, w: &impl Words) -> SharedString {
    let key = match kind {
        Some(FeedbackKind::Bug) => "rooms.kindBug",
        Some(FeedbackKind::Friction) => "rooms.kindFriction",
        Some(FeedbackKind::Idea) => "rooms.kindIdea",
        Some(FeedbackKind::Praise) => "rooms.kindPraise",
        None => "rooms.kindNote",
    };
    w.text(key, &[])
}

/// The chip's `data-tone`: red, amber, accent, green or neutral.
pub fn feedback_tone(kind: Option<FeedbackKind>) -> &'static str {
    match kind {
        Some(FeedbackKind::Bug) => "red",
        Some(FeedbackKind::Friction) => "amber",
        Some(FeedbackKind::Idea) => "accent",
        Some(FeedbackKind::Praise) => "green",
        None => "neutral",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::workspace::test_words::en;

    fn channel(id: &str, kind: &str) -> Channel {
        Channel {
            id: id.into(),
            kind: Some(kind.into()),
            ..Default::default()
        }
    }

    fn archived(mut c: Channel) -> Channel {
        c.archived = true;
        c
    }

    fn message(id: &str, ts_ms: u64) -> Message {
        Message {
            id: id.into(),
            ts_ms,
            text: id.into(),
            channels: vec!["r".into()],
            ..Default::default()
        }
    }

    fn ids<'a>(list: impl IntoIterator<Item = &'a Channel>) -> Vec<&'a str> {
        list.into_iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn a_typed_name_is_normalized_the_way_the_bus_does_it() {
        assert_eq!(normalize_room_name("  #Design Crit "), "design-crit");
    }

    #[test]
    fn a_name_the_bus_would_refuse_is_refused_before_sending() {
        assert_eq!(name_problem("", &[]), Some(RoomNameProblem::Empty));
        assert_eq!(
            name_problem("bad!name", &[]),
            Some(RoomNameProblem::Invalid)
        );
        assert_eq!(
            name_problem(&"x".repeat(49), &[]),
            Some(RoomNameProblem::TooLong)
        );
        assert_eq!(name_problem(&"x".repeat(48), &[]), None);
        assert_eq!(name_problem("commons", &[]), Some(RoomNameProblem::BuiltIn));
        assert_eq!(
            name_problem("orchard", &["orchard"]),
            Some(RoomNameProblem::Taken)
        );
    }

    #[test]
    fn the_dialog_shows_an_empty_name_only_after_a_try() {
        let v = WorkspaceView {
            channels: vec![channel("orchard", "project")],
            ..Default::default()
        };
        assert_eq!(room_name_problem(&v, "", false), None);
        assert_eq!(
            room_name_problem(&v, "", true),
            Some(RoomNameProblem::Empty)
        );
        assert!(!room_name_ok(&v, ""));
        let w = en();
        assert_eq!(
            room_name_note(&v, "Design Crit", false, &w).text,
            "It will be created as #design-crit"
        );
        assert_eq!(
            room_name_note(&v, "crit", false, &w).text,
            "Lowercase, no spaces. Letters, digits, dots, dashes and underscores."
        );
        let taken = room_name_note(&v, "#orchard", false, &w);
        assert!(taken.problem);
        assert_eq!(taken.text, "There is already a channel with that name");
        assert_eq!(
            RoomNameProblem::TooLong.message(&w),
            "At most 48 characters"
        );
    }

    #[test]
    fn the_three_built_in_rooms_are_always_there() {
        let v = WorkspaceView::default();
        let s = sidebar_rooms(&v);
        assert_eq!(
            ids(s.rooms.iter().map(|c| c.as_ref())),
            ["lobby", "commons", "feedback"]
        );
    }

    #[test]
    fn built_ins_come_first_and_the_operators_rooms_follow_by_name() {
        let mut fb = channel("feedback", "room");
        fb.built_in = true;
        let v = WorkspaceView {
            channels: vec![channel("zeta", "room"), channel("alpha", "room"), fb],
            ..Default::default()
        };
        let s = sidebar_rooms(&v);
        assert_eq!(
            ids(s.rooms.iter().map(|c| c.as_ref())),
            ["lobby", "commons", "feedback", "alpha", "zeta"]
        );
        assert!(matches!(s.rooms[2], Cow::Borrowed(_)));
    }

    #[test]
    fn anything_archived_leaves_its_list_for_the_archived_group() {
        let v = WorkspaceView {
            channels: vec![
                channel("kept", "room"),
                archived(channel("old", "room")),
                archived(channel("repo", "project")),
            ],
            ..Default::default()
        };
        let s = sidebar_rooms(&v);
        assert!(!ids(s.rooms.iter().map(|c| c.as_ref())).contains(&"old"));
        assert_eq!(ids(s.archived), ["old", "repo"]);
    }

    #[test]
    fn what_the_channel_menu_offers() {
        let mut lobby = channel("lobby", "room");
        lobby.built_in = true;
        assert!(channel_actions(&lobby).is_empty());
        assert_eq!(
            channel_actions(&channel("repo", "project")),
            [ChannelAction::Archive]
        );
        assert_eq!(
            channel_actions(&archived(channel("repo", "project"))),
            [ChannelAction::Unarchive]
        );
        assert_eq!(
            channel_actions(&channel("r", "room")),
            [ChannelAction::Archive]
        );
        assert_eq!(
            channel_actions(&archived(channel("r", "room"))),
            [ChannelAction::Unarchive, ChannelAction::Delete]
        );
        assert_eq!(ChannelAction::Delete.label(&en()), "Delete room");
    }

    #[test]
    fn the_contracts_refusal_codes_are_worded_and_an_unknown_one_keeps_serves_message() {
        assert_eq!(
            channel_refusal(Some("room_built_in"), "x", &en()),
            "A built-in room cannot be archived or deleted"
        );
        assert_eq!(
            channel_refusal(Some("something_new"), "serve said so", &en()),
            "serve said so"
        );
        assert_eq!(
            channel_refusal(None, "serve said so", &en()),
            "serve said so"
        );
    }

    #[test]
    fn replies_stay_in_the_flat_stream_and_protocol_records_do_not() {
        let mut reply = message("reply", 1);
        reply.reply_to = Some("m".into());
        let mut record = message("record", 2);
        record.structured = Some("{}".into());
        let mut other = message("other", 3);
        other.channels = vec!["z".into()];
        let v = WorkspaceView {
            messages: vec![reply, record, other],
            ..Default::default()
        };
        let got: Vec<&str> = room_stream(&v, "r").iter().map(|m| m.id.as_str()).collect();
        assert_eq!(got, ["reply"]);
    }

    #[test]
    fn history_and_the_live_view_join_without_duplicates_oldest_first() {
        let history = vec![message("b", 2), message("a", 1)];
        let c = message("c", 3);
        let mut fresher = message("b", 2);
        fresher.text = "fresher".into();
        let merged = merge_room_messages(&history, &[&c, &fresher]);
        let got: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(got, ["a", "b", "c"]);
        assert_eq!(merged[1].text, "fresher");
    }

    #[test]
    fn the_history_page_reads_serves_json() {
        let page: RoomHistoryPage =
            serde_json::from_str(r#"{"messages":[{"id":"m","ts_ms":4}],"next_before":"c1"}"#)
                .unwrap();
        assert_eq!(page.messages[0].id, "m");
        assert_eq!(page.next_before.as_deref(), Some("c1"));
        let end: RoomHistoryPage =
            serde_json::from_str(r#"{"messages":[],"next_before":null}"#).unwrap();
        assert!(end.next_before.is_none());
    }

    #[test]
    fn the_composer_notice_follows_the_room() {
        let w = en();
        let mut commons = channel("commons", "room");
        commons.reach = Some("ready".into());
        assert!(
            room_composer_notice(&commons, &w)
                .unwrap()
                .contains("no one's work")
        );
        assert!(
            room_composer_notice(&channel("lobby", "room"), &w)
                .unwrap()
                .contains("no one's work")
        );
        let mut crit = channel("crit", "room");
        crit.reach = Some("ready".into());
        assert!(room_composer_notice(&crit, &w).is_none());
        crit.reach = Some("none".into());
        assert!(
            room_composer_notice(&crit, &w)
                .unwrap()
                .contains("stays here")
        );
    }

    #[test]
    fn a_built_in_room_always_has_a_purpose_to_show() {
        let w = en();
        assert!(
            room_purpose(&channel("commons", "room"), &w)
                .unwrap()
                .contains("not work")
        );
        assert!(room_purpose(&channel("mine", "room"), &w).is_none());
        let mut ours = channel("mine", "room");
        ours.purpose = Some("Ours".into());
        assert_eq!(room_purpose(&ours, &w).as_deref(), Some("Ours"));
    }

    #[test]
    fn feedback_kind_from_the_first_line_and_version_from_a_later_one() {
        assert_eq!(
            parse_feedback("[bug] It breaks\nharbard 0.42.1"),
            FeedbackNote {
                kind: Some(FeedbackKind::Bug),
                text: "It breaks".into(),
                version: Some("0.42.1".into())
            }
        );
        assert_eq!(
            parse_feedback("[idea] One\nTwo\nharbard 1.0.0").text,
            "One\nTwo"
        );
        assert_eq!(
            parse_feedback("[weird] Hello"),
            FeedbackNote {
                kind: None,
                text: "[weird] Hello".into(),
                version: None
            }
        );
        assert_eq!(parse_feedback("Plain").kind, None);
        assert_eq!(parse_feedback("[praise] harbard 2").version, None);
        assert_eq!(feedback_tone(Some(FeedbackKind::Idea)), "accent");
        assert_eq!(feedback_kind_label(None, &en()), "Note");
    }
}
