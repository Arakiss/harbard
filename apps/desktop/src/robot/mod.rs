//! Robot avatars: the console's `components/robot/*`, drawn natively.
//!
//! A robot is its tile, its painted body (baked once, `art.rs`), a live face
//! painted into the model's screen (`face.rs`) and the presence dot. The
//! model, light and tile come from the name (`identity.rs`); the face from
//! the agent's live activity and its microstate (`activity.rs`); motion
//! from the shared robot clock (`clock.rs`), which draws nothing while
//! nothing moves.
//!
//! ```ignore
//! robot::robot("keychain", tier::DENSE).agent(&agent).dot(true) // any size
//! robot::at(Some(&agent), &agent.name, Tier::Card)          // by surface
//! robot::big(Some(&agent), &agent.name, 104.0)              // full face, breathing
//! robot::status_line(&agent)                                // icon, words, timer
//! robot::status::status_icon_sized(&key, &activity, 19.0, window, cx)
//! ```
//!
//! Sizes come by surface ([`tier`], `DESIGN.md` §3.1): 36 in dense lists, 48
//! in rows, 64 in headers, 96–104 on cards, 120 on the calm screen, about 200
//! on the Strip. From 40 points up a robot smiles and breathes; below, the
//! tiny face. The tile's corners are 22 % of the size, and a robot leans 3°
//! to the left under the pointer (not with motion reduced).

pub mod activity;
pub mod art;
pub mod clock;
pub mod draw;
pub mod face;
pub mod gallery;
pub mod identity;
pub mod status;

pub use activity::Face;
pub use identity::{Glow, robot_for};
pub use status::status_line;

use crate::desk::Desk;
use crate::model::{Activity, Agent, Reaction, Thresholds};
use crate::perf::Source;
use crate::theme::Theme;
use activity::{
    ReactionKind, face_for, micro_next_change, micro_state, next_reaction, spend_reactions,
};
use draw::{EASE_IN_OUT, EASE_OUT, Ease, phase, track};
use face::FaceFrame;
use gpui::{
    AnyElement, App, Bounds, ContentMask, Context, Corners, ElementId, EntityId, Hsla,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, RenderOnce, SharedString,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, canvas, div, fill, point, px,
    size,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// From this size up, the full face: the smile and the thin marks.
/// Below it, the tiny face: bolder eyes, no mouth or extras.
pub const SMILE_MIN: f32 = 40.0;
/// From this size up, the robot breathes (`DESIGN.md` §3.1: the smile and
/// the breath from 40 up). Below it the body holds still and only blinks.
pub const BREATHE_MIN: f32 = 40.0;

/// A robot's size by the surface it stands on (`DESIGN.md` §3.1). Use the
/// constant, or [`Tier`] with [`at`].
pub mod tier {
    /// Dense lists: the sidebar, stacks of messages.
    pub const DENSE: f32 = 36.0;
    /// Rows: DM list, activity, history, deliveries.
    pub const ROW: f32 = 48.0;
    /// Headers: a DM's or a pane's header.
    pub const HEADER: f32 = 64.0;
    /// The agent hover card.
    pub const HOVER_CARD: f32 = 96.0;
    /// Cards: the Lobby's portrait, the agent card.
    pub const CARD: f32 = 104.0;
    /// The calm screen and the away card.
    pub const CALM: f32 = 120.0;
    /// The Strip's shelf, before it fits its slots ([`super::Tier::Strip`]).
    pub const STRIP: f32 = 200.0;
}

/// The surfaces a robot stands on, each with its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Dense,
    Row,
    Header,
    HoverCard,
    Card,
    Calm,
    Strip,
}

impl Tier {
    pub const fn size(self) -> f32 {
        match self {
            Tier::Dense => tier::DENSE,
            Tier::Row => tier::ROW,
            Tier::Header => tier::HEADER,
            Tier::HoverCard => tier::HOVER_CARD,
            Tier::Card => tier::CARD,
            Tier::Calm => tier::CALM,
            Tier::Strip => tier::STRIP,
        }
    }
}

/// The robot of `agent` (or of `name`, at rest) at its surface's size.
pub fn at(agent: Option<&Agent>, name: &str, tier: Tier) -> Robot {
    big(agent, name, tier.size())
}

/// The tiny face: bolder eyes, no thin marks, no glow.
pub fn is_tiny(size: f32) -> bool {
    size < SMILE_MIN
}

