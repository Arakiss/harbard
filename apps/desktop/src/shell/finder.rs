//! ⌘K: one finder for agents, channels and messages (`finder.tsx`, `search`
//! in `lib/workspace.ts`), over what the read holds. Arrows move (clamped),
//! Enter opens, Escape closes. Nothing leaves the app.

use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::WorkspaceView;
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{self, icon, kit_theme};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, FontWeight, Global, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    Agent,
    Channel,
    Message,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub kind: HitKind,
    pub id: String,
    pub title: String,
    pub sub: String,
    pub to: Route,
}

fn has(s: Option<&str>, q: &str) -> bool {
    s.is_some_and(|s| s.to_lowercase().contains(q))
}

/// The hits for a query: agents (6), then channels (6), then messages (8).
pub fn finder_hits(view: &WorkspaceView, query: &str, cx: &App) -> Vec<Hit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let agents = view
        .agents
        .iter()
        .filter(|a| {
            !a.temporary
                && (has(Some(&a.name), &q)
                    || has(a.handle.as_deref(), &q)
                    || has(a.team.as_deref(), &q)
                    || has(a.description.as_deref(), &q)
                    || has(a.brief.as_deref(), &q))
        })
        .take(6)
        .map(|a| Hit {
            kind: HitKind::Agent,
            id: format!("agent:{}", a.key),
            title: a.name.clone(),
            sub: a.handle.as_ref().map_or_else(|| a.label.clone(), |h| format!("@{h}")),
            to: Route::direct(Some(a.key.clone())),
        });
    let channels = view
        .channels
        .iter()
        .filter(|c| has(Some(&c.id), &q) || has(Some(&c.repo), &q))
        .take(6)
        .map(|c| Hit {
            kind: HitKind::Channel,
            id: format!("channel:{}", c.id),
            title: format!("#{}", c.id),
            sub: if c.quiet {
                t(cx, "shell.inMoreChannels").to_string()
            } else if c.members.len() == 1 {
                t(cx, "shell.oneAgent").to_string()
            } else {
                tv(cx, "shell.agentsCount", &[("count", &c.members.len().to_string())]).to_string()
            },
            to: Route::channel(c.id.clone()),
        });
    let mut messages: Vec<_> = view
        .messages
        .iter()
        .filter(|m| m.structured.is_none() && (has(Some(&m.text), &q) || has(Some(&m.author), &q)))
        .collect();
    messages.sort_by_key(|m| std::cmp::Reverse(m.ts_ms));
    let messages = messages.into_iter().take(8).map(|m| {
        let agent = view.agent(&m.agent);
        let place = m.channels.first();
        let to = match (agent, place) {
            (Some(a), None) => Route::direct(Some(a.key.clone())),
            (_, Some(c)) => Route::Home {
                channel: Some(c.clone()),
                thread: Some(m.reply_to.clone().unwrap_or_else(|| m.id.clone())),
                item: None,
            },
            _ => Route::home(),
        };
        let who = agent.map_or(m.author.as_str(), |a| a.name.as_str());
        let mut sub = format!("{who} · {}", ui::format::hhmm(m.ts_ms));
        if let Some(c) = place {
            sub.push_str(&format!(" · #{c}"));
        }
        Hit {
            kind: HitKind::Message,
            id: format!("message:{}", m.id),
            title: m.text.clone(),
            sub,
            to,
        }
    });
    agents.chain(channels).chain(messages).collect()
}

/// Whether the Finder is open in the main window.
#[derive(Default)]
struct FinderOpen(bool);

impl Global for FinderOpen {}

/// ⌘K: open the Finder, or close it when it is open.
pub fn toggle(window: &mut Window, cx: &mut App) {
    if cx.try_global::<FinderOpen>().is_some_and(|o| o.0) {
        window.close_dialog(cx);
        cx.set_global(FinderOpen(false));
        return;
    }
    if Desk::get(cx).read(cx).view.is_none() {
        return;
    }
    cx.set_global(FinderOpen(true));
    let finder = cx.new(|cx| Finder::new(window, cx));
    let top = window.viewport_size().height * 0.12;
    let focus = Some(finder.read(cx).input.clone());
    let first = std::cell::Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false)
            && let Some(input) = &focus
        {
            input.update(cx, |i, cx| i.focus(window, cx));
        }
        kit_theme::hb_dialog(dialog, cx)
            .w(px(640.))
            .margin_top(top)
            .close_button(false)
            .p_0()
            .gap_0()
            .min_h(px(0.))
            .line_height(gpui::relative(1.46))
            .on_close(|_, _, cx| cx.set_global(FinderOpen(false)))
            // The dialog takes Enter as "confirm": open the selected hit.
            .on_ok({
                let finder = finder.clone();
                move |_, window, cx| {
                    let to = finder.update(cx, |f, cx| f.hits(cx).get(f.at).map(|h| h.to.clone()));
                    if let Some(to) = to {
                        cx.set_global(FinderOpen(false));
                        nav::navigate(to, cx);
                        let _ = window;
                        return true;
                    }
                    false
                }
            })
            .child(finder.clone())
    });
}

