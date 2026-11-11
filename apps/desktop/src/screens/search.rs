//! Search (`/search?q=`; `later.tsx` `SearchScreen`): one field over the
//! agents, reports and messages the day's view holds. The address follows
//! what is typed, 250 ms after the last key.

use crate::chat::{self, MessageOpts};
use crate::derive::agent_name;
use crate::derive::search::search;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::nav::{self, Nav, Route};
use crate::screens::inbox::{self as parts, ListKey};
use crate::screens::later::context_of;
use crate::theme::Theme;
use crate::ui::{self, avatar_size, row, space};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext, Context, Entity, FocusHandle, Focusable as _, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
};
use gpui_kit::component::input::{InputEvent, InputState};
use std::time::Duration;

/// How long after the last key the address follows what is typed.
const DEBOUNCE: Duration = Duration::from_millis(250);
/// The most messages a search lists.
const MAX_MESSAGES: usize = 100;

pub struct SearchScreen {
    input: Entity<InputState>,
    debounce: Option<Task<()>>,
    /// The results' focus: the list keys act while it holds it.
    focus: FocusHandle,
    /// The keyboard's focus row, an index into `targets`.
    cursor: Option<usize>,
    /// Where `Enter` takes each result the keyboard can reach, and which
    /// child of the results it is.
    targets: Vec<(usize, Route)>,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

impl SearchScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let q = match Nav::route(cx) {
            Route::Search { q } => q.clone(),
            _ => String::new(),
        };
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(t(cx, "later.searchPlaceholder"));
            s.set_value(q.clone(), window, cx);
            s
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        let focus = cx.focus_handle();
        cx.set_global(parts::ListFocus(focus.clone()));
        let subs = vec![
            cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
            cx.observe_global::<Theme>(|_, cx| cx.notify()),
            cx.subscribe(&input, |this, input, ev: &InputEvent, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                this.cursor = None;
                cx.notify();
                let q = input.read(cx).value().to_string();
                this.debounce = Some(cx.spawn(async move |_, cx| {
                    cx.background_executor().timer(DEBOUNCE).await;
                    cx.update(|cx| {
                        if matches!(Nav::route(cx), Route::Search { .. }) {
                            nav::replace(Route::Search { q }, cx);
                        }
                    });
                }));
            }),
        ];
        SearchScreen {
            input,
            debounce: None,
            focus,
            cursor: None,
            targets: Vec::new(),
            scroll: ScrollHandle::new(),
            _subs: subs,
        }
    }

    fn move_cursor(&mut self, to: Option<usize>) {
        self.cursor = to;
        if let Some((child, _)) = to.and_then(|ix| self.targets.get(ix)) {
            self.scroll.scroll_to_item(*child);
        }
    }

    /// The keys of the results (K4): from the field, the down arrow steps
    /// into them; in them, Esc goes back to the field.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = parts::list_key(ev) else {
            return;
        };
        if self.input.focus_handle(cx).is_focused(window) {
            if ev.keystroke.key == "down" && !self.targets.is_empty() {
                window.focus(&self.focus, cx);
                self.move_cursor(Some(0));
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        if !self.focus.is_focused(window) {
            return;
        }
        match key {
            ListKey::Next | ListKey::Prev => {
                self.move_cursor(parts::step(self.cursor, key, self.targets.len()));
            }
            ListKey::Open => {
                let target = self.cursor.and_then(|ix| self.targets.get(ix).cloned());
                if let Some((_, route)) = target {
                    nav::navigate(route, cx);
                }
            }
            ListKey::Back => {
                self.cursor = None;
                self.input.update(cx, |s, cx| s.focus(window, cx));
            }
            ListKey::Read => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for SearchScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let q = self.input.read(cx).value().to_string();
        let desk = Desk::get(cx).read(cx);
        let Some(view) = desk.view.clone() else {
            return div()
                .size_full()
                .bg(theme.c("surface"))
                .child(parts::loading(cx))
                .into_any_element();
        };
        let r = search(&view, &q);
        let label = |s: SharedString| {
            ui::section_label(s, &theme)
                .mt(px(space::XL))
                .mb(px(space::XS))
                .px(px(space::GUTTER))
                .into_any_element()
        };
        let opts = MessageOpts {
            in_thread: true,
            fold: true,
            continued: false,
            ..Default::default()
        };
        // The results as one run of children; `targets` remembers which of
        // them the keyboard walks and where each leads.
        let mut children: Vec<AnyElement> = Vec::new();
        let mut targets: Vec<(usize, Route)> = Vec::new();
        let cursor = self.cursor;
        let ring = ui::focus_ring(&theme, true);
        if q.trim().is_empty() {
            children.push(
                parts::empty(None, Some(t(cx, "later.searchHint")), &theme).into_any_element(),
            );
        } else if r.total() == 0 {
            children.push(
                parts::empty(
                    None,
                    Some(tv(cx, "later.noMatches", &[("query", q.trim())])),
                    &theme,
                )
                .into_any_element(),
            );
        }
        if !r.agents.is_empty() {
            children.push(label(t(cx, "later.agents")));
            for a in &r.agents {
                let route = Route::direct(Some(a.key.clone()));
                let on = cursor == Some(targets.len());
                targets.push((children.len(), route.clone()));
                children.push(
                    div()
                        .id(SharedString::from(format!("hb-found-{}", a.key)))
                        .flex()
                        .items_center()
                        .gap(px(space::LG))
                        .px(px(space::GUTTER))
                        .py(px(space::MD))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme.c("hover")))
                        .when(on, |d| d.shadow(vec![ring.clone()]))
                        .child(ui::avatar::agent_avatar(a, avatar_size::ROW, true, cx))
                        .child(
                            parts::item_body()
                                .child(parts::item_title(agent_name(a, cx), true, &theme))
                                .child(parts::item_preview(
                                    a.description
                                        .clone()
                                        .or_else(|| a.brief.clone())
                                        .unwrap_or_default(),
                                    1,
                                    &theme,
                                )),
                        )
                        .on_click(move |_, _, cx| nav::navigate(route.clone(), cx))
                        .into_any_element(),
                );
            }
        }
        if !r.reports.is_empty() {
            children.push(label(t(cx, "later.reports")));
            for (path, _) in &r.reports {
                children.push(
                    div()
                        .px(px(space::GUTTER))
                        .py(px(space::XS))
                        .child(chat::file_card(path, cx))
                        .into_any_element(),
                );
            }
        }
        if !r.messages.is_empty() {
            children.push(label(t(cx, "later.messages")));
            for m in r.messages.iter().take(MAX_MESSAGES) {
                let on = cursor == Some(targets.len());
                targets.push((children.len(), context_of(m)));
                children.push(
                    div()
                        .when(on, |d| d.shadow(vec![ring.clone()]))
                        .child(chat::message_item(m, &view, &opts, cx))
                        .into_any_element(),
                );
            }
        }
        self.targets = targets;
        let typing = self.input.focus_handle(cx).is_focused(window);
        div()
            .id("hb-search-screen")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme.c("surface"))
            .child(
                div()
                    .h(px(row::HEADER))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(space::GUTTER))
                    .border_b_1()
                    .border_color(theme.c("line"))
                    .child(
                        div()
                            .flex_1()
                            .child(parts::search_box(&self.input, typing, &theme)),
                    ),
            )
            .child(
                div()
                    .id("hb-results")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pb(px(space::LG))
                    .children(children),
            )
            .into_any_element()
    }
}
