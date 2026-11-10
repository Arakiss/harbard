//! Preferences › Teams (`pairing-section.tsx`): every device paired to this
//! hub, its role, when it was last seen and what it did. Operator is given
//! on purpose, by typing the device's name; Revoke asks first. Invites
//! nobody used yet are listed, and Cancel stops a leaked one.

use super::shell_update::UpdateStore;
use super::widgets::{self, Choice, Kind, Size};
use crate::api::{self, ApiError};
use crate::derive::pairing::{
    ROLES, confirm_matches, describe_action, device_version, expires_line, refusal_text, relative,
    role_label, status_line,
};
use crate::i18n::{t, tv};
use crate::model::pairing::{Device, DeviceAction, DeviceLog, Invite, PairingView};
use crate::theme::Theme;
use crate::ui;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    Window, div, px,
};
use gpui_kit::component::input::{InputEvent, InputState};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, PartialEq)]
enum Confirming {
    Operator(String),
    Revoke(String),
}

pub struct PairingPage {
    view: Option<PairingView>,
    loaded: bool,
    confirming: Option<Confirming>,
    typed: Entity<InputState>,
    /// Open logs by device id; `None` while loading.
    logs: HashMap<String, Option<Vec<DeviceAction>>>,
    error: Option<SharedString>,
    status: Option<SharedString>,
    role_open: Option<String>,
    _poll: Task<()>,
    _subs: Vec<Subscription>,
}

