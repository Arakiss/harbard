//! Avatars: an agent's robot, a monogram square, the lead's tile, the
//! operator's photo or initials. Owned by the chat lane (robots live in
//! `src/robot/`); every screen calls only these functions.

use super::{avatar_size, radius, space, weight};
use crate::model::Agent;
use crate::robot;
use crate::theme::Theme;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Global, Hsla, Image, ImageFormat, IntoElement, ParentElement,
    RenderOnce, SharedString, Styled, Window, div, img, px,
};
use std::sync::Arc;

/// An agent's avatar at `size` px, with its presence dot when `dot` is set
/// (`AgentAvatar`): its robot, live face and reactions.
pub fn agent_avatar(agent: &Agent, size: f32, dot: bool, cx: &App) -> AnyElement {
    let _ = cx;
    robot::robot(crate::derive::robot_name(agent).to_string(), size)
        .agent(agent)
        .dot(dot)
        .into_any_element()
}

/// `AgentAvatar` without an agent (a peer known only by name): the same
/// robot by name, resting, no dot.
pub fn name_avatar(name: &str, size: f32, cx: &App) -> AnyElement {
    let _ = cx;
    robot::robot(name.to_string(), size).into_any_element()
}

/// The console's `Avatar` (`span.hb-av`): a monogram square in the name's
/// tone (or `colour`), with `initials` instead of the derived monogram, and
/// a presence dot of tone `dot` (green, amber, red or grey).
pub fn monogram_avatar(
    name: &str,
    colour: Option<Hsla>,
    initials: Option<&str>,
    size: f32,
    dot: Option<&'static str>,
    cx: &App,
) -> AnyElement {
    let theme = cx.global::<Theme>();
    let text = initials
        .filter(|i| !i.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| crate::ui::format::monogram(name));
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .rounded(px(tile_radius(size)))
        .bg(colour.unwrap_or_else(|| theme.tone_for(name)))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.c("av-ink"))
        .font_family(crate::ui::SANS)
        .font_weight(weight::HEAVY)
        .text_size(px((size * MONOGRAM_SCALE).round()))
        .child(text)
        .when_some(dot, |d, tone| d.child(presence_dot(theme, tone, size)))
        .into_any_element()
}

/// The lead session's tile (`LeadAvatar`): a compass on the accent.
pub fn lead_avatar(size: f32, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(tile_radius(size)))
        .bg(theme.c("accent-deep"))
        .border_2()
        .border_color(theme.c("accent"))
        .flex()
        .items_center()
        .justify_center()
        .child(crate::ui::icon(
            "compass",
            (size * 0.58).round(),
            theme.c("accent-ink"),
        ))
        .into_any_element()
}

/// The operator's avatar (photo when set, else the gallery bust picked by
/// name, else initials on `av-operator`), without a presence mark.
pub fn operator_avatar(initials: &str, size: f32, cx: &App) -> AnyElement {
    let name = crate::desk::Desk::get(cx)
        .read(cx)
        .view
        .as_ref()
        .map(|v| v.operator.clone())
        .unwrap_or_default();
    operator_avatar_with(&name, initials, size, None, cx)
}

/// `OperatorAvatar`: the operator's face with a presence mark —
/// `available` a green dot, `do-not-disturb` a bell, any other state
/// (`focused`, `swamped`, `away`, `reviewing`, `done`) its icon in a badge.
pub fn operator_avatar_with(
    name: &str,
    initials: &str,
    size: f32,
    presence: Option<&str>,
    cx: &App,
) -> AnyElement {
    let _ = cx;
    OperatorAvatar {
        name: name.to_string().into(),
        initials: initials.to_string().into(),
        size,
        presence: presence.map(|p| p.to_string().into()),
    }
    .into_any_element()
}

/// Read the operator's photo again (after the Profile changes it).
pub fn refresh_operator_photo(cx: &mut App) {
    cx.set_global(OperatorPhoto::default());
    load_photo(cx);
}

/// An avatar tile's corner by its size (R1): 8 px at the row size and
/// above, 6 px in the sidebar, 4 px in a stack. Robots and initials share it.
pub fn tile_radius(size: f32) -> f32 {
    if size > avatar_size::SIDEBAR {
        radius::MD
    } else if size > avatar_size::STACK {
        radius::BASE
    } else {
        radius::SM
    }
}

/// The presence dot's diameter on an avatar: 10 px at the row size, 8 px
/// below it.
pub fn dot_size(size: f32) -> f32 {
    if size >= avatar_size::ROW {
        DOT_ROW
    } else {
        DOT_SMALL
    }
}

const DOT_ROW: f32 = 10.;
const DOT_SMALL: f32 = 8.;
/// The surface ring around a presence dot or a badge.
const DOT_RING: f32 = space::HAIR;
/// A presence badge's disc (do not disturb, away) and its glyph.
const BADGE: f32 = 16.;
const BADGE_ICON: f32 = 10.;
/// A monogram's letters, as a share of its tile.
const MONOGRAM_SCALE: f32 = 0.4;

/// The presence dot at an avatar's bottom right, ringed in `ring` so it
/// reads on the tile; `size` is the avatar's.
pub fn presence_mark(size: f32, ring: Hsla, fill: Hsla, edge: Hsla) -> gpui::Div {
    let dot = dot_size(size);
    div()
        .absolute()
        .right(px(-DOT_RING))
        .bottom(px(-DOT_RING))
        .size(px(dot + DOT_RING * 2.))
        .rounded_full()
        .bg(ring)
        .flex()
        .items_center()
        .justify_center()
        .child(crate::ui::dot(fill, edge, px(dot)))
}

