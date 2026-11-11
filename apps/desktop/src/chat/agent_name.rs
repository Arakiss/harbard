//! An agent's name with its role line (`AgentName`), renamed with a double
//! click, and the two votes on a workshop name (`NameVotes`), from `parts.tsx`.

use crate::derive::workspace::agent_role_line;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::Agent;
use crate::theme::Theme;
use crate::ui::{self, icon_size, leading, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, ElementId, Entity, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
// `FontWeight` stays in the builder's signature; call sites pass `weight::`.
use gpui_kit::component::input::{Input, InputEvent, InputState};
use serde_json::json;

/// How the name sits where it is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum NameLayout {
    /// Name over its role line (`.hb-agent-name`).
    #[default]
    Stacked,
    /// Name and role line on one baseline (the sidebar).
    Inline,
}

/// The name element. Build with [`agent_name`].
#[derive(IntoElement)]
pub struct AgentName {
    agent: Agent,
    renamable: bool,
    layout: NameLayout,
    size: f32,
    weight: FontWeight,
    color: Option<Hsla>,
    role_color: Option<Hsla>,
}

/// `AgentName`: `renamable` allows a double click to rename when the agent
/// has a workshop name (`name_source`).
pub fn agent_name(agent: &Agent, renamable: bool) -> AgentName {
    AgentName {
        agent: agent.clone(),
        renamable,
        layout: NameLayout::Stacked,
        size: size::BODY,
        weight: weight::HEAVY,
        color: None,
        role_color: None,
    }
}

impl AgentName {
    pub fn layout(mut self, layout: NameLayout) -> Self {
        self.layout = layout;
        self
    }

    pub fn text_size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn role_color(mut self, color: Hsla) -> Self {
        self.role_color = Some(color);
        self
    }
}

/// Per-name state: the rename box while it is open, and the last refusal.
#[derive(Default)]
struct Rename {
    input: Option<Entity<InputState>>,
    error: Option<SharedString>,
    _sub: Option<Subscription>,
}

impl Rename {
    fn open(&mut self, agent: &Agent, window: &mut Window, cx: &mut Context<Self>) {
        let name = agent.name.clone();
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(name.clone(), window, cx);
            s
        });
        let key = agent.key.clone();
        let sub = cx.subscribe_in(
            &input,
            window,
            move |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    let draft = input.read(cx).value().trim().to_string();
                    this.close(cx);
                    if draft.is_empty() || draft == name {
                        return;
                    }
                    let task = Desk::get(cx).update(cx, |d, cx| {
                        d.post("rename-agent", json!({ "agent": key, "name": draft }), cx)
                    });
                    cx.spawn_in(window, async move |this, cx| {
                        let out = task.await;
                        let _ = this.update(cx, |this, cx| {
                            this.error = match out {
                                Ok(_) => None,
                                Err(e) if e.status == Some(409) => {
                                    Some(t(cx, "messages.nameTaken"))
                                }
                                Err(_) => Some(t(cx, "messages.nameSaveFailed")),
                            };
                            cx.notify();
                        });
                    })
                    .detach();
                }
                _ => {}
            },
        );
        input.update(cx, |s, cx| s.focus(window, cx));
        self.input = Some(input);
        self._sub = Some(sub);
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.input = None;
        self._sub = None;
        cx.notify();
    }
}

impl RenderOnce for AgentName {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let can_rename = self.renamable && self.agent.name_source.is_some();
        let state = window.use_keyed_state(
            ElementId::Name(format!("hb-agent-name-{}", self.agent.key).into()),
            cx,
            |_, _| Rename::default(),
        );
        let shown = crate::derive::agent_name(&self.agent, cx);
        let role = self
            .agent
            .role_line
            .as_ref()
            .map(|_| agent_role_line(&self.agent, cx));
        let editing = state.read(cx).input.clone();
        let error = state.read(cx).error.clone();
        let color = self.color.unwrap_or(theme.c("ink"));
        let role_color = self.role_color.unwrap_or(theme.c("ink-3"));
        let inline = self.layout == NameLayout::Inline;