/// Holds still: no breathing.
pub fn is_still(size: f32) -> bool {
    size < BREATHE_MIN
}

/// What surrounds the presence dot: its ring is cut in this colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ring {
    /// Panes and messages (`--hb-surface`).
    #[default]
    Surface,
    /// The Home sidebar (`--hb-sidebar`, sidebar dot edges).
    Sidebar,
    /// The rail (`--hb-frame`, rail dot edges).
    Rail,
    /// Popovers and cards (`--hb-surface-raised`).
    Raised,
}

impl Ring {
    /// The ring colour, the dot fill and its 1 px edge for `tone`.
    pub fn colors(self, theme: &Theme, tone: &str) -> (Hsla, Hsla, Hsla) {
        let (fill, edge) = crate::ui::tone_colors(theme, tone);
        let tone = if matches!(tone, "green" | "amber" | "red") {
            tone
        } else {
            "grey"
        };
        match self {
            Ring::Surface => (theme.c("surface"), fill, edge),
            Ring::Raised => (theme.c("surface-raised"), fill, edge),
            Ring::Sidebar => (
                theme.c("sidebar"),
                fill,
                theme.c(&format!("sidebar-{tone}-edge")),
            ),
            Ring::Rail => (
                theme.c("frame"),
                fill,
                theme.c(&format!("rail-{tone}-edge")),
            ),
        }
    }
}

/// Another model, light or tile than the name's (comparison sheets).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Pick {
    pub model: Option<&'static str>,
    pub glow: Option<Glow>,
    pub tile: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Props {
    name: SharedString,
    /// The agent key (reactions are per agent), else the name.
    key: SharedString,
    size: f32,
    face: Option<Face>,
    activity: Option<Activity>,
    reactions: Vec<Reaction>,
    thresholds: Option<Thresholds>,
    show_dot: bool,
    tone: Option<&'static str>,
    ring: Ring,
    tile_ring: Option<Hsla>,
    pick: Option<Pick>,
    live: bool,
    /// No tile: the body stands on whatever is behind it (the Strip).
    bare: bool,
    // Filled at render: what the drawing depends on outside the props.
    reduce: bool,
    theme: (String, bool, Option<String>),
}

/// One robot. Build with [`robot`] or [`big`]; it reads the agent's live
/// activity when drawn.
#[derive(IntoElement)]
pub struct Robot {
    props: Props,
}

/// The robot for `name` at `size` points, resting, without a dot.
pub fn robot(name: impl Into<SharedString>, size: f32) -> Robot {
    let name = name.into();
    Robot {
        props: Props {
            key: name.clone(),
            name,
            size,
            face: None,
            activity: None,
            reactions: Vec::new(),
            thresholds: None,
            show_dot: false,
            tone: None,
            ring: Ring::Surface,
            tile_ring: None,
            pick: None,
            live: false,
            bare: false,
            reduce: false,
            theme: Default::default(),
        },
    }
}

/// A large robot (the agent card, the Lobby portrait, the Strip; see
/// [`tier`] and [`at`]): the full face, breathing, reactions. With an agent it shows
/// its live face; without one, the robot of `name` at rest.
pub fn big(agent: Option<&Agent>, name: &str, size: f32) -> Robot {
    match agent {
        Some(a) => robot(crate::derive::robot_name(a).to_string(), size).agent(a),
        None => robot(name.to_string(), size),
    }
}

impl Robot {
    /// Follow `agent`: its live face, reactions and presence (the dot shows
    /// only with [`Robot::dot`]).
    pub fn agent(mut self, agent: &Agent) -> Self {
        let p = &mut self.props;
        p.key = agent.key.clone().into();
        p.activity = Some(agent.activity.clone());
        p.reactions = agent.reactions.clone();
        p.tone = Some(crate::derive::dot_tone(
            &agent.presence,
            agent.dormant.as_deref(),
        ));
        p.live = true;
        self
    }

    /// Draw the presence dot (agents only).
    pub fn dot(mut self, show: bool) -> Self {
        self.props.show_dot = show;
        self
    }

    /// A dot of this tone whatever the agent says.
    pub fn tone(mut self, tone: &'static str) -> Self {
        self.props.tone = Some(tone);
        self.props.show_dot = true;
        self
    }

