//! "Add a workspace": three steps in one sheet, an onboarding rather than
//! a form.
//!
//! ```text
//! ┌───────────────────────────────────────────────── [x] ┐
//! │  hero: the workspace's own colour, a few robots       │  216
//! │        gathered round its tile, live as you type      │
//! ├───────────────────────────────────────────────────────┤
//! │  Step 1 of 3                                          │
//! │  Name your workspace            (display type)        │
//! │  [ name ………………………… ]  [AB]                           │
//! ├───────────────────────────────────────────────────────┤
//! │  ━ • •                               Back  Continue   │
//! └───────────────────────────────────────────────────────┘
//! ```
//!
//! 1. Name your workspace: the name and, if wanted, the tile's letters,
//!    with the tile drawn large in the hero, live.
//! 2. What belongs here: the origin owners harbard has seen, each on its
//!    initials tile with its repositories as pills; skippable.
//! 3. All set: the workspace exists; in the hero its tile drops onto a
//!    small rail and the robots hop. Open goes to it.
//!
//! The workspace is created on the way to step 3, in one request that
//! carries the name, the letters and the owners: a refusal leaves nothing
//! half made. Enter goes forward, Back goes back, Esc closes (after step 2
//! it still goes to the new workspace). Each step slides in over 180 ms
//! and the robots arrive over 380 ms; with reduced motion (M7) nothing
//! moves.

use super::widgets::{self, Kind, Size};
use super::workspace_dialogs::failure;
use crate::api::{self, ApiError};
use crate::derive::workspace::Words;
use crate::derive::workspaces::{
    Move, NAME_MAX, Step, clean_name, join_names, move_of, move_summary, name_problem,
    predicted_id, toggle_owner,
};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::workspaces::{KnownOwner, WorkspacesList};
use crate::robot::draw::Ease;
use crate::shell::emblem::{emblem, emblem_of, palettes_for};
use crate::theme::Theme;
use crate::ui::{self, control, icon_size, leading, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext, Context, ElementId, Entity,
    FocusHandle, Hsla, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};
use gpui_kit::base::actions::Confirm;
use gpui_kit::component::input::{InputEvent, InputState};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const SHEET_W: f32 = 680.;
/// The hero band, the big tile in it and its corner (22 % of its side).
const HERO_H: f32 = 216.;
const TILE_BIG: f32 = 104.;
const TILE_BIG_R: f32 = 23.;
/// The letters on the big tile.
const TILE_LETTERS: f32 = 40.;
/// The robots: the two beside the tile, the two further out.
const ROBOT_NEAR: f32 = 84.;
const ROBOT_FAR: f32 = 64.;
/// Where the robots stand: this far above the band's foot, on a shelf this
/// thick and wide; the tile is raised a little above them.
const SHELF: f32 = 26.;
const SHELF_H: f32 = 6.;
const SHELF_W: f32 = 520.;
const TILE_LIFT: f32 = 18.;
/// The shelf: the workspace's ground, a little stronger than the band.
const SHELF_WASH: f32 = 0.45;
/// The small rail on the last step: its width, a tile, the new tile's bar.
const MINI_RAIL_W: f32 = 78.;
const MINI_TILE: f32 = control::LG;
const MINI_BAR: f32 = 20.;
/// How many of the other workspaces the small rail shows above the new one.
const MINI_OTHERS: usize = 2;
/// The step's title (display face) and the inline padding of the body.
const STEP_TITLE: f32 = 28.;
const STEP_LEADING: f32 = 34.;
const BODY_PAD: f32 = 32.;
/// The tile letters' field.
const LETTERS_W: f32 = 72.;
/// An owner's initials tile and its row; the list scrolls past this.
const OWNER_TILE: f32 = 36.;
const OWNER_ROW: f32 = 56.;
const OWNERS_MAX_H: f32 = 248.;
/// A pill on an owner row, and the round check at its end.
const PILL_H: f32 = 20.;
const CHECK: f32 = 22.;
/// A step dot, and the current step's longer one.
const DOT: f32 = 8.;
const DOT_ON: f32 = 22.;
/// The hero band: the workspace's ground colour over the sheet, this strong.
const BAND_WASH: f32 = 0.22;

