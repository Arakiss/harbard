//! "Remove <workspace>?" (from its settings): what goes, where its
//! repositories go, and that nothing is deleted; on success the app goes
//! back to the default. Adding one is `workspace_add`.

use super::widgets::{self, Kind, Size};
use crate::api::{self, ApiError};
use crate::derive::workspaces::refusal;
use crate::i18n::{t, tv};
use crate::theme::Theme;
use crate::ui::{leading, radius, size, space};
use gpui::{
    App, AppContext, Context, FocusHandle, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div, px,
};
use serde_json::{Value, json};

/// Like the workspace settings it sits beside.
const DIALOG_W: f32 = 480.;

/// What a write's failure says: serve's refusal worded, or that it could
/// not be reached.
pub(super) fn failure(e: &ApiError, cx: &App) -> SharedString {
    if e.status.is_none() {
        return t(cx, "shell.serveNotResponding");
    }
    refusal(e.code.as_deref(), &e.message, cx)
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
