//! Preferences › General (language, motion, the Strip, external memory) and
//! › Notifications (macOS, the Dock, system notifications, sound, Do not
//! disturb, the held digest, Away).

use super::appearance::theme_write;
use super::shell_profile::{ProfileStore, refusal_text};
use super::stores::update_profile;
use super::widgets::{self, Kind, Mark, Size};
use crate::api;
use crate::derive::profile::DND_CHOICES;
use crate::desk::Desk;
use crate::i18n::{I18n, Locale, t, tv};
use crate::model::workspaces::{DeskCheck, FeedbackPrefs, MemorySink};
use crate::native::StripMode;
use crate::theme::Theme;
use crate::ui::{leading, size, space};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, px,
};
use serde_json::{Value, json};

/// A checkbox and its label, on a line of its own.
fn check_line(
    id: &'static str,
    on: bool,
    label: SharedString,
    disabled: bool,
    theme: &Theme,
    click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    widgets::check_row(id, on, label, disabled, theme, click).into_any_element()
}

// ---------------------------------------------------------------------------
// General
// ---------------------------------------------------------------------------

pub struct GeneralPage {
    os_reduce: bool,
    motion_error: Option<SharedString>,
    /// The night-dim choice shows at once; serve's read is the truth again
    /// once it agrees.
    dim_pending: Option<bool>,
    dim_error: Option<SharedString>,
    sink: Option<MemorySink>,
    checks: Vec<DeskCheck>,
    sink_confirm: Option<String>,
    sink_error: Option<SharedString>,
}

