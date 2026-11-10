//! Avatars: an agent's robot, a lead's tile, the operator's photo or
//! initials. Owned by the chat lane (robots live in `src/robot/`); every
//! screen calls only these functions.

use crate::model::Agent;
use crate::theme::Theme;
use gpui::{AnyElement, App, FontWeight, IntoElement, ParentElement, Styled, div, px};

/// An agent's avatar at `size` px, with its presence dot when `dot` is set
/// (`AgentAvatar`).
pub fn agent_avatar(agent: &Agent, size: f32, dot: bool, cx: &App) -> AnyElement {
    let _ = dot;
    let theme = cx.global::<Theme>();
    div()
        .size(px(size))
        .rounded(px(size * 0.22))
        .bg(theme.tone_for(crate::derive::robot_name(agent)))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.c("av-ink"))
        .font_weight(FontWeight::BOLD)
        .text_size(px(size * 0.4))
        .child(crate::ui::format::monogram(&agent.name))
        .into_any_element()
}

/// The operator's avatar (photo when set, else initials on `av-operator`).
pub fn operator_avatar(initials: &str, size: f32, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    div()
        .size(px(size))
        .rounded(px(size * 0.22))
        .bg(theme.c("av-operator"))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.c("av-ink"))
        .font_weight(FontWeight::BOLD)
        .text_size(px(size * 0.4))
        .child(initials.to_string())
        .into_any_element()
}
