//! One message row (`MessageItem`, `parts.tsx`): the avatar, the meta line,
//! the text (readable line, folding, mentions), the operator note's
//! delivery line, attachments, files under "Technical details", the
//! verified pill, repeats, the replies link and the hover tools.
//!
//! Sizes and colours follow `workspace.css` (`.hb-msg*`, `.hb-chip`,
//! `.hb-tech`, `.hb-file`, `.hb-replies`, `.hb-msg-tools`, `.hb-delivery`,
//! `.hb-pill`, `.hb-machine`, `.hb-mention`, `.hb-tok`).

use super::MessageOpts;
use super::focusable;
use super::inline_chip::{self, Chip};
use crate::derive::legible::{self, MachineMark};
use crate::derive::messages as words;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Agent, Message, WorkspaceView};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{
    self, avatar, avatar_size, control, format::hhmm, icon_size, leading, pill_chat as pill,
    radius, size, space, weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ElementId, Empty, HighlightStyle, InteractiveElement, InteractiveText,
    IntoElement, ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled,
    StyledText, Window, canvas, div, px,
};
use serde_json::json;
use std::ops::Range;

/// The group name hover styles of a row hang from.
const ROW_GROUP: &str = "hb-msg-row";

/// One message row (the contract's `chat::message_item`).
pub fn item(msg: &Message, view: &WorkspaceView, opts: &MessageOpts, cx: &App) -> AnyElement {
    item_with(msg, view, opts, None, cx)
}

/// A row with its "written for machines" marker decided by the stream
/// (`machine = Some(..)`), or computed for the message alone (`None`).
pub(crate) fn item_with(
    msg: &Message,
    view: &WorkspaceView,
    opts: &MessageOpts,
    machine: Option<Option<MachineMark>>,
    cx: &App,
) -> AnyElement {
    // A repeat is collapsed under its first copy, so it is not drawn twice.
    if msg
        .repeat_of
        .as_deref()
        .is_some_and(|r| view.message(r).is_some())
    {
        return Empty.into_any_element();
    }
    MessageRow::new(msg, view, opts, machine, cx).into_any_element()
}

/// Who a row shows at its left.
enum Face {
    /// `AgentAvatar` with an agent: its robot, live face and dot.
    Agent(Box<Agent>),
    /// `AgentAvatar` without one: the same robot by name, resting.
    Named(String),
    /// The lead with no known session: the compass tile.
    Lead,
    /// The operator's photo or bust.
    Operator,
    /// Anyone else: a monogram on the lead colour.
    Monogram,
}

/// What a row needs from the read, gathered while the read is borrowed.
#[derive(IntoElement)]
struct MessageRow {
    msg: Message,
    opts: MessageOpts,
    face: Face,
    name: SharedString,
    /// The agent the name links to (`/direct/<key>`) and the hover card shows.
    link: Option<Box<Agent>>,
    saved: bool,
    repeats: Vec<u64>,
    reader: Option<String>,
    operator_initials: String,
    machine: Option<MachineMark>,
}

