//! Later (`/later`; `later.tsx` `LaterScreen`): what the operator saved to
//! read later, with the way back to it while it is still in the day's view.

use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Message, Saved, WorkspaceView};
use crate::nav::{self, Route};
use crate::screens::inbox::{self as parts, ListKey, RowState};
use crate::theme::Theme;
use crate::ui::agent_card::post;
use crate::ui::format::{at_date, hhmm};
use crate::ui::{self, avatar_size, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, px,
};
use serde_json::json;

pub struct LaterScreen {
    /// The list's focus: its keys act while it holds it.
    focus: FocusHandle,
    /// The keyboard's focus row.
    cursor: Option<usize>,
    /// Where `Enter` takes each row, when it is still in the day's view.
    targets: Vec<Option<Route>>,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

impl LaterScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.set_global(parts::ListFocus(focus.clone()));
        LaterScreen {
            focus,
            cursor: None,
            targets: Vec::new(),
            scroll: ScrollHandle::new(),
            _subs: vec![
                cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
                cx.observe_global::<Theme>(|_, cx| cx.notify()),
            ],
        }
    }

    /// The keys of the list (K4). Nothing here is read or unread, so `U`
    /// has nothing to mark.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let Some(key) = parts::list_key(ev) else {
            return;
        };
        match key {
            ListKey::Next | ListKey::Prev => {
                self.cursor = parts::step(self.cursor, key, self.targets.len());
                if let Some(ix) = self.cursor {
                    self.scroll.scroll_to_item(ix);
                }
            }
            ListKey::Open => {
                let target = self.cursor.and_then(|ix| self.targets.get(ix).cloned());
                if let Some(Some(route)) = target {
                    nav::navigate(route, cx);
                }
            }
            ListKey::Back => self.cursor = None,
            ListKey::Read => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

/// Where a saved item is now: its thread, or the agent's conversation.
pub fn context_of(m: &Message) -> Route {
    match m.channels.first() {
        Some(channel) => Route::Home {
            channel: Some(channel.clone()),
            thread: Some(m.reply_to.clone().unwrap_or_else(|| m.id.clone())),
            item: None,
        },
        None => Route::direct(Some(m.agent.clone())),
    }
}

/// One saved item: who and where as the title line, its words under it. The
/// row opens it in context; its one action removes it.
fn saved_item(
    view: &WorkspaceView,
    s: &Saved,
    cursor: bool,
    now: u64,
    theme: &Theme,
    cx: &mut Context<LaterScreen>,
) -> (Option<Route>, AnyElement) {
    let live = view.message(&s.id);
    let target = live.map(context_of);
    let live_agent = live.and_then(|m| view.agent(&m.agent));
    let avatar = match (&s.kind[..], &s.agent, live_agent) {
        ("message", Some(_), Some(a)) => ui::avatar::agent_avatar(a, avatar_size::ROW, true, cx),
        ("message", Some(name), None) => ui::avatar::name_avatar(name, avatar_size::ROW, cx),
        // A report is a file: one icon, not a face.
        _ => parts::icon_slot("file-text", theme.c("ink-3")).into_any_element(),
    };
    let name: SharedString = if s.kind == "report" {
        t(cx, "later.report")
    } else {
        s.agent
            .clone()
            .map(Into::into)
            .unwrap_or_else(|| t(cx, "later.message"))
    };
    // "#channel · 22:51 · saved yesterday", said once beside the name.
    let saved = tv(
        cx,
        "later.savedWhen",
        &[("when", &at_date(s.saved_ms, now, cx))],
    );
    let aside: Vec<String> = [
        s.channel.as_ref().map(|c| format!("#{c}")),
        s.ts_ms.map(hhmm),
        Some(saved.trim_start_matches(['·', ' ']).to_string()),
    ]
    .into_iter()
    .flatten()
    .collect();
    let gone = target.is_none() && s.kind == "message";
    let id = s.id.clone();
    let state = RowState {
        active: false,
        cursor,
        dot: false,
    };
    let row = parts::item_row(
        SharedString::from(format!("hb-later-{}", s.id)),
        state,
        theme,
    )
    .when(target.is_none(), |d| d.cursor_default())
    // A report's card is taller than two lines of text.
    .when(s.path.is_some(), |d| d.h_auto().min_h(px(parts::ROW_H)))
    .child(avatar)
    .child(
        parts::item_body()
            .child(
                parts::item_line()
                    .child(parts::item_title(name, true, theme).flex_none())
                    .child(parts::item_aside(aside.join(" · "), theme).flex_1())
                    .child(
                        parts::icon_btn(
                            SharedString::from(format!("hb-later-remove-{}", s.id)),
                            "bookmark-x",
                            false,
                            theme,
                        )
                        .tooltip(parts::tip(t(cx, "later.remove"), &[]))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            post("unsave", json!({ "id": id }), cx).detach();
                        }),
                    ),
            )
            .child(parts::item_preview(
                s.text.clone(),
                if gone || s.path.is_some() { 1 } else { 2 },
                theme,
            ))
            .children(
                s.path
                    .as_ref()
                    .map(|p| div().mt(px(space::XS)).child(crate::chat::file_card(p, cx))),
            )
            .when(gone, |d| {
                d.child(
                    ui::text(
                        t(cx, "later.outOfWindow"),
                        size::SMALL,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .truncate(),
                )
            }),
    )
    .when_some(target.clone(), |d, route| {
        d.tooltip(parts::tip(t(cx, "later.viewInContext"), &["\u{21b5}"]))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.cursor = None;
                nav::navigate(route.clone(), cx)
            }))
    });
    (target, row.into_any_element())
}

impl Render for LaterScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let desk = Desk::get(cx).read(cx);
        let now = desk.now_ms();
        let Some(view) = desk.view.clone() else {
            return div()
                .size_full()
                .bg(theme.c("surface"))
                .child(parts::loading(cx))
                .into_any_element();
        };
        self.cursor = self.cursor.filter(|ix| *ix < view.saved.len());
        let cursor = self.cursor;
        let (targets, rows): (Vec<Option<Route>>, Vec<AnyElement>) = view
            .saved
            .iter()
            .enumerate()
            .map(|(ix, s)| saved_item(&view, s, cursor == Some(ix), now, &theme, cx))
            .unzip();
        self.targets = targets;
        div()
            .id("hb-later-screen")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(theme.c("surface"))
            .child(
                ui::pane_header(t(cx, "later.title"), &theme).child(
                    ui::text(
                        t(cx, "later.topic"),
                        size::SMALL,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .min_w_0()
                    .truncate(),
                ),
            )
            .child(
                div()
                    .id("hb-later")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows)
                    .when(view.saved.is_empty(), |d| {
                        d.child(parts::empty(
                            Some(t(cx, "later.emptyTitle")),
                            Some(t(cx, "later.emptyHint")),
                            &theme,
                        ))
                    })
                    .pb(px(space::LG)),
            )
            .into_any_element()
    }
}
