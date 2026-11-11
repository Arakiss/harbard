//! "<workspace> settings" (`workspace-settings.tsx`): the workspace's name,
//! its rail tile, and the repositories it holds, each with where it can be
//! moved. A move takes effect at once here and for every session at its
//! next prompt; the history goes with the repository.

use super::widgets::{self, Choice, Kind, Size};
use crate::api::{self, ApiError};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::workspaces::{PlacedRepo, WorkspaceEntry, WorkspacesList};
use crate::theme::Theme;
use crate::ui::{self, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use gpui_kit::component::input::{InputEvent, InputState};
use serde_json::{Value, json};
use std::rc::Rc;

/// The dialog and the rail icon's field.
const DIALOG_W: f32 = 480.;
const ICON_W: f32 = 88.;

/// A repository's last path segment.
pub fn repo_name(path: &str) -> &str {
    let p = path.trim_end_matches('/');
    p.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
}

/// Why a repository is here, in the operator's words.
fn placement_label(r: &PlacedRepo, cx: &App) -> SharedString {
    if r.moved {
        return t(cx, "shell.placementMoved");
    }
    if let Some(owner) = r.reason.strip_prefix("origin owner ") {
        return tv(cx, "shell.placementOriginOwner", &[("owner", owner)]);
    }
    if r.reason.starts_with("paired device ") {
        return t(cx, "shell.placementPairedDevice");
    }
    t(cx, "shell.placementDefault")
}

pub struct WorkspaceSettings {
    list: Option<WorkspacesList>,
    error: Option<SharedString>,
    busy: bool,
    name: Entity<InputState>,
    icon: Entity<InputState>,
    move_open: Option<String>,
    _subs: Vec<Subscription>,
}

impl WorkspaceSettings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let name = cx.new(|cx| InputState::new(window, cx));
        let icon = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_text_align(gpui::TextAlign::Center, cx);
            s
        });
        let subs = vec![
            cx.subscribe_in(&name, window, |_, s, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let v = s.read(cx).value().to_string();
                    if v.chars().count() > 40 {
                        let cut: String = v.chars().take(40).collect();
                        s.update(cx, |s, cx| s.set_value(cut, window, cx));
                    }
                    cx.notify();
                }
            }),
            cx.subscribe_in(&icon, window, |_, s, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    let v = s.read(cx).value().to_string();
                    if v.chars().count() > 2 {
                        let cut: String = v.chars().take(2).collect();
                        s.update(cx, |s, cx| s.set_value(cut, window, cx));
                    }
                    cx.notify();
                }
            }),
        ];
        let mut this = WorkspaceSettings {
            list: None,
            error: None,
            busy: false,
            name,
            icon,
            move_open: None,
            _subs: subs,
        };
        this.load(true, window, cx);
        this
    }

    /// The workspace on screen: the current one, else the default.
    fn me(&self, cx: &App) -> Option<WorkspaceEntry> {
        let list = self.list.as_ref()?;
        let id = Desk::get(cx)
            .read(cx)
            .workspace
            .clone()
            .filter(|w| w != "all")
            .unwrap_or_else(|| list.default.clone());
        list.workspaces
            .iter()
            .find(|w| w.id == id)
            .or_else(|| list.workspaces.iter().find(|w| w.default))
            .cloned()
    }

    fn load(&mut self, fill: bool, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_spawn(async { api::get_as::<WorkspacesList>("/workspaces") })
                .await;
            let _ = this.update(cx, |this, cx| {
                match out {
                    Ok(list) => {
                        this.list = Some(list);
                        if fill && let Some(me) = this.me(cx) {
                            let (name, icon) = (this.name.clone(), this.icon.clone());
                            let _ = handle.update(cx, |_, window, cx| {
                                name.update(cx, |s, cx| s.set_value(me.name.clone(), window, cx));
                                icon.update(cx, |s, cx| {
                                    s.set_value(me.icon.clone(), window, cx);
                                    s.set_placeholder(me.tile.clone(), window, cx);
                                });
                            });
                        }
                    }
                    Err(e) => this.error = Some(e.message.into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `POST /workspaces/<action>`, then the list and the workspace read again.
    fn run(
        &mut self,
        action: &'static str,
        body: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.error = None;
        self.move_open = None;
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let out: Result<Value, ApiError> = cx
                .background_spawn(async move { api::post(&format!("/workspaces/{action}"), &body) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Err(e) = out {
                    this.error = Some(e.message.into());
                }
                Desk::get(cx).read(cx).hub.refresh();
                cx.notify();
            });
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| this.load(false, window, cx));
            });
        })
        .detach();
        cx.notify();
    }
}

