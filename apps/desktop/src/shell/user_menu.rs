//! The menu behind the operator's avatar at the foot of the rail
//! (`user-menu.tsx`, `operator-avatar.tsx`): who and in which state, the
//! status message, the six coordination states, Do not disturb, Profile and
//! context, Preferences, and the fixed password notice. Writes go through
//! [`ProfileStore::update`] (`POST /profile`).

use super::menu::menu_step;
use super::profile::{
    ProfileStore, STATES, custom_until, dnd_until, done_until, now_ms, quiet_label, refusal_text,
    state_info, status_expiry, tone_token,
};
use crate::i18n::t;
use crate::theme::Theme;
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, AppContext, Context, Entity, FocusHandle, FontWeight, Hsla,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point, px,
};
use gpui_kit::component::input::{Input, InputState};
use serde_json::json;

/// The face the operator chose with their state's mark (`OperatorAvatar`).
/// `ring` is what surrounds it (the frame on the rail, the raised menu in it).
pub fn operator_face(
    initials: &str,
    size: f32,
    presence: Option<&str>,
    ring: Hsla,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.global::<Theme>();
    let face = ui::avatar::operator_avatar(initials, size, cx);
    let badge = presence.map(|p| -> AnyElement {
        if p == "available" {
            return div()
                .absolute()
                .right(px(-3.))
                .bottom(px(-3.))
                .size(px(12.))
                .rounded_full()
                .bg(theme.c("green"))
                .border_2()
                .border_color(ring)
                .into_any_element();
        }
        let (name, tone) = if p == "do-not-disturb" {
            ("bell-off", "grey")
        } else {
            let s = state_info(p);
            (s.icon, s.tone)
        };
        div()
            .absolute()
            .right(px(-4.))
            .bottom(px(-4.))
            .size(px(17.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.c("surface-raised"))
            .border_1()
            .border_color(theme.c("line-strong"))
            .shadow(vec![gpui::BoxShadow {
                color: ring,
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
                inset: false,
            }])
            .child(icon(name, 10., theme.c(tone_token(tone))))
            .into_any_element()
    });
    div().relative().flex_none().child(face).children(badge)
}

/// What a row of the menu does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Photo,
    Message,
    Clear,
    State(usize),
    Dnd,
    Duration(usize),
    Apply,
    Profile,
    Preferences,
}

const DURATIONS: [(&str, &str); 3] = [
    ("30m", "profile.duration30Minutes"),
    ("1h", "profile.duration1Hour"),
    ("tomorrow", "profile.dndUntilTomorrow"),
];

pub struct UserMenu {
    pub open: bool,
    durations: bool,
    error: Option<SharedString>,
    /// The keyboard's row, once a key was used.
    kb: Option<usize>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    custom: Entity<InputState>,
}