/// Motion (§3.1: the console's durations).
const STEP_IN: Duration = Duration::from_millis(180);
const STEP_TRAVEL: f32 = 16.;
const ARRIVAL: Duration = Duration::from_millis(380);
const STAGGER_MS: f32 = 70.;
const ARRIVAL_RISE: f32 = 14.;
const HOP: f32 = 10.;
const DROP: Duration = Duration::from_millis(560);
const DROP_FROM: f32 = 150.;
/// `cubic-bezier(0.34, 1.56, 0.64, 1)`: lands with a small bounce.
const LAND: Ease = Ease(0.34, 1.56, 0.64, 1.0);

/// Robots for the hero when the fleet has fewer distinct ones.
const CAST_SPARE: [&str; 8] = [
    "Ada", "Bruno", "Cleo", "Dario", "Elsa", "Fermin", "Gala", "Hugo",
];
/// The hero's cast.
const CAST: usize = 4;

/// What exists once step 2 went through.
#[derive(Clone)]
struct Created {
    id: String,
    name: String,
    moved: Option<Move>,
}

struct AddWorkspace {
    step: Step,
    /// The last move went back: the step slides in from the left.
    went_back: bool,
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
    go_to: Rc<RefCell<Option<(String, String)>>>,
    /// Holds the keyboard on the steps without a field, so Enter still works.
    focus: FocusHandle,
    /// The robots in the hero, picked once.
    cast: Vec<String>,
    _subs: Vec<Subscription>,
}

