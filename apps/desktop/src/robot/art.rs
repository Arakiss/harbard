//! A robot's painted body, prepared once per model, size and look.
//!
//! The console stacks the body image, a glass layer clipped to the model's
//! screen mask (a soft tint of the light and the face) blended with
//! `mix-blend-mode: screen`, and CSS filters for some states. GPUI has no
//! masks or blend modes, so the parts that do not move are baked here:
//!
//! - `base`: the body with its filter and the tint already blended in;
//! - `over`: the same pixels outside the screen mask (bezel, reflection),
//!   painted after the face so the face ends exactly where the mask does.
//!
//! The face itself is drawn live between the two (see `face.rs`).

use gpui::{Hsla, RenderImage, Rgba};
use image::{Frame, RgbaImage};
use std::collections::HashMap;
use std::sync::Arc;

/// The CSS filter the console puts on the body image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyFilter {
    None,
    /// `saturate(0.4) brightness(0.92)`
    Disconnected,
    /// `saturate(0.75)`
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArtKey {
    /// `robots` or `robots/<season>`.
    pub dir: String,
    pub model: &'static str,
    pub size: u32,
    /// The light, as 8-bit RGB.
    pub glow: [u8; 3],
    /// The tint's opacity in percent: 100, or 15 asleep and disconnected.
    pub tint: u8,
    pub filter: BodyFilter,
}

pub struct Art {
    pub base: Arc<RenderImage>,
    pub over: Arc<RenderImage>,
}

thread_local! {
    static CACHE: std::cell::RefCell<HashMap<ArtKey, Option<Arc<Art>>>> = Default::default();
}

/// The baked art for `key`; built on first use (a few hundred microseconds
/// for list sizes) and kept for the life of the app.
pub fn art(key: &ArtKey) -> Option<Arc<Art>> {
    if let Some(hit) = CACHE.with(|c| c.borrow().get(key).cloned()) {
        return hit;
    }
    let built = bake(key).map(Arc::new);
    CACHE.with(|c| c.borrow_mut().insert(key.clone(), built.clone()));
    built
}

pub fn glow_rgb(c: Hsla) -> [u8; 3] {
    let c: Rgba = c.into();
    [(c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8]
}

fn decode(path: &str, format: image::ImageFormat) -> Option<RgbaImage> {
    let bytes = crate::assets::Assets::local(path)?;
    image::load_from_memory_with_format(&bytes, format).ok().map(|i| i.into_rgba8())
}

/// The CSS `saturate()` matrix, then `brightness()`.
fn filter_rgb(rgb: [f32; 3], filter: BodyFilter) -> [f32; 3] {
    let (s, b) = match filter {
        BodyFilter::None => return rgb,
        BodyFilter::Disconnected => (0.4, 0.92),
        BodyFilter::Finished => (0.75, 1.0),
    };
    let [r, g, bl] = rgb;
    let out = [
        (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * bl,
        (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * bl,
        (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * bl,
    ];
    out.map(|v| (v * b).clamp(0.0, 1.0))
}

/// The tint's alpha at a point: `radial-gradient(60% 50% at 36% 40%,
/// <light> 22%, transparent 70%)` over the whole tile.
fn tint_alpha(x: f32, y: f32) -> f32 {
    let dx = (x - 0.36) / 0.6;
    let dy = (y - 0.40) / 0.5;
    let d = (dx * dx + dy * dy).sqrt();
    if d >= 0.7 { 0.0 } else { 0.22 * (1.0 - d / 0.7) }
}

fn bake(key: &ArtKey) -> Option<Art> {
    let body = decode(&super::identity::body_path(&key.dir, key.model, key.size), image::ImageFormat::WebP)?;
    let mask = decode(&super::identity::mask_path(&key.dir, key.model, key.size), image::ImageFormat::Png)?;
    let (w, h) = body.dimensions();
    if mask.dimensions() != (w, h) {
        return None;
    }
    let glow = key.glow.map(|c| c as f32 / 255.0);
    let tint = key.tint as f32 / 100.0;
    let mut base = RgbaImage::new(w, h);
    let mut over = RgbaImage::new(w, h);
    for (x, y, px) in body.enumerate_pixels() {
        let [r, g, b, a] = px.0;
        let m = mask.get_pixel(x, y).0[3] as f32 / 255.0;
        let rgb = filter_rgb([r, g, b].map(|c| c as f32 / 255.0), key.filter);
        // The glass layer over the body, `screen`-blended where the mask is.
        let alpha = tint * m * tint_alpha((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
        let mixed: [f32; 3] = std::array::from_fn(|i| {
            let screen = rgb[i] + glow[i] - rgb[i] * glow[i];
            rgb[i] * (1.0 - alpha) + screen * alpha
        });
        let to8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        // BGRA, as GPUI's atlas wants it.
        base.put_pixel(x, y, image::Rgba([to8(mixed[2]), to8(mixed[1]), to8(mixed[0]), a]));
        let outside = (a as f32 * (1.0 - m)).round() as u8;
        over.put_pixel(x, y, image::Rgba([to8(mixed[2]), to8(mixed[1]), to8(mixed[0]), outside]));
    }
    Some(Art {
        base: Arc::new(RenderImage::new([Frame::new(base)])),
        over: Arc::new(RenderImage::new([Frame::new(over)])),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_bakes_at_every_size_with_its_reflection_kept_outside() {
        for m in super::super::identity::ROBOT_MODELS {
            let key = ArtKey {
                dir: "robots".into(),
                model: m.id,
                size: 72,
                glow: [0xa8, 0xff, 0xd8],
                tint: 100,
                filter: BodyFilter::None,
            };
            let art = bake(&key).expect(m.id);
            let base = art.base.as_bytes(0).unwrap();
            let over = art.over.as_bytes(0).unwrap();
            // The overlay keeps the body outside the screen and is clear inside it.
            let opaque = over.chunks(4).filter(|p| p[3] > 200).count();
            let body = base.chunks(4).filter(|p| p[3] > 200).count();
            assert!(opaque > 0 && opaque < body, "{}", m.id);
        }
    }

    #[test]
    fn filters_follow_css() {
        let grey = filter_rgb([0.5, 0.5, 0.5], BodyFilter::Disconnected);
        assert!((grey[0] - 0.46).abs() < 1e-3);
        assert_eq!(filter_rgb([0.2, 0.4, 0.6], BodyFilter::None), [0.2, 0.4, 0.6]);
        assert!(tint_alpha(0.36, 0.40) > 0.21 && tint_alpha(0.0, 0.0) == 0.0);
    }
}