impl MessageRow {
    fn new(
        msg: &Message,
        view: &WorkspaceView,
        opts: &MessageOpts,
        machine: Option<Option<MachineMark>>,
        cx: &App,
    ) -> Self {
        let agent = view.agent(&msg.agent);
        let from = msg.from.as_str();
        let face = match from {
            "agent" => agent.map_or_else(
                || Face::Named(String::new()),
                |a| Face::Agent(Box::new(a.clone())),
            ),
            "peer" | "session" => view
                .agents
                .iter()
                .find(|a| a.key == msg.agent || a.name == msg.author)
                .map_or_else(
                    || Face::Named(String::new()),
                    |a| Face::Agent(Box::new(a.clone())),
                ),
            "lead" => match words::lead_key_of(&view.agents, &msg.agent) {
                Some(key) => view.agent(&key).map_or(Face::Named(key.clone()), |a| {
                    Face::Agent(Box::new(a.clone()))
                }),
                None => Face::Lead,
            },
            "operator" => Face::Operator,
            _ => Face::Monogram,
        };
        let name: SharedString = match (from, agent) {
            ("agent", Some(a)) => crate::derive::agent_name(a, cx),
            ("operator", _) => {
                crate::derive::shown_name(&words::operator_name(&msg.author, cx), cx)
            }
            ("lead", _) => crate::derive::shown_name(words::lead_display_name(&msg.author), cx),
            _ => crate::derive::shown_name(&msg.author, cx),
        };
        // A robot without an agent is drawn from the name shown.
        let face = match face {
            Face::Named(n) if n.is_empty() => Face::Named(name.to_string()),
            other => other,
        };
        let link = if from == "agent" {
            agent.map(|a| Box::new(a.clone()))
        } else {
            None
        };
        let machine = match machine {
            Some(decided) => decided,
            None => legible::machine_mark(msg),
        };
        MessageRow {
            msg: msg.clone(),
            opts: opts.clone(),
            face,
            name,
            link,
            saved: view.saved.iter().any(|s| s.id == msg.id),
            repeats: words::repeats_of(view, &msg.id)
                .iter()
                .map(|m| m.ts_ms)
                .collect(),
            reader: msg
                .delivery
                .as_ref()
                .and_then(|_| words::note_reader(&view.agents, &msg.agent)),
            operator_initials: view.operator_initials.clone(),
            machine,
        }
    }
}

/// What a row remembers between frames.
#[derive(Default)]
struct RowState {
    /// A folded text the reader opened ("Show more").
    unfolded: bool,
    /// "Technical details" opened or closed by hand (else open in a thread).
    details: Option<bool>,
    /// The hover tools hang below the row: above it there is no room in
    /// the stream (the first row, under the header).
    tools_below: bool,
}

/// `.hb-msg-tools { top: -12px }`: how far the tools rise above the row.
const TOOLS_RISE: f32 = space::LG;

/// Whether the hover tools must hang below a row whose top sits `room`
/// pixels under the top of the visible stream: they rise `TOOLS_RISE` and
/// need their 1 px edge to show.
pub(crate) fn tools_below(room: f32) -> bool {
    room < TOOLS_RISE + 1.
}

/// The hover card around an agent's avatar and name (`AgentHover`); `slot`
/// keeps the two triggers on one row apart.
fn agent_hover(child: AnyElement, agent: Option<&Agent>, msg: &str, slot: &str) -> AnyElement {
    match agent {
        Some(a) => crate::ui::agent_card::agent_hover(
            SharedString::from(format!("hb-msg-hover-{msg}-{slot}")),
            &a.key,
            false,
            child,
        ),
        None => child,
    }
}

/// What a message's feedback is about (`messageTarget`): none for the
/// operator's own notes, "delivery" for an agent's DONE, else "message".
fn feedback_kind(msg: &Message) -> Option<&'static str> {
    match (msg.from.as_str(), msg.label.as_deref()) {
        ("operator", _) => None,
        ("agent", Some("DONE")) => Some("delivery"),
        _ => Some("message"),
    }
}

/// "10/11/2026, 22:41:07": the full time in the timestamp's tooltip.
fn full_time(ms: u64) -> String {
    let (y, mo, d, _) = ui::format::local_date(ms);
    let (h, m) = ui::format::local_hm(ms);
    let s = (ms / 1000) % 60;
    format!("{d:02}/{mo:02}/{y}, {h:02}:{m:02}:{s:02}")
}

fn tooltip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    ui::kit_theme::tip(text)
}

/// A message's type chip, or the lead tag: one shared chip (`ui::chip`).
/// A state tone only for a state (C3); the lead tag is neutral.
pub(crate) fn chip(text: SharedString, tone: &str, theme: &Theme) -> gpui::Div {
    ui::chip(text, tone, theme)
}