impl UserMenu {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ProfileStore::get(cx), |_, _, cx| cx.notify())
            .detach();
        // A moment an hour ahead, in the form the field reads.
        let hint = {
            let at = now_ms() + 3_600_000;
            let (y, m, d, _) = crate::ui::format::local_date(at);
            format!("{y}-{m:02}-{d:02} {}", crate::ui::format::hhmm(at))
        };
        let custom = cx.new(|cx| InputState::new(window, cx).placeholder(hint));
        UserMenu {
            open: false,
            durations: false,
            error: None,
            kb: None,
            focus: cx.focus_handle(),
            restore: None,
            custom,
        }
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close(window, cx);
        } else {
            self.open = true;
            self.kb = None;
            self.restore = window.focused(cx);
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.durations = false;
        self.error = None;
        if let Some(f) = self.restore.take() {
            f.focus(window, cx);
        }
        cx.notify();
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let store = ProfileStore::get(cx).read(cx);
        let now = now_ms();
        let mut rows = vec![Row::Photo, Row::Message];
        if store.status(now).is_some() {
            rows.push(Row::Clear);
        }
        rows.extend((0..STATES.len()).map(Row::State));
        rows.push(Row::Dnd);
        if self.durations && !store.quiet(now) {
            rows.extend((0..DURATIONS.len()).map(Row::Duration));
            rows.push(Row::Apply);
        }
        rows.push(Row::Profile);
        rows.push(Row::Preferences);
        rows
    }

    /// Write a patch; close on success, say why on refusal.
    fn write(&mut self, patch: serde_json::Value, cx: &mut Context<Self>) {
        self.error = None;
        let task = ProfileStore::get(cx).update(cx, |p, cx| p.update(patch, cx));
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                let _ = this.update(cx, |m, cx| {
                    m.error = Some(refusal_text(&e, cx));
                    m.open = true;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn run(&mut self, row: Row, window: &mut Window, cx: &mut Context<Self>) {
        let store = ProfileStore::get(cx).read(cx);
        let now = now_ms();
        let known = store.view.is_some();
        match row {
            Row::Photo | Row::Profile => {
                self.close(window, cx);
                crate::prefs::open_profile(window, cx);
            }
            Row::Message => {
                self.close(window, cx);
                crate::prefs::open_status(window, cx);
            }
            Row::Clear => self.write(json!({ "status": null }), cx),
            Row::State(i) if known => {
                let s = STATES[i];
                if s.id == store.state_id(now) {
                    self.close(window, cx);
                    return;
                }
                let until = (s.id == "done").then(|| {
                    done_until(
                        store.view.as_ref().and_then(|v| v.profile.hours.as_ref()),
                        now,
                    )
                });
                self.write(json!({ "state": { "id": s.id, "until_ms": until } }), cx);
                self.close(window, cx);
            }
            Row::Dnd if known => {
                if store.quiet(now) {
                    self.write(json!({ "dnd": null }), cx);
                } else {
                    self.durations = !self.durations;
                    cx.notify();
                }
            }
            Row::Duration(i) => {
                let until = dnd_until(DURATIONS[i].0, now);
                self.write(json!({ "dnd": { "until_ms": until } }), cx);
                self.close(window, cx);
            }
            Row::Apply => {
                let value = self.custom.read(cx).value().to_string();
                match custom_until(&value, now) {
                    Some(at) => {
                        self.write(json!({ "dnd": { "until_ms": at } }), cx);
                        self.close(window, cx);
                    }
                    None => {
                        self.error = Some(t(cx, "profile.pickMoment"));
                        cx.notify();
                    }
                }
            }
            Row::Preferences => {
                self.close(window, cx);
                crate::prefs::open_preferences(None, window, cx);
            }
            _ => {}
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows(cx);
        match ev.keystroke.key.as_str() {
            k @ ("up" | "down" | "home" | "end") => {
                self.kb = menu_step(self.kb, rows.len(), k);
            }
            "escape" => self.close(window, cx),
            "enter" | "space" => {
                if let Some(row) = self.kb.and_then(|i| rows.get(i).copied()) {
                    self.run(row, window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for UserMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let store = ProfileStore::get(cx).read(cx);
        let desk = crate::desk::Desk::get(cx).read(cx);
        let operator = desk
            .view
            .as_ref()
            .map(|v| v.operator.clone())
            .unwrap_or_default();
        let initials = desk
            .view
            .as_ref()
            .map(|v| v.operator_initials.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "OP".into());
        let name: SharedString = store
            .view
            .as_ref()
            .and_then(|v| v.profile.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or(operator)
            .into();
        let now = now_ms();
        let quiet = store.quiet(now);
        let state = state_info(store.state_id(now));
        let presence = if quiet { "do-not-disturb" } else { state.id };
        let me = cx.entity();

        let button = div()
            .id("hb-me-button")
            .occlude()
            .mt(px(10.))
            // The console's avatar sat on a text line: its descender left 7px under it.
            .mb(px(7.))
            .rounded(px(10.))
            .cursor_pointer()
            .on_click(cx.listener(|m, _, window, cx| m.toggle(window, cx)))
            .child(operator_face(
                &initials,
                42.,
                Some(presence),
                theme.c("frame"),
                cx,
            ));

        let menu = self.open.then(|| {
            let rows = self.rows(cx);
            let lit = |row: Row| self.kb.and_then(|i| rows.get(i)) == Some(&row);
            let item = |id: &'static str, row: Row, lit_now: bool| {
                let hover_bg = theme.c("menu-hover");
                let hover_ink = theme.c("menu-hover-ink");
                let me = me.clone();
                div()
                    .id(id)
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(16.))
                    .py(px(6.))
                    .cursor_pointer()
                    .group(id)
                    .when(lit_now, |d| d.bg(hover_bg).text_color(hover_ink))
                    .hover(move |s| s.bg(hover_bg).text_color(hover_ink))
                    .on_click(move |_, window, cx| me.update(cx, |m, cx| m.run(row, window, cx)))
            };
            let ink_icon = |name: &'static str, size: f32, group: &'static str, lit_now: bool| {
                let hover_ink = theme.c("menu-hover-ink");
                icon(name, size, if lit_now { hover_ink } else { theme.c("ink") })
                    .group_hover(group, move |s| s.text_color(hover_ink))
            };
            let chevron = |group: &'static str, open: bool, lit_now: bool| {
                let hover_ink = theme.c("menu-hover-ink");
                div().ml_auto().child(
                    icon(
                        "chevron-right",
                        16.,
                        if lit_now { hover_ink } else { theme.c("ink-3") },
                    )
                    .group_hover(group, move |s| s.text_color(hover_ink))
                    .when(open, |s| {
                        s.with_transformation(gpui::Transformation::rotate(gpui::radians(
                            std::f32::consts::FRAC_PI_2,
                        )))
                    }),
                )
            };
            let sep = || div().h(px(1.)).my(px(6.)).bg(theme.c("line"));

            // 1. Head: photo, name, state line.
            let until = (state.id != "available")
                .then(|| store.view.as_ref().and_then(|v| v.state_until_ms))
                .flatten();
            let headline: SharedString = match until {
                Some(u) => format!("{} · {}", state.label(cx), quiet_label(u, now, cx)).into(),
                None => state.label(cx),
            };
            let scrim = theme.c("scrim");
            let raised = theme.c("surface-raised");
            let photo = div()
                .id("hb-me-photo")
                .relative()
                .rounded(px(12.))
                .cursor_pointer()
                .group("hb-me-photo")
                .on_click({
                    let me = me.clone();
                    move |_, window, cx| me.update(cx, |m, cx| m.run(Row::Photo, window, cx))
                })
                .child(operator_face(&initials, 56., Some(presence), raised, cx))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(12.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(scrim)
                        .opacity(if lit(Row::Photo) { 1. } else { 0. })
                        .group_hover("hb-me-photo", |s| s.opacity(1.))
                        .child(icon("camera", 20., raised)),
                );
            let head = div()
                .flex()
                .items_center()
                .gap(px(14.))
                .px(px(18.))
                .pt(px(18.))
                .pb(px(12.))
                .child(photo)
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BLACK)
                                .line_height(gpui::relative(1.2))
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .mt(px(4.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(14.))
                                .text_color(theme.c("ink-2"))
                                .child(icon(state.icon, 16., theme.c(tone_token(state.tone))))
                                .child(headline),
                        ),
                );

            // 2. The status message.
            let status = store.status(now).cloned();
            let message_hover = theme.c("line-hover");
            let message = div()
                .id("hb-me-message")
                .mx(px(16.))
                .mb(px(6.))
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(10.))
                .py(px(6.))
                .border_1()
                .border_color(if lit(Row::Message) {
                    message_hover
                } else {
                    theme.c("line")
                })
                .rounded(px(8.))
                .text_size(px(14.))
                .text_color(theme.c("ink-2"))
                .cursor_pointer()
                .hover(move |s| s.border_color(message_hover))
                .on_click({
                    let me = me.clone();
                    move |_, window, cx| me.update(cx, |m, cx| m.run(Row::Message, window, cx))
                })
                .map(|d| match &status {
                    Some(s) => {
                        let emoji: SharedString = if s.emoji.is_empty() {
                            "💬".into()
                        } else {
                            s.emoji.clone().into()
                        };
                        d.child(
                            div()
                                .text_size(px(18.))
                                .line_height(gpui::relative(1.))
                                .child(emoji),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .text_size(px(15.))
                                .text_color(theme.c("ink"))
                                .child(s.text.clone())
                                .children(status_expiry(s.clear_at_ms, now, cx).map(|e| {
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme.c("ink-3"))
                                        .child(e)
                                })),
                        )
                    }
                    None => d
                        .child(icon("message-square-text", 16., theme.c("ink-2")))
                        .child(
                            div()
                                .text_size(px(15.))
                                .text_color(theme.c("ink-3"))
                                .child(t(cx, "profile.addStatusMessage")),
                        ),
                });
            let clear = status.is_some().then(|| {
                let l = lit(Row::Clear);
                item("hb-me-clear", Row::Clear, l)
                    .child(ink_icon("x", 16., "hb-me-clear", l))
                    .child(t(cx, "profile.clearStatusMessage"))
            });

            // 3. Your status.
            let states = div().flex().flex_col().gap(px(2.)).px(px(8.)).children(
                STATES.iter().enumerate().map(|(i, s)| {
                    let on = s.id == state.id;
                    let tone = theme.c(tone_token(s.tone));
                    let hover_bg = theme.c("hover-2");
                    let me = me.clone();
                    let tile_bg = mix(tone, raised, 0.16);
                    div()
                        .id(SharedString::from(format!("hb-me-state-{}", s.id)))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(8.))
                        .text_color(theme.c("ink"))
                        .cursor_pointer()
                        .when(store.view.is_none(), |d| d.opacity(0.5))
                        .when(on, |d| d.bg(theme.c("accent-soft")))
                        .when(!on && lit(Row::State(i)), |d| d.bg(hover_bg))
                        .when(!on, |d| d.hover(move |st| st.bg(hover_bg)))
                        .on_click(move |_, window, cx| {
                            me.update(cx, |m, cx| m.run(Row::State(i), window, cx))
                        })
                        .child(
                            div()
                                .flex_none()
                                .size(px(34.))
                                .rounded(px(8.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(tile_bg)
                                .child(icon(s.icon, 18., tone)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .line_height(gpui::relative(1.25))
                                .child(
                                    div()
                                        .text_size(px(15.))
                                        .font_weight(FontWeight::BLACK)
                                        .child(s.label(cx)),
                                )
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .text_color(theme.c("ink-3"))
                                        .child(s.hint(cx)),
                                ),
                        )
                        .when(on, |d| {
                            d.child(div().ml_auto().child(icon("check", 20., theme.c("accent"))))
                        })
                }),
            );

            // 4. Do not disturb.
            let preferred = store
                .view
                .as_ref()
                .and_then(|v| v.profile.dnd_default.clone())
                .unwrap_or_else(|| "1h".into());
            let dnd_lit = lit(Row::Dnd);
            let dnd = if quiet {
                let until = store.view.as_ref().and_then(|v| v.quiet_until_ms);
                let when =
                    until.map_or_else(|| t(cx, "profile.dndActive"), |u| quiet_label(u, now, cx));
                item("hb-me-dnd", Row::Dnd, dnd_lit)
                    .child(ink_icon("bell-off", 18., "hb-me-dnd", dnd_lit))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(format!("{} · {}", t(cx, "profile.doNotDisturb"), when))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .opacity(0.8)
                                    .child(t(cx, "profile.dndResumeHint")),
                            ),
                    )
                    .into_any_element()
            } else {
                let short = match preferred.as_str() {
                    "30m" => t(cx, "profile.dndShort30Minutes"),
                    "tomorrow" => t(cx, "profile.dndShortTomorrow"),
                    _ => t(cx, "profile.dndShort1Hour"),
                };
                item("hb-me-dnd", Row::Dnd, dnd_lit)
                    .when(store.view.is_none(), |d| d.opacity(0.5))
                    .child(ink_icon("bell-off", 18., "hb-me-dnd", dnd_lit))
                    .child(format!("{} · {}", t(cx, "profile.doNotDisturb"), short))
                    .child(chevron("hb-me-dnd", self.durations, dnd_lit))
                    .into_any_element()
            };
            let durations = (self.durations && !quiet).then(|| {
                let ids = ["hb-me-d0", "hb-me-d1", "hb-me-d2"];
                div()
                    .flex()
                    .flex_col()
                    .children(DURATIONS.iter().enumerate().map(|(i, (id, key))| {
                        let l = lit(Row::Duration(i));
                        item(ids[i], Row::Duration(i), l)
                            .pl(px(42.))
                            .text_size(px(14.))
                            .child(t(cx, key))
                            .when(*id == preferred, |d| {
                                d.child(div().ml_auto().child(ink_icon("check", 16., ids[i], l)))
                            })
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .pl(px(42.))
                            .pr(px(16.))
                            .py(px(6.))
                            .text_size(px(14.))
                            .child(
                                div()
                                    .text_color(theme.c("ink-2"))
                                    .child(t(cx, "profile.customMoment")),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(32.))
                                    .text_size(px(13.))
                                    .child(Input::new(&self.custom)),
                            )
                            .child(
                                super::dialogs::button(
                                    "hb-me-apply",
                                    t(cx, "profile.apply"),
                                    32.,
                                    &theme,
                                )
                                .when(lit(Row::Apply), |d| d.bg(theme.c("btn-hover")))
                                .on_click({
                                    let me = me.clone();
                                    move |_, window, cx| {
                                        me.update(cx, |m, cx| m.run(Row::Apply, window, cx))
                                    }
                                }),
                            ),
                    )
                    .child(
                        div()
                            .mx(px(16.))
                            .mt(px(2.))
                            .mb(px(4.))
                            .text_size(px(12.))
                            .text_color(theme.c("ink-3"))
                            .child(t(cx, "profile.dndMenuHint")),
                    )
            });
            let profile_lit = lit(Row::Profile);
            let prefs_lit = lit(Row::Preferences);
            let profile_row = item("hb-me-profile", Row::Profile, profile_lit)
                .child(ink_icon("user-round", 18., "hb-me-profile", profile_lit))
                .child(t(cx, "profile.profileAndContext"))
                .child(chevron("hb-me-profile", false, profile_lit));
            let prefs_row = item("hb-me-prefs", Row::Preferences, prefs_lit)
                .child(ink_icon("settings", 18., "hb-me-prefs", prefs_lit))
                .child(t(cx, "profile.preferences"))
                .child(chevron("hb-me-prefs", false, prefs_lit));
            let error = self.error.clone().map(|e| {
                div()
                    .mx(px(16.))
                    .mt(px(6.))
                    .text_size(px(13.))
                    .text_color(theme.c("red-ink"))
                    .child(e)
            });
            let notice = div()
                .flex()
                .gap(px(8.))
                .mx(px(18.))
                .mt(px(4.))
                .mb(px(6.))
                .text_size(px(13.))
                .line_height(gpui::relative(1.4))
                .text_color(theme.c("ink-3"))
                .child(div().pt(px(1.)).child(icon("info", 16., theme.c("ink-3"))))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(t(cx, "workspace.passwordNotice")),
                );

            let max_h = window.viewport_size().height - px(32.);
            deferred(
                anchored()
                    .anchor(Anchor::BottomLeft)
                    .position(point(px(80.), window.viewport_size().height - px(16.)))
                    .snap_to_window()
                    .child(
                        div()
                            .id("hb-me-menu")
                            .occlude()
                            .track_focus(&self.focus)
                            .on_key_down(cx.listener(Self::on_key))
                            .on_mouse_down_out(cx.listener(|m, _, window, cx| m.close(window, cx)))
                            .w(px(380.))
                            .max_h(max_h)
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .pb(px(8.))
                            .bg(raised)
                            .text_color(theme.c("ink"))
                            .rounded(px(8.))
                            .shadow(theme.pop_shadow())
                            .font_family(ui::SANS)
                            .text_size(px(ui::size::BODY))
                            .line_height(gpui::relative(1.46))
                            .child(head)
                            .child(message)
                            .children(clear)
                            .child(sep())
                            .child(
                                div()
                                    .px(px(18.))
                                    .pt(px(4.))
                                    .pb(px(6.))
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::BLACK)
                                    .child(t(cx, "profile.yourStatus")),
                            )
                            .child(states)
                            .child(sep())
                            .child(dnd)
                            .children(durations)
                            .child(profile_row)
                            .child(prefs_row)
                            .children(error)
                            .child(sep())
                            .child(notice),
                    ),
            )
            .with_priority(1)
        });

        div().relative().child(button).children(menu)
    }
}

/// `color-mix(in srgb, a p%, b)`.
fn mix(a: Hsla, b: Hsla, p: f32) -> Hsla {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    gpui::Rgba {
        r: a.r * p + b.r * (1. - p),
        g: a.g * p + b.g * (1. - p),
        b: a.b * p + b.b * (1. - p),
        a: 1.,
    }
    .into()
}