impl AddWorkspace {
    fn new(
        go_to: Rc<RefCell<Option<(String, String)>>>,
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
        let fleet: Vec<String> = Desk::get(cx)
            .read(cx)
            .view
            .as_ref()
            .map(|v| {
                v.agents
                    .iter()
                    .filter(|a| !a.temporary)
                    .map(|a| crate::derive::robot_name(a).to_string())
                    .collect()
            })
            .unwrap_or_default();
        let mut this = AddWorkspace {
            step: Step::Name,
            went_back: false,
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
            cast: hero_cast(&fleet),
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
                    self.went_back = false;
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
            self.went_back = true;
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
                        *this.go_to.borrow_mut() =
                            (!id.is_empty()).then(|| (id.clone(), name.clone()));
                        this.created = Some(Created { id, name, moved });
                        this.step = Step::Ready;
                        this.went_back = false;
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
        let to = self.go_to.borrow_mut().take();
        widgets::close_top(window, cx);
        if let Some((id, name)) = to {
            go_to_new(id, name, cx);
        }
    }

    /// The new workspace's id: the one serve gave, or the one it will give.
    fn tile_id(&self, cx: &App) -> String {
        match &self.created {
            Some(c) => c.id.clone(),
            None => {
                let list = self.list.clone().unwrap_or_default();
                predicted_id(&list, &self.name_now(cx))
            }
        }
    }

    /// Its colours among the others', as the rail will pick them.
    fn palette(&self, id: &str) -> Option<usize> {
        let list = self.list.clone().unwrap_or_default();
        let mut ids: Vec<&str> = list.workspaces.iter().map(|w| w.id.as_str()).collect();
        if !ids.contains(&id) {
            ids.push(id);
        }
        palettes_for(ids).get(id).copied()
    }

    /// The letters on the tile: the ones typed, else none (the emblem alone,
    /// as the rail draws it).
    fn letters_on_tile(&self, cx: &App) -> Option<SharedString> {
        let l = self.letters_now(cx);
        (!l.is_empty()).then(|| l.into())
    }

    /// The tile at `side`, with the letters typed over it.
    fn tile(&self, side: f32, corner: f32, letters_size: f32, cx: &App) -> gpui::Div {
        let theme = cx.global::<Theme>();
        let id = self.tile_id(cx);
        let palette = self.palette(&id);
        div()
            .relative()
            .flex_none()
            .size(px(side))
            .child(emblem(&id, palette, side, corner))
            .when_some(self.letters_on_tile(cx), |d, l| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(ui::text(
                            l,
                            letters_size,
                            weight::HEAVY,
                            theme.c("rail-ink"),
                        )),
                )
            })
    }

    /// The workspace's ground colour, at `wash`: the band and its shelf.
    fn ground(&self, wash: f32, cx: &App) -> Hsla {
        let id = self.tile_id(cx);
        let ground = emblem_of(&id, self.palette(&id)).palette.ground;
        Hsla::from(gpui::rgb(ground)).opacity(wash)
    }

    /// The shelf the robots stand on, centred at the band's foot.
    fn shelf(&self, cx: &App) -> AnyElement {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(SHELF - SHELF_H / 2.))
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(SHELF_W))
                    .h(px(SHELF_H))
                    .rounded_full()
                    .bg(self.ground(SHELF_WASH, cx)),
            )
            .into_any_element()
    }

    /// A robot of the cast, arriving (or hopping, on the last step) after
    /// `i` others; still with reduced motion.
    fn cast_member(&self, i: usize, side: f32, still: bool) -> AnyElement {
        let name = self.cast.get(i).cloned().unwrap_or_default();
        let bot = div()
            .relative()
            .flex_none()
            .child(crate::robot::robot(name, side));
        if still {
            return bot.into_any_element();
        }
        let delay = STAGGER_MS * i as f32;
        let total = ARRIVAL.as_secs_f32() * 1000. + delay;
        let step = self.step;
        let key = step_index(self.step) as u64 * 10 + i as u64;
        bot.with_animation(
            ElementId::NamedInteger("nw-cast".into(), key),
            Animation::new(Duration::from_millis(total as u64)),
            move |d, t| {
                let local = ((t * total - delay) / (total - delay)).clamp(0., 1.);
                match step {
                    // First sight: they arrive.
                    Step::Name => {
                        let e = gpui::ease_out_quint()(local);
                        d.top(px(ARRIVAL_RISE * (1. - e))).opacity(e)
                    }
                    // On to the next step: a hop; done: two, a wave.
                    _ => d.top(px(hop_at(step, local))),
                }
            },
        )
        .into_any_element()
    }

    /// Steps 1 and 2: the robots gathered round the big tile.
    fn gathering(&self, still: bool, cx: &App) -> AnyElement {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(SHELF))
            .flex()
            .items_end()
            .justify_center()
            .gap(px(space::LG))
            .child(self.cast_member(0, ROBOT_FAR, still))
            .child(self.cast_member(1, ROBOT_NEAR, still))
            .child(div().mx(px(space::MD)).mb(px(TILE_LIFT)).child(self.tile(
                TILE_BIG,
                TILE_BIG_R,
                TILE_LETTERS,
                cx,
            )))
            .child(self.cast_member(2, ROBOT_NEAR, still))
            .child(self.cast_member(3, ROBOT_FAR, still))
            .into_any_element()
    }

    /// Step 3: a small rail with the other workspaces, the new tile
    /// dropping into its place, the robots hopping either side.
    fn arrival(&self, still: bool, cx: &App) -> AnyElement {
        let theme = cx.global::<Theme>();
        let list = self.list.clone().unwrap_or_default();
        let id = self.tile_id(cx);
        let others: Vec<String> = list
            .workspaces
            .iter()
            .filter(|w| w.id != id)
            .map(|w| w.id.clone())
            .collect();
        let all: Vec<&str> = others
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(id.as_str()))
            .collect();
        let palettes = palettes_for(all);
        let rail_ink = theme.c("rail-ink");
        let new_tile = div()
            .relative()
            .child(self.tile(MINI_TILE, radius::MD, size::UI, cx))
            .child(
                div()
                    .absolute()
                    .top(px(-space::XS))
                    .left(px(-space::XS))
                    .right(px(-space::XS))
                    .bottom(px(-space::XS))
                    .rounded(px(radius::LG))
                    .border_1()
                    .border_color(rail_ink),
            )
            .child(
                div()
                    .absolute()
                    .left(px(-(MINI_RAIL_W - MINI_TILE) / 2.))
                    .top(px((MINI_TILE - MINI_BAR) / 2.))
                    .w(px(space::XS))
                    .h(px(MINI_BAR))
                    .rounded_r(px(radius::SM))
                    .bg(rail_ink),
            );
        let new_tile = if still {
            new_tile.into_any_element()
        } else {
            new_tile
                .with_animation(
                    "nw-drop",
                    Animation::new(DROP).with_easing(|t| LAND.at(t)),
                    |d, t| d.top(px(-DROP_FROM * (1. - t))),
                )
                .into_any_element()
        };
        let skip = others.len().saturating_sub(MINI_OTHERS);
        let rail = div()
            .flex_none()
            .w(px(MINI_RAIL_W))
            .pt(px(space::LG))
            .pb(px(space::XL))
            .rounded_t(px(radius::LG))
            .bg(theme.c("frame"))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(space::LG))
            .children(others.iter().skip(skip).map(|o| {
                div()
                    .opacity(0.6)
                    .child(emblem(o, palettes.get(o).copied(), MINI_TILE, radius::MD))
            }))
            .child(new_tile);
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .items_end()
            .justify_center()
            .gap(px(space::LG))
            .child(
                div()
                    .pb(px(SHELF))
                    .child(self.cast_member(0, ROBOT_FAR, still)),
            )
            .child(
                div()
                    .pb(px(SHELF))
                    .child(self.cast_member(1, ROBOT_NEAR, still)),
            )
            .child(div().mx(px(space::XL)).child(rail))
            .child(
                div()
                    .pb(px(SHELF))
                    .child(self.cast_member(2, ROBOT_NEAR, still)),
            )
            .child(
                div()
                    .pb(px(SHELF))
                    .child(self.cast_member(3, ROBOT_FAR, still)),
            )
            .into_any_element()
    }

    fn hero(&self, still: bool, cx: &App) -> AnyElement {
        let theme = cx.global::<Theme>();
        let scene = if self.step == Step::Ready && self.created.is_some() {
            self.arrival(still, cx)
        } else {
            self.gathering(still, cx)
        };
        div()
            .relative()
            .flex_none()
            .w_full()
            .h(px(HERO_H))
            .overflow_hidden()
            .bg(self.ground(BAND_WASH, cx))
            .border_b_1()
            .border_color(theme.c("line"))
            .child(self.shelf(cx))
            .child(scene)
            .child(
                div()
                    .absolute()
                    .top(px(space::LG))
                    .right(px(space::LG))
                    .child(widgets::close_button("nw-close", theme, |_, w, cx| {
                        widgets::close_top(w, cx)
                    })),
            )
            .into_any_element()
    }

    fn name_step(&self, cx: &App) -> AnyElement {
        let theme = cx.global::<Theme>();
        let problem = name_problem(&self.name_now(cx), self.tried, cx);
        let name_field = widgets::text_input(&self.name, theme)
            .text_size(px(size::BODY))
            .when(problem.is_some(), |i| i.border_color(theme.c("red-line")));
        div()
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
            .child(
                div()
                    .mt(px(space::LG))
                    .flex()
                    .items_center()
                    .gap(px(space::LG))
                    .child(
                        div()
                            .flex_none()
                            .w(px(LETTERS_W))
                            .font_weight(weight::STRONG)
                            .child(widgets::text_input(&self.letters, theme)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(widgets::label(t(cx, "shell.nwLetters"), theme))
                            .child(ui::text(
                                t(cx, "shell.railIconHint"),
                                size::SMALL,
                                weight::REGULAR,
                                theme.c("ink-3"),
                            )),
                    ),
            )
            .into_any_element()
    }

    fn owner_row(
        &self,
        i: usize,
        list: &WorkspacesList,
        o: &KnownOwner,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on = self.picked.iter().any(|p| p.eq_ignore_ascii_case(&o.owner));
        let owner = o.owner.clone();
        let hover = theme.c("hover");
        let pill = |text: SharedString, bg: Hsla, ink: Hsla| {
            div()
                .flex_none()
                .h(px(PILL_H))
                .px(px(space::MD))
                .flex()
                .items_center()
                .rounded_full()
                .bg(bg)
                .child(ui::text(text, size::META, weight::STRONG, ink).whitespace_nowrap())
        };
        let check = div()
            .flex_none()
            .size(px(CHECK))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .map(|d| {
                if on {
                    d.bg(theme.c("accent")).child(ui::icon(
                        "check",
                        icon_size::INLINE,
                        theme.c("accent-ink"),
                    ))
                } else {
                    d.border_1().border_color(theme.c("line-hover"))
                }
            });
        widgets::focusable(
            div().id(SharedString::from(format!("nw-owner-{i}"))),
            theme,
            true,
        )
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::LG))
        .h(px(OWNER_ROW))
        .px(px(space::LG))
        .rounded(px(radius::MD))
        .cursor_pointer()
        .when(on, |d| d.bg(theme.c("selected")))
        .when(!on, |d| d.hover(move |s| s.bg(hover)))
        .child(ui::avatar::monogram_avatar(
            &o.owner, None, None, OWNER_TILE, None, cx,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(space::HAIR))
                .child(
                    ui::text(o.owner.clone(), size::BODY, weight::STRONG, theme.c("ink"))
                        .truncate(),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(space::XS))
                        .child(pill(
                            owner_repos(o, cx),
                            theme.c("accent-soft"),
                            theme.c("accent-text"),
                        ))
                        .children(
                            owner_where(list, o, cx)
                                .map(|w| pill(w, theme.c("chip"), theme.c("chip-ink"))),
                        ),
                ),
        )
        .child(check)
        .on_click(cx.listener(move |this, _, _, cx| {
            toggle_owner(&mut this.picked, &owner);
            cx.notify();
        }))
        .into_any_element()
    }

    fn owners_step(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.global::<Theme>().clone();
        let list = self.list.clone().unwrap_or_default();
        let rows: Vec<AnyElement> = list
            .owners
            .iter()
            .enumerate()
            .map(|(i, o)| self.owner_row(i, &list, o, &theme, cx))
            .collect();
        let moved = move_of(&list, &self.picked);
        div()
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .child(ui::text(
                t(cx, "shell.nwOwnersIntro"),
                size::UI,
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
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::SM))
                        .child(ui::icon(
                            if moved.is_some() {
                                "folder-input"
                            } else {
                                "sparkles"
                            },
                            icon_size::ROW,
                            theme.c("ink-3"),
                        ))
                        .child(ui::text(
                            move_summary(moved.as_ref(), false, cx),
                            size::UI,
                            weight::REGULAR,
                            theme.c("ink"),
                        )),
                )
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
            .flex_col()
            .gap(px(space::XS))
            .child(ui::text(c.name.clone(), size::TITLE, weight::HEAVY, theme.c("ink")).truncate())
            .child(ui::text(
                move_summary(c.moved.as_ref(), true, cx),
                size::UI,
                weight::REGULAR,
                theme.c("ink-2"),
            ))
            .into_any_element()
    }

    /// Where you are among the three: the current step a longer accent
    /// bar, the others dots.
    fn dots(&self, cx: &App) -> AnyElement {
        let theme = cx.global::<Theme>();
        let at = step_index(self.step);
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
                div()
                    .h(px(DOT))
                    .w(px(if i == at { DOT_ON } else { DOT }))
                    .rounded_full()
                    .bg(if i == at {
                        theme.c("accent")
                    } else if i < at {
                        theme.c("ink-3")
                    } else {
                        theme.c("line-strong")
                    })
            }))
            .into_any_element()
    }
}