/// A secondary line under a message's text: 13/400 with a 14 px mark.
fn quiet_line(id: impl Into<ElementId>, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    focusable(
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(space::XS))
            .rounded(px(radius::SM))
            .cursor_pointer()
            .font_family(ui::SANS)
            .text_size(px(size::SMALL))
            .line_height(px(leading::SMALL))
            .font_weight(weight::REGULAR),
        false,
        theme,
    )
}

impl RenderOnce for MessageRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.msg.id.clone();
        let state = window.use_keyed_state(
            ElementId::Name(format!("hb-msg-{id}").into()),
            cx,
            |_, _| RowState::default(),
        );
        let (unfolded, details_open, below) = {
            let s = state.read(cx);
            (
                s.unfolded,
                s.details.unwrap_or(self.opts.in_thread),
                s.tools_below,
            )
        };
        let attachments =
            super::attachments::message_attachments(&self.msg, self.opts.in_thread, window, cx);
        // The 👍/👎 row in the hover tools (`FeedbackRow`).
        let feedback = feedback_kind(&self.msg)
            .filter(|_| !self.opts.read_only)
            .and_then(|k| crate::ui::feedback::feedback_row(k, &self.msg.id, None, cx));
        // An operator's answer to a view reads as the view's words (`AnswerEcho`).
        let echo = crate::huginn::operator_answer(&self.msg).map(|_| {
            let msg = self.msg.clone();
            window.use_keyed_state(
                ElementId::Name(format!("hb-echo-{id}").into()),
                cx,
                move |window, cx| crate::huginn::AnswerEcho::new(msg, window, cx),
            )
        });
        let cx: &App = cx;
        let theme = cx.global::<Theme>();
        let msg = &self.msg;
        let opts = &self.opts;
        let selected = opts.selected.as_deref() == Some(id.as_str());

        // Avatar column.
        let face: AnyElement = if opts.continued {
            div().w(px(avatar_size::ROW)).flex_none().into_any_element()
        } else {
            let el = match &self.face {
                Face::Agent(a) => avatar::agent_avatar(a, avatar_size::ROW, true, cx),
                Face::Named(n) => avatar::name_avatar(n, avatar_size::ROW, cx),
                Face::Lead => avatar::lead_avatar(avatar_size::ROW, cx),
                Face::Operator => avatar::operator_avatar_with(
                    &self.name,
                    &self.operator_initials,
                    avatar_size::ROW,
                    None,
                    cx,
                ),
                Face::Monogram => avatar::monogram_avatar(
                    &self.name,
                    Some(theme.c("av-lead")),
                    None,
                    avatar_size::ROW,
                    None,
                    cx,
                ),
            };
            let agent = match &self.face {
                Face::Agent(a) => Some(a.as_ref()),
                _ => None,
            };
            agent_hover(
                div().flex_none().child(el).into_any_element(),
                agent,
                &id,
                "face",
            )
        };

        // Meta line: name, lead tag, label, time, "written for machines".
        let meta = (!opts.continued).then(|| {
            // The author is what a reader scans for: the row's one 900.
            let name_el = div()
                .id("hb-msg-name")
                .font_weight(weight::HEAVY)
                .text_color(theme.c("ink"))
                .child(self.name.clone());
            let name_el: AnyElement = match &self.link {
                Some(agent) => {
                    let key = agent.key.clone();
                    let el = focusable(
                        name_el
                            .rounded(px(radius::SM))
                            .cursor_pointer()
                            .hover(|s| s.underline()),
                        false,
                        theme,
                    )
                    .on_click(move |_, _, cx| nav::navigate(Route::direct(Some(key.clone())), cx))
                    .into_any_element();
                    agent_hover(el, Some(agent), &id, "name")
                }
                None => name_el.into_any_element(),
            };
            let meta_ink = theme.c("ink-3");
            let time = div()
                .id("hb-msg-time")
                .tooltip(tooltip(full_time(msg.ts_ms).into()))
                .child(ui::text(
                    hhmm(msg.ts_ms),
                    size::META,
                    weight::REGULAR,
                    meta_ink,
                ));
            let machine = self.machine.as_ref().map(|mark| {
                div()
                    .id("hb-machine")
                    .tooltip(tooltip(mark.hint(cx)))
                    .child(ui::text(
                        legible::machine_text(cx),
                        size::META,
                        weight::REGULAR,
                        meta_ink,
                    ))
            });
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(space::MD))
                .child(name_el)
                .when(msg.from == "lead", |d| {
                    d.child(pill::state_chip(t(cx, "workspace.leadRole"), "lead", theme))
                })
                .when_some(msg.label.as_deref(), |d, label| {
                    let (text, tone) = crate::derive::label_chip(label, cx);
                    d.child(pill::state_chip(text, tone, theme))
                })
                .child(time)
                .children(machine)
        });

        // Content: the answer's echo, the readable line, the text (folded), or "(message has no text)".
        let content: AnyElement = if let Some(echo) = echo {
            div().mt(px(space::HAIR)).child(echo).into_any_element()
        } else if let Some(parts) = msg.readable.as_ref().filter(|p| !p.is_empty()) {
            readable_line(&id, parts, theme, cx)
        } else if !msg.text.is_empty() {
            let text = words::without_label_prefix(&msg.text, msg.label.as_deref());
            let fold = opts.fold && !opts.in_thread;
            message_text(
                &id,
                &text,
                &msg.mentions,
                fold,
                unfolded,
                state.clone(),
                theme,
                cx,
            )
        } else {
            ui::text(
                t(cx, "messages.noText"),
                size::SMALL,
                weight::REGULAR,
                theme.c("ink-4"),
            )
            .mt(px(space::HAIR))
            .into_any_element()
        };

        // The operator note's delivery line.
        let delivery = msg.delivery.as_ref().map(|d| {
            // M6: a quiet `META` line; answered is the one state it shows.
            let answered = d.state == "answered";
            ui::text(
                words::delivery_line(d, self.reader.as_deref(), cx),
                size::META,
                weight::REGULAR,
                if answered {
                    theme.c("ok-ink")
                } else {
                    theme.c("ink-3")
                },
            )
            .mt(px(space::HAIR))
        });

        // Files, under "Technical details" when there is a technical body.
        let files = (!msg.files.is_empty()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap(px(space::MD))
                .max_w_full()
                .children(msg.files.iter().map(|f| file_card(f, cx)))
                .into_any_element()
        });
        let tech: Option<AnyElement> = match legible::tech_body(msg) {
            Some(body) => Some(tech_details(
                body,
                files,
                details_open,
                state.clone(),
                theme,
                cx,
            )),
            None => files,
        };

        let verified =
            msg.verified.as_ref().map(|v| {
                let title = tv(
                    cx,
                    "messages.verifiedTitle",
                    &[
                        ("sha", &v.sha),
                        ("branch", &v.branch),
                        ("time", &hhmm(v.decided_ms)),
                    ],
                );
                // `.hb-pill`: the console's round, ringed "verified" (§3.1).
                div().flex().mt(px(space::SM)).child(
                    div()
                        .id("hb-pill")
                        .tooltip(tooltip(title))
                        .child(pill::ok_pill(
                            "circle-check",
                            t(cx, "messages.verified"),
                            theme,
                        )),
                )
            });

        let repeats = (!self.repeats.is_empty()).then(|| {
            let times = self
                .repeats
                .iter()
                .map(|&ms| hhmm(ms))
                .collect::<Vec<_>>()
                .join(", ");
            let line = if self.repeats.len() == 1 {
                tv(cx, "messages.repeatOnce", &[("times", &times)])
            } else {
                tv(
                    cx,
                    "messages.repeatMany",
                    &[("n", &self.repeats.len().to_string()), ("times", &times)],
                )
            };
            ui::text(line, size::SMALL, weight::REGULAR, theme.c("ink-3")).mt(px(space::SM))
        });

        let replies = match &opts.on_thread {
            Some(open) if !opts.in_thread && !msg.replies.is_empty() => {
                let open = open.clone();
                let mid = id.clone();
                // Quiet ink, not the accent: in themes whose accent is a
                // coral it read as one more red, and red means blocked (C3).
                let ink = theme.c("ink-2");
                let link = theme.c("link");
                Some(
                    div().flex().mt(px(space::SM)).child(
                        quiet_line("hb-replies", theme)
                            .gap(px(space::SM))
                            .text_size(px(size::UI))
                            .line_height(px(leading::UI))
                            .font_weight(weight::STRONG)
                            .text_color(ink)
                            .hover(move |s| s.underline().text_color(link))
                            .on_click(move |_, window, cx| (open.0)(&mid, window, cx))
                            .child(ui::icon("message-square", icon_size::ROW, theme.c("ink-3")))
                            .child(words::reply_count(msg.replies.len(), cx)),
                    ),
                )
            }
            _ => None,
        };

        let tools = (!opts.read_only).then(|| self.tools(feedback, below, theme, cx));
        // Where the row sits in the visible stream, read as it paints: the
        // tools flip below when there is no room above.
        let probe = (!opts.read_only).then(|| {
            let state = state.clone();
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let room = f32::from(bounds.top() - window.content_mask().bounds.top());
                    let flip = tools_below(room);
                    if state.read(cx).tools_below != flip {
                        state.update(cx, |s, cx| {
                            s.tools_below = flip;
                            cx.notify();
                        });
                    }
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        });

        // One gutter and one avatar size in every stream (S2, Z4); a direct
        // conversation only breathes a little more between rows.
        let pad_y = if opts.dm { space::LG } else { space::MD };
        let hover_bg = theme.c("hover");
        div()
            .id(ElementId::Name(format!("hb-msg-row-{id}").into()))
            .group(ROW_GROUP)
            .relative()
            .w_full()
            .flex()
            .gap(px(space::LG))
            .px(px(space::GUTTER))
            .py(px(pad_y))
            .font_family(ui::SANS)
            .text_size(px(size::BODY))
            .line_height(px(leading::BODY))
            .font_weight(weight::REGULAR)
            .text_color(theme.c("ink"))
            .when(selected, |d| d.bg(theme.c("selected")))
            .when(!selected, |d| d.hover(move |s| s.bg(hover_bg)))
            .children(probe)
            .child(face)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .children(meta)
                    .child(content)
                    .children(delivery)
                    .children(attachments)
                    .children(tech)
                    .children(verified)
                    .children(repeats)
                    .children(replies),
            )
            .children(tools)
    }
}