impl PairingPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&UpdateStore::get(cx), |_, _, cx| cx.notify())
            .detach();
        let typed = cx.new(|cx| InputState::new(window, cx));
        let subs = vec![
            cx.subscribe_in(&typed, window, |_, _, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        // Read on open and every 15 s while the page is shown.
        let poll = cx.spawn(async move |this, cx| {
            loop {
                let read = cx
                    .background_spawn(async { api::get_as::<PairingView>("/pairing") })
                    .await;
                if this
                    .update(cx, |p, cx| {
                        p.view = read.ok();
                        p.loaded = true;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_secs(15))
                    .await;
            }
        });
        PairingPage {
            view: None,
            loaded: false,
            confirming: None,
            typed,
            logs: HashMap::new(),
            error: None,
            status: None,
            role_open: None,
            _poll: poll,
            _subs: subs,
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_spawn(async { api::get_as::<PairingView>("/pairing") })
                .await;
            let _ = this.update(cx, |p, cx| {
                p.view = read.ok();
                cx.notify();
            });
        })
        .detach();
    }

    fn device(&self, id: &str) -> Option<Device> {
        self.view
            .as_ref()?
            .devices
            .iter()
            .find(|d| d.id == id)
            .cloned()
    }

    /// One write; on success the status line, then a fresh read.
    fn send(&mut self, path: String, body: Value, done: SharedString, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let out: Result<Value, ApiError> = cx
                .background_spawn(async move { api::post(&path, &body) })
                .await;
            let _ = this.update(cx, |p, cx| {
                match out {
                    Ok(_) => {
                        p.confirming = None;
                        p.error = None;
                        p.status = Some(done);
                        p.reload(cx);
                    }
                    Err(e) => {
                        p.error = Some(refusal_text(&e, cx));
                        p.status = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_role(
        &mut self,
        d: &Device,
        role: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.role_open = None;
        if role == d.access {
            cx.notify();
            return;
        }
        if role == "operator" {
            self.typed.update(cx, |s, cx| s.set_value("", window, cx));
            self.confirming = Some(Confirming::Operator(d.id.clone()));
            self.error = None;
            self.status = None;
            cx.notify();
            return;
        }
        let done = tv(
            cx,
            "teams.statusRoleSet",
            &[("name", &d.name), ("role", &role_label(role, cx))],
        );
        self.send(
            format!("/pairing/{}/role", api::enc(&d.id)),
            json!({ "role": role }),
            done,
            cx,
        );
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let Some(c) = self.confirming.clone() else {
            return;
        };
        match c {
            Confirming::Operator(id) => {
                let Some(d) = self.device(&id) else { return };
                let typed = self.typed.read(cx).value().to_string();
                if !confirm_matches(&typed, &d.name) {
                    return;
                }
                let done = tv(
                    cx,
                    "teams.statusRoleSet",
                    &[("name", &d.name), ("role", &role_label("operator", cx))],
                );
                self.send(
                    format!("/pairing/{}/role", api::enc(&d.id)),
                    json!({ "role": "operator", "confirm": typed }),
                    done,
                    cx,
                );
            }
            Confirming::Revoke(id) => {
                let Some(d) = self.device(&id) else { return };
                let done = tv(cx, "teams.statusDeviceRevoked", &[("name", &d.name)]);
                self.send(
                    format!("/pairing/{}/revoke", api::enc(&d.id)),
                    json!({}),
                    done,
                    cx,
                );
            }
        }
    }

    fn revoke_invite(&mut self, i: &Invite, cx: &mut Context<Self>) {
        let done = tv(cx, "teams.statusInviteCancelled", &[("name", &i.name)]);
        self.send(
            format!("/pairing/invites/{}/revoke", api::enc(&i.reference)),
            json!({}),
            done,
            cx,
        );
    }

    fn toggle_log(&mut self, id: String, cx: &mut Context<Self>) {
        if self.logs.remove(&id).is_some() {
            cx.notify();
            return;
        }
        self.logs.insert(id.clone(), None);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let path = format!("/pairing/{}/log?limit=100", api::enc(&id));
            let out = cx
                .background_spawn(async move { api::get_as::<DeviceLog>(&path) })
                .await;
            let _ = this.update(cx, |p, cx| {
                match out {
                    Ok(log) => {
                        if let Some(slot) = p.logs.get_mut(&id) {
                            *slot = Some(log.actions);
                        }
                    }
                    Err(e) => {
                        p.logs.remove(&id);
                        p.error = Some(e.message.into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    // -----------------------------------------------------------------------

    fn render_log(
        &self,
        actions: Option<&Vec<DeviceAction>>,
        now: u64,
        theme: &Theme,
        cx: &Context<Self>,
    ) -> AnyElement {
        let Some(actions) = actions else {
            return widgets::hint(t(cx, "teams.logLoading"), theme).into_any_element();
        };
        if actions.is_empty() {
            return widgets::hint(t(cx, "teams.logEmpty"), theme).into_any_element();
        }
        div()
            .id("device-log")
            .mt(px(6.))
            .pt(px(6.))
            .max_h(px(240.))
            .overflow_y_scroll()
            .border_t_1()
            .border_color(theme.c("line"))
            .text_size(px(13.))
            .children(actions.iter().map(|a| {
                let refused = a.status == 401 || a.status == 403;
                div()
                    .flex()
                    .gap(px(8.))
                    .py(px(2.))
                    .when(refused, |d| d.text_color(theme.c("red-ink")))
                    .child(
                        div()
                            .w(px(88.))
                            .flex_none()
                            .text_color(theme.c("ink-4"))
                            .child(relative(a.ts_ms, now, cx)),
                    )
                    .child(div().flex_1().min_w_0().child(describe_action(a, cx)))
                    .child(
                        div()
                            .flex_none()
                            .font_family(ui::MONO)
                            .text_color(theme.c("ink-4"))
                            .child(format!("{} {} · {}", a.method, a.path, a.status)),
                    )
            }))
            .into_any_element()
    }

    fn render_row(
        &self,
        d: &Device,
        now: u64,
        hub: Option<(String, Option<String>)>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let build = device_version(d, hub.as_ref().map(|(v, c)| (v.as_str(), c.as_deref())));
        let live = !d.hub && d.revoked_ms.is_none_or(|r| r == 0);
        let revoked = d.revoked_ms.is_some_and(|r| r > 0);
        let confirming = match &self.confirming {
            Some(Confirming::Operator(id)) if *id == d.id => Some(true),
            Some(Confirming::Revoke(id)) if *id == d.id => Some(false),
            _ => None,
        };
        let log = self.logs.get(&d.id);
        let last = d.recent.first();
        let mut recent = match last {
            Some(a) => format!(
                "{} · {}",
                describe_action(a, cx),
                relative(a.ts_ms, now, cx)
            ),
            None => t(cx, "teams.noRemoteActivity").to_string(),
        };
        if d.reads + d.writes > 0 {
            let reads = if d.reads == 1 {
                tv(cx, "teams.readsOne", &[("count", "1")])
            } else {
                tv(cx, "teams.readsMany", &[("count", &d.reads.to_string())])
            };
            let writes = if d.writes == 1 {
                tv(cx, "teams.writesOne", &[("count", "1")])
            } else {
                tv(cx, "teams.writesMany", &[("count", &d.writes.to_string())])
            };
            recent = format!("{recent} · {reads}, {writes}");
        }
        let meta = format!(
            "{}{}",
            status_line(d, now, cx),
            if d.host.is_empty() {
                String::new()
            } else {
                format!(" · {}", d.host)
            }
        );
        let initial = d
            .name
            .trim()
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into());
        let id = d.id.clone();
        let role_chip = widgets::chip(
            role_label(&d.access, cx),
            if d.access == "operator" { "accent" } else { "" },
            theme,
        );

        let mut actions: Vec<AnyElement> = Vec::new();
        if live {
            let open = self.role_open.as_deref() == Some(d.id.as_str());
            let shown = if confirming == Some(true) {
                "operator"
            } else {
                d.access.as_str()
            };
            let choices = ROLES
                .iter()
                .map(|(role, _)| {
                    let dev = d.clone();
                    let role: &'static str = role;
                    let this = cx.entity().downgrade();
                    Choice {
                        label: role_label(role, cx),
                        selected: role == shown,
                        on: Rc::new(move |w, cx| {
                            let _ = this.update(cx, |p, cx| p.on_role(&dev, role, w, cx));
                        }),
                    }
                })
                .collect();
            let this = cx.entity().downgrade();
            let rid = d.id.clone();
            actions.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .font_weight(FontWeight::BOLD)
                    .text_size(px(14.))
                    .child(t(cx, "teams.roleField"))
                    .child(widgets::dropdown(
                        format!("role-{}", d.id),
                        role_label(shown, cx),
                        open,
                        32.,
                        choices,
                        false,
                        theme,
                        Rc::new(move |_, cx| {
                            let rid = rid.clone();
                            let _ = this.update(cx, |p, cx| {
                                p.role_open = if p.role_open.as_deref() == Some(rid.as_str()) {
                                    None
                                } else {
                                    Some(rid)
                                };
                                cx.notify();
                            });
                        }),
                    ))
                    .into_any_element(),
            );
        }
        if !d.hub {
            let lid = d.id.clone();
            actions.push(
                widgets::button(
                    SharedString::from(format!("log-{}", d.id)),
                    if log.is_some() {
                        t(cx, "teams.logHide")
                    } else {
                        t(cx, "teams.logShow")
                    },
                    Kind::Plain,
                    Size::Small,
                    None,
                    false,
                    theme,
                    cx.listener(move |p, _, _, cx| p.toggle_log(lid.clone(), cx)),
                )
                .into_any_element(),
            );
        }
        if live {
            let rid = d.id.clone();
            actions.push(
                widgets::button(
                    SharedString::from(format!("revoke-{}", d.id)),
                    t(cx, "teams.revoke"),
                    Kind::Danger,
                    Size::Small,
                    None,
                    false,
                    theme,
                    cx.listener(move |p, _, _, cx| {
                        p.confirming = Some(Confirming::Revoke(rid.clone()));
                        p.error = None;
                        p.status = None;
                        cx.notify();
                    }),
                )
                .into_any_element(),
            );
        }

        let confirm_box = confirming.map(|operator| {
            let name = d.name.clone();
            let base = div()
                .mt(px(6.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(8.))
                .border_1()
                .border_color(theme.c("flag-line"))
                .bg(theme.c("flag-bg"))
                .text_color(theme.c("ink"))
                .text_size(px(14.));
            if operator {
                let warning = t(cx, "teams.operatorWarning");
                let (before, after) = warning.split_once("{strong}").unwrap_or(("", &warning));
                let matches = confirm_matches(&self.typed.read(cx).value(), &name);
                base.child(
                    div()
                        .child(before.to_string())
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .child(t(cx, "teams.operatorWarningLead")),
                        )
                        .child(after.to_string())
                        .flex()
                        .flex_wrap(),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .font_weight(FontWeight::BOLD)
                        .child(tv(cx, "teams.typeNameToConfirm", &[("name", &name)]))
                        .child(
                            div()
                                .flex_1()
                                .child(widgets::text_input(&self.typed, theme)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(widgets::button(
                            "make-operator",
                            t(cx, "teams.makeOperator"),
                            Kind::Primary,
                            Size::Small,
                            None,
                            !matches,
                            theme,
                            cx.listener(|p, _, _, cx| p.confirm(cx)),
                        ))
                        .child(widgets::button(
                            "confirm-cancel",
                            t(cx, "teams.cancel"),
                            Kind::Plain,
                            Size::Small,
                            None,
                            false,
                            theme,
                            cx.listener(|p, _, _, cx| {
                                p.confirming = None;
                                cx.notify();
                            }),
                        )),
                )
            } else {
                let text = t(cx, "teams.revokeConfirm");
                let (before, after) = text.split_once("{name}").unwrap_or(("", &text));
                base.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .child(before.to_string())
                        .child(div().font_weight(FontWeight::BOLD).child(name.clone()))
                        .child(after.to_string()),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(widgets::button(
                            "revoke-confirm",
                            t(cx, "teams.revoke"),
                            Kind::Danger,
                            Size::Small,
                            None,
                            false,
                            theme,
                            cx.listener(|p, _, _, cx| p.confirm(cx)),
                        ))
                        .child(widgets::button(
                            "revoke-cancel",
                            t(cx, "teams.cancel"),
                            Kind::Plain,
                            Size::Small,
                            None,
                            false,
                            theme,
                            cx.listener(|p, _, _, cx| {
                                p.confirming = None;
                                cx.notify();
                            }),
                        )),
                )
            }
        });

        let hover = theme.c("hover");
        div()
            .id(SharedString::from(format!("device-{id}")))
            .flex()
            .gap(px(12.))
            .px(px(14.))
            .py(px(12.))
            .bg(theme.c("surface"))
            .hover(move |s| s.bg(hover))
            .child(
                div()
                    .flex_none()
                    .size(px(36.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::BLACK)
                    .bg(theme.c("accent-soft"))
                    .text_color(theme.c("accent-text"))
                    .child(initial),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .when(revoked, |d| d.text_color(theme.c("ink-3")))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .text_size(px(15.))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::BOLD)
                                                    .child(d.name.clone()),
                                            )
                                            .child(role_chip)
                                            .when(build.as_ref().is_some_and(|b| b.1), |r| {
                                                r.child(widgets::chip(
                                                    t(cx, "teams.versionBehind"),
                                                    "amber",
                                                    theme,
                                                ))
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .text_color(theme.c("ink-3"))
                                            .child(meta),
                                    )
                                    .when_some(build.clone(), |c, (text, _)| {
                                        c.child(
                                            div()
                                                .font_family(ui::MONO)
                                                .text_size(px(13.))
                                                .text_color(theme.c("ink-3"))
                                                .child(format!("harbard {text}")),
                                        )
                                    })
                                    .when(!d.hub, |c| {
                                        c.child(
                                            div()
                                                .text_size(px(13.))
                                                .text_color(theme.c("ink-3"))
                                                .child(recent),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_none()
                                    .flex_wrap()
                                    .items_center()
                                    .justify_end()
                                    .gap(px(8.))
                                    .children(actions),
                            ),
                    )
                    .children(confirm_box)
                    .when_some(log, |c, log| {
                        c.child(self.render_log(log.as_ref(), now, theme, cx))
                    }),
            )
            .into_any_element()
    }

    fn render_invites(
        &self,
        invites: &[Invite],
        now: u64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let n = invites.len();
        div()
            .mt(px(16.))
            .child(
                div()
                    .mb(px(8.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(t(cx, "teams.invitesTitle")),
            )
            .child(
                div()
                    .mb(px(8.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.c("line"))
                    .overflow_hidden()
                    .children(invites.iter().enumerate().map(|(i, inv)| {
                        let inv2 = inv.clone();
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .px(px(14.))
                            .py(px(12.))
                            .bg(theme.c("surface"))
                            .when(i + 1 < n, |d| d.border_b_1().border_color(theme.c("line")))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::BOLD)
                                                    .child(inv.name.clone()),
                                            )
                                            .child(widgets::chip(
                                                role_label(&inv.access, cx),
                                                if inv.access == "operator" {
                                                    "accent"
                                                } else {
                                                    ""
                                                },
                                                theme,
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .text_color(theme.c("ink-3"))
                                            .child(format!(
                                                "{} · {}",
                                                expires_line(inv, now, cx),
                                                inv.reference
                                            )),
                                    ),
                            )
                            .child(widgets::button(
                                SharedString::from(format!("invite-{}", inv.reference)),
                                t(cx, "teams.revokeInvite"),
                                Kind::Danger,
                                Size::Small,
                                None,
                                false,
                                theme,
                                cx.listener(move |p, _, _, cx| p.revoke_invite(&inv2, cx)),
                            ))
                    })),
            )
            .child(widgets::hint(t(cx, "teams.invitesHint"), theme))
            .into_any_element()
    }
}

impl Render for PairingPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let hub = UpdateStore::get(cx)
            .read(cx)
            .view
            .as_ref()
            .map(|v| (v.installed.version.clone(), v.installed.commit.clone()));
        let view = self.view.clone();
        let status: Option<AnyElement> = match (&self.error, &self.status) {
            (Some(e), _) => Some(widgets::error(e.clone(), &theme).into_any_element()),
            (None, Some(s)) => Some(s.clone().into_any_element()),
            _ => None,
        };
        let mut body = div()
            .py(px(18.))
            .child(widgets::hint(t(cx, "teams.sectionHint"), &theme));
        match &view {
            None => {
                if self.loaded {
                    body = body.child(widgets::banner(t(cx, "teams.unreachable"), &theme));
                }
            }
            Some(v) => {
                let mut devices: Vec<&Device> = v.devices.iter().filter(|d| d.hub).collect();
                devices.extend(v.devices.iter().filter(|d| !d.hub));
                let n = devices.len();
                let rows: Vec<AnyElement> = devices
                    .into_iter()
                    .enumerate()
                    .map(|(i, d)| {
                        let row = self.render_row(d, v.now_ms, hub.clone(), &theme, cx);
                        div()
                            .when(i + 1 < n, |x| x.border_b_1().border_color(theme.c("line")))
                            .child(row)
                            .into_any_element()
                    })
                    .collect();
                body = body.child(
                    div()
                        .mb(px(8.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme.c("line"))
                        .overflow_hidden()
                        .children(rows),
                );
                if !v.invites.is_empty() {
                    body = body.child(self.render_invites(&v.invites, v.now_ms, &theme, cx));
                }
                if v.devices.iter().all(|d| d.hub) {
                    let text = t(cx, "teams.onlyThisMac");
                    let (before, after) = text.split_once("{command}").unwrap_or((&text, ""));
                    body = body.child(
                        div()
                            .mt(px(2.))
                            .text_size(px(14.))
                            .text_color(theme.c("ink-3"))
                            .child(before.to_string())
                            .child(
                                div()
                                    .font_family(ui::MONO)
                                    .text_size(px(13.))
                                    .child(t(cx, "teams.inviteCommand")),
                            )
                            .child(after.to_string()),
                    );
                }
            }
        }
        body.child(widgets::status_line(status, &theme))
    }
}