impl GeneralPage {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |this: &mut Self, desk, cx| {
            let stored = desk.read(cx).view.as_ref().map(|v| v.night_dim);
            if this.dim_pending.is_some() && stored == this.dim_pending {
                this.dim_pending = None;
            }
            cx.notify();
        })
        .detach();
        cx.observe_global::<I18n>(|_, cx| cx.notify()).detach();
        cx.spawn(async move |this, cx| {
            let reduce = cx
                .background_spawn(async { super::os::reduce_motion() })
                .await;
            let feedback = cx
                .background_spawn(async { api::get_as::<FeedbackPrefs>("/feedback") })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.os_reduce = reduce;
                if let Ok(f) = feedback {
                    this.sink = Some(f.memory_sink.unwrap_or_default());
                    this.checks = f.checks;
                }
                cx.notify();
            });
        })
        .detach();
        GeneralPage {
            os_reduce: false,
            motion_error: None,
            dim_pending: None,
            dim_error: None,
            sink: None,
            checks: Vec::new(),
            sink_confirm: None,
            sink_error: None,
        }
    }

    fn set_motion(&mut self, reduce: bool, cx: &mut Context<Self>) {
        self.motion_error = None;
        let task = Desk::get(cx).update(cx, |d, cx| {
            d.post("motion", json!({ "reduce": reduce }), cx)
        });
        cx.spawn(async move |this, cx| {
            let failed = task.await.is_err();
            let _ = this.update(cx, |this, cx| {
                if failed {
                    this.motion_error = Some(t(cx, "preferences.couldNotSave"));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_dim(&mut self, on: bool, cx: &mut Context<Self>) {
        self.dim_error = None;
        self.dim_pending = Some(on);
        let task = Desk::get(cx).update(cx, |d, cx| d.post("night-dim", json!({ "on": on }), cx));
        cx.spawn(async move |this, cx| {
            let failed = task.await.is_err();
            let _ = this.update(cx, |this, cx| {
                if failed {
                    this.dim_pending = None;
                    this.dim_error = Some(t(cx, "preferences.couldNotSave"));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// `POST /workspace/memory-sink-allow`: off at once; on asks first.
    fn sink_post(&mut self, body: Value, cx: &mut Context<Self>) {
        let before = self.sink.clone();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_spawn(async move { api::post("/workspace/memory-sink-allow", &body) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(v) => {
                        if let Some(id) = v["confirm"]["id"].as_str() {
                            this.sink_confirm = Some(id.to_string());
                        } else if let Some(s) = this.sink.as_mut() {
                            s.allowed = v["ok"].as_bool() == Some(true) && s.allowed;
                        }
                    }
                    Err(_) => {
                        this.sink = before;
                        this.sink_error = Some(t(cx, "feedback.memorySinkSaveFailed"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn sink_toggle(&mut self, on: bool, cx: &mut Context<Self>) {
        self.sink_error = None;
        if let Some(s) = self.sink.as_mut() {
            s.allowed = on;
        }
        self.sink_post(json!({ "on": on }), cx);
        cx.notify();
    }

    fn sink_confirm(&mut self, yes: bool, cx: &mut Context<Self>) {
        let Some(id) = self.sink_confirm.take() else {
            return;
        };
        if yes {
            self.sink_post(json!({ "on": true, "confirm": id }), cx);
        } else if let Some(s) = self.sink.as_mut() {
            s.allowed = false;
        }
        cx.notify();
    }

    fn render_language(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let current = cx.global::<I18n>().locale;
        let langs = [(Locale::En, "English"), (Locale::Es, "Español")];
        div()
            .child(widgets::legend_icon(
                "languages",
                t(cx, "preferences.languageLegend"),
                theme,
            ))
            .child(widgets::hint(t(cx, "preferences.languageHint"), theme))
            .child(widgets::mode_row(
                langs
                    .into_iter()
                    .map(|(l, name)| {
                        widgets::mode_box(
                            SharedString::from(format!("lang-{}", l.code())),
                            Mark::Radio,
                            current == l,
                            None,
                            name,
                            false,
                            theme,
                            move |_, _, cx| crate::i18n::set_locale(l, cx),
                        )
                        .into_any_element()
                    })
                    .collect(),
            ))
            .into_any_element()
    }

    fn render_motion(&self, reduce: bool, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .child(widgets::legend_icon(
                "sparkles",
                t(cx, "preferences.motion"),
                theme,
            ))
            .child(div().child(check_line(
                "reduce-motion",
                reduce,
                t(cx, "preferences.reduceMotion"),
                false,
                theme,
                cx.listener(move |this, _, _, cx| this.set_motion(!reduce, cx)),
            )))
            .child(div().child(widgets::hint(
                if self.os_reduce {
                    t(cx, "preferences.reduceMotionOsOn")
                } else {
                    t(cx, "preferences.reduceMotionHint")
                },
                theme,
            )))
            .when_some(self.motion_error.clone(), |d, e| {
                d.child(widgets::error(e, theme))
            })
            .into_any_element()
    }

    fn render_strip(&self, stored: bool, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        // Where the desktop put the Strip this run: the 2560×720 display or
        // an ordinary window.
        let what = match crate::native::strip_mode(cx) {
            StripMode::Wide => t(cx, "preferences.stripWide"),
            StripMode::Window => t(cx, "preferences.stripWindow"),
        };
        let text = format!("{} {}", t(cx, "preferences.stripWhat"), what);
        let on = self.dim_pending.unwrap_or(stored);
        div()
            .child(widgets::legend_icon(
                "panel-top",
                t(cx, "preferences.strip"),
                theme,
            ))
            .child(widgets::hint(text, theme))
            .child(div().flex().mt(px(space::MD)).child(widgets::button(
                "open-strip",
                t(cx, "preferences.openStrip"),
                Kind::Plain,
                Size::Normal,
                None,
                false,
                theme,
                |_, _, cx| crate::native::open_strip(cx),
            )))
            .child(check_line(
                "night-dim",
                on,
                t(cx, "preferences.dimStripAtNight"),
                false,
                theme,
                cx.listener(move |this, _, _, cx| this.set_dim(!on, cx)),
            ))
            .child(div().child(widgets::hint(t(cx, "preferences.dimStripHint"), theme)))
            .when_some(self.dim_error.clone(), |d, e| {
                d.child(widgets::error(e, theme))
            })
            .into_any_element()
    }

    fn render_sink(&self, sink: &MemorySink, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let allowed = sink.allowed;
        div()
            .child(widgets::legend_icon(
                "brain",
                t(cx, "feedback.memorySinkLegend"),
                theme,
            ))
            .child(div().child(check_line(
                "memory-sink-allow",
                allowed,
                t(cx, "feedback.memorySinkAllow"),
                !sink.configured,
                theme,
                cx.listener(move |this, _, _, cx| this.sink_toggle(!allowed, cx)),
            )))
            .child(div().child(widgets::hint(
                if sink.configured {
                    t(cx, "feedback.memorySinkHint")
                } else {
                    t(cx, "feedback.memorySinkMissing")
                },
                theme,
            )))
            .when(self.sink_confirm.is_some(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(space::MD))
                        .mt(px(space::MD))
                        .child(div().w_full().child(t(cx, "feedback.memorySinkConfirm")))
                        .child(widgets::button(
                            "sink-allow",
                            t(cx, "feedback.memorySinkAllowButton"),
                            Kind::Primary,
                            Size::Normal,
                            None,
                            false,
                            theme,
                            cx.listener(|this, _, _, cx| this.sink_confirm(true, cx)),
                        ))
                        .child(widgets::button(
                            "sink-cancel",
                            t(cx, "feedback.cancel"),
                            Kind::Plain,
                            Size::Normal,
                            None,
                            false,
                            theme,
                            cx.listener(|this, _, _, cx| this.sink_confirm(false, cx)),
                        )),
                )
            })
            .when_some(self.sink_error.clone(), |d, e| {
                d.child(widgets::error(e, theme))
            })
            .into_any_element()
    }

    fn render_checks(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        div()
            .child(widgets::legend_icon(
                "list-checks",
                t(cx, "feedback.checksLegend"),
                theme,
            ))
            .child(
                div()
                    .mt(px(space::MD))
                    .flex()
                    .flex_col()
                    .gap(px(space::XS))
                    .text_size(px(size::SMALL))
                    .line_height(px(leading::SMALL))
                    .text_color(theme.c("ink-2"))
                    .children(
                        self.checks
                            .iter()
                            .map(|c| div().child(check_line_text(c, cx))),
                    ),
            )
            .into_any_element()
    }
}

/// One check: what changed, and what the operator answered (`lib/checks.ts`).
fn check_line_text(c: &DeskCheck, cx: &App) -> String {
    let spanish = cx.global::<I18n>().locale == Locale::Es;
    let day = |ms: u64| {
        let (_, mo, d, _) = crate::ui::format::local_date(ms);
        crate::derive::themes::season_day(
            &format!("{mo:02}-{d:02}"),
            if spanish { Locale::Es } else { Locale::En },
        )
    };
    let short = |s: &str| {
        let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if s.chars().count() > 80 {
            format!("{}…", s.chars().take(79).collect::<String>())
        } else {
            s
        }
    };
    let text = c.text.clone().unwrap_or_default();
    let what = match c.what.as_str() {
        "rule" => tv(cx, "workspace.checkLineRule", &[("text", &short(&text))]),
        "context" if !text.trim().is_empty() => {
            tv(cx, "workspace.checkLineContext", &[("text", &short(&text))])
        }
        "context" => t(cx, "workspace.checkLineContextCleared"),
        "memory_sink" => t(cx, "workspace.checkLineMemorySink"),
        _ => t(cx, "workspace.checkLineOther"),
    };
    let answered = day(c.answered_ms.unwrap_or(c.since_ms));
    let answer = match c.outcome.as_deref() {
        Some("kept") => tv(cx, "workspace.checkLineKept", &[("day", &answered)]),
        Some("revoked") => tv(cx, "workspace.checkLineRevoked", &[("day", &answered)]),
        _ => t(cx, "workspace.checkLineUnanswered"),
    };
    format!("{what} · {} · {answer}", day(c.since_ms))
}

impl Render for GeneralPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let view = Desk::get(cx).read(cx).view.clone();
        let mut blocks = vec![self.render_language(&theme, cx)];
        if let Some(v) = &view {
            blocks.push(self.render_motion(v.motion.as_deref() == Some("reduce"), &theme, cx));
            blocks.push(self.render_strip(v.night_dim, &theme, cx));
        }
        if let Some(sink) = self.sink.clone() {
            blocks.push(self.render_sink(&sink, &theme, cx));
        }
        if !self.checks.is_empty() {
            blocks.push(self.render_checks(&theme, cx));
        }
        widgets::sections(blocks, &theme)
    }
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

pub struct NotificationsPage {
    os: Option<super::os::OsNotifications>,
    dock_error: Option<SharedString>,
    mute: bool,
    alerts_error: Option<SharedString>,
    dnd_error: Option<SharedString>,
    digest_error: Option<SharedString>,
    away_error: Option<SharedString>,
}

impl NotificationsPage {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ProfileStore::get(cx), |_, _, cx| cx.notify())
            .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        // The tray's "Mute notifications" is this switch too.
        cx.observe_global::<crate::native::Mute>(|this, cx| {
            this.mute = cx.global::<crate::native::Mute>().0;
            cx.notify();
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let os = cx
                .background_spawn(async { super::os::notifications() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.os = os;
                cx.notify();
            });
        })
        .detach();
        NotificationsPage {
            os: None,
            dock_error: None,
            mute: crate::desk::desk_prefs().mute,
            alerts_error: None,
            dnd_error: None,
            digest_error: None,
            away_error: None,
        }
    }

    fn set_dock(&mut self, on: bool, cx: &mut Context<Self>) {
        self.dock_error = None;
        let task = theme_write("theme-dock", json!({ "on": on }), cx);
        cx.spawn(async move |this, cx| {
            let failed = task.await.is_err();
            let _ = this.update(cx, |this, cx| {
                if failed {
                    this.dock_error = Some(t(cx, "preferences.couldNotSaveNotResponding"));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `desk.json` `mute`, the same switch as the menu bar's "Mute".
    fn set_alerts(&mut self, show: bool, cx: &mut Context<Self>) {
        self.alerts_error = None;
        crate::native::set_mute(!show, cx);
        self.mute = crate::native::muted();
        if self.mute == show {
            self.alerts_error = Some(t(cx, "preferences.couldNotSaveDesktop"));
        }
        cx.notify();
    }

    fn profile_patch(
        &mut self,
        patch: Value,
        slot: fn(&mut Self) -> &mut Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        *slot(self) = None;
        let task = update_profile(patch, cx);
        cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = out {
                    *slot(this) = Some(refusal_text(&e, cx));
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for NotificationsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let mut blocks: Vec<AnyElement> = Vec::new();

        if let Some(os) = self.os.filter(|o| !(o.allowed && o.badge)) {
            blocks.push(
                div()
                    .child(widgets::legend_icon(
                        "bell-off",
                        t(cx, "preferences.macNotifications"),
                        &theme,
                    ))
                    .child(widgets::hint(
                        if os.allowed {
                            t(cx, "preferences.macNoBadge")
                        } else {
                            t(cx, "preferences.macNotificationsOff")
                        },
                        &theme,
                    ))
                    .child(div().flex().mt(px(space::MD)).child(widgets::button(
                        "open-system-settings",
                        t(cx, "preferences.openSystemSettings"),
                        Kind::Plain,
                        Size::Normal,
                        None,
                        false,
                        &theme,
                        |_, _, _| super::os::open_notification_settings(),
                    )))
                    .into_any_element(),
            );
        }

        let state = &theme.state;
        let has_state = state.is_object();
        let dock = state["dock_attention"].as_bool().unwrap_or(true);
        blocks.push(
            div()
                .child(widgets::legend_icon(
                    "panel-bottom",
                    t(cx, "preferences.dock"),
                    &theme,
                ))
                .child(div().child(check_line(
                    "dock-attention",
                    dock,
                    t(cx, "preferences.dockAttention"),
                    !has_state,
                    &theme,
                    cx.listener(move |this, _, _, cx| this.set_dock(!dock, cx)),
                )))
                .child(div().child(widgets::hint(
                    t(cx, "preferences.dockAttentionHint"),
                    &theme,
                )))
                .when_some(self.dock_error.clone(), |d, e| {
                    d.child(widgets::error(e, &theme))
                })
                .into_any_element(),
        );

        let show = !self.mute;
        blocks.push(
            div()
                .child(widgets::legend_icon(
                    "bell-ring",
                    t(cx, "preferences.systemAlerts"),
                    &theme,
                ))
                .child(div().child(check_line(
                    "system-alerts",
                    show,
                    t(cx, "preferences.showMacAlerts"),
                    false,
                    &theme,
                    cx.listener(move |this, _, _, cx| this.set_alerts(!show, cx)),
                )))
                .child(div().child(widgets::hint(t(cx, "preferences.systemAlertsHint"), &theme)))
                .when_some(self.alerts_error.clone(), |d, e| {
                    d.child(widgets::error(e, &theme))
                })
                .into_any_element(),
        );

        blocks.push(
            div()
                .child(widgets::legend_icon(
                    "volume-x",
                    t(cx, "preferences.sound"),
                    &theme,
                ))
                .child(widgets::hint(t(cx, "preferences.soundHint"), &theme))
                .into_any_element(),
        );

        let profile = ProfileStore::get(cx).read(cx).view.clone();
        let loaded = profile.is_some();
        let p = profile.map(|v| v.profile).unwrap_or_default();
        let current = p.dnd_default.clone().unwrap_or_else(|| "1h".into());
        blocks.push(
            div()
                .child(widgets::legend_icon(
                    "moon-star",
                    t(cx, "preferences.doNotDisturb"),
                    &theme,
                ))
                .child(widgets::hint(t(cx, "preferences.dndHint"), &theme))
                .child(widgets::mode_row(
                    DND_CHOICES
                        .iter()
                        .map(|(id, key)| {
                            let id = *id;
                            widgets::mode_box(
                                SharedString::from(format!("dnd-default-{id}")),
                                Mark::Radio,
                                current == id,
                                None,
                                t(cx, key),
                                !loaded,
                                &theme,
                                cx.listener(move |this, _, _, cx| {
                                    this.profile_patch(
                                        json!({ "dnd_default": id }),
                                        |s| &mut s.dnd_error,
                                        cx,
                                    )
                                }),
                            )
                            .into_any_element()
                        })
                        .collect(),
                ))
                .child(div().child(widgets::hint(t(cx, "preferences.dndDefaultHint"), &theme)))
                .when_some(self.dnd_error.clone(), |d, e| {
                    d.child(widgets::error(e, &theme))
                })
                .into_any_element(),
        );

        let digest = p.digest_hourly;
        blocks.push(
            div()
                .child(widgets::legend_icon(
                    "inbox",
                    t(cx, "preferences.heldDigest"),
                    &theme,
                ))
                .child(div().child(check_line(
                    "digest-hourly",
                    digest,
                    t(cx, "preferences.hourlyDigest"),
                    !loaded,
                    &theme,
                    cx.listener(move |this, _, _, cx| {
                        this.profile_patch(
                            json!({ "digest_hourly": !digest }),
                            |s| &mut s.digest_error,
                            cx,
                        )
                    }),
                )))
                .child(div().child(widgets::hint(t(cx, "preferences.hourlyDigestHint"), &theme)))
                .when_some(self.digest_error.clone(), |d, e| {
                    d.child(widgets::error(e, &theme))
                })
                .into_any_element(),
        );

        let away = p.away_suggest != Some(false);
        blocks.push(
            div()
                .child(widgets::legend_icon(
                    "coffee",
                    t(cx, "preferences.awayLegend"),
                    &theme,
                ))
                .child(div().child(check_line(
                    "away-suggest",
                    away,
                    t(cx, "preferences.awaySuggest"),
                    !loaded,
                    &theme,
                    cx.listener(move |this, _, _, cx| {
                        this.profile_patch(
                            json!({ "away_suggest": !away }),
                            |s| &mut s.away_error,
                            cx,
                        )
                    }),
                )))
                .child(div().child(widgets::hint(t(cx, "preferences.awaySuggestHint"), &theme)))
                .when_some(self.away_error.clone(), |d, e| {
                    d.child(widgets::error(e, &theme))
                })
                .into_any_element(),
        );

        widgets::sections(blocks, &theme)
    }
}
