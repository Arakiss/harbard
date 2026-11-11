//! Rooms (`rooms.tsx`): a room's screen, its feed with older pages, the
//! channel menu and its actions, the archived strip, the create and delete
//! dialogs, and the `#feedback` inbox.

pub mod dialogs;
pub mod feed;
pub mod inbox;
pub mod view;

pub use feed::RoomFeed;
#[allow(unused_imports)]
pub use view::RoomView;

use crate::chat::row_menu::RowAction;
use crate::derive::rooms::{self as rules, ChannelAction};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::Channel;
use crate::theme::Theme;
use crate::ui::{self, control, icon_size, leading, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled, Task, Window, div, px,
};

/// A channel menu's least width.
const MENU_WIDTH: f32 = 220.;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use serde_json::{Value, json};

/// `POST /api/v1/channels {action, name}` (+ the current workspace), then
/// a fresh read. Resolves to serve's answer.
pub fn post_channel_action(
    mut body: Value,
    cx: &mut App,
) -> Task<Result<Value, crate::api::ApiError>> {
    if let (Some(ws), Some(obj)) = (feed::current_workspace(cx), body.as_object_mut()) {
        obj.insert("workspace".into(), json!(ws));
    }
    Desk::get(cx).update(cx, |d, cx| d.post_to("/channels", body, cx))
}

/// serve's refusal in the operator's words (`reasonOf`).
pub fn refusal(error: &crate::api::ApiError, cx: &App) -> SharedString {
    rules::channel_refusal(error.code.as_deref(), &error.message, cx)
}

/// Archive or unarchive at once; delete asks first (`useChannelActions().run`).
/// A refusal shows "That did not work: {reason}" until dismissed.
pub fn run_channel_action(action: ChannelAction, name: &str, window: &mut Window, cx: &mut App) {
    window.remove_notification::<ChannelToast>(cx);
    if action == ChannelAction::Delete {
        dialogs::open_delete_room(name.to_string(), window, cx);
        return;
    }
    let task = post_channel_action(json!({ "action": action.as_str(), "name": name }), cx);
    let handle = window.window_handle();
    cx.spawn(async move |cx| {
        if let Err(e) = task.await {
            let _ = handle.update(cx, |_, window, cx| {
                let reason = refusal(&e, cx);
                let text = rules::action_failed(&reason, cx);
                show_toast(text, window, cx);
            });
        }
    })
    .detach();
}

/// Marks the channel-action toast, so a new one replaces the old.
pub struct ChannelToast;

/// `div.hb-banner.hb-room-toast[role=alert]`: the refusal and Dismiss, at
/// the bottom right until dismissed.
pub fn show_toast(text: SharedString, window: &mut Window, cx: &mut App) {
    let theme = cx.global::<Theme>().clone();
    let dismiss = t(cx, "rooms.dismiss");
    let note = Notification::new()
        .id::<ChannelToast>()
        .autohide(false)
        .placement(gpui::Anchor::BottomRight)
        .bg(theme.c("red-bg"))
        .border_0()
        .rounded(px(radius::MD))
        .px(px(space::LG))
        .py(px(space::MD))
        .content(move |_, _, cx| {
            let theme = cx.global::<Theme>().clone();
            div()
                .flex()
                .items_center()
                .gap(px(space::LG))
                .font_family(ui::SANS)
                .text_size(px(size::UI))
                .text_color(theme.c("banner-ink"))
                .child(div().flex_1().child(text.clone()))
                .child(
                    feed::hist_more("hb-room-toast-dismiss", dismiss.clone(), false, &theme)
                        .on_click(cx.listener(|note, _, window, cx| note.dismiss(window, cx))),
                )
                .into_any_element()
        });
    window.push_notification(note, cx);
}

/// The actions a channel's ⋯ menu offers (`useChannelActions().itemsFor`):
/// archive / unarchive run at once, delete asks first. Built-in rooms: none.
pub fn channel_actions(channel: &Channel, cx: &App) -> Vec<RowAction> {
    rules::channel_actions(channel)
        .into_iter()
        .map(|action| {
            let name = channel.id.clone();
            RowAction::new(action.label(cx), move |window, cx| {
                run_channel_action(action, &name, window, cx)
            })
        })
        .collect()
}

