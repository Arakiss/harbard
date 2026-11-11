//! "Add a workspace": three steps in one sheet.
//!
//! 1. Name your workspace: the name and, if wanted, the tile's letters,
//!    with the tile drawn as the rail will draw it, live.
//! 2. What belongs here: the origin owners harbard has seen, with how many
//!    repositories each would bring and from where; skippable.
//! 3. All set: the workspace exists, its tile is on the rail; Open goes to it.
//!
//! The workspace is created on the way to step 3, in one request that
//! carries the name, the letters and the owners: a refusal leaves nothing
//! half made. Enter goes forward, Back goes back, Esc closes (after step 2
//! it still goes to the new workspace).

use super::widgets::{self, Kind, Size};
use super::workspace_dialogs::failure;
use crate::api::{self, ApiError};
use crate::derive::workspaces::{
    Move, NAME_MAX, Step, clean_name, move_of, move_summary, name_problem, owner_meta,
    predicted_id, toggle_owner,
};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::workspaces::WorkspacesList;
use crate::shell::emblem::palettes_for;
use crate::shell::rail::tile_preview;
use crate::theme::Theme;
use crate::ui::{self, control, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, px,
};
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::input::{InputEvent, InputState};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

const SHEET_W: f32 = 560.;
/// The tile letters' field.
const LETTERS_W: f32 = 88.;
/// The owners' list scrolls past this.
const OWNERS_MAX_H: f32 = 280.;
/// A step dot.
const DOT: f32 = 8.;

/// What exists once step 2 went through.
#[derive(Clone)]
struct Created {
    id: String,
    name: String,
    moved: Option<Move>,
}

