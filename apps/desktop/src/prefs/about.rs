//! "About harbard" (`AboutHarbard` in `workspace-menu.tsx`): whether a newer
//! version exists, and this build's version, commit and protocol
//! (`GET /meta`). A small dialog, 480 wide, 12% down the window. The one
//! About: the workspace menu and the app menu both open this.

use super::shell_update::{APP_COMMIT, UpdateStore};
use super::stores::check_updates;
use super::update_dialog::pane_head;
use super::widgets::{self, Kind, Size};
use crate::api;
use crate::i18n::t;
use crate::theme::Theme;
use crate::ui;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, FontWeight, IntoElement, ParentElement, Render, SharedString, Styled, Window,
    div, px,
};

/// `GET /api/v1/meta`: the hub's name, version and protocol.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
struct Meta {
    version: String,
    protocol: Option<String>,
}

pub struct About {
    meta: Option<Meta>,
    /// `/meta` answered or failed: before that the rows say nothing yet.
    asked: bool,
}

impl About {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.observe(&UpdateStore::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.spawn(async move |this, cx| {
            let meta = cx.background_spawn(async { api::get_as::<Meta>("/meta") }).await;
            let _ = this.update(cx, |a, cx| {
                a.meta = meta.ok();
                a.asked = true;
                cx.notify();
            });
        })
        .detach();
        About {
            meta: None,
            asked: false,
        }
    }
}

pub fn open(window: &mut Window, cx: &mut App) {
    let view = cx.new(About::new);
    widgets::open_modal(view.into(), 480., None, widgets::Place::Top(0.12), None, window, cx);
}

/// One `dt`/`dd` row of `.hb-shortcuts`.
pub fn row(label: SharedString, value: AnyElement, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(6.))
        .border_b_1()
        .border_color(theme.c("line"))
        .child(div().w(px(150.)).flex_none().font_weight(FontWeight::BOLD).child(label))
        .child(div().flex_1().min_w_0().child(value))
        .into_any_element()
}

impl Render for About {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let store = UpdateStore::get(cx).read(cx);
        let known = store.view.is_some() && !store.unsupported;
        let available = store.available();
        let checking = store.checking;
        let mono = |s: String| {
            div()
                .font_family(ui::MONO)
                .text_size(px(13.))
                .child(s)
                .into_any_element()
        };
        let mut rows = Vec::new();
        if known {
            let updates = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.))
                .map(|d| {
                    if available {
                        d.child(widgets::chip(t(cx, "shell.newVersionAvailable"), "amber", &theme))
                            .child(widgets::button(
                                "about-whats-new",
                                t(cx, "shell.seeWhatsNew"),
                                Kind::Plain,
                                Size::Tiny,
                                None,
                                false,
                                &theme,
                                |_, w, cx| {
                                    widgets::close_top(w, cx);
                                    super::open_update_dialog(w, cx);
                                },
                            ))
                    } else {
                        d.child(widgets::chip(t(cx, "shell.upToDate"), "green", &theme))
                            .child(widgets::button(
                                "about-check",
                                if checking {
                                    t(cx, "shell.checking")
                                } else {
                                    t(cx, "shell.checkNow")
                                },
                                Kind::Plain,
                                Size::Tiny,
                                None,
                                checking,
                                &theme,
                                |_, _, cx| check_updates(cx),
                            ))
                    }
                })
                .into_any_element();
            rows.push(row(t(cx, "shell.updates"), updates, &theme));
        }
        let version = match (&self.meta, self.asked) {
            (Some(m), _) => m.version.clone(),
            (None, true) => t(cx, "shell.serveNotResponding").to_string(),
            (None, false) => String::new(),
        };
        rows.push(row(t(cx, "shell.version"), mono(version), &theme));
        let commit = if APP_COMMIT.is_empty() {
            t(cx, "shell.noData").to_string()
        } else {
            APP_COMMIT.chars().take(7).collect()
        };
        rows.push(row(t(cx, "shell.consoleCommit"), mono(commit), &theme));
        let protocol = self.meta.as_ref().and_then(|m| m.protocol.clone()).unwrap_or_else(|| {
            if self.asked {
                t(cx, "shell.noData").to_string()
            } else {
                String::new()
            }
        });
        rows.push(row(t(cx, "shell.protocol"), mono(protocol), &theme));
        widgets::base(&theme)
            .flex()
            .flex_col()
            .child(pane_head(t(cx, "shell.aboutHarbard"), &theme))
            .child(div().px(px(18.)).pt(px(14.)).pb(px(18.)).children(rows))
    }
}