/// How high a robot is `t` into its hop on `step`: one hop on the way to
/// the owners, two (a wave) once the workspace exists.
fn hop_at(step: Step, t: f32) -> f32 {
    let frames: &[(f32, f32)] = match step {
        Step::Ready => &[
            (0., 0.),
            (0.25, -HOP),
            (0.5, 0.),
            (0.75, -HOP * 0.6),
            (1., 0.),
        ],
        _ => &[(0., 0.), (0.4, -HOP * 0.6), (1., 0.)],
    };
    crate::robot::draw::track(t, frames, crate::robot::draw::EASE_IN_OUT)
}

/// The step's place among the three.
fn step_index(step: Step) -> usize {
    Step::ALL.iter().position(|s| *s == step).unwrap_or(0)
}

/// Up to [`CAST`] robots of different models: the fleet's first, then
/// spares, so the hero never shows two of a kind.
fn hero_cast(fleet: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut models: Vec<&'static str> = Vec::new();
    let spares = CAST_SPARE.iter().map(|s| s.to_string());
    for name in fleet.iter().cloned().chain(spares) {
        if out.len() == CAST {
            break;
        }
        let model = crate::robot::robot_for(&name).model.id;
        if name.trim().is_empty() || models.contains(&model) {
            continue;
        }
        models.push(model);
        out.push(name);
    }
    out
}