    /// The colour the dot's ring is cut in.
    pub fn ring(mut self, ring: Ring) -> Self {
        self.props.ring = ring;
        self
    }

    /// A 2 px ring around the tile (`.hb-members-stack`).
    pub fn tile_ring(mut self, color: Hsla) -> Self {
        self.props.tile_ring = Some(color);
        self
    }

    /// Draw this face whatever the activity says.
    pub fn face(mut self, face: Face) -> Self {
        self.props.face = Some(face);
        self
    }

    /// An activity to show (for a robot not tied to a live agent).
    pub fn activity(mut self, activity: Activity) -> Self {
        self.props.activity = Some(activity);
        self
    }

    pub fn pick(mut self, pick: Pick) -> Self {
        self.props.pick = Some(pick);
        self
    }

    /// Draw without the tile: the painted body alone, standing on what is
    /// behind it (`.cs-bot`: the Strip's robots on the conveyor).
    pub fn bare(mut self, bare: bool) -> Self {
        self.props.bare = bare;
        self
    }
}

impl RenderOnce for Robot {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut props = self.props;
        let desk = Desk::get(cx).read(cx);
        if props.live {
            // The faces slice is newer than the full read.
            if let Some(a) = desk
                .activity
                .as_ref()
                .and_then(|v| v.agents.iter().find(|a| a.key == props.key.as_ref()))
            {
                props.activity = Some(a.activity.clone());
                props.reactions = a.reactions.clone();
                props.tone = Some(crate::derive::dot_tone(&a.presence, a.dormant.as_deref()));
            }
        }
        if props.activity.is_some() && props.thresholds.is_none() {
            props.thresholds = desk
                .activity
                .as_ref()
                .map(|a| a.thresholds.clone())
                .or_else(|| desk.view.as_ref().map(|v| v.thresholds.clone()))
                .filter(|t| *t != Thresholds::default());
        }
        let theme = cx.global::<Theme>();
        props.theme = (theme.revision.clone(), theme.dark, theme.season.clone());
        props.reduce = clock::still(window, cx);
        let key = ElementId::Name(format!("robot:{}:{}", props.key, props.size).into());
        let style = StyleRefinement::default().size(px(props.size)).flex_none();
        match clock::nested::<RobotView>(key, style, props, window, cx) {
            Ok(el) => el,
            Err(props) => draw(&props, window, cx),
        }
    }
}

pub struct RobotView {
    props: Props,
}

impl clock::Nested for RobotView {
    type Props = Props;
    fn from_props(props: Props) -> Self {
        RobotView { props }
    }
    fn props_mut(&mut self) -> &mut Props {
        &mut self.props
    }
}

impl Render for RobotView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        draw(&self.props, window, cx)
    }
}

thread_local! {
    /// Every reaction played in this app, by `key:kind:at`.
    static PLAYED: RefCell<activity::Played> = RefCell::default();
    /// The robots under the pointer, by the view that draws them, and since when.
    static HOVERED: RefCell<HashMap<EntityId, Instant>> = RefCell::default();
}

/// The hover tilt (`.hb-bot:hover .hb-bot-rig`): -3° with a springy
/// 350 ms ease (`cubic-bezier(.3, 1.5, .5, 1)`), back at once on leaving.
/// Bodies are baked per half degree, so at most seven per robot and size.
pub const TILT_DEG: f32 = -3.0;
pub const TILT_MS: f32 = 350.0;
const TILT_EASE: Ease = Ease(0.3, 1.5, 0.5, 1.0);

/// The tilt `ms` after the pointer came in, in half degrees (the baked
/// bodies come in half-degree steps), and whether it is still moving.
pub fn tilt_at(ms: Option<f32>) -> (i8, bool) {
    match ms {
        None => (0, false),
        Some(ms) => {
            let u = (ms / TILT_MS).clamp(0.0, 1.0);
            let deg = TILT_DEG * TILT_EASE.at(u);
            ((deg * 2.0).round() as i8, u < 1.0)
        }
    }
}

/// What the canvas paints.
struct Spec {
    size: f32,
    /// `None`: no tile (bare).
    tile: Option<Hsla>,
    tile_ring: Option<Hsla>,
    art: Option<Arc<art::Art>>,
    screen: identity::ScreenBox,
    rig: (f32, f32),
    face: FaceFrame,
}