impl MessageRow {
    /// `.hb-msg-tools`: feedback, reply in a thread, save for later; shown
    /// while the row is hovered.
    fn tools(
        &self,
        feedback: Option<AnyElement>,
        below: bool,
        theme: &Theme,
        cx: &App,
    ) -> AnyElement {
        let msg = &self.msg;
        let tool = |id: &'static str, icon: &str, title: SharedString| {
            focusable(
                div()
                    .id(id)
                    .size(px(control::SM))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(radius::SM))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.c("control")))
                    .tooltip(tooltip(title)),
                true,
                theme,
            )
            .child(ui::icon(icon, icon_size::ROW, theme.c("ink-2")))
        };
        let thread = match &self.opts.on_thread {
            Some(open) if msg.reply_to.is_none() && !self.opts.in_thread => {
                let open = open.clone();
                let mid = msg.id.clone();
                Some(
                    tool(
                        "hb-tool-thread",
                        "message-square",
                        t(cx, "messages.replyInThread"),
                    )
                    .on_click(move |_, window, cx| (open.0)(&mid, window, cx)),
                )
            }
            _ => None,
        };
        let saved = self.saved;
        let mid = msg.id.clone();
        let save = tool(
            "hb-tool-save",
            if saved { "bookmark-check" } else { "bookmark" },
            if saved {
                t(cx, "messages.unsave")
            } else {
                t(cx, "messages.save")
            },
        )
        .on_click(move |_, _, cx| {
            let action = if saved { "unsave" } else { "save" };
            // Errors are swallowed, as in the console: the next read shows the truth.
            Desk::get(cx)
                .update(cx, |d, cx| d.post(action, json!({ "id": mid }), cx))
                .detach();
        });
        // `.hb-msg-tools`: a raised edge with the console's small shadow;
        // it ends at the gutter, and hangs below the row when the row is
        // the first one under the header.
        div()
            .absolute()
            .map(|d| {
                if below {
                    d.bottom(px(-TOOLS_RISE))
                } else {
                    d.top(px(-TOOLS_RISE))
                }
            })
            .right(px(space::GUTTER))
            .shadow(pill::tools_shadow(theme))
            .flex()
            .gap(px(space::HAIR))
            .p(px(space::HAIR))
            .bg(theme.c("surface-raised"))
            .border_1()
            .border_color(theme.c("line-strong"))
            .rounded(px(radius::BASE))
            .invisible()
            .group_hover(ROW_GROUP, |s| s.visible())
            .children(feedback)
            .children(thread)
            .child(save)
            .into_any_element()
    }
}

