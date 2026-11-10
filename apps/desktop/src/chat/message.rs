//! One message row (`MessageItem`, `parts.tsx`): the avatar, the meta line,
//! the text (readable line, folding, mentions), the operator note's
//! delivery line, attachments, files under "Technical details", the
//! verified pill, repeats, the replies link and the hover tools.
//!
//! Sizes and colours follow `workspace.css` (`.hb-msg*`, `.hb-chip`,
//! `.hb-tech`, `.hb-file`, `.hb-replies`, `.hb-msg-tools`, `.hb-delivery`,
//! `.hb-pill`, `.hb-machine`, `.hb-mention`, `.hb-tok`).

use super::MessageOpts;
use crate::derive::legible::{self, MachineMark};
use crate::derive::messages as words;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Agent, Message, WorkspaceView};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{self, avatar, format::hhmm};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ElementId, Empty, FontWeight, HighlightStyle, InteractiveElement, InteractiveText,
    IntoElement, ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, StyledText, Window, div, px, relative,
};
use gpui_kit::component::tooltip::Tooltip;
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
    if msg.repeat_of.as_deref().is_some_and(|r| view.message(r).is_some()) {
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
    fn new(msg: &Message, view: &WorkspaceView, opts: &MessageOpts, machine: Option<Option<MachineMark>>, cx: &App) -> Self {
        let agent = view.agent(&msg.agent);
        let from = msg.from.as_str();
        let face = match from {
            "agent" => agent.map_or_else(|| Face::Named(String::new()), |a| Face::Agent(Box::new(a.clone()))),
            "peer" | "session" => view
                .agents
                .iter()
                .find(|a| a.key == msg.agent || a.name == msg.author)
                .map_or_else(|| Face::Named(String::new()), |a| Face::Agent(Box::new(a.clone()))),
            "lead" => match words::lead_key_of(&view.agents, &msg.agent) {
                Some(key) => view.agent(&key).map_or(Face::Named(key.clone()), |a| Face::Agent(Box::new(a.clone()))),
                None => Face::Lead,
            },
            "operator" => Face::Operator,
            _ => Face::Monogram,
        };
        let name: SharedString = match (from, agent) {
            ("agent", Some(a)) => crate::derive::agent_name(a, cx),
            ("operator", _) => crate::derive::shown_name(&words::operator_name(&msg.author, cx), cx),
            ("lead", _) => crate::derive::shown_name(words::lead_display_name(&msg.author), cx),
            _ => crate::derive::shown_name(&msg.author, cx),
        };
        // A robot without an agent is drawn from the name shown.
        let face = match face {
            Face::Named(n) if n.is_empty() => Face::Named(name.to_string()),
            other => other,
        };
        let link = if from == "agent" { agent.map(|a| Box::new(a.clone())) } else { None };
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
            repeats: words::repeats_of(view, &msg.id).iter().map(|m| m.ts_ms).collect(),
            reader: msg.delivery.as_ref().and_then(|_| words::note_reader(&view.agents, &msg.agent)),
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
}

/// The hover card around an agent's avatar and name (`AgentHover`).
/// Hook for the agents lane: returns the child unchanged for now.
fn agent_hover(child: AnyElement, agent: Option<&Agent>, cx: &App) -> AnyElement {
    let _ = (agent, cx);
    child
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

/// The 👍/👎 row in the hover tools (`FeedbackRow`).
/// Hook for the activity lane: draws nothing for now.
fn feedback_row(kind: &'static str, msg: &Message, cx: &App) -> Option<AnyElement> {
    let _ = (kind, msg, cx);
    None
}

/// "10/11/2026, 22:41:07": the full time in the timestamp's tooltip.
fn full_time(ms: u64) -> String {
    let (y, mo, d, _) = ui::format::local_date(ms);
    let (h, m) = ui::format::local_hm(ms);
    let s = (ms / 1000) % 60;
    format!("{d:02}/{mo:02}/{y}, {h:02}:{m:02}:{s:02}")
}

fn tooltip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

/// `.hb-chip`: a label chip in its tone, or the lead tag.
pub(crate) fn chip(text: SharedString, tone: &str, theme: &Theme) -> gpui::Div {
    let (bg, fg, edge) = match tone {
        "amber" => (theme.c("amber-bg"), theme.c("amber-ink"), None),
        "red" => (theme.c("red-bg"), theme.c("red-ink"), Some(theme.c("red-line"))),
        "green" => (theme.c("green-bg"), theme.c("green-ink"), Some(theme.c("green-line"))),
        "lead" => (theme.c("accent-soft"), theme.c("accent-text"), None),
        _ => (theme.c("chip"), theme.c("chip-ink"), None),
    };
    let lead = tone == "lead";
    div()
        .flex_none()
        .flex()
        .items_center()
        .h(px(if lead { 20. } else { 22. }))
        .px(px(8.))
        .rounded(px(6.))
        .bg(bg)
        .text_color(fg)
        .text_size(px(if lead { 12. } else { 13. }))
        .font_weight(FontWeight::BOLD)
        .when_some(edge, |d, e| d.border_1().border_color(e))
        .child(text)
}

impl RenderOnce for MessageRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.msg.id.clone();
        let state = window.use_keyed_state(ElementId::Name(format!("hb-msg-{id}").into()), cx, |_, _| RowState::default());
        let (unfolded, details_open) = {
            let s = state.read(cx);
            (s.unfolded, s.details.unwrap_or(self.opts.in_thread))
        };
        let attachments = super::attachments::message_attachments(&self.msg, self.opts.in_thread, window, cx);
        let cx: &App = cx;
        let theme = cx.global::<Theme>();
        let msg = &self.msg;
        let opts = &self.opts;
        let dm = opts.dm;
        let text_size = if dm { 16. } else { 15. };
        let selected = opts.selected.as_deref() == Some(id.as_str());

        // Avatar column.
        let face: AnyElement = if opts.continued {
            div().w(px(36.)).flex_none().into_any_element()
        } else {
            let el = match &self.face {
                Face::Agent(a) => avatar::agent_avatar(a, 36., true, cx),
                Face::Named(n) => avatar::name_avatar(n, 36., cx),
                Face::Lead => avatar::lead_avatar(36., cx),
                Face::Operator => avatar::operator_avatar_with(&self.name, &self.operator_initials, 36., None, cx),
                Face::Monogram => avatar::monogram_avatar(&self.name, Some(theme.c("av-lead")), None, 36., None, cx),
            };
            let agent = match &self.face {
                Face::Agent(a) => Some(a.as_ref()),
                _ => None,
            };
            agent_hover(div().flex_none().child(el).into_any_element(), agent, cx)
        };

        // Meta line: name, lead tag, label, time, "written for machines".
        let meta = (!opts.continued).then(|| {
            let name_el = div()
                .id("hb-msg-name")
                .font_weight(FontWeight::BLACK)
                .text_size(px(if dm { 17. } else { 15. }))
                .text_color(theme.c("ink"))
                .child(self.name.clone());
            let name_el: AnyElement = match &self.link {
                Some(agent) => {
                    let key = agent.key.clone();
                    let el = name_el
                        .cursor_pointer()
                        .hover(|s| s.underline())
                        .on_click(move |_, _, cx| nav::navigate(Route::direct(Some(key.clone())), cx))
                        .into_any_element();
                    agent_hover(el, Some(agent), cx)
                }
                None => name_el.into_any_element(),
            };
            let time = div()
                .id("hb-msg-time")
                .text_size(px(12.))
                .text_color(theme.c("ink-3"))
                .tooltip(tooltip(full_time(msg.ts_ms).into()))
                .child(hhmm(msg.ts_ms));
            let machine = self.machine.as_ref().map(|mark| {
                div()
                    .id("hb-machine")
                    .text_size(px(12.))
                    .text_color(theme.c("ink-4"))
                    .text_decoration_1()
                    .text_decoration_color(theme.c("line"))
                    .tooltip(tooltip(mark.hint(cx)))
                    .child(legible::machine_text(cx))
            });
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .line_height(relative(1.3))
                .child(name_el)
                .when(msg.from == "lead", |d| d.child(chip(t(cx, "workspace.leadRole"), "lead", theme)))
                .when_some(msg.label.as_deref(), |d, label| {
                    let (text, tone) = crate::derive::label_chip(label, cx);
                    d.child(chip(text, tone, theme))
                })
                .child(time)
                .children(machine)
        });

        // Content: the readable line, the text (folded), or "(message has no text)".
        let content: AnyElement = if let Some(parts) = msg.readable.as_ref().filter(|p| !p.is_empty()) {
            readable_line(&id, parts, theme, cx)
        } else if !msg.text.is_empty() {
            let text = words::without_label_prefix(&msg.text, msg.label.as_deref());
            let fold = opts.fold && !opts.in_thread;
            message_text(&id, &text, &msg.mentions, fold, unfolded, state.clone(), theme, cx)
        } else {
            div()
                .mt(px(2.))
                .child(
                    div()
                        .text_size(px(13.))
                        .italic()
                        .text_color(theme.c("ink-4"))
                        .child(t(cx, "messages.noText")),
                )
                .into_any_element()
        };

        // The operator note's delivery line.
        let delivery = msg.delivery.as_ref().map(|d| {
            let answered = d.state == "answered";
            div()
                .mt(px(2.))
                .text_size(px(12.))
                .text_color(if answered { theme.c("ok-ink") } else { theme.c("ink-4") })
                .when(answered, |d| d.font_weight(FontWeight::BOLD))
                .child(words::delivery_line(d, self.reader.as_deref(), cx))
        });

        // Files, under "Technical details" when there is a technical body.
        let files = (!msg.files.is_empty()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .max_w_full()
                .children(msg.files.iter().map(|f| file_card(f, cx)))
                .into_any_element()
        });
        let tech: Option<AnyElement> = match legible::tech_body(msg) {
            Some(body) => Some(tech_details(body, files, details_open, state.clone(), theme, cx)),
            None => files,
        };

        let verified = msg.verified.as_ref().map(|v| {
            let title = tv(cx, "messages.verifiedTitle", &[("sha", &v.sha), ("branch", &v.branch), ("time", &hhmm(v.decided_ms))]);
            div().flex().child(
                div()
                    .id("hb-pill")
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .h(px(24.))
                    .px(px(8.))
                    .mt(px(6.))
                    .rounded(px(12.))
                    .bg(theme.c("ok-bg"))
                    .border_1()
                    .border_color(theme.c("ok-line"))
                    .text_color(theme.c("ok-ink"))
                    .text_size(px(12.))
                    .font_weight(FontWeight::BOLD)
                    .tooltip(tooltip(title))
                    .child(ui::icon("circle-check", 14., theme.c("ok-ink")))
                    .child(t(cx, "messages.verified")),
            )
        });

        let repeats = (!self.repeats.is_empty()).then(|| {
            let times = self.repeats.iter().map(|&ms| hhmm(ms)).collect::<Vec<_>>().join(", ");
            let line = if self.repeats.len() == 1 {
                tv(cx, "messages.repeatOnce", &[("times", &times)])
            } else {
                tv(cx, "messages.repeatMany", &[("n", &self.repeats.len().to_string()), ("times", &times)])
            };
            div().mt(px(6.)).text_size(px(13.)).text_color(theme.c("ink-3")).child(line)
        });

        let replies = match &opts.on_thread {
            Some(open) if !opts.in_thread && !msg.replies.is_empty() => {
                let open = open.clone();
                let mid = id.clone();
                let link = theme.c("link");
                Some(
                    div().flex().child(
                        div()
                            .id("hb-replies")
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .mt(px(6.))
                            .text_size(px(14.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(link)
                            .cursor_pointer()
                            .hover(|s| s.underline())
                            .on_click(move |_, window, cx| (open.0)(&mid, window, cx))
                            .child(ui::icon("message-square", 18., link))
                            .child(words::reply_count(msg.replies.len(), cx)),
                    ),
                )
            }
            _ => None,
        };

        let tools = (!opts.read_only).then(|| self.tools(theme, cx));

        let (pad_x, pad_y, gap) = if dm { (28., 12., 16.) } else { (20., 8., 10.) };
        let hover_bg = theme.c("hover");
        div()
            .id(ElementId::Name(format!("hb-msg-row-{id}").into()))
            .group(ROW_GROUP)
            .relative()
            .w_full()
            .flex()
            .gap(px(gap))
            .px(px(pad_x))
            .py(px(pad_y))
            .font_family(ui::SANS)
            .text_size(px(text_size))
            .line_height(relative(1.46))
            .text_color(theme.c("ink"))
            .when(selected, |d| d.bg(theme.c("selected")))
            .when(!selected, |d| d.hover(move |s| s.bg(hover_bg)))
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
    fn tools(&self, theme: &Theme, cx: &App) -> AnyElement {
        let msg = &self.msg;
        let tool = |id: &'static str, icon: &str, title: SharedString| {
            div()
                .id(id)
                .h(px(28.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(theme.c("control")))
                .tooltip(tooltip(title))
                .child(ui::icon(icon, 16., theme.c("ink-2")))
        };
        let thread = match &self.opts.on_thread {
            Some(open) if msg.reply_to.is_none() && !self.opts.in_thread => {
                let open = open.clone();
                let mid = msg.id.clone();
                Some(
                    tool("hb-tool-thread", "message-square", t(cx, "messages.replyInThread"))
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
            if saved { t(cx, "messages.unsave") } else { t(cx, "messages.save") },
        )
        .on_click(move |_, _, cx| {
            let action = if saved { "unsave" } else { "save" };
            // Errors are swallowed, as in the console: the next read shows the truth.
            Desk::get(cx).update(cx, |d, cx| d.post(action, json!({ "id": mid }), cx)).detach();
        });
        div()
            .absolute()
            .top(px(-12.))
            .right(px(20.))
            .flex()
            .gap(px(2.))
            .p(px(2.))
            .bg(theme.c("surface"))
            .border_1()
            .border_color(theme.c("line-strong"))
            .rounded(px(6.))
            .shadow(tools_shadow(theme))
            .invisible()
            .group_hover(ROW_GROUP, |s| s.visible())
            .children(feedback_kind(msg).and_then(|k| feedback_row(k, msg, cx)))
            .children(thread)
            .child(save)
            .into_any_element()
    }
}

/// `--hb-tools-shadow`: a small drop under the hover tools.
fn tools_shadow(theme: &Theme) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color: if theme.dark { gpui::hsla(0., 0., 0., 0.45) } else { gpui::hsla(0., 0., 0., 0.12) },
        offset: gpui::point(px(0.), px(1.)),
        blur_radius: px(3.),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// `.hb-mention` and the like as text highlights.
fn mention_style(theme: &Theme) -> HighlightStyle {
    HighlightStyle {
        color: Some(theme.c("accent-text")),
        background_color: Some(theme.c("accent-soft")),
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    }
}

/// Text with its known `@mentions` highlighted (`MentionText`); returns the
/// text and the highlight ranges.
fn mention_runs(text: &str, mentions: &[String]) -> (String, Vec<Range<usize>>) {
    let mut out = String::with_capacity(text.len());
    let mut ranges = Vec::new();
    for seg in words::mention_segments(text, mentions) {
        let start = out.len();
        out.push_str(&seg.text);
        if seg.mention {
            ranges.push(start..out.len());
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
        let styled = StyledText::new(body).with_highlights(ranges.into_iter().map(|r| (r, mention)));
        return div().mt(px(2.)).w_full().child(styled).into_any_element();
    };
    let (mut body, mut highlights): (String, Vec<(Range<usize>, HighlightStyle)>) = if unfolded {
        let (b, r) = mention_runs(text, mentions);
        (b, r.into_iter().map(|r| (r, mention)).collect())
    } else {
        let (b, r) = mention_runs(&format!("{head}…"), mentions);
        (b, r.into_iter().map(|r| (r, mention)).collect())
    };
    body.push(' ');
    let start = body.len();
    body.push_str(&if unfolded { t(cx, "messages.seeLess") } else { t(cx, "messages.seeMore") });
    let toggle = start..body.len();
    highlights.push((
        toggle.clone(),
        HighlightStyle { color: Some(theme.c("link")), font_weight: Some(FontWeight::BOLD), ..Default::default() },
    ));
    let text = InteractiveText::new(
        ElementId::Name(format!("hb-msg-text-{id}").into()),
        StyledText::new(body).with_highlights(highlights),
    )
    .on_click(vec![toggle], move |_, _, cx| {
        state.update(cx, |s, cx| {
            s.unfolded = !s.unfolded;
            cx.notify();
        })
    });
    div().mt(px(2.)).w_full().child(text).into_any_element()
}

/// `ReadableLine`: the plain line shown for a line written for machines,
/// with commit codes, branches and paths as small grey monospaced chips
/// (the whole value in their tooltip).
fn readable_line(id: &str, parts: &[crate::model::ReadablePart], theme: &Theme, cx: &App) -> AnyElement {
    let mut body = String::new();
    let mut highlights = Vec::new();
    let mut mono = Vec::new();
    let mut tips: Vec<(Range<usize>, SharedString)> = Vec::new();
    let token = HighlightStyle { color: Some(theme.c("ink-4")), background_color: Some(theme.c("hover-2")), ..Default::default() };
    for p in parts {
        let start = body.len();
        if p.kind == "text" {
            body.push_str(&legible::readable_part_text(p, cx));
            continue;
        }
        body.push_str(&p.text);
        let range = start..body.len();
        highlights.push((range.clone(), token));
        mono.push((range.clone(), SharedString::from(ui::MONO)));
        tips.push((range, p.full.clone().unwrap_or_else(|| p.text.clone()).into()));
    }
    let styled = StyledText::new(body).with_highlights(highlights).with_font_family_overrides(mono);
    let text = InteractiveText::new(ElementId::Name(format!("hb-readable-{id}").into()), styled).tooltip(move |ix, window, cx| {
        tips.iter().find(|(r, _)| r.contains(&ix)).map(|(_, tip)| Tooltip::new(tip.clone()).build(window, cx))
    });
    div().mt(px(2.)).w_full().child(text).into_any_element()
}

/// `TechDetails`: "▸ Technical details", opening the body and the files.
fn tech_details(
    body: String,
    files: Option<AnyElement>,
    open: bool,
    state: gpui::Entity<RowState>,
    theme: &Theme,
    cx: &App,
) -> AnyElement {
    let summary = div()
        .id("hb-tech")
        .flex()
        .items_center()
        .gap(px(4.))
        .cursor_pointer()
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.c("ink-3"))
        .on_click(move |_, _, cx| {
            state.update(cx, |s, cx| {
                s.details = Some(!open);
                cx.notify();
            })
        })
        .child(div().text_size(px(11.)).child(if open { "▾" } else { "▸" }))
        .child(t(cx, "messages.techDetails"));
    div()
        .mt(px(6.))
        .flex()
        .flex_col()
        .child(div().flex().child(summary))
        .when(open, |d| {
            d.child(
                div()
                    .mt(px(6.))
                    .px(px(12.))
                    .py(px(10.))
                    .border_l(px(3.))
                    .border_color(theme.c("line-strong"))
                    .bg(theme.c("surface-2"))
                    .text_color(theme.c("ink-2"))
                    .font_family(ui::MONO)
                    .text_size(px(12.5))
                    .line_height(relative(1.5))
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
    FileCard { path: path.to_string().into() }.into_any_element()
}

#[derive(IntoElement)]
struct FileCard {
    path: SharedString,
}

impl RenderOnce for FileCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let error = window.use_keyed_state(ElementId::Name(format!("hb-file-err-{}", self.path).into()), cx, |_, _| None::<SharedString>);
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
        let card = div()
            .id(ElementId::Name(format!("hb-file-{path}").into()))
            .flex()
            .items_center()
            .gap(px(12.))
            .mt(px(8.))
            .min_w(px(240.))
            .max_w(px(420.))
            .px(px(16.))
            .py(px(12.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.c("line-strong"))
            .bg(theme.c("surface-2"))
            .text_color(theme.c("ink"))
            .font_weight(FontWeight::BOLD)
            .text_size(px(15.))
            .cursor_pointer()
            .overflow_hidden()
            .hover(move |s| s.bg(hover_bg).border_color(hover_line))
            .tooltip(tooltip(title))
            .on_click({
                let path = path.clone();
                let error = error.clone();
                move |_, _, cx| {
                    error.update(cx, |e, cx| {
                        *e = None;
                        cx.notify();
                    });
                    let task = Desk::get(cx).update(cx, |d, cx| d.post("open", json!({ "path": path.as_ref() }), cx));
                    let error = error.clone();
                    let (missing, failed) = (missing.clone(), failed.clone());
                    cx.spawn(async move |cx| {
                        if let Err(e) = task.await {
                            let text = if e.status == Some(404) { missing } else { failed };
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
            .child(ui::icon("file-text", 24., theme.c("ink")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .overflow_hidden()
                    .child(div().truncate().child(name))
                    .child(
                        div()
                            .truncate()
                            .font_weight(FontWeight::NORMAL)
                            .text_size(px(12.))
                            .text_color(theme.c("ink-4"))
                            .child(ui::format::short_path(&path)),
                    ),
            );
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .max_w_full()
            .child(card)
            .when_some(message, |d, m| d.child(div().text_size(px(13.)).text_color(theme.c("red-ink")).child(m)))
    }
}