struct AddWorkspace {
    step: Step,
    list: Option<WorkspacesList>,
    name: Entity<InputState>,
    letters: Entity<InputState>,
    picked: Vec<String>,
    /// A Continue was tried: an empty name is now worth saying.
    tried: bool,
    busy: bool,
    error: Option<SharedString>,
    created: Option<Created>,
    /// The new workspace to go to, once: by Open, or by closing the sheet.
    go_to: Rc<RefCell<Option<String>>>,
    /// Holds the keyboard on the steps without a field, so Enter still works.
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl AddWorkspace {
    fn new(
        go_to: Rc<RefCell<Option<String>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let name = cx.new(|cx| InputState::new(window, cx));
        let letters = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_text_align(gpui::TextAlign::Center, cx);
            s
        });
        let cap = |max: usize| {
            move |_: &mut AddWorkspace,
                  s: &Entity<InputState>,
                  ev: &InputEvent,
                  window: &mut Window,
                  cx: &mut Context<AddWorkspace>| {
                if matches!(ev, InputEvent::Change) {
                    let v = s.read(cx).value().to_string();
                    if v.chars().count() > max {
                        let cut: String = v.chars().take(max).collect();
                        s.update(cx, |s, cx| s.set_value(cut, window, cx));
                    }
                    cx.notify();
                }
            }
        };
        let subs = vec![
            cx.subscribe_in(&name, window, cap(NAME_MAX)),
            cx.subscribe_in(&letters, window, cap(2)),
        ];
        let mut this = AddWorkspace {
            step: Step::Name,
            list: None,
            name,
            letters,
            picked: Vec::new(),
            tried: false,
            busy: false,
            error: None,
            created: None,
            go_to,
            focus: cx.focus_handle(),
            _subs: subs,
        };
        this.load(cx);
        this
    }

    /// The hub's workspaces and the owners it knows.
    fn load(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let out = cx
                .background_spawn(async { api::get_as::<WorkspacesList>("/workspaces") })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Ok(list) = out {
                    this.list = Some(list);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn name_now(&self, cx: &App) -> String {
        self.name.read(cx).value().to_string()
    }

    fn letters_now(&self, cx: &App) -> String {
        self.letters.read(cx).value().trim().to_string()
    }

    /// Continue, or Enter.
    fn forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        match self.step {
            Step::Name => {
                self.tried = true;
                if let Some(next) = self.step.next(&self.name_now(cx)) {
                    self.step = next;
                    self.error = None;
                    self.focus.focus(window, cx);
                }
                cx.notify();
            }
            Step::Owners => self.create(window, cx),
            Step::Ready => self.open(window, cx),
        }
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(step) = self.step.back() {
            self.step = step;
            self.error = None;
            if step == Step::Name {
                self.name.update(cx, |s, cx| s.focus(window, cx));
            }
            cx.notify();
        }
    }

    /// One request: the name, the letters and the owners.
    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        self.error = None;
        cx.notify();
        let name = clean_name(&self.name_now(cx));
        let mut body = json!({ "name": name, "owners": self.picked });
        let letters = self.letters_now(cx);
        if !letters.is_empty() {
            body["icon"] = json!(letters);
        }
        let moved = self
            .list
            .as_ref()
            .and_then(|list| move_of(list, &self.picked));
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let out: Result<Value, ApiError> = cx
                .background_spawn(async move { api::post("/workspaces/add", &body) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match out {
                    Ok(v) => {
                        let id = v["workspace"].as_str().unwrap_or_default().to_string();
                        let name = v["name"].as_str().unwrap_or(&name).to_string();
                        *this.go_to.borrow_mut() = (!id.is_empty()).then(|| id.clone());
                        this.created = Some(Created { id, name, moved });
                        this.step = Step::Ready;
                        // The tile appears on the rail behind the sheet.
                        Desk::get(cx).read(cx).hub.refresh();
                        let focus = this.focus.clone();
                        let _ = handle.update(cx, |_, window, cx| focus.focus(window, cx));
                    }
                    Err(e) => this.error = Some(failure(&e, cx)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "Open <name>": go there and close.
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.go_to.borrow_mut().take();
        widgets::close_top(window, cx);
        if let Some(id) = id {
            crate::shell::rail::switch_to(id, cx);
        }
    }

    /// The tile as the rail will draw it: the id serve will give (or gave),
    /// its colours among the others', and the letters typed.
    fn preview(&self, cx: &App) -> AnyElement {
        let theme = cx.global::<Theme>();
        let list = self.list.clone().unwrap_or_default();
        let id = match &self.created {
            Some(c) => c.id.clone(),
            None => predicted_id(&list, &self.name_now(cx)),
        };
        let mut ids: Vec<&str> = list.workspaces.iter().map(|w| w.id.as_str()).collect();
        if !ids.contains(&id.as_str()) {
            ids.push(&id);
        }
        let palette = palettes_for(ids).get(&id).copied();
        let letters = self.letters_now(cx);
        tile_preview(
            &id,
            palette,
            (!letters.is_empty()).then(|| letters.into()),
            theme,
        )
    }

    fn name_step(&self, cx: &App) -> AnyElement {
        let theme = cx.global::<Theme>();
        let problem = name_problem(&self.name_now(cx), self.tried, cx);
        let name_field = widgets::text_input(&self.name, theme)
            .text_size(px(size::BODY))
            .when(problem.is_some(), |i| i.border_color(theme.c("red-line")));
        div()
            .flex()
            .items_start()
            .gap(px(space::SECTION))
            .child(self.preview(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::SM))
                    .child(widgets::label(t(cx, "shell.name"), theme))
                    .child(
                        div()
                            .h(px(control::LG))
                            .child(name_field.h(px(control::LG))),
                    )
                    .child(match problem {
                        Some(p) => widgets::error(p, theme),
                        None => widgets::hint(t(cx, "shell.nwNameHint"), theme),
                    })
                    .child(widgets::label(t(cx, "shell.nwLetters"), theme).mt(px(space::LG)))
                    .child(
                        div()
                            .w(px(LETTERS_W))
                            .font_weight(weight::STRONG)
                            .child(widgets::text_input(&self.letters, theme)),
                    )
                    .child(widgets::hint(t(cx, "shell.railIconHint"), theme)),
            )
            .into_any_element()
    }

    fn owners_step(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let list = self.list.clone().unwrap_or_default();
        let selected = theme.c("selected");
        let hover = theme.c("hover");
        let rows: Vec<AnyElement> = list
            .owners
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let on = self.picked.iter().any(|p| p.eq_ignore_ascii_case(&o.owner));
                let owner = o.owner.clone();
                widgets::focusable(
                    div().id(SharedString::from(format!("nw-owner-{i}"))),
                    &theme,
                    true,
                )
                .flex()
                .flex_none()
                .items_center()
                .gap(px(space::LG))
                .min_h(px(row::TABS))
                .px(px(space::LG))
                .rounded(px(radius::BASE))
                .cursor_pointer()
                .when(on, |d| d.bg(selected))
                .when(!on, |d| d.hover(move |s| s.bg(hover)))
                .child(widgets::mark(widgets::Mark::Check, on, &theme))
                .child(
                    ui::text(o.owner.clone(), size::UI, weight::STRONG, theme.c("ink"))
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                )
                .child(
                    ui::text(
                        owner_meta(&list, o, cx),
                        size::SMALL,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .flex_none(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    toggle_owner(&mut this.picked, &owner);
                    cx.notify();
                }))
                .into_any_element()
            })
            .collect();
        let moved = move_of(&list, &self.picked);
        div()
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .child(ui::text(
                t(cx, "shell.nwOwnersIntro"),
                size::SMALL,
                weight::REGULAR,
                theme.c("ink-2"),
            ))
            .child(if rows.is_empty() {
                widgets::hint(t(cx, "shell.nwNoOwners"), &theme).into_any_element()
            } else {
                div()
                    .id("nw-owners")
                    .max_h(px(OWNERS_MAX_H))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(space::HAIR))
                    .mx(px(-space::LG))
                    .children(rows)
                    .into_any_element()
            })
            .when(!list.owners.is_empty(), |d| {
                d.child(ui::text(
                    move_summary(moved.as_ref(), false, cx),
                    size::UI,
                    weight::REGULAR,
                    theme.c("ink"),
                ))
            })
            .into_any_element()
    }

    fn ready_step(&self, cx: &App) -> AnyElement {
        let theme = cx.global::<Theme>();
        let Some(c) = &self.created else {
            return div().into_any_element();
        };
        div()
            .flex()
            .items_center()
            .gap(px(space::SECTION))
            .child(self.preview(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::SM))
                    .child(
                        ui::text(c.name.clone(), size::TITLE, weight::HEAVY, theme.c("ink"))
                            .truncate(),
                    )
                    .child(ui::text(
                        move_summary(c.moved.as_ref(), true, cx),
                        size::UI,
                        weight::REGULAR,
                        theme.c("ink-2"),
                    )),
            )
            .into_any_element()
    }

    /// Where you are among the three: a dot each, the current one in ink.
    fn dots(&self, cx: &App) -> AnyElement {
        let theme = cx.global::<Theme>();
        let at = Step::ALL.iter().position(|s| *s == self.step).unwrap_or(0);
        let label = tv(
            cx,
            "shell.nwStepOf",
            &[
                ("n", &(at + 1).to_string()),
                ("total", &Step::ALL.len().to_string()),
            ],
        );
        div()
            .id("nw-dots")
            .flex()
            .items_center()
            .gap(px(space::SM))
            .tooltip(widgets::tip(label, None))
            .children(Step::ALL.iter().enumerate().map(|(i, _)| {
                div().size(px(DOT)).rounded_full().bg(if i == at {
                    theme.c("ink")
                } else {
                    theme.c("line-strong")
                })
            }))
            .into_any_element()
    }
}