/// `.hb-mention`: bold accent ink; its rounded wash is painted underneath
/// ([`inline_chip`]).
fn mention_style(theme: &Theme) -> HighlightStyle {
    HighlightStyle {
        color: Some(theme.c("accent-text")),
        font_weight: Some(weight::STRONG),
        ..Default::default()
    }
}

/// `.hb-mention`'s wash: `accent-soft`, radius 4.
fn mention_chip(range: Range<usize>, theme: &Theme) -> Chip {
    Chip {
        range,
        fill: theme.c("accent-soft"),
        radius: radius::SM,
    }
}

/// Text with its known `@mentions` set apart (`MentionText`); returns the
/// text and, per mention, the range of its chip (padding included).
fn mention_runs(text: &str, mentions: &[String]) -> (String, Vec<Range<usize>>) {
    let mut out = String::with_capacity(text.len());
    let mut ranges = Vec::new();
    for seg in words::mention_segments(text, mentions) {
        if seg.mention {
            let (chip, _) = inline_chip::push_padded(&mut out, &seg.text, inline_chip::MENTION_PAD);
            ranges.push(chip);
        } else {
            out.push_str(&seg.text);
        }
    }
    (out, ranges)
}

/// `MessageText`: the text, folded behind "Show more" when long.
#[allow(clippy::too_many_arguments)]
fn message_text(
    id: &str,
    text: &str,
    mentions: &[String],
    fold: bool,
    unfolded: bool,
    state: gpui::Entity<RowState>,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    let head = if fold { words::fold_text(text) } else { None };
    let mention = mention_style(theme);
    let Some(head) = head else {
        let (body, ranges) = mention_runs(text, mentions);
        let chips = ranges
            .iter()
            .map(|r| mention_chip(r.clone(), theme))
            .collect();
        let styled =
            StyledText::new(body.clone()).with_highlights(ranges.into_iter().map(|r| (r, mention)));
        let layout = styled.layout().clone();
        return div()
            .mt(px(space::HAIR))
            .w_full()
            .relative()
            .child(inline_chip::backdrop(layout, &body, chips))
            .child(styled)
            .into_any_element();
    };
    let (mut body, ranges) = if unfolded {
        mention_runs(text, mentions)
    } else {
        mention_runs(&format!("{head}…"), mentions)
    };
    let chips: Vec<Chip> = ranges
        .iter()
        .map(|r| mention_chip(r.clone(), theme))
        .collect();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> =
        ranges.into_iter().map(|r| (r, mention)).collect();
    body.push(' ');
    let start = body.len();
    body.push_str(&if unfolded {
        t(cx, "messages.seeLess")
    } else {
        t(cx, "messages.seeMore")
    });
    let toggle = start..body.len();
    highlights.push((
        toggle.clone(),
        HighlightStyle {
            color: Some(theme.c("link")),
            ..Default::default()
        },
    ));
    let styled = StyledText::new(body.clone()).with_highlights(highlights);
    let layout = styled.layout().clone();
    let text = InteractiveText::new(ElementId::Name(format!("hb-msg-text-{id}").into()), styled)
        .on_click(vec![toggle], move |_, _, cx| {
            state.update(cx, |s, cx| {
                s.unfolded = !s.unfolded;
                cx.notify();
            })
        });
    div()
        .mt(px(space::HAIR))
        .w_full()
        .relative()
        .child(inline_chip::backdrop(layout, &body, chips))
        .child(text)
        .into_any_element()
}

