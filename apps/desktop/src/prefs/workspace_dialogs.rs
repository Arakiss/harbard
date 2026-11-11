//! "New workspace" (from the workspace menu) and "Remove <workspace>?"
//! (from its settings): the two registry edits that add or take away a
//! tile on the rail. serve picks the new id from the name and refuses what
//! the registry forbids; on success the app goes to the new workspace, or
//! back to the default after a removal.

use super::widgets::{self, Kind, Size};
use crate::api::{self, ApiError};
use crate::derive::workspaces::{NAME_MAX, name_ok, name_problem, parse_owners, refusal};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::theme::Theme;
use crate::ui::{leading, radius, size, space};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Styled, Subscription, Window, div, px,
};
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::input::{InputEvent, InputState};
use serde_json::{Value, json};

/// Both dialogs, like the workspace settings they sit beside.
const DIALOG_W: f32 = 480.;

/// What a write's failure says: serve's refusal worded, or that it could
/// not be reached.
fn failure(e: &ApiError, cx: &App) -> SharedString {
    if e.status.is_none() {
        return t(cx, "shell.serveNotResponding");
    }
    refusal(e.code.as_deref(), &e.message, cx)
}

// ---------------------------------------------------------------------------
// New workspace
// ---------------------------------------------------------------------------

struct NewWorkspace {
    name: Entity<InputState>,
    owners: Entity<InputState>,
    /// A submit was tried: an empty name is now worth saying.
    tried: bool,
    busy: bool,
    error: Option<SharedString>,
    _subs: Vec<Subscription>,
}

impl NewWorkspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let name = cx.new(|cx| InputState::new(window, cx));
        let placeholder = t(cx, "shell.newWorkspaceOwnersPlaceholder");
        let owners = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subs = vec![
            cx.subscribe_in(
                &name,
                window,
                |this, s, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => {
                        let v = s.read(cx).value().to_string();
                        if v.chars().count() > NAME_MAX {
                            let cut: String = v.chars().take(NAME_MAX).collect();
                            s.update(cx, |s, cx| s.set_value(cut, window, cx));
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(&owners, window, |this, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.submit(window, cx);
                }
            }),
        ];
        NewWorkspace {
            name,
            owners,
            tried: false,
            busy: false,
            error: None,
            _subs: subs,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tried = true;
        cx.notify();
        let raw = self.name.read(cx).value().to_string();
        if self.busy || !name_ok(&raw) {
            return;
        }
        self.busy = true;
        self.error = None;
        let owners = parse_owners(self.owners.read(cx).value().as_ref());
        let body = json!({ "name": raw.trim(), "owners": owners });
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let out: Result<Value, ApiError> = cx
                .background_spawn(async move { api::post("/workspaces/add", &body) })
                .await;
            let _ = this.update(cx, |this, cx| match out {
                Ok(v) => {
                    let id = v["workspace"].as_str().unwrap_or_default().to_string();
                    let _ = handle.update(cx, |_, window, cx| widgets::close_top(window, cx));
                    if id.is_empty() {
                        Desk::get(cx).read(cx).hub.refresh();
                    } else {
                        crate::shell::rail::switch_to(id, cx);
                    }
                }
                Err(e) => {
                    this.busy = false;
                    this.error = Some(failure(&e, cx));
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for NewWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let raw = self.name.read(cx).value().to_string();
        let problem = name_problem(&raw, self.tried, cx);
        let foot = widgets::dialog_foot(&theme)
            .mt(px(space::GUTTER))
            .child(widgets::button(
                "ws-new-cancel",
                t(cx, "shell.cancel"),
                Kind::Plain,
                Size::Normal,
                None,
                false,
                &theme,
                |_, w, cx| widgets::close_top(w, cx),
            ))
            .child(widgets::button(
                "ws-new-create",
                if self.busy {
                    t(cx, "shell.creating")
                } else {
                    t(cx, "shell.create")
                },
                Kind::Primary,
                Size::Normal,
                None,
                self.busy,
                &theme,
                cx.listener(|this, _, window, cx| this.submit(window, cx)),
            ));
        widgets::base(&theme)
            // Enter in a field submits (`PressEnter` above); the dialog
            // layer would also take it as "OK" and close before serve answers.
            .on_action(|_: &Confirm, _, _| {})
            .flex()
            .flex_col()
            .child(widgets::dialog_head(
                t(cx, "shell.newWorkspaceTitle"),
                &theme,
            ))
            .child(
                div()
                    .px(px(space::GUTTER))
                    .pt(px(space::GUTTER))
                    .flex()
                    .flex_col()
                    .gap(px(space::XL))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::SM))
                            .child(widgets::label(t(cx, "shell.name"), &theme))
                            .child(
                                widgets::text_input(&self.name, &theme)
                                    .when(problem.is_some(), |i| {
                                        i.border_color(theme.c("red-line"))
                                    }),
                            )
                            .children(problem.map(|p| widgets::error(p, &theme))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(space::SM))
                            .child(widgets::label(t(cx, "shell.newWorkspaceOwners"), &theme))
                            .child(widgets::text_input(&self.owners, &theme))
                            .child(widgets::hint(t(cx, "shell.newWorkspaceOwnersHint"), &theme)),
                    )
                    .children(self.error.clone().map(|e| widgets::error(e, &theme))),
            )
            .child(foot)
    }
}

