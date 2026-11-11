//! The menu behind the operator's avatar at the foot of the rail
//! (`user-menu.tsx`, `operator-avatar.tsx`): who and in which state, the
//! status message, the six coordination states, Do not disturb, Profile and
//! context, Preferences, and the fixed password notice. Writes go through
//! [`ProfileStore::update`] (`POST /profile`).

use super::menu::{float, item_icon, menu_step, pop_item, pop_panel, pop_sep, pop_title};
use super::profile::{
    ProfileStore, STATES, custom_until, dnd_until, done_until, now_ms, quiet_label, refusal_text,
    state_info, status_expiry, tone_token,
};
use crate::i18n::{t, tv};
use crate::theme::Theme;
use crate::ui::kit_theme::{self, FocusRing as _};
use crate::ui::{self, avatar_size, control, icon, icon_size, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, AppContext, Context, Entity, FocusHandle, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, div, point, px,
};
use gpui_kit::component::input::{Input, InputState};
use serde_json::json;

/// The menu's width.
const WIDTH: f32 = 380.;
/// The "available" dot on the face: 8 px inside a 2 px cut-out.
const FACE_DOT: f32 = 12.;
/// Any other state's mark on the face, and the glyph inside it. The glyph
/// is smaller than the icon scale on purpose: it is a mark on a 36 px
/// avatar, not an icon in a row.
const FACE_MARK: f32 = 16.;
const FACE_GLYPH: f32 = 10.;

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
                .right(px(-space::XS))
                .bottom(px(-space::XS))
                .size(px(FACE_DOT))
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
            .right(px(-space::XS))
            .bottom(px(-space::XS))
            .size(px(FACE_MARK + space::XS))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.c("surface-raised"))
            .border_2()
            .border_color(ring)
            .child(icon(name, FACE_GLYPH, theme.c(tone_token(tone))))
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

        kit_theme::mark_menu(kit_theme::MENU_USER, self.open);
        let button = div()
            .id("hb-me-button")
            .occlude()
            .mt(px(space::SM))
            .rounded(px(radius::MD))
            .cursor_pointer()
            .focus_ring(&theme, false)
            .tooltip(kit_theme::tip(tv(
                cx,
                "shell.menuOf",
                &[("name", name.as_ref())],
            )))
            .on_click(cx.listener(|m, _, window, cx| m.toggle(window, cx)))
            .relative()
            .child(super::decor::keep_clear("me"))
            .child(operator_face(
                &initials,
                avatar_size::ROW,
                Some(presence),
                theme.c("frame"),
                cx,
            ));

        let menu = self.open.then(|| {
            let rows = self.rows(cx);
            let lit = |row: Row| self.kb.and_then(|i| rows.get(i)) == Some(&row);
            let item = |id: &'static str, row: Row, lit_now: bool| {
                let me = me.clone();
                pop_item(id, lit_now, &theme)
                    .group(id)
                    .on_click(move |_, window, cx| me.update(cx, |m, cx| m.run(row, window, cx)))
            };
            let label = |text: SharedString| div().min_w_0().truncate().child(text);
            let chevron = |group: &'static str, open: bool, lit_now: bool| {
                let hover_ink = theme.c("menu-hover-ink");
                div().ml_auto().flex_none().child(
                    icon(
                        "chevron-right",
                        icon_size::ROW,
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
                .flex_none()
                .rounded(px(radius::MD))
                .cursor_pointer()
                .group("hb-me-photo")
                .focus_ring(&theme, false)
                .tooltip(kit_theme::tip(t(cx, "profile.changePhoto")))
                .on_click({
                    let me = me.clone();
                    move |_, window, cx| me.update(cx, |m, cx| m.run(Row::Photo, window, cx))
                })
                .child(operator_face(
                    &initials,
                    avatar_size::ROW,
                    Some(presence),
                    raised,
                    cx,
                ))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(px(radius::MD))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(scrim)
                        .opacity(if lit(Row::Photo) { 1. } else { 0. })
                        .group_hover("hb-me-photo", |s| s.opacity(1.))
                        .child(icon("camera", icon_size::ROW, raised)),
                );
            let head = div()
                .flex()
                .items_center()
                .gap(px(space::LG))
                .px(px(space::LG))
                .pt(px(space::MD))
                .pb(px(space::LG))
                .child(photo)
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            ui::text(name.clone(), size::BODY, weight::HEAVY, theme.c("ink"))
                                .truncate(),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::XS))
                                .child(icon(
                                    state.icon,
                                    icon_size::INLINE,
                                    theme.c(tone_token(state.tone)),
                                ))
                                .child(
                                    ui::text(
                                        headline,
                                        size::SMALL,
                                        weight::REGULAR,
                                        theme.c("ink-2"),
                                    )
                                    .truncate(),
                                ),
                        ),
                );

            // 2. The status message.
            let status = store.status(now).cloned();
            let message_hover = theme.c("line-hover");
            let message = div()
                .id("hb-me-message")
                .flex_none()
                .mx(px(space::LG))
                .mb(px(space::MD))
                .h(px(control::MD))
                .flex()
                .items_center()
                .gap(px(space::MD))
                .px(px(space::MD))
                .border_1()
                .border_color(if lit(Row::Message) {
                    message_hover
                } else {
                    theme.c("line-strong")
                })
                .rounded(px(radius::BASE))
                .cursor_pointer()
                .hover(move |s| s.border_color(message_hover))
                .focus_ring(&theme, false)
                .on_click({
                    let me = me.clone();
                    move |_, window, cx| me.update(cx, |m, cx| m.run(Row::Message, window, cx))
                })
                .map(|d| match &status {
                    Some(s) => d
                        .map(|d| {
                            if s.emoji.is_empty() {
                                d.child(icon(
                                    "message-square-text",
                                    icon_size::ROW,
                                    theme.c("ink-3"),
                                ))
                            } else {
                                d.child(
                                    ui::text(
                                        s.emoji.clone(),
                                        size::UI,
                                        weight::REGULAR,
                                        theme.c("ink"),
                                    )
                                    .flex_none(),
                                )
                            }
                        })
                        .child(
                            ui::text(s.text.clone(), size::UI, weight::REGULAR, theme.c("ink"))
                                .flex_1()
                                .min_w_0()
                                .truncate(),
                        )
                        .children(status_expiry(s.clear_at_ms, now, cx).map(|e| {
                            ui::text(e, size::META, weight::REGULAR, theme.c("ink-3")).flex_none()
                        })),
                    None => d
                        .child(icon(
                            "message-square-text",
                            icon_size::ROW,
                            theme.c("ink-3"),
                        ))
                        .child(
                            ui::text(
                                t(cx, "profile.addStatusMessage"),
                                size::UI,
                                weight::REGULAR,
                                theme.c("ink-3"),
                            )
                            .truncate(),
                        ),
                });
            let clear = status.is_some().then(|| {
                let l = lit(Row::Clear);
                item("hb-me-clear", Row::Clear, l)
                    .child(item_icon("x", "hb-me-clear", l, &theme))
                    .child(label(t(cx, "profile.clearStatusMessage")))
            });

            // 3. Your status: a list with the one in force selected.
            let states = div()
                .flex()
                .flex_col()
                .gap(px(space::HAIR))
                .px(px(space::XS))
                .children(STATES.iter().enumerate().map(|(i, s)| {
                    let on = s.id == state.id;
                    let tone = theme.c(tone_token(s.tone));
                    let hover_bg = theme.c("hover");
                    let me = me.clone();
                    div()
                        .id(SharedString::from(format!("hb-me-state-{}", s.id)))
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .px(px(space::MD))
                        .py(px(space::XS))
                        .rounded(px(radius::BASE))
                        .cursor_pointer()
                        .when(store.view.is_none(), |d| d.opacity(0.5))
                        .when(on, |d| d.bg(theme.c("selected")))
                        .when(!on && lit(Row::State(i)), |d| d.bg(hover_bg))
                        .when(!on, |d| d.hover(move |st| st.bg(hover_bg)))
                        .focus_ring(&theme, true)
                        .on_click(move |_, window, cx| {
                            me.update(cx, |m, cx| m.run(Row::State(i), window, cx))
                        })
                        .child(icon(s.icon, icon_size::ROW, tone))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .child(
                                    ui::text(
                                        s.label(cx),
                                        size::UI,
                                        if on { weight::STRONG } else { weight::REGULAR },
                                        theme.c("ink"),
                                    )
                                    .truncate(),
                                )
                                .child(
                                    ui::text(
                                        s.hint(cx),
                                        size::META,
                                        weight::REGULAR,
                                        theme.c("ink-3"),
                                    )
                                    .truncate(),
                                ),
                        )
                        .when(on, |d| {
                            d.child(icon("check", icon_size::ROW, theme.c("ink")))
                        })
                }));

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
                    .tooltip(kit_theme::tip(t(cx, "profile.dndResumeHint")))
                    .child(item_icon("bell-off", "hb-me-dnd", dnd_lit, &theme))
                    .child(label(
                        format!("{} · {}", t(cx, "profile.doNotDisturb"), when).into(),
                    ))
                    .into_any_element()
            } else {
                let short = match preferred.as_str() {
                    "30m" => t(cx, "profile.dndShort30Minutes"),
                    "tomorrow" => t(cx, "profile.dndShortTomorrow"),
                    _ => t(cx, "profile.dndShort1Hour"),
                };
                item("hb-me-dnd", Row::Dnd, dnd_lit)
                    .when(store.view.is_none(), |d| d.opacity(0.5))
                    .child(item_icon("bell-off", "hb-me-dnd", dnd_lit, &theme))
                    .child(label(
                        format!("{} · {}", t(cx, "profile.doNotDisturb"), short).into(),
                    ))
                    .child(chevron("hb-me-dnd", self.durations, dnd_lit))
                    .into_any_element()
            };
            // The sub-rows start where the rows' labels do.
            let indent = space::MD + icon_size::ROW + space::MD;
            let durations = (self.durations && !quiet).then(|| {
                let ids = ["hb-me-d0", "hb-me-d1", "hb-me-d2"];
                div()
                    .flex()
                    .flex_col()
                    .children(DURATIONS.iter().enumerate().map(|(i, (id, key))| {
                        let l = lit(Row::Duration(i));
                        item(ids[i], Row::Duration(i), l)
                            .pl(px(indent))
                            .child(label(t(cx, key)))
                            .when(*id == preferred, |d| {
                                d.child(
                                    div()
                                        .ml_auto()
                                        .flex_none()
                                        .child(item_icon("check", ids[i], l, &theme)),
                                )
                            })
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::MD))
                            .mx(px(space::XS))
                            .pl(px(indent))
                            .pr(px(space::MD))
                            .py(px(space::XS))
                            .child(
                                ui::text(
                                    t(cx, "profile.customMoment"),
                                    size::UI,
                                    weight::REGULAR,
                                    theme.c("ink-2"),
                                )
                                .flex_none(),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .h(px(control::MD))
                                    .child(Input::new(&self.custom)),
                            )
                            .child(
                                super::dialogs::button(
                                    "hb-me-apply",
                                    t(cx, "profile.apply"),
                                    control::MD,
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
                            .pl(px(space::XS + indent))
                            .pr(px(space::LG))
                            .pb(px(space::XS))
                            .child(ui::text(
                                t(cx, "profile.dndMenuHint"),
                                size::META,
                                weight::REGULAR,
                                theme.c("ink-3"),
                            )),
                    )
            });
            let profile_lit = lit(Row::Profile);
            let prefs_lit = lit(Row::Preferences);
            let profile_row = item("hb-me-profile", Row::Profile, profile_lit)
                .child(item_icon(
                    "user-round",
                    "hb-me-profile",
                    profile_lit,
                    &theme,
                ))
                .child(label(t(cx, "profile.profileAndContext")))
                .child(chevron("hb-me-profile", false, profile_lit));
            let prefs_row = item("hb-me-prefs", Row::Preferences, prefs_lit)
                .child(item_icon("settings", "hb-me-prefs", prefs_lit, &theme))
                .child(label(t(cx, "profile.preferences")))
                .child(chevron("hb-me-prefs", false, prefs_lit));
            let error = self.error.clone().map(|e| {
                div()
                    .mx(px(space::LG))
                    .mt(px(space::SM))
                    .px(px(space::MD))
                    .py(px(space::XS))
                    .rounded(px(radius::BASE))
                    .bg(theme.c("red-bg"))
                    .child(ui::text(
                        e,
                        size::SMALL,
                        weight::REGULAR,
                        theme.c("red-ink"),
                    ))
            });
            let notice = div()
                .flex()
                .gap(px(space::MD))
                .px(px(space::LG))
                .pt(px(space::XS))
                .pb(px(space::MD))
                .child(div().pt(px(space::HAIR / 2.)).child(icon(
                    "info",
                    icon_size::INLINE,
                    theme.c("ink-3"),
                )))
                .child(
                    ui::text(
                        t(cx, "workspace.passwordNotice"),
                        size::META,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .flex_1()
                    .min_w_0(),
                );

            let view = window.viewport_size();
            let max_h = view.height - px(space::BREAK);
            float(
                "hb-me-menu-in",
                pop_panel(&theme)
                    .id("hb-me-menu")
                    .occlude()
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(Self::on_key))
                    .on_mouse_down_out(cx.listener(|m, _, window, cx| m.close(window, cx)))
                    .w(px(WIDTH))
                    .max_h(max_h)
                    .overflow_y_scroll()
                    .child(head)
                    .child(message)
                    .children(clear)
                    .child(pop_sep(&theme))
                    .child(pop_title(t(cx, "profile.yourStatus"), &theme))
                    .child(states)
                    .child(pop_sep(&theme))
                    .child(dnd)
                    .children(durations)
                    .child(profile_row)
                    .child(prefs_row)
                    .children(error)
                    .child(pop_sep(&theme))
                    .child(notice),
                // Beside the avatar, its foot on the avatar's.
                point(px(super::RAIL + space::HAIR), view.height - px(space::LG)),
                Anchor::BottomLeft,
                window,
                cx,
            )
        });

        div().relative().child(button).children(menu)
    }
}