impl Render for AddWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let body = match self.step {
            Step::Name => self.name_step(cx),
            Step::Owners => self.owners_step(cx),
            Step::Ready => self.ready_step(cx),
        };
        let primary: SharedString = match self.step {
            Step::Name => t(cx, "shell.continue"),
            Step::Owners if self.busy => t(cx, "shell.creating"),
            Step::Owners if self.picked.is_empty() => t(cx, "shell.startEmpty"),
            Step::Owners => t(cx, "shell.createWorkspace"),
            Step::Ready => tv(
                cx,
                "shell.openWorkspace",
                &[(
                    "name",
                    self.created.as_ref().map_or("", |c| c.name.as_str()),
                )],
            ),
        };
        let foot = widgets::dialog_foot(&theme)
            .mt(px(space::GUTTER))
            .child(self.dots(cx))
            .child(div().flex_1())
            .when(self.step.back().is_some(), |d| {
                d.child(widgets::button(
                    "nw-back",
                    t(cx, "shell.back"),
                    Kind::Plain,
                    Size::Normal,
                    None,
                    self.busy,
                    &theme,
                    cx.listener(|this, _, window, cx| this.back(window, cx)),
                ))
            })
            .child(widgets::button(
                "nw-forward",
                primary,
                Kind::Primary,
                Size::Normal,
                None,
                self.busy,
                &theme,
                cx.listener(|this, _, window, cx| this.forward(window, cx)),
            ));
        widgets::base(&theme)
            .track_focus(&self.focus)
            // Enter goes forward, from a field too; the dialog layer would
            // take it as "OK" and close the sheet.
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.forward(window, cx)))
            .flex()
            .flex_col()
            .child(widgets::dialog_head(t(cx, self.step.title_key()), &theme))
            .child(
                div()
                    .px(px(space::GUTTER))
                    .pt(px(space::GUTTER))
                    .flex()
                    .flex_col()
                    .gap(px(space::LG))
                    .child(body)
                    .children(self.error.clone().map(|e| widgets::error(e, &theme))),
            )
            .child(foot)
    }
}

/// Open "Add a workspace" with the caret in the name.
pub fn open(window: &mut Window, cx: &mut App) {
    let go_to: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let view = cx.new(|cx| AddWorkspace::new(go_to.clone(), window, cx));
    let first = view.read(cx).name.clone();
    // Closed with Esc after step 2: the workspace exists, so go to it.
    let on_close: widgets::Handler = Rc::new(move |_, cx| {
        if let Some(id) = go_to.borrow_mut().take() {
            crate::shell::rail::switch_to(id, cx);
        }
    });
    widgets::open_modal(
        view.into(),
        SHEET_W,
        None,
        widgets::Place::Top(0.12),
        Some(on_close),
        window,
        cx,
    );
    widgets::autofocus(first, window, cx);
}