/// The presence dot on a surface.
fn presence_dot(theme: &Theme, tone: &str, size: f32) -> impl IntoElement {
    let (ring, fill, edge) = robot::Ring::Surface.colors(theme, tone);
    presence_mark(size, ring, fill, edge)
}

/// The operator gallery: the first twelve busts are the default pool.
const DEFAULT_POOL: [&str; 12] = [
    "operadora",
    "zorro",
    "piloto",
    "gato",
    "inventora",
    "electricista",
    "buho",
    "nutria",
    "panda",
    "conejo",
    "pinguino",
    "mapache",
];

/// The bust an operator has before choosing one: stable for a name.
pub fn default_avatar_id(name: &str) -> &'static str {
    let mut h: u32 = 0;
    for ch in name.trim().to_lowercase().chars() {
        h = h.wrapping_mul(31).wrapping_add(ch as u32);
    }
    DEFAULT_POOL[(h % DEFAULT_POOL.len() as u32) as usize]
}

#[derive(Default)]
enum Photo {
    #[default]
    Unread,
    Loading,
    Ready(Arc<Image>),
    None,
}

#[derive(Default)]
struct OperatorPhoto(Photo);

impl Global for OperatorPhoto {}

fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some(ImageFormat::Png),
        [0xFF, 0xD8, ..] => Some(ImageFormat::Jpeg),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some(ImageFormat::Webp),
        [b'G', b'I', b'F', ..] => Some(ImageFormat::Gif),
        _ => None,
    }
}

/// Fetch the photo once, off the main thread.
fn load_photo(cx: &mut App) {
    cx.default_global::<OperatorPhoto>().0 = Photo::Loading;
    let fetch = cx.background_spawn(async move {
        let profile = crate::api::get("/profile").ok()?;
        profile["profile"]["photo"]
            .as_str()
            .filter(|p| !p.is_empty())?;
        let bytes = crate::api::get_bytes("/profile/photo").ok()?;
        let format = sniff(&bytes)?;
        Some(Arc::new(Image::from_bytes(format, bytes)))
    });
    cx.spawn(async move |cx| {
        let photo = fetch.await;
        cx.update(|cx| {
            cx.set_global(OperatorPhoto(photo.map_or(Photo::None, Photo::Ready)));
            cx.refresh_windows();
        });
    })
    .detach();
}

#[derive(IntoElement)]
struct OperatorAvatar {
    name: SharedString,
    initials: SharedString,
    size: f32,
    presence: Option<SharedString>,
}

impl RenderOnce for OperatorAvatar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if matches!(cx.default_global::<OperatorPhoto>().0, Photo::Unread) {
            load_photo(cx);
        }
        let theme = cx.global::<Theme>().clone();
        let size = self.size;
        let r = px(tile_radius(size));
        let face: AnyElement = match &cx.global::<OperatorPhoto>().0 {
            Photo::Ready(image) => img(image.clone())
                .size(px(size))
                .rounded(r)
                .bg(theme.c("av-operator"))
                .into_any_element(),
            _ if !self.name.is_empty() => img(SharedString::from(format!(
                "avatars/{}-512.webp",
                default_avatar_id(&self.name)
            )))
            .size(px(size))
            .rounded(r)
            .bg(theme.c("av-operator"))
            .into_any_element(),
            _ => monogram_avatar(
                &self.name,
                Some(theme.c("av-operator")),
                Some(&self.initials),
                size,
                None,
                cx,
            ),
        };
        let badge = |icon: &str, tone: Hsla| {
            div()
                .absolute()
                .right(px(-space::XS))
                .bottom(px(-space::XS))
                .size(px(BADGE + DOT_RING * 2.))
                .rounded_full()
                .bg(theme.c("frame"))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(px(BADGE))
                        .rounded_full()
                        .bg(theme.c("surface-raised"))
                        .border_1()
                        .border_color(theme.c("line-strong"))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(crate::ui::icon(icon, BADGE_ICON, tone)),
                )
                .into_any_element()
        };
        let mark: Option<AnyElement> = match self.presence.as_deref() {
            None => None,
            Some("available") => Some(presence_dot(&theme, "green", size).into_any_element()),
            Some("do-not-disturb") => Some(badge("bell-off", theme.c("ink-3"))),
            Some(state) => {
                let (icon, tone) = match state {
                    "focused" => ("focus", theme.c("accent")),
                    "swamped" => ("flame", theme.c("red")),
                    "away" => ("moon", theme.c("ink-3")),
                    "reviewing" => ("scan-search", theme.c("focus")),
                    "done" => ("flag", theme.c("amber-edge")),
                    _ => ("circle", theme.c("green")),
                };
                Some(badge(icon, tone))
            }
        };
        div()
            .relative()
            .flex_none()
            .size(px(size))
            .child(face)
            .children(mark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_bust_is_stable_and_from_the_first_twelve() {
        assert_eq!(default_avatar_id("Petru"), default_avatar_id("  petru "));
        assert!(DEFAULT_POOL.contains(&default_avatar_id("someone else")));
        for id in DEFAULT_POOL {
            assert!(
                crate::assets::Assets::local(&format!("avatars/{id}-512.webp")).is_some(),
                "{id}"
            );
        }
    }

    #[test]
    fn photos_are_sniffed_by_their_first_bytes() {
        assert!(matches!(
            sniff(b"RIFF\0\0\0\0WEBPVP8 "),
            Some(ImageFormat::Webp)
        ));
        assert!(matches!(
            sniff(&[0x89, b'P', b'N', b'G', 0]),
            Some(ImageFormat::Png)
        ));
        assert!(sniff(b"nope").is_none());
    }
}