/// An owner's repositories, for its first pill.
fn owner_repos(owner: &KnownOwner, w: &impl Words) -> SharedString {
    if owner.repos == 1 {
        w.text("chrome.reposOne", &[])
    } else {
        w.text("chrome.reposMany", &[("count", &owner.repos.to_string())])
    }
}

/// Where its repositories are now, for the second pill; none when nowhere.
fn owner_where(list: &WorkspacesList, owner: &KnownOwner, w: &impl Words) -> Option<SharedString> {
    let places: Vec<String> = owner
        .from
        .iter()
        .map(|p| {
            list.workspaces
                .iter()
                .find(|ws| ws.id == p.workspace)
                .map_or_else(|| p.workspace.clone(), |ws| ws.name.clone())
        })
        .collect();
    (!places.is_empty()).then(|| w.text("chrome.ownerIn", &[("where", &join_names(&places, w))]))
}

/// Go to the new workspace and say so.
fn go_to_new(id: String, name: String, cx: &mut App) {
    crate::shell::rail::switch_to(id, cx);
    let line = tv(cx, "chrome.workspaceOpened", &[("name", &name)]);
    crate::ui::toast_chrome::success(line, cx);
}

impl Render for AddWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let still = crate::robot::clock::reduced(cx);
        let body = match self.step {
            Step::Name => self.name_step(cx),
            Step::Owners => self.owners_step(cx),
            Step::Ready => self.ready_step(cx),
        };
        let at = step_index(self.step);
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
        let kicker = tv(
            cx,
            "shell.nwStepOf",
            &[
                ("n", &(at + 1).to_string()),
                ("total", &Step::ALL.len().to_string()),
            ],
        );
        let content = div()
            .flex()
            .flex_col()
            .child(ui::text(
                kicker,
                size::META,
                weight::STRONG,
                theme.c("accent-text"),
            ))
            .child(
                div()
                    .mt(px(space::XS))
                    .font_family(ui::DISPLAY)
                    .text_size(px(STEP_TITLE))
                    .line_height(px(STEP_LEADING))
                    .font_weight(weight::HEAVY)
                    .text_color(theme.c("ink"))
                    .child(t(cx, self.step.title_key())),
            )
            .child(div().mt(px(space::GUTTER)).child(body))
            .children(
                self.error
                    .clone()
                    .map(|e| widgets::error(e, &theme).mt(px(space::LG))),
            );
        // Each step slides in from where it comes: the right going on, the
        // left coming back.
        let content = if still {
            content.into_any_element()
        } else {
            let from = if self.went_back {
                -STEP_TRAVEL
            } else {
                STEP_TRAVEL
            };
            content
                .with_animation(
                    ElementId::NamedInteger(
                        "nw-step".into(),
                        (at * 2 + self.went_back as usize) as u64,
                    ),
                    Animation::new(STEP_IN).with_easing(gpui::ease_out_quint()),
                    move |d, t| d.opacity(t).ml(px(from * (1. - t))),
                )
                .into_any_element()
        };
        let lg = |b: gpui::Stateful<gpui::Div>| b.h(px(control::LG)).px(px(space::XL));
        let foot = widgets::dialog_foot(&theme)
            .mt(px(space::SECTION))
            .px(px(BODY_PAD))
            .py(px(space::XL))
            .child(self.dots(cx))
            .child(div().flex_1())
            .when(self.step.back().is_some(), |d| {
                d.child(lg(widgets::button(
                    "nw-back",
                    t(cx, "shell.back"),
                    Kind::Quiet,
                    Size::Normal,
                    None,
                    self.busy,
                    &theme,
                    cx.listener(|this, _, window, cx| this.back(window, cx)),
                )))
            })
            .child(lg(widgets::button(
                "nw-forward",
                primary,
                Kind::Primary,
                Size::Normal,
                if self.step == Step::Ready {
                    Some("arrow-right")
                } else {
                    None
                },
                self.busy,
                &theme,
                cx.listener(|this, _, window, cx| this.forward(window, cx)),
            )));
        widgets::base(&theme)
            .track_focus(&self.focus)
            // Enter goes forward, from a field too; the dialog layer would
            // take it as "OK" and close the sheet.
            .on_action(cx.listener(|this, _: &Confirm, window, cx| this.forward(window, cx)))
            .flex()
            .flex_col()
            .line_height(px(leading::UI))
            .child(self.hero(still, cx))
            .child(div().px(px(BODY_PAD)).pt(px(space::SECTION)).child(content))
            .child(foot)
    }
}

