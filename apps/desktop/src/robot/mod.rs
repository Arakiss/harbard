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
//! robot::robot("keychain", 36.0).agent(&agent).dot(true)   // any size
//! robot::big(Some(&agent), &agent.name, 104.0)             // full face, breathing
//! robot::status_line(&agent)                               // icon, words, timer
//! ```

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
use crate::theme::Theme;
use activity::{
    ReactionKind, face_for, micro_next_change, micro_state, next_reaction, spend_reactions,
};
use draw::{EASE_IN_OUT, EASE_OUT, Ease, phase, track};
use face::FaceFrame;
use gpui::{
    AnyElement, App, Bounds, ContentMask, Context, Corners, ElementId, Hsla, IntoElement,
    ParentElement, Pixels, Render, RenderOnce, SharedString, StyleRefinement, Styled, Window,
    canvas, div, fill, point, px, size,
};
use std::cell::RefCell;
use std::sync::Arc;
use std::time::Instant;

/// At this size and below, the tiny face: bolder eyes, no mouth or extras.
pub const TINY_MAX: f32 = 40.0;
/// Below this a breath is under a device pixel at 2x, and is not drawn.
pub const BREATHE_MIN: f32 = 56.0;

/// The tiny face: bolder eyes, no thin marks, no glow.
pub fn is_tiny(size: f32) -> bool {
    size <= TINY_MAX
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
            reduce: false,
            theme: Default::default(),
        },
    }
}

/// A large robot (104 points and up: the agent card, the Lobby portrait,
/// the Strip): the full face, breathing, reactions. With an agent it shows
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
        props.reduce = clock::reduced(cx);
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
}

/// What the canvas paints.
struct Spec {
    size: f32,
    tile: Hsla,
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
            want(clock::next_grid(t, clock::LOOP_MS));
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
            let ph = phase(loop_t, period, id.breathe_delay_s as f64 * 1000.0);
            rig.1 = -0.008 * p.size * track(ph, &[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)], EASE_IN_OUT);
            want(clock::next_grid(t, clock::LOOP_MS));
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
    if every_frame {
        window.request_animation_frame();
    } else if let Some(at) = next {
        clock::wake_at(view, clock::instant_at(at), window, cx);
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
    };
    let spec = Spec {
        size: p.size,
        tile: theme.c(&format!("robot-tile-{}", id.tile)),
        tile_ring: p.tile_ring,
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
    div()
        .relative()
        .flex_none()
        .size(px(p.size))
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

fn paint(s: &Spec, bounds: Bounds<Pixels>, window: &mut Window) {
    clock::note_painted();
    // The tile's corner is the avatar token for its size, so a robot and an
    // initials tile in one list have the same silhouette.
    let radius = px(crate::ui::avatar::tile_radius(s.size));
    if let Some(ring) = s.tile_ring {
        let halo = px(crate::ui::space::HAIR);
        window
            .paint_quad(fill(bounds.dilate(halo), ring).corner_radii(Corners::all(radius + halo)));
    }
    window.paint_quad(fill(bounds, s.tile).corner_radii(Corners::all(radius)));
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
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        face::paint(&s.face, screen, window)
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
    fn at_40_and_below_the_tiny_face_and_below_56_no_breathing() {
        assert!(is_tiny(40.0) && !is_tiny(41.0));
        assert!(is_still(48.0) && !is_still(56.0));
    }
}