/// `ReadableLine`: the plain line shown for a line written for machines,
/// with commit codes, branches and paths as small grey monospaced chips
/// (`.hb-tok`, the whole value in their tooltip).
fn readable_line(
    id: &str,
    parts: &[crate::model::ReadablePart],
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    let mut body = String::new();
    let mut highlights = Vec::new();
    let mut mono = Vec::new();
    let mut chips = Vec::new();
    let mut tips: Vec<(Range<usize>, SharedString)> = Vec::new();
    let token = HighlightStyle {
        color: Some(theme.c("ink-4")),
        ..Default::default()
    };
    for p in parts {
        if p.kind == "text" {
            body.push_str(&legible::readable_part_text(p, cx));
            continue;
        }
        let (chip, word) = inline_chip::push_padded(&mut body, &p.text, inline_chip::TOKEN_PAD);
        highlights.push((word.clone(), token));
        mono.push((word, SharedString::from(ui::MONO)));
        chips.push(Chip {
            range: chip.clone(),
            fill: theme.c("hover-2"),
            radius: TOKEN_RADIUS,
        });
        tips.push((
            chip,
            p.full.clone().unwrap_or_else(|| p.text.clone()).into(),
        ));
    }
    let styled = StyledText::new(body.clone())
        .with_highlights(highlights)
        .with_font_family_overrides(mono);
    let layout = styled.layout().clone();
    let text = InteractiveText::new(ElementId::Name(format!("hb-readable-{id}").into()), styled)
        .tooltip(move |ix, window, cx| {
            tips.iter()
                .find(|(r, _)| r.contains(&ix))
                .map(|(_, tip)| ui::kit_theme::tip(tip.clone())(window, cx))
        });
    div()
        .mt(px(space::HAIR))
        .w_full()
        .relative()
        .child(inline_chip::backdrop(layout, &body, chips))
        .child(text)
        .into_any_element()
}