/// Open "Add a workspace" with the caret in the name.
pub fn open(window: &mut Window, cx: &mut App) {
    let go_to: Rc<RefCell<Option<(String, String)>>> = Rc::new(RefCell::new(None));
    let view = cx.new(|cx| AddWorkspace::new(go_to.clone(), window, cx));
    let first = view.read(cx).name.clone();
    // Closed with Esc after step 2: the workspace exists, so go to it.
    let on_close: widgets::Handler = Rc::new(move |_, cx| {
        if let Some((id, name)) = go_to.borrow_mut().take() {
            go_to_new(id, name, cx);
        }
    });
    widgets::open_modal(
        view.into(),
        SHEET_W,
        None,
        widgets::Place::Top(0.08),
        Some(on_close),
        window,
        cx,
    );
    widgets::autofocus(first, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::workspace::test_words::{en, es};
    use crate::model::workspaces::{OwnerPlace, WorkspaceEntry};

    #[test]
    fn the_cast_never_shows_two_of_a_kind() {
        let fleet: Vec<String> = ["Roy", "Roy", "Ivy", "Gary", "Jonah", "Vivian"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let cast = hero_cast(&fleet);
        assert_eq!(cast.len(), CAST);
        let mut models: Vec<&str> = cast
            .iter()
            .map(|n| crate::robot::robot_for(n).model.id)
            .collect();
        models.sort();
        models.dedup();
        assert_eq!(models.len(), CAST);
        // The fleet comes first.
        assert_eq!(cast[0], "Roy");
    }

    #[test]
    fn an_empty_fleet_still_gathers_robots() {
        let cast = hero_cast(&[]);
        assert_eq!(cast.len(), CAST);
        assert!(cast.iter().all(|n| CAST_SPARE.contains(&n.as_str())));
    }

    fn owner(name: &str, repos: u64, from: &[&str]) -> KnownOwner {
        KnownOwner {
            owner: name.into(),
            repos,
            from: from
                .iter()
                .map(|w| OwnerPlace {
                    workspace: w.to_string(),
                    repos,
                })
                .collect(),
            claimed_by: None,
        }
    }

    #[test]
    fn an_owner_shows_its_repositories_and_where_they_are() {
        let list = WorkspacesList {
            workspaces: vec![WorkspaceEntry {
                id: "personal".into(),
                name: "Personal".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let acme = owner("acme", 2, &["personal"]);
        assert_eq!(owner_repos(&acme, &en()), "2 repositories");
        assert_eq!(owner_repos(&owner("x", 1, &[]), &en()), "1 repository");
        assert_eq!(owner_where(&list, &acme, &en()).unwrap(), "in Personal");
        assert_eq!(owner_where(&list, &acme, &es()).unwrap(), "en Personal");
        assert_eq!(owner_repos(&acme, &es()), "2 repositorios");
        // Nowhere yet: no second pill.
        assert!(owner_where(&list, &owner("new", 1, &[]), &en()).is_none());
    }

    #[test]
    fn the_robots_hop_and_land() {
        for step in [Step::Owners, Step::Ready] {
            assert!(hop_at(step, 0.).abs() < 1e-3);
            assert!(hop_at(step, 1.).abs() < 1e-3);
        }
        assert!((hop_at(Step::Ready, 0.25) + HOP).abs() < 1e-3);
        assert!(hop_at(Step::Owners, 0.4) < 0.);
    }

    #[test]
    fn the_steps_count_from_one_place() {
        assert_eq!(step_index(Step::Name), 0);
        assert_eq!(step_index(Step::Ready), 2);
    }
}