pub struct Finder {
    input: Entity<InputState>,
    query: String,
    at: usize,
}

impl Finder {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = t(cx, "shell.searchPlaceholder");
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        cx.subscribe_in(&input, window, |f, input, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                f.query = input.read(cx).value().to_string();
                f.at = 0;
                cx.notify();
            }
        })
        .detach();
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        Finder {
            input,
            query: String::new(),
            at: 0,
        }
    }

    fn hits(&self, cx: &App) -> Vec<Hit> {
        match &Desk::get(cx).read(cx).view {
            Some(v) => finder_hits(v, &self.query, cx),
            None => Vec::new(),
        }
    }

    fn go(to: Route, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
        cx.set_global(FinderOpen(false));
        nav::navigate(to, cx);
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let hits = self.hits(cx);
        match k.key.as_str() {
            "down" => self.at = (self.at + 1).min(hits.len().saturating_sub(1)),
            "up" => self.at = self.at.saturating_sub(1),
            "enter" => {
                if let Some(h) = hits.get(self.at) {
                    Self::go(h.to.clone(), window, cx);
                }
            }
            "escape" => {
                window.close_dialog(cx);
                cx.set_global(FinderOpen(false));
            }
            "k" if k.modifiers.platform || k.modifiers.control => {
                window.close_dialog(cx);
                cx.set_global(FinderOpen(false));
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for Finder {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let hits = self.hits(cx);
        let query = self.query.trim().to_string();
        let max_list = window.viewport_size().height * 0.7 - px(60.);
        let sel_bg = theme.c("menu-hover");
        let sel_ink = theme.c("menu-hover-ink");
        div()
            .id("hb-finder")
            .capture_key_down(cx.listener(Self::on_key))
            .flex()
            .flex_col()
            .font_family(ui::SANS)
            .text_size(px(ui::size::BODY))
            .text_color(theme.c("ink"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(16.))
                    .py(px(12.))
                    .border_b_1()
                    .border_color(theme.c("line"))
                    .child(icon("search", 20., theme.c("ink")))
                    .child(div().flex_1().text_size(px(17.)).child(Input::new(&self.input).appearance(false).text_size(px(17.))))
                    .child(
                        div()
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(theme.c("line-strong"))
                            .bg(theme.c("surface-2"))
                            .text_size(px(12.))
                            .text_color(theme.c("ink-3"))
                            .child(t(cx, "shell.escapeKey")),
                    ),
            )
            .child(
                div()
                    .id("hb-finder-list")
                    .flex()
                    .flex_col()
                    .py(px(6.))
                    .max_h(max_list)
                    .overflow_y_scroll()
                    .when(!query.is_empty() && hits.is_empty(), |d| {
                        d.child(
                            div()
                                .px(px(16.))
                                .py(px(12.))
                                .text_color(theme.c("ink-3"))
                                .child(tv(cx, "shell.nothingMatches", &[("query", &query)])),
                        )
                    })
                    .children(hits.into_iter().enumerate().map(|(i, h)| {
                        let selected = i == self.at;
                        let ink = if selected { sel_ink } else { theme.c("ink") };
                        let name = match h.kind {
                            HitKind::Agent => "user-round",
                            HitKind::Channel => "hash",
                            HitKind::Message => "message-square",
                        };
                        let to = h.to.clone();
                        let title: SharedString = h.title.lines().next().unwrap_or_default().to_string().into();
                        div()
                            .id(SharedString::from(h.id.clone()))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .px(px(16.))
                            .py(px(8.))
                            .cursor_pointer()
                            .when(selected, |d| d.bg(sel_bg).text_color(sel_ink))
                            .on_hover(cx.listener(move |f, on: &bool, _, cx| {
                                if *on && f.at != i {
                                    f.at = i;
                                    cx.notify();
                                }
                            }))
                            .on_click(move |_, window, cx| Self::go(to.clone(), window, cx))
                            .child(icon(name, 16., ink))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .child(div().font_weight(FontWeight::BOLD).truncate().child(title))
                                    .child(div().text_size(px(13.)).opacity(0.75).truncate().child(h.sub)),
                            )
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_is_case_blind() {
        assert!(has(Some("Harbard"), "harb"));
        assert!(!has(None, "x"));
        assert!(!has(Some("abc"), "abd"));
    }
}