/// `.hb-tok { border-radius: 3px }`.
const TOKEN_RADIUS: f32 = 3.;

/// `.hb-tech-body { border-left: 3px; padding: 10px 12px }`.
const RULE_W: f32 = 3.;
const TECH_PAD_Y: f32 = 10.;

/// `TechDetails`: "▸ Technical details", opening the body and the files.
fn tech_details(
    body: String,
    files: Option<AnyElement>,
    open: bool,
    state: gpui::Entity<RowState>,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    // `.hb-tech > summary`: 13/700 in `ink-3`.
    let ink = theme.c("ink-3");
    let summary = quiet_line("hb-tech", theme)
        .font_weight(weight::STRONG)
        .text_color(ink)
        .hover(|s| s.text_color(theme.c("ink-2")))
        .on_click(move |_, _, cx| {
            state.update(cx, |s, cx| {
                s.details = Some(!open);
                cx.notify();
            })
        })
        .child(ui::icon(
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            icon_size::INLINE,
            ink,
        ))
        .child(t(cx, "messages.techDetails"));
    div()
        .mt(px(space::XS))
        .flex()
        .flex_col()
        .child(div().flex().child(summary))
        .when(open, |d| {
            // `.hb-tech-body`: a 3 px rule at its leading edge, not a box.
            d.child(
                div()
                    .mt(px(space::SM))
                    .py(px(TECH_PAD_Y))
                    .px(px(space::LG))
                    .border_l(px(RULE_W))
                    .border_color(theme.c("line-strong"))
                    .bg(theme.c("surface-2"))
                    .text_color(theme.c("ink-2"))
                    .text_size(px(size::SMALL))
                    .line_height(px(leading::SMALL))
                    .child(body),
            )
            .children(files)
        })
        .into_any_element()
}

