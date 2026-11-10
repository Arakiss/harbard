//! Colours: the same `--hb-*` tokens the console painted with.
//!
//! serve's theme read (`theme_body`, computed in-process) carries every theme
//! in both modes plus the mode layer shared by all of them. The app picks the
//! active one (the season's theme while a season is on), the shade (the
//! stored mode, or the system appearance for `system`), and merges the mode
//! layer under the theme layer, exactly as the console's injected stylesheet
//! did. Screens ask for a token by its CSS name without the `--hb-` prefix:
//! `theme.c("ink-3")`.

use gpui::{App, BoxShadow, Global, Hsla, Rgba, WindowAppearance, hsla, point, px};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Theme {
    pub id: String,
    pub dark: bool,
    pub revision: String,
    pub season: Option<String>,
    pub decorations: bool,
    /// The stored choice: light, dark or system.
    pub mode: String,
    /// The whole serve answer, for Preferences.
    pub state: Value,
    tokens: HashMap<String, String>,
    colors: HashMap<String, Hsla>,
}

impl Global for Theme {}

impl Theme {
    /// A colour token; magenta when the token does not exist, so a typo shows.
    pub fn c(&self, name: &str) -> Hsla {
        self.colors.get(name).copied().unwrap_or_else(|| {
            debug_assert!(false, "unknown theme token {name}");
            hsla(0.83, 1.0, 0.5, 1.0)
        })
    }

    /// The raw token string (shadows and the like).
    pub fn raw(&self, name: &str) -> Option<&str> {
        self.tokens.get(name).map(String::as_str)
    }

    /// `--hb-pop-shadow`: a hairline ring plus a soft drop.
    pub fn pop_shadow(&self) -> Vec<BoxShadow> {
        let (ring, drop) = if self.dark {
            (hsla(0.0, 0.0, 1.0, 0.10), hsla(0.0, 0.0, 0.0, 0.6))
        } else {
            (hsla(0.0, 0.0, 0.11, 0.13), hsla(0.0, 0.0, 0.0, 0.35))
        };
        vec![
            BoxShadow {
                color: ring,
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(1.),
                inset: false,
            },
            BoxShadow {
                color: drop,
                offset: point(px(0.), px(18.)),
                blur_radius: px(48.),
                spread_radius: px(0.),
                inset: false,
            },
        ]
    }

    /// One of the eight name tones (`--hb-tone-N`), picked the console's way.
    pub fn tone_for(&self, name: &str) -> Hsla {
        let mut h: u32 = 0;
        for c in name.encode_utf16() {
            h = h.wrapping_mul(31).wrapping_add(c as u32);
        }
        self.c(&format!("tone-{}", h % 8 + 1))
    }

    /// Build from serve's theme answer and the system appearance.
    pub fn from_state(state: Value, appearance: WindowAppearance) -> Theme {
        let mode = state["mode"].as_str().unwrap_or("system").to_string();
        let dark = match mode.as_str() {
            "light" => false,
            "dark" => true,
            _ => matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark),
        };
        let shade = if dark { "dark" } else { "light" };
        let season_on = state["season"]["active"].as_bool().unwrap_or(false);
        let season = season_on
            .then(|| state["season"]["theme"].as_str().map(str::to_string))
            .flatten();
        let decorations = season_on && state["season"]["decorations"].as_bool().unwrap_or(false);
        let id = season
            .clone()
            .or_else(|| state["theme"].as_str().map(str::to_string))
            .unwrap_or_else(|| "ink".into());
        let mut tokens: HashMap<String, String> = HashMap::new();
        if let Some(layer) = state["modes"][shade].as_object() {
            for (k, v) in layer {
                if let Some(s) = v.as_str() {
                    tokens.insert(k.clone(), s.to_string());
                }
            }
        }
        let entry = state["themes"]
            .as_array()
            .and_then(|all| all.iter().find(|t| t["id"].as_str() == Some(id.as_str())))
            .or_else(|| {
                state["themes"]
                    .as_array()
                    .and_then(|all| all.iter().find(|t| t["id"].as_str() == Some("ink")))
            });
        if let Some(layer) = entry.and_then(|e| e[shade].as_object()) {
            for (k, v) in layer {
                if let Some(s) = v.as_str() {
                    tokens.insert(k.clone(), s.to_string());
                }
            }
        }
        let colors = tokens
            .iter()
            .filter_map(|(k, v)| parse_color(v).map(|c| (k.clone(), c)))
            .collect();
        Theme {
            id,
            dark,
            revision: state["revision"].as_str().unwrap_or_default().to_string(),
            season,
            decorations,
            mode,
            state,
            tokens,
            colors,
        }
    }
}

/// serve's theme answer, read in-process.
pub fn read_state() -> Value {
    let store = harbard_core::store::Store::resolve();
    harbard_cli::serve::theme_route::theme_body(&store, None)
}

/// Load the theme for the current appearance and install it as a global.
pub fn init(cx: &mut App) {
    let theme = Theme::from_state(read_state(), cx.window_appearance());
    cx.set_global(theme);
}

/// Re-read after a theme write or an appearance change.
pub fn reload(cx: &mut App) {
    let theme = Theme::from_state(read_state(), cx.window_appearance());
    cx.set_global(theme);
    cx.refresh_windows();
}

/// `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(…)` and `rgba(…)`.
pub fn parse_color(s: &str) -> Option<Hsla> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let v = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        let (r, g, b, a) = match hex.len() {
            3 => {
                let d = |i: usize| u8::from_str_radix(&hex[i..i + 1].repeat(2), 16).ok();
                (d(0)?, d(1)?, d(2)?, 255)
            }
            6 => (v(0)?, v(2)?, v(4)?, 255),
            8 => (v(0)?, v(2)?, v(4)?, v(6)?),
            _ => return None,
        };
        return Some(
            Rgba {
                r: r as f32 / 255.,
                g: g as f32 / 255.,
                b: b as f32 / 255.,
                a: a as f32 / 255.,
            }
            .into(),
        );
    }
    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<f32> = inner
        .split([',', ' ', '/'])
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.trim().parse::<f32>().ok())
        .collect();
    let (r, g, b) = (*parts.first()?, *parts.get(1)?, *parts.get(2)?);
    let a = parts.get(3).copied().unwrap_or(1.0);
    Some(
        Rgba {
            r: r / 255.,
            g: g / 255.,
            b: b / 255.,
            a,
        }
        .into(),
    )
}

/// Shorthand for screens: `cx.theme_hb()`.
pub trait ThemeExt {
    fn hb(&self) -> &Theme;
}

impl ThemeExt for App {
    fn hb(&self) -> &Theme {
        self.global::<Theme>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_console_colour_forms() {
        assert!(parse_color("#c24f38").is_some());
        assert!(parse_color("rgba(255, 255, 255, 0.22)").is_some());
        assert!(parse_color("#fff").is_some());
        assert!(parse_color("light").is_none());
    }

    #[test]
    fn tones_match_the_console_hash() {
        // h = h*31 + code, unsigned 32-bit; N = h % 8 + 1.
        let mut h: u32 = 0;
        for c in "Adrian".encode_utf16() {
            h = h.wrapping_mul(31).wrapping_add(c as u32);
        }
        assert_eq!(h % 8 + 1, {
            let mut x: u32 = 0;
            for c in "Adrian".chars() {
                x = x.wrapping_mul(31).wrapping_add(c as u32);
            }
            x % 8 + 1
        });
    }
}