/// Draw one robot frame and ask the clock for the next.
fn draw(p: &Props, window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = cx.global::<Theme>().clone();
    let now = Instant::now();
    let t = clock::clock_ms(now);
    let desk_now = Desk::get(cx).read(cx).now_ms();
    let wall = harbard_core::clock::now_ms();
    let reduce = p.reduce;

    let mut id = robot_for(&p.name);
    if let Some(pick) = p.pick {
        if let Some(m) = pick
            .model
            .and_then(|m| identity::ROBOT_MODELS.iter().find(|x| x.id == m))
        {
            id.model = *m;
        }
        id.glow = pick.glow.unwrap_or(id.glow);
        id.tile = pick.tile.unwrap_or(id.tile);
    }
    let (model, dir) = identity::dressed(id.model, theme.season.as_deref());

    let state = p.activity.as_ref().map(|a| a.state.as_str());
    let micro = match (&p.activity, &p.thresholds) {
        (Some(a), Some(th)) => Some(micro_state(&a.state, a.since_ms, desk_now, th)),
        _ => None,
    };
    let face_id = face_for(p.face, state, micro.as_ref());
    let tiny = is_tiny(p.size);
    let still = is_still(p.size);
    let glow = theme.c(if face_id.amber() {
        Glow::Amber
    } else {
        id.glow
    }
    .token());

    // When this robot next looks different, on the clock's time.
    let mut next: Option<f64> = None;
    let mut want = |at: f64| next = Some(next.map_or(at, |n: f64| n.min(at)));
    let mut every_frame = false;

    if let (Some(a), Some(th)) = (&p.activity, &p.thresholds)
        && p.face.is_none()
        && let Some(change) = micro_next_change(&a.state, a.since_ms, desk_now, th)
    {
        want(t + (change - desk_now) as f64);
    }

    // A reaction plays once, in every view that shows this agent now.
    let reaction = PLAYED.with(|played| {
        let mut played = played.borrow_mut();
        if reduce {
            spend_reactions(
                &p.key,
                &p.reactions,
                clock::baseline_ms(),
                &mut played,
                wall,
            );
            return None;
        }
        let (kind, started) = next_reaction(
            &p.key,
            &p.reactions,
            clock::baseline_ms(),
            &mut played,
            wall,
        )?;
        let ms = wall.saturating_sub(started);
        (ms < kind.ms()).then_some((kind, ms as f32 / kind.ms() as f32))
    });

    let mut frame = FaceFrame {
        sleepy: micro.is_some_and(|m| m.sleepy),
        ..FaceFrame::resting(face_id, glow, theme.c("robot-off"), tiny)
    };
    let mut rig = (0.0f32, 0.0f32);
    if !reduce {
        let loop_t = clock::on_grid(t, clock::LOOP_MS);
        frame.t = Some(loop_t);
        if face::loops(face_id, tiny) {
            let px = face::unit_px(model.screen.w * p.size, model.screen.h * p.size)
                * window.scale_factor();
            want(loop_wake(face_id, tiny, loop_t, px));
        }
        if face::blinks(face_id) {
            let (scale, wake) = blink(
                t,
                id.blink_s as f64 * 1000.0,
                id.blink_delay_s as f64 * 1000.0,
            );
            frame.blink = scale;
            want(wake);
        }
        let breathes = !still && !matches!(face_id, Face::Disconnected | Face::Finished);
        if breathes {
            let period = if face_id == Face::Asleep {
                7000.0
            } else {
                4600.0
            };
            let (lift, wake) = breath(
                loop_t,
                period,
                id.breathe_delay_s as f64 * 1000.0,
                p.size,
                window.scale_factor(),
            );
            rig.1 = lift;
            want(wake);
        }
        // The blocked face shakes once as it arrives.
        if face_id == Face::Blocked
            && let Some(a) = &p.activity
        {
            let age = desk_now.saturating_sub(a.since_ms);
            if age < 600 {
                frame.dx = track(
                    age as f32 / 600.0,
                    &[(0.0, 0.0), (0.25, -2.0), (0.75, 2.0), (1.0, 0.0)],
                    EASE_IN_OUT,
                );
                every_frame = true;
            }
        }
        if let Some((kind, q)) = reaction {
            every_frame = true;
            match kind {
                ReactionKind::Hop => {
                    rig = (
                        0.0,
                        -0.05
                            * p.size
                            * track(
                                q,
                                &[(0.0, 0.0), (0.35, 1.0), (1.0, 0.0)],
                                Ease(0.3, 1.6, 0.5, 1.0),
                            ),
                    );
                }
                ReactionKind::Shake => {
                    // A rotation of the body in the console; images do not
                    // rotate here, so the body sways sideways by as much.
                    let deg = track(
                        q,
                        &[(0.0, 0.0), (0.2, -2.5), (0.5, 2.0), (0.8, -1.0), (1.0, 0.0)],
                        EASE_IN_OUT,
                    );
                    rig = (deg.to_radians() * p.size * 0.5, 0.0);
                }
                ReactionKind::Glance => {
                    frame.dx = track(
                        q,
                        &[(0.0, 0.0), (0.3, -9.0), (0.7, -9.0), (1.0, 0.0)],
                        EASE_IN_OUT,
                    );
                }
                ReactionKind::Spark => {
                    frame.scale = track(q, &[(0.0, 1.0), (0.3, 1.25), (1.0, 1.0)], EASE_OUT);
                }
                ReactionKind::Boot => {
                    let ms = q * 1600.0;
                    if ms < 800.0 {
                        rig = (
                            0.0,
                            p.size
                                * track(
                                    ms / 800.0,
                                    &[(0.0, -0.6), (0.7, 0.02), (1.0, 0.0)],
                                    Ease(0.2, 1.4, 0.4, 1.0),
                                ),
                        );
                    }
                    frame.power = match q {
                        q if q < 0.5 => 0.0,
                        q if q < 0.55 => 1.0,
                        q if q < 0.62 => 0.0,
                        _ => 1.0,
                    };
                }
            }
        }
    }

    let view = window.current_view();
    let since = HOVERED.with(|h| h.borrow().get(&view).copied());
    let (tilt, tilting) = tilt_at(
        since
            .filter(|_| !reduce)
            .map(|at| now.saturating_duration_since(at).as_secs_f32() * 1000.0),
    );
    every_frame |= tilting;
    if every_frame {
        crate::perf::count(Source::Reactions, 1);
        window.request_animation_frame();
    } else if let Some(at) = next {
        clock::wake_at(view, clock::instant_at(at), Source::Robots, window, cx);
    }

    let filter = match face_id {
        Face::Disconnected => art::BodyFilter::Disconnected,
        Face::Finished => art::BodyFilter::Finished,
        _ => art::BodyFilter::None,
    };
    let key = art::ArtKey {
        dir,
        model: model.id,
        size: identity::asset_size(p.size, window.scale_factor()),
        glow: art::glow_rgb(glow),
        tint: if matches!(face_id, Face::Disconnected | Face::Asleep) {
            15
        } else {
            100
        },
        filter,
        tilt,
    };
    frame.tilt = tilt as f32 / 2.0;
    let spec = Spec {
        size: p.size,
        tile: (!p.bare).then(|| theme.c(&format!("robot-tile-{}", id.tile))),
        tile_ring: p.tile_ring.filter(|_| !p.bare),
        art: art::art(&key),
        screen: model.screen,
        rig,
        face: frame,
    };
    let dot = match (p.show_dot, p.tone) {
        (true, Some(tone)) => {
            let (ring, fill, edge) = p.ring.colors(&theme, tone);
            Some(crate::ui::avatar::presence_mark(p.size, ring, fill, edge))
        }
        _ => None,
    };
    let hover_id = ElementId::Name(format!("robot-hover:{}:{}", p.key, p.size).into());
    div()
        .id(hover_id)
        .relative()
        .flex_none()
        .size(px(p.size))
        .on_hover(move |on, _, cx| {
            HOVERED.with(|h| {
                let mut h = h.borrow_mut();
                if *on {
                    h.entry(view).or_insert_with(Instant::now);
                } else {
                    h.remove(&view);
                }
            });
            cx.notify(view);
        })
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| paint(&spec, bounds, window),
            )
            .size_full(),
        )
        .children(dot)
        .into_any_element()
}