/// Open "New workspace" with the caret in the name.
pub fn open_new(window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| NewWorkspace::new(window, cx));
    let first = view.read(cx).name.clone();
    widgets::open_modal(
        view.into(),
        DIALOG_W,
        None,
        widgets::Place::Top(0.12),
        None,
        window,
        cx,
    );
    widgets::autofocus(first, window, cx);
}

// ---------------------------------------------------------------------------
// Remove a workspace
// ---------------------------------------------------------------------------

/// What goes and, in the same breath, that nothing is deleted.
struct RemoveWorkspace {
    title: SharedString,
    id: String,
    /// Where its repositories go, and where the app goes after.
    default_id: String,
    default_name: String,
    busy: bool,
    error: Option<SharedString>,
    /// Cancel has the focus when the dialog opens.
    cancel: FocusHandle,
}

impl RemoveWorkspace {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let body = json!({ "workspace": self.id });
        let default_id = self.default_id.clone();
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let out: Result<Value, ApiError> = cx
                .background_spawn(async move { api::post("/workspaces/remove", &body) })
                .await;
            let _ = this.update(cx, |this, cx| match out {
                Ok(_) => {
                    // This dialog, then the settings of what is gone.
                    let _ = handle.update(cx, |_, window, cx| {
                        widgets::close_top(window, cx);
                        widgets::close_top(window, cx);
                    });
                    crate::shell::rail::switch_to(default_id, cx);
                }
                Err(e) => {
                    this.busy = false;
                    this.error = Some(failure(&e, cx));
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for RemoveWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let foot = widgets::dialog_foot(&theme)
            .mt(px(space::GUTTER))
            .child(
                widgets::button(
                    "ws-remove-cancel",
                    t(cx, "shell.cancel"),
                    Kind::Plain,
                    Size::Normal,
                    None,
                    false,
                    &theme,
                    |_, w, cx| widgets::close_top(w, cx),
                )
                .track_focus(&self.cancel),
            )
            .child(widgets::button(
                "ws-remove-confirm",
                if self.busy {
                    t(cx, "shell.removing")
                } else {
                    t(cx, "shell.removeConfirm")
                },
                Kind::Danger,
                Size::Normal,
                Some("trash"),
                self.busy,
                &theme,
                cx.listener(|this, _, window, cx| this.confirm(window, cx)),
            ));
        widgets::base(&theme)
            .flex()
            .flex_col()
            .child(widgets::dialog_head(self.title.clone(), &theme))
            .child(
                div()
                    .px(px(space::GUTTER))
                    .pt(px(space::GUTTER))
                    .flex()
                    .flex_col()
                    .child(div().child(tv(
                        cx,
                        "shell.removeWorkspaceBody",
                        &[("default", &self.default_name)],
                    )))
                    .child(
                        div()
                            .mt(px(space::LG))
                            .px(px(space::LG))
                            .py(px(space::MD))
                            .rounded(px(radius::MD))
                            .bg(theme.c("ok-bg"))
                            .text_color(theme.c("ok-ink"))
                            .text_size(px(size::SMALL))
                            .line_height(px(leading::SMALL))
                            .child(t(cx, "shell.removeWorkspaceKept")),
                    )
                    .children(self.error.clone().map(|e| widgets::error(e, &theme))),
            )
            .child(foot)
    }
}

/// Ask before removing the workspace `id` called `name`; its repositories
/// go to the default, `default_id` called `default_name`.
pub fn open_remove(
    id: String,
    name: &str,
    default_id: String,
    default_name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let title = tv(cx, "shell.removeWorkspaceTitle", &[("name", name)]);
    let view = cx.new(|cx| RemoveWorkspace {
        title,
        id,
        default_id,
        default_name,
        busy: false,
        error: None,
        cancel: cx.focus_handle(),
    });
    let cancel = view.read(cx).cancel.clone();
    widgets::open_modal(
        view.into(),
        DIALOG_W,
        None,
        widgets::Place::Top(0.12),
        None,
        window,
        cx,
    );
    window.defer(cx, move |window, cx| cancel.focus(window, cx));
}