pub fn open(window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| WorkspaceSettings::new(window, cx));
    widgets::open_modal(
        view.into(),
        DIALOG_W,
        None,
        widgets::Place::Top(0.12),
        None,
        window,
        cx,
    );
}

impl Render for WorkspaceSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let me = self.me(cx);
        let title = match &me {
            Some(m) => tv(cx, "shell.workspaceSettingsFor", &[("name", &m.name)]),
            None => t(cx, "shell.workspaceSettings"),
        };
        let label = |s: SharedString| widgets::label(s, &theme);
        let small = |s: SharedString| ui::text(s, size::SMALL, weight::REGULAR, theme.c("ink-3"));
        let mut body = div()
            .px(px(space::GUTTER))
            .pt(px(space::XL))
            .pb(px(space::GUTTER));
        if self.list.is_none() && self.error.is_none() {
            body = body.child(widgets::hint(t(cx, "shell.loading"), &theme));
        }
        if let Some(e) = &self.error {
            body = body.child(widgets::hint(e.clone(), &theme));
        }
        if let (Some(me), Some(list)) = (me, self.list.clone()) {
            let name_now = self.name.read(cx).value().trim().to_string();
            let icon_now = self.icon.read(cx).value().to_string();
            let busy = self.busy;
            let others: Vec<WorkspaceEntry> = list
                .workspaces
                .iter()
                .filter(|w| w.id != me.id)
                .cloned()
                .collect();
            let mut repos: Vec<PlacedRepo> = list
                .repos
                .iter()
                .filter(|r| r.workspace == me.id)
                .cloned()
                .collect();
            repos.sort_by(|a, b| repo_name(&a.repo).cmp(repo_name(&b.repo)));
            let (id1, id2) = (me.id.clone(), me.id.clone());
            let owners_line = (!me.owners.is_empty()).then(|| {
                let mut s = tv(
                    cx,
                    "shell.reposJoinByOrigin",
                    &[("origin", "origin"), ("owners", &me.owners.join(", "))],
                )
                .to_string();
                if me.default {
                    s = format!("{s} {}", t(cx, "shell.reposJoinByDefault"));
                }
                s
            });
            let this = cx.entity().downgrade();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::SECTION))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::SM))
                            .child(label(t(cx, "shell.name")))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::MD))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(widgets::text_input(&self.name, &theme)),
                                    )
                                    .child(widgets::button(
                                        "ws-rename",
                                        t(cx, "shell.save"),
                                        Kind::Plain,
                                        Size::Normal,
                                        None,
                                        busy || name_now.is_empty() || name_now == me.name,
                                        &theme,
                                        cx.listener(move |this, _, window, cx| {
                                            let name =
                                                this.name.read(cx).value().trim().to_string();
                                            this.run(
                                                "rename",
                                                json!({ "workspace": id1, "name": name }),
                                                window,
                                                cx,
                                            )
                                        }),
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::SM))
                            .child(label(t(cx, "shell.railIcon")))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::MD))
                                    .child(
                                        div()
                                            .w(px(ICON_W))
                                            .font_weight(weight::STRONG)
                                            .child(widgets::text_input(&self.icon, &theme)),
                                    )
                                    .child(widgets::button(
                                        "ws-icon",
                                        t(cx, "shell.save"),
                                        Kind::Plain,
                                        Size::Normal,
                                        None,
                                        busy || icon_now == me.icon,
                                        &theme,
                                        cx.listener(move |this, _, window, cx| {
                                            let icon = this.icon.read(cx).value().to_string();
                                            this.run(
                                                "icon",
                                                json!({ "workspace": id2, "icon": icon }),
                                                window,
                                                cx,
                                            )
                                        }),
                                    )),
                            )
                            .child(small(t(cx, "shell.railIconHint"))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::SM))
                            .child(label(t(cx, "shell.repositories")))
                            .when_some(owners_line, |d, l| d.child(small(l.into())))
                            .when(repos.is_empty(), |d| {
                                d.child(widgets::hint(t(cx, "shell.noReposYet"), &theme))
                            })
                            .children(repos.into_iter().enumerate().map(|(i, r)| {
                                let open = self.move_open.as_deref() == Some(r.repo.as_str());
                                let choices: Vec<Choice> = others
                                    .iter()
                                    .map(|w| {
                                        let (repo, ws, this) =
                                            (r.repo.clone(), w.id.clone(), this.clone());
                                        Choice {
                                            label: w.name.clone().into(),
                                            selected: false,
                                            on: Rc::new(move |window, cx| {
                                                let body = json!({ "repo": repo, "workspace": ws });
                                                let _ = this.update(cx, |s, cx| {
                                                    s.run("move", body, window, cx)
                                                });
                                            }),
                                        }
                                    })
                                    .collect();
                                let (toggle_this, toggle_repo) = (this.clone(), r.repo.clone());
                                let reset_repo = r.repo.clone();
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::MD))
                                    .py(px(space::MD))
                                    .when(i > 0, |d| d.border_t_1().border_color(theme.c("line")))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                ui::text(
                                                    repo_name(&r.repo).to_string(),
                                                    size::UI,
                                                    weight::STRONG,
                                                    theme.c("ink"),
                                                )
                                                .truncate(),
                                            )
                                            .child(small(placement_label(&r, cx)).truncate()),
                                    )
                                    .when(!others.is_empty(), |d| {
                                        d.child(widgets::dropdown(
                                            format!("move-{i}"),
                                            t(cx, "shell.moveTo"),
                                            open,
                                            choices,
                                            busy,
                                            &theme,
                                            Rc::new(move |_, cx| {
                                                let repo = toggle_repo.clone();
                                                let _ = toggle_this.update(cx, |s, cx| {
                                                    s.move_open = if s.move_open.as_deref()
                                                        == Some(repo.as_str())
                                                    {
                                                        None
                                                    } else {
                                                        Some(repo)
                                                    };
                                                    cx.notify();
                                                });
                                            }),
                                        ))
                                    })
                                    .when(r.moved, |d| {
                                        d.child(widgets::button(
                                            SharedString::from(format!("reset-{i}")),
                                            t(cx, "shell.returnToRule"),
                                            Kind::Plain,
                                            Size::Normal,
                                            None,
                                            busy,
                                            &theme,
                                            cx.listener(move |this, _, window, cx| {
                                                this.run(
                                                    "reset",
                                                    json!({ "repo": reset_repo }),
                                                    window,
                                                    cx,
                                                )
                                            }),
                                        ))
                                    })
                            })),
                    ),
            );
        }
        let max_h = f32::from(window.viewport_size().height) * 0.7;
        widgets::base(&theme)
            .flex()
            .flex_col()
            .max_h(px(max_h))
            .child(widgets::dialog_head(title, &theme))
            .child(
                div()
                    .id("ws-settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::repo_name;

    #[test]
    fn repo_names_are_the_last_segment() {
        assert_eq!(repo_name("/Users/x/code/harbard/"), "harbard");
        assert_eq!(repo_name("harbard"), "harbard");
    }
}