/// The blink: its eye height now, and when it next changes (clock ms).
/// Blinks fall every `period` from where the console's 93% point lands
/// after the name's `delay`, aligned to the clock's ticks.
fn blink(t: f64, period: f64, delay: f64) -> (f32, f64) {
    let tick = clock::TICK_MS;
    let first = ((period * 0.93 + delay) % period + period) % period;
    let align = |x: f64| (x / tick).ceil() * tick;
    let k = ((t - first) / period).floor();
    if k >= 0.0 {
        let start = align(first + k * period);
        let dt = t - start;
        if (0.0..3.0 * tick).contains(&dt) {
            return (
                face::blink_at(clock::on_grid(dt, tick)),
                start + (clock::on_grid(dt, tick) + tick),
            );
        }
        if dt < 0.0 {
            return (1.0, start + tick);
        }
    }
    (1.0, align(first + (k + 1.0) * period) + tick)
}

/// When a looping face next looks different (clock ms), at `px` device
/// pixels per face unit: a small robot's eyes drift under a pixel and a
/// cursor blinks twice a second, so a frame on every tick redrew the window
/// for nothing visible.
fn loop_wake(face_id: Face, tiny: bool, loop_t: f64, px: f32) -> f64 {
    let step = clock::LOOP_MS;
    if face::loop_look(face_id, tiny, loop_t, px).is_none() {
        return loop_t + step;
    }
    // The longest loop is 3.6 s.
    clock::next_change(loop_t, step, 3600.0, |t| {
        face::loop_look(face_id, tiny, t, px)
    })
}