/// The header's ⋯ button with the same actions (`ChannelMenu`); `None` for
/// built-in rooms.
pub fn channel_menu(channel: &Channel, cx: &App) -> Option<AnyElement> {
    let actions = channel_actions(channel, cx);
    if actions.is_empty() {
        return None;
    }
    let theme = cx.global::<Theme>();
    let label = tv(cx, "rooms.channelMenu", &[("channel", &channel.id)]);
    let actions = std::rc::Rc::new(actions);
    let icon = gpui_kit::component::Icon::default()
        .path("icons/ellipsis.svg")
        .size(px(icon_size::ROW))
        .text_color(theme.c("ink-3"));
    Some(
        Button::new(ElementId::Name(
            format!("hb-room-menu-{}", channel.id).into(),
        ))
        .ghost()
        .icon(icon)
        .accessibility_label(label)
        .size(px(control::SM))
        .p(px(space::SM))
        .rounded(px(radius::BASE))
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |mut menu, _, _| {
            menu = menu.min_w(px(MENU_WIDTH));
            for item in actions.iter() {
                let run = item.run.clone();
                menu = menu.item(
                    PopupMenuItem::new(item.label.clone())
                        .on_click(move |_, window, cx| run(window, cx)),
                );
            }
            menu
        })
        .into_any_element(),
    )
}

/// `div.hb-room-strip`: "#{channel} is archived." with Unarchive.
pub fn archived_strip(channel: &Channel, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let (lead, body) = rules::archived_strip(channel, cx);
    let name = channel.id.clone();
    // The same quiet row as a channel's alerts, with its one action.
    div()
        .flex_none()
        .px(px(space::GUTTER))
        .pt(px(space::MD))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(space::LG))
                .min_h(px(control::MD + space::MD))
                .pl(px(space::LG))
                .pr(px(space::XS))
                .rounded(px(radius::MD))
                .bg(theme.c("warn-bg"))
                .text_color(theme.c("warn-ink"))
                .font_family(ui::SANS)
                .text_size(px(size::SMALL))
                .line_height(px(leading::SMALL))
                .child(div().flex_1().min_w_0().child(
                    gpui::StyledText::new(format!("{lead} {body}")).with_highlights([(
                        0..lead.len(),
                        gpui::HighlightStyle {
                            font_weight: Some(weight::STRONG),
                            ..Default::default()
                        },
                    )]),
                ))
                .child(
                    button(
                        ElementId::Name(format!("hb-room-unarchive-{name}").into()),
                        ChannelAction::Unarchive.label(cx),
                        Tone::Plain,
                        false,
                        theme,
                    )
                    .on_click(move |_, window, cx| {
                        run_channel_action(ChannelAction::Unarchive, &name, window, cx)
                    }),
                ),
        )
        .into_any_element()
}

/// Open "Create a room" (from the sidebar's +).
pub fn open_create_room(window: &mut Window, cx: &mut App) {
    dialogs::open_create_room(window, cx);
}

/// `.hb-btn`, `.hb-btn-primary`, `.hb-btn-danger`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Primary,
    Danger,
}

/// A button: 32 px, 14/700, an outline; `disabled` at half opacity.
pub fn button(
    id: impl Into<ElementId>,
    label: SharedString,
    tone: Tone,
    disabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    button_shell(id, tone, disabled, theme).child(label)
}

/// [`button`] without its label, for an icon before the words.
pub fn button_shell(
    id: impl Into<ElementId>,
    tone: Tone,
    disabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let (bg, edge, ink, hover) = match tone {
        Tone::Plain => (
            theme.c("surface"),
            theme.c("line-strong"),
            theme.c("ink"),
            theme.c("btn-hover"),
        ),
        Tone::Primary => (
            theme.c("accent-deep"),
            theme.c("accent-deep"),
            theme.c("accent-ink"),
            theme.c("accent-hover"),
        ),
        Tone::Danger => (
            theme.c("surface"),
            theme.c("red-line"),
            theme.c("red-ink"),
            theme.c("red-bg"),
        ),
    };
    let ring = ui::focus_ring(theme, false);
    div()
        .id(id.into())
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::SM))
        .h(px(control::MD))
        .px(px(space::LG))
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(edge)
        .bg(bg)
        .text_color(ink)
        .font_family(ui::SANS)
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .font_weight(weight::STRONG)
        .when(disabled, |d| d.opacity(0.5))
        .when(!disabled, |d| {
            d.cursor_pointer()
                .hover(move |s| s.bg(hover))
                .tab_index(0)
                .focus_visible(move |s| s.shadow(vec![ring]))
        })
}