/// `FileCard`: a local file; clicking asks serve to open it on this Mac.
/// An error ("no longer on this Mac", "couldn't open") shows under it.
pub fn file_card(path: &str, cx: &App) -> AnyElement {
    let _ = cx;
    FileCard {
        path: path.to_string().into(),
    }
    .into_any_element()
}

/// A file card's narrowest and widest.
const FILE_CARD: (f32, f32) = (240., 420.);

#[derive(IntoElement)]
struct FileCard {
    path: SharedString,
}

impl RenderOnce for FileCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let error = window.use_keyed_state(
            ElementId::Name(format!("hb-file-err-{}", self.path).into()),
            cx,
            |_, _| None::<SharedString>,
        );
        let message = error.read(cx).clone();
        let cx: &App = cx;
        let theme = cx.global::<Theme>();
        let path = self.path.clone();
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        let title = tv(cx, "messages.openFile", &[("path", &path)]);
        let missing = t(cx, "messages.fileMissing");
        let failed = t(cx, "messages.fileOpenFailed");
        let hover_bg = theme.c("file-hover");
        let hover_line = theme.c("line-hover");
        let card = focusable(
            div()
                .id(ElementId::Name(format!("hb-file-{path}").into()))
                .flex()
                .items_center()
                .gap(px(space::LG))
                .mt(px(space::MD))
                .min_w(px(FILE_CARD.0))
                .max_w(px(FILE_CARD.1))
                .px(px(space::LG))
                .py(px(space::MD))
                .rounded(px(radius::MD))
                .border_1()
                .border_color(theme.c("line"))
                .text_color(theme.c("ink"))
                .cursor_pointer()
                .overflow_hidden()
                .hover(move |s| s.bg(hover_bg).border_color(hover_line))
                .tooltip(tooltip(title)),
            false,
            theme,
        )
        .on_click({
            let path = path.clone();
            let error = error.clone();
            move |_, _, cx| {
                error.update(cx, |e, cx| {
                    *e = None;
                    cx.notify();
                });
                let task = Desk::get(cx).update(cx, |d, cx| {
                    d.post("open", json!({ "path": path.as_ref() }), cx)
                });
                let error = error.clone();
                let (missing, failed) = (missing.clone(), failed.clone());
                cx.spawn(async move |cx| {
                    if let Err(e) = task.await {
                        let text = if e.status == Some(404) {
                            missing
                        } else {
                            failed
                        };
                        cx.update(|cx| {
                            error.update(cx, |slot, cx| {
                                *slot = Some(text);
                                cx.notify();
                            })
                        });
                    }
                })
                .detach();
            }
        })
        .child(ui::icon("file-text", icon_size::RAIL, theme.c("ink-3")))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .overflow_hidden()
                .child(ui::text(name, size::UI, weight::STRONG, theme.c("ink")).truncate())
                .child(
                    ui::text(
                        ui::format::short_path(&path),
                        size::META,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .font_family(ui::MONO)
                    .truncate(),
                ),
        );
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .max_w_full()
            .child(card)
            .when_some(message, |d, m| {
                d.child(
                    ui::text(m, size::SMALL, weight::REGULAR, theme.c("red-ink")).mt(px(space::XS)),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_hang_below_only_without_room_above() {
        assert!(tools_below(0.));
        assert!(tools_below(TOOLS_RISE));
        assert!(!tools_below(TOOLS_RISE + 1.));
        assert!(!tools_below(200.));
    }

    #[test]
    fn mentions_are_padded_inside_their_chip() {
        let (text, ranges) = mention_runs("ask @cart now", &["cart".to_string()]);
        assert_eq!(ranges.len(), 1);
        let chip = &text[ranges[0].clone()];
        assert!(chip.starts_with(inline_chip::MENTION_PAD));
        assert!(chip.ends_with(inline_chip::MENTION_PAD));
        assert!(chip.contains("@cart"));
    }
}