/// The breath at clock time `loop_t` (on the loop grid): how far the body
/// rises, snapped to the screen's pixels, and the first later loop tick at
/// which that changes. The rise is under a pixel at most sizes, so a frame
/// on every tick redrew the window for no visible change.
fn breath(loop_t: f64, period: f64, delay: f64, size: f32, scale: f32) -> (f32, f64) {
    let device = |at: f64| {
        let ph = phase(at, period, delay);
        (-0.008 * size * track(ph, &[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)], EASE_IN_OUT) * scale)
            .round()
    };
    let now = device(loop_t);
    let mut next = loop_t + clock::LOOP_MS;
    while next < loop_t + period && device(next) == now {
        next += clock::LOOP_MS;
    }
    (now / scale, next)
}

fn paint(s: &Spec, bounds: Bounds<Pixels>, window: &mut Window) {
    clock::note_painted();
    // The tile's corners are 22 % of its size, as in the console; a bare
    // robot is clipped to its square only.
    let radius = match s.tile {
        Some(_) => px(crate::ui::avatar::robot_tile_radius(s.size)),
        None => px(0.),
    };
    if let Some(ring) = s.tile_ring {
        let halo = px(crate::ui::space::HAIR);
        window
            .paint_quad(fill(bounds.dilate(halo), ring).corner_radii(Corners::all(radius + halo)));
    }
    if let Some(tile) = s.tile {
        window.paint_quad(fill(bounds, tile).corner_radii(Corners::all(radius)));
    }
    let rig = Bounds {
        origin: bounds.origin + point(px(s.rig.0), px(s.rig.1)),
        size: bounds.size,
    };
    if let Some(art) = &s.art {
        let _ = window.paint_image(
            bounds,
            rig,
            Corners::all(radius),
            art.base.clone(),
            0,
            false,
        );
    }
    let b = s.screen;
    let screen = Bounds {
        origin: rig.origin + point(px((b.x - 0.025 * b.w) * s.size), px(b.y * s.size)),
        size: size(px(b.w * s.size), px(b.h * s.size)),
    };
    let mut face = s.face;
    face.pivot = (f32::from(rig.center().x), f32::from(rig.center().y));
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        face::paint(&face, screen, window)
    });
    if let Some(art) = &s.art {
        let _ = window.paint_image(
            bounds,
            rig,
            Corners::all(radius),
            art.over.clone(),
            0,
            false,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every loop tick of one breath, through `breath`'s own wakes.
    fn wakes_per_breath(size: f32, scale: f32) -> usize {
        let period = 4600.0;
        let mut t = 0.0;
        let mut wakes = 0;
        while t < period {
            t = breath(t, period, 0.0, size, scale).1;
            wakes += 1;
        }
        wakes
    }

    #[test]
    fn a_breath_wakes_only_when_the_body_moves_a_pixel() {
        // 48 px at 2x rises under one device pixel: up and down once.
        assert!(
            wakes_per_breath(48.0, 2.0) <= 3,
            "{}",
            wakes_per_breath(48.0, 2.0)
        );
        // A big robot still moves smoothly, yet below the loop's 19 ticks.
        let big = wakes_per_breath(128.0, 2.0);
        assert!((3..19).contains(&big), "{big}");
    }

    /// The loop ticks in `span` ms at which `face` asks to be drawn.
    fn loop_wakes(face: Face, tiny: bool, px: f32, span: f64) -> usize {
        let (mut t, mut n) = (0.0, 0);
        while t < span {
            t = loop_wake(face, tiny, t, px);
            n += 1;
        }
        n
    }

    #[test]
    fn looping_faces_wake_only_for_ticks_that_change_them() {
        // 4 ticks a second is 40 in 10 s.
        assert!(loop_wakes(Face::Running, false, 1.0, 10_000.0) <= 21);
        // A 24 px robot at 2x: its pondering eyes move a pixel or two.
        assert!(loop_wakes(Face::Thinking, true, 0.25, 10_000.0) < 20);
        // Dots that fade on every tick still get every tick.
        assert_eq!(loop_wakes(Face::Thinking, false, 0.5, 10_000.0), 40);
        // The check holds for most of its 2.8 s.
        assert!(loop_wakes(Face::Verified, false, 0.5, 10_000.0) < 25);
    }

    #[test]
    fn a_breath_lifts_by_whole_device_pixels() {
        let period = 4600.0;
        for k in 0..19 {
            let (lift, wake) = breath(k as f64 * 250.0, period, 0.0, 128.0, 2.0);
            assert_eq!((lift * 2.0).fract(), 0.0);
            assert!((-1.024..=0.0).contains(&lift));
            assert!(wake > k as f64 * 250.0);
        }
    }

    #[test]
    fn blinks_land_on_ticks_and_close_then_open() {
        let period = 5000.0;
        let first = period * 0.93; // no delay: 4650, aligned to 4750
        let (open, wake) = blink(100.0, period, 0.0);
        assert_eq!((open, wake), (1.0, 4750.0 + 125.0));
        let (shut, wake) = blink(4875.0, period, 0.0);
        assert!(shut < 0.5);
        assert_eq!(wake, 5000.0);
        let (opening, wake) = blink(5000.0, period, 0.0);
        assert!(opening > shut && opening < 1.0);
        assert_eq!(wake, 5125.0);
        let (done, wake) = blink(5125.0, period, 0.0);
        assert_eq!(done, 1.0);
        assert_eq!(wake, ((first + period) / 125.0).ceil() * 125.0 + 125.0);
    }

    #[test]
    fn from_40_up_the_smile_and_the_breath() {
        assert!(is_tiny(39.0) && !is_tiny(40.0));
        assert!(is_still(39.0) && !is_still(40.0));
    }

    #[test]
    fn each_surface_has_its_size_and_the_rules_follow_it() {
        assert_eq!(Tier::Dense.size(), 36.0);
        assert_eq!(Tier::Row.size(), 48.0);
        assert_eq!(Tier::Header.size(), 64.0);
        assert_eq!(Tier::Card.size(), 104.0);
        assert_eq!(Tier::Calm.size(), 120.0);
        // Dense lists keep the tiny face; rows and up smile and breathe.
        assert!(is_tiny(Tier::Dense.size()) && is_still(Tier::Dense.size()));
        for t in [
            Tier::Row,
            Tier::Header,
            Tier::HoverCard,
            Tier::Card,
            Tier::Calm,
            Tier::Strip,
        ] {
            assert!(!is_tiny(t.size()) && !is_still(t.size()), "{t:?}");
        }
    }

    #[test]
    fn the_hover_tilt_leans_three_degrees_in_half_degree_steps() {
        assert_eq!(tilt_at(None), (0, false));
        assert_eq!(tilt_at(Some(0.0)), (0, true));
        // The spring peaks near -3.24°, inside the last half-degree step.
        let steps: Vec<i8> = (0..=20)
            .map(|i| tilt_at(Some(TILT_MS * i as f32 / 20.0)).0)
            .collect();
        assert!(steps.iter().all(|&s| (-7..=0).contains(&s)));
        assert!(steps.windows(2).filter(|w| w[1] < w[0]).count() >= 5);
        assert_eq!(tilt_at(Some(TILT_MS)), (-6, false));
        assert_eq!(tilt_at(Some(10_000.0)), (-6, false));
    }
}