        let name: AnyElement = match editing {
            Some(input) => div()
                .px(px(space::XS))
                .rounded(px(radius::SM))
                .border_1()
                .border_color(theme.c("focus-line"))
                .bg(theme.c("surface"))
                .text_size(px(self.size))
                .on_key_down(|ev, _, cx| {
                    // Esc leaves the name as it was and must not mark anything read.
                    if ev.keystroke.key == "escape" {
                        cx.stop_propagation();
                    }
                })
                .child(Input::new(&input).appearance(false))
                .on_action({
                    let state = state.clone();
                    move |_: &gpui_kit::component::input::Escape, _, cx| {
                        state.update(cx, |s, cx| s.close(cx));
                    }
                })
                .into_any_element(),
            None => {
                let agent = self.agent.clone();
                let state = state.clone();
                div()
                    .id(ElementId::Name(
                        format!("hb-name-{}", self.agent.key).into(),
                    ))
                    .min_w_0()
                    .truncate()
                    // Inline: the name keeps its width and the role line takes what is left.
                    .when(inline, |d| d.flex_initial())
                    .text_size(px(self.size))
                    .line_height(px(leading::for_size(self.size)))
                    .font_weight(self.weight)
                    .text_color(color)
                    .child(shown.clone())
                    .when(can_rename, |d| {
                        d.tooltip(move |window, cx| {
                            crate::ui::kit_theme::tip(t(cx, "messages.doubleClickRename"))(
                                window, cx,
                            )
                        })
                        .on_click(move |ev: &ClickEvent, window, cx| {
                            if ev.click_count() == 2 {
                                state.update(cx, |s, cx| s.open(&agent, window, cx));
                            }
                        })
                    })
                    .into_any_element()
            }
        };
        div()
            .flex()
            .min_w_0()
            .when(inline, |d| d.flex_row().items_baseline().gap(px(space::SM)))
            .when(!inline, |d| d.flex_col())
            .child(name)
            .children(role.map(|r| {
                ui::text(r, size::META, weight::REGULAR, role_color)
                    .when(inline, |d| d.flex_1())
                    .min_w_0()
                    .truncate()
            }))
            .children(error.map(|e| ui::text(e, size::SMALL, weight::REGULAR, theme.c("red-ink"))))
    }
}

/// The two votes on a workshop name (`NameVotes`); nothing without a
/// workshop name.
pub fn name_votes(agent: &Agent) -> Option<AnyElement> {
    agent.name_source.as_ref()?;
    Some(
        Votes {
            key: agent.key.clone(),
            name: agent.name.clone(),
        }
        .into_any_element(),
    )
}

#[derive(IntoElement)]
struct Votes {
    key: String,
    name: String,
}

/// The vote shown pressed: optimistic, cleared when serve refuses.
#[derive(Default)]
struct Voted(Option<&'static str>);

impl RenderOnce for Votes {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let state = window.use_keyed_state(
            ElementId::Name(format!("hb-votes-{}", self.key).into()),
            cx,
            |_, _| Voted::default(),
        );
        let done = state.read(cx).0;
        // Lucide marks, not emoji; the vote given sits on the control wash.
        let button = |vote: &'static str, icon: &'static str, title: SharedString| {
            let key = self.key.clone();
            let state = state.clone();
            let pressed = done == Some(vote);
            super::focusable(
                div()
                    .id(ElementId::Name(
                        format!("hb-vote-{vote}-{}", self.key).into(),
                    ))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(leading::BODY))
                    .rounded(px(radius::SM))
                    .cursor_pointer()
                    .when(pressed, |d| d.bg(theme.c("control")))
                    .hover(|s| s.bg(theme.c("control")))
                    .tooltip(super::tip(title)),
                false,
                &theme,
            )
            .child(ui::icon(
                icon,
                icon_size::INLINE,
                if pressed {
                    theme.c("ink")
                } else {
                    theme.c("ink-3")
                },
            ))
            .on_click(move |_, _, cx| {
                state.update(cx, |s, cx| {
                    s.0 = Some(vote);
                    cx.notify();
                });
                let task = Desk::get(cx).update(cx, |d, cx| {
                    d.post("name-vote", json!({ "agent": key, "vote": vote }), cx)
                });
                let state = state.clone();
                cx.spawn(async move |cx| {
                    if task.await.is_err() {
                        state.update(cx, |s, cx| {
                            s.0 = None;
                            cx.notify();
                        });
                    }
                })
                .detach();
            })
        };
        let _ = tv(cx, "messages.likeName", &[("name", &self.name)]);
        div()
            .flex()
            .gap(px(space::HAIR))
            .ml(px(space::SM))
            .child(button("up", "thumbs-up", t(cx, "messages.nameUp")))
            .child(button("down", "thumbs-down", t(cx, "messages.nameDown")))
    }
}
