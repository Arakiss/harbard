//! Console themes: a theme layer chosen by name and a mode layer chosen by
//! light or dark, the way a team chat client separates "theme" from "mode".
//!
//! - The **theme layer** is the window's colour family: frame, rail,
//!   sidebar and accent ([`THEME_KEYS`]). A theme names as few of them as it
//!   likes (`frame` and `accent` at least); the rest are derived in OKLCH so
//!   any two colours give a coherent family, in light and in dark.
//! - The **mode layer** is the neutral surface every theme sits on: lists,
//!   lines, ink, status colours, and `color-scheme`. It depends only on the
//!   mode ([`mode_layer`]).
//!
//! A theme is resolved whole or not at all: both modes are derived and every
//! text-on-background pair in [`PAIRS`] is checked against WCAG AA. A pair
//! below its minimum refuses the theme with the pair and the ratio; derived
//! tokens are moved until they pass, a colour the operator wrote never is.
//!
//! The operator's custom themes are TOML files in `{home}/themes`
//! ([`file`]); the choice of theme and mode lives in `workspace.json`.

// A refusal is the ordinary answer to a hand-edited file, built once per
// read and shown whole; boxing it would only add a deref at every caller.
#![allow(clippy::result_large_err)]

pub mod colour;
pub mod file;
pub mod season;
pub mod state;
pub mod zone;

use crate::coded::{Params, params};
use colour::{BLACK, Oklch, Rgba, WHITE, contrast, ink_on, reach};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;

/// The theme layer, in the fixed order the share string uses.
pub const THEME_KEYS: [&str; 19] = [
    "frame",
    "frame-2",
    "rail-tile",
    "rail-tile-hover",
    "rail-active",
    "rail-ink",
    "ws-tile",
    "sidebar",
    "sidebar-hover",
    "sidebar-ink",
    "sidebar-ink-2",
    "accent",
    "accent-deep",
    "accent-hover",
    "accent-ink",
    "accent-soft",
    "accent-text",
    "link",
    "badge",
];

/// The two keys that are washes laid over another surface, and so the only
/// ones that may carry alpha (`#rrggbbaa`). Every other key is a base the
/// shell paints straight over the page, and must be opaque.
pub const WASH_KEYS: [&str; 2] = ["rail-active", "sidebar-hover"];

/// The console page under `.hb` (the body of `<html class="dark">`). A
/// translucent base would show it through; the check measures on it.
pub const CANVAS: Rgba = Rgba::rgb(0x18, 0x1b, 0x1f);

/// What each theme key paints, for the example file and the docs.
pub const KEY_DOCS: [(&str, &str); 19] = [
    ("frame", "window frame and rail background (required)"),
    ("frame-2", "second tone of the frame"),
    ("rail-tile", "round rail buttons (search, create)"),
    ("rail-tile-hover", "those buttons on hover"),
    (
        "rail-active",
        "background of the active rail destination (accepts alpha, #rrggbbaa)",
    ),
    ("rail-ink", "text and icons over the frame"),
    ("ws-tile", "logo tile on the rail"),
    ("sidebar", "Home sidebar"),
    ("sidebar-hover", "sidebar row on hover (accepts alpha)"),
    ("sidebar-ink", "sidebar text"),
    ("sidebar-ink-2", "sidebar section titles and counters"),
    (
        "accent",
        "accent: selected channel, tabs, badges (required)",
    ),
    ("accent-deep", "dark accent: primary button, counters"),
    ("accent-hover", "primary button on hover"),
    ("accent-ink", "text over the accent"),
    ("accent-soft", "soft accent background (pressed button)"),
    ("accent-text", "accent used as text over the lists"),
    ("link", "links and replies"),
    ("badge", "unread badge on the rail"),
];

/// Minimum ratios (WCAG 2.x AA).
pub const BODY: f64 = 4.5;
/// Large text, icons and the presence dots.
pub const LARGE: f64 = 3.0;

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

/// The operator's mode choice, as stored and as the CLI spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Light,
    Dark,
    System,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Light, Mode::Dark, Mode::System];

    pub fn parse(raw: &str) -> Option<Mode> {
        // Exists only for old files and installed tools that still spell it in Spanish.
        match crate::old_spelling::theme_mode(raw.trim().to_lowercase().as_str()) {
            "light" => Some(Mode::Light),
            "dark" => Some(Mode::Dark),
            "system" => Some(Mode::System),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
            Mode::System => "system",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Light => "Light",
            Mode::Dark => "Dark",
            Mode::System => "Same as system",
        }
    }
}

/// Default when nothing is stored.
pub const DEFAULT_MODE: Mode = Mode::Light;
/// The theme a desk that never chose gets, or whose choice cannot be
/// applied. Changing it is this line plus
/// `cargo test -p harbard-core write_console_files -- --ignored`, which
/// rewrites the console's token block, default id and mock tokens.
pub const DEFAULT_THEME: &str = "ink";

/// Built-in ids that were renamed: `(old, new)`. A desk that stored the old
/// id reads as the new one (`crate::old_spelling::theme_id`), so no custom
/// file may take the old name either.
pub const RETIRED: &[(&str, &str)] = &[("aubergine", "melanzane")];

/// Which palette a resolution produces. "Sistema" ships both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Shade {
    Light,
    Dark,
}

impl Shade {
    pub const BOTH: [Shade; 2] = [Shade::Light, Shade::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Shade::Light => "light",
            Shade::Dark => "dark",
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a theme was refused: the file, the line and key when there is one,
/// and what to change. Shown as is by the CLI and the preferences dialog.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThemeError {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// `syntax`, `unknown_key`, `unknown_section`, `duplicate_key`,
    /// `bad_colour`, `missing_key`, `low_contrast`, `bad_name`,
    /// `read_failed`, `bad_code`, `exists`.
    pub code: &'static str,
    pub message: String,
    pub fix: String,
    /// The console's codes for `message` and `fix`, filled from `params`.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub message_code: &'static str,
    #[serde(skip_serializing_if = "str::is_empty")]
    pub fix_code: &'static str,
    #[serde(skip_serializing_if = "Params::is_empty")]
    pub params: Params,
}

impl ThemeError {
    pub fn new(
        file: impl Into<String>,
        code: &'static str,
        message: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        ThemeError {
            file: file.into(),
            line: None,
            key: None,
            code,
            message: message.into(),
            fix: fix.into(),
            message_code: "",
            fix_code: "",
            params: Params::new(),
        }
    }

    /// The codes the console words this refusal from.
    pub fn coded(
        mut self,
        message_code: &'static str,
        fix_code: &'static str,
        params: Params,
    ) -> Self {
        self.message_code = message_code;
        self.fix_code = fix_code;
        self.params = params;
        self
    }

    pub fn at(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        write!(f, ": ")?;
        if let Some(key) = &self.key {
            write!(f, "`{key}`: ")?;
        }
        write!(f, "{}. Fix: {}", self.message, self.fix)
    }
}

impl std::error::Error for ThemeError {}

// ---------------------------------------------------------------------------
// Specs and resolution
// ---------------------------------------------------------------------------

/// A theme as written: the keys it names, for light and for `[dark]`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThemeSpec {
    /// Built-in id or file stem: lowercase, digits and hyphens.
    pub id: String,
    pub label: String,
    pub builtin: bool,
    /// Where the spec came from, for errors: a path, or `incluido:<id>`.
    pub source: String,
    pub light: BTreeMap<&'static str, Rgba>,
    pub dark: BTreeMap<&'static str, Rgba>,
    /// The season this built-in paints ([`season::SEASONS`]); none for a
    /// theme the operator chooses.
    pub season: Option<&'static str>,
}

/// A theme ready for the console: every theme-layer and computed token, as
/// CSS, for both modes. The mode layer is shared and sent once.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedTheme {
    pub id: String,
    pub label: String,
    pub builtin: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub light: BTreeMap<String, String>,
    pub dark: BTreeMap<String, String>,
    /// `harbard-theme:v1:<id>:<hex>,...` (see [`file::share_string`]).
    pub share: String,
    /// A seasonal theme is applied by its season, never offered in the picker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season: Option<&'static str>,
}

/// The canonical key for `name`, if it is one.
pub fn theme_key(name: &str) -> Option<&'static str> {
    THEME_KEYS.iter().copied().find(|k| *k == name)
}

/// The full theme layer for one shade: what the spec names, the rest derived.
pub fn layer(spec: &ThemeSpec, shade: Shade) -> BTreeMap<&'static str, Rgba> {
    let light_frame = spec
        .light
        .get("frame")
        .copied()
        .unwrap_or(Rgba::rgb(0x2a, 0x24, 0x1d));
    let light_accent = spec
        .light
        .get("accent")
        .copied()
        .unwrap_or(Rgba::rgb(0xc2, 0x4f, 0x38));
    match shade {
        Shade::Light => derive(&spec.light, light_frame, light_accent, Shade::Light),
        Shade::Dark => {
            let frame = spec
                .dark
                .get("frame")
                .copied()
                .unwrap_or_else(|| dark_frame(light_frame));
            let accent = spec
                .dark
                .get("accent")
                .copied()
                .unwrap_or_else(|| dark_accent(light_accent));
            derive(&spec.dark, frame, accent, Shade::Dark)
        }
    }
}

fn dark_frame(frame: Rgba) -> Rgba {
    let f = frame.oklch();
    Oklch::new(f.l.min(0.25), f.c * 0.9, f.h).to_rgb()
}

/// An accent that carries white text (4.5:1) and still marks a tab on the
/// dark surface (3:1): WCAG luminance between about 0.14 and 0.18.
fn dark_accent(accent: Rgba) -> Rgba {
    let a = accent.oklch();
    let (mut lo, mut hi) = (0.0, 1.0);
    let mut best = accent;
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        best = a.with_l(mid).to_rgb();
        if best.luminance() < 0.16 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    best
}

fn step(c: Rgba, dl: f64, chroma: f64) -> Rgba {
    let o = c.oklch();
    Oklch::new(o.l + dl, o.c * chroma, o.h).to_rgb()
}

fn derive(
    given: &BTreeMap<&'static str, Rgba>,
    frame: Rgba,
    accent: Rgba,
    shade: Shade,
) -> BTreeMap<&'static str, Rgba> {
    let mut t: BTreeMap<&'static str, Rgba> = BTreeMap::new();
    let pick = |t: &BTreeMap<&'static str, Rgba>,
                key: &'static str,
                derived: &dyn Fn(&BTreeMap<&'static str, Rgba>) -> Rgba| {
        given.get(key).copied().unwrap_or_else(|| derived(t))
    };
    let dark = shade == Shade::Dark;
    let f = frame.oklch();
    let a = accent.oklch();
    let mode = mode_colours(shade);

    t.insert("frame", frame);
    let v = pick(&t, "frame-2", &|_| {
        step(frame, if dark { 0.03 } else { 0.035 }, 1.0)
    });
    t.insert("frame-2", v);
    let v = pick(&t, "rail-ink", &|_| ink_on(frame));
    t.insert("rail-ink", v);
    let rail_ink = t["rail-ink"];
    let lift = if rail_ink == WHITE || rail_ink.luminance() > 0.5 {
        1.0
    } else {
        -1.0
    };
    // The active destination's wash, as strong as the ink allows: on a
    // mid-tone frame a 22% white wash would take white text below AA.
    let v = pick(&t, "rail-active", &|t| {
        let ink = t["rail-ink"];
        let (wash, alpha, other) = if ink.luminance() > 0.5 {
            (WHITE, if dark { 46 } else { 56 }, BLACK)
        } else {
            (BLACK, 31, WHITE)
        };
        // Lighter steps of the usual wash, then the opposite wash, which
        // only moves the background away from the ink.
        (12..=alpha)
            .rev()
            .map(|a| wash.with_alpha(a))
            .chain((24..=64).map(|a| other.with_alpha(a)))
            .find(|w| contrast(ink, w.over(frame)) >= BODY + 0.05)
            .unwrap_or(other.with_alpha(64))
    });
    t.insert("rail-active", v);
    let v = pick(&t, "rail-tile", &|t| {
        reach(step(frame, lift * 0.09, 0.85), t["rail-ink"], LARGE)
    });
    t.insert("rail-tile", v);
    let v = pick(&t, "rail-tile-hover", &|t| {
        reach(step(frame, lift * 0.14, 0.85), t["rail-ink"], LARGE)
    });
    t.insert("rail-tile-hover", v);

    // Sidebar: the frame's hue, quiet chroma, at the mode's lightness.
    let v = pick(&t, "sidebar", &|_| {
        if dark {
            Oklch::new(0.255, (f.c * 0.25).min(0.03), f.h).to_rgb()
        } else {
            Oklch::new(0.82, (f.c * 0.3).min(0.035), f.h).to_rgb()
        }
    });
    t.insert("sidebar", v);
    let v = pick(&t, "sidebar-hover", &|_| {
        if dark {
            WHITE.with_alpha(15)
        } else {
            frame.with_alpha(20)
        }
    });
    t.insert("sidebar-hover", v);
    let hovered = t["sidebar-hover"].over(t["sidebar"]);
    let v = pick(&t, "sidebar-ink", &|_| {
        let l = if dark { 0.92 } else { 0.27 };
        reach(
            Oklch::new(l, (f.c * 0.2).min(0.025), f.h).to_rgb(),
            hovered,
            BODY,
        )
    });
    t.insert("sidebar-ink", v);
    // Counts sit in rows that take sidebar-hover: fit to the hovered row.
    let v = pick(&t, "sidebar-ink-2", &|_| {
        let l = if dark { 0.78 } else { 0.43 };
        reach(
            Oklch::new(l, (f.c * 0.2).min(0.025), f.h).to_rgb(),
            hovered,
            BODY,
        )
    });
    t.insert("sidebar-ink-2", v);

    // Accent family.
    t.insert("accent", accent);
    let v = pick(&t, "accent-ink", &|_| ink_on(accent));
    t.insert("accent-ink", v);
    let carries_white = t["accent-ink"].luminance() > 0.5;
    let toward = if carries_white { -1.0 } else { 1.0 };
    let v = pick(&t, "accent-deep", &|t| {
        let dl = if dark { 0.08 } else { 0.17 };
        reach(step(accent, toward * dl, 0.85), t["accent-ink"], BODY)
    });
    t.insert("accent-deep", v);
    let v = pick(&t, "accent-hover", &|t| {
        let dl = if dark { 0.04 } else { 0.09 };
        reach(step(accent, toward * dl, 0.9), t["accent-ink"], BODY)
    });
    t.insert("accent-hover", v);
    let v = pick(&t, "accent-soft", &|t| {
        let seed = if dark {
            Oklch::new(0.30, (a.c * 0.4).min(0.06), a.h).to_rgb()
        } else {
            Oklch::new(0.94, (a.c * 0.2).min(0.03), a.h).to_rgb()
        };
        reach(seed, t["accent"], LARGE)
    });
    t.insert("accent-soft", v);
    let text_seed = if dark {
        Oklch::new(0.80, a.c.min(0.12), a.h).to_rgb()
    } else {
        Oklch::new(a.l - 0.17, a.c * 0.85, a.h).to_rgb()
    };
    let v = pick(&t, "accent-text", &|_| reach(text_seed, mode.list, BODY));
    t.insert("accent-text", v);
    let v = pick(&t, "link", &|_| {
        if dark {
            reach(text_seed, mode.list, BODY)
        } else {
            reach(accent, mode.list, BODY)
        }
    });
    t.insert("link", v);
    let v = pick(&t, "ws-tile", &|t| {
        let lit = step(accent, if dark { 0.05 } else { 0.06 }, 1.0);
        reach(lit, t["rail-ink"], LARGE)
    });
    t.insert("ws-tile", v);
    let v = pick(&t, "badge", &|t| {
        let l = if dark { 0.82 } else { 0.84 };
        let seed = Oklch::new(l, (a.c * 0.5).min(0.07), a.h).to_rgb();
        reach(seed, t["accent-deep"], BODY)
    });
    t.insert("badge", v);
    t
}

/// The presence colours. Dots keep these hues; only their edges are fitted.
pub const STATUS: [(&str, Rgba); 4] = [
    ("green", Rgba::rgb(0x2b, 0xac, 0x76)),
    ("amber", Rgba::rgb(0xe8, 0xa3, 0x17)),
    ("red", Rgba::rgb(0xd4, 0x42, 0x4a)),
    ("grey", Rgba::rgb(0xa3, 0xa1, 0xa3)),
];

/// `ink` at `alpha`, or as little more opaque as it takes to read on `bg`.
fn softened(ink: Rgba, bg: Rgba, alpha: u8) -> Rgba {
    let target = BODY + 0.05;
    (alpha..=255)
        .map(|a| ink.with_alpha(a))
        .find(|c| contrast(c.over(bg), bg) >= target)
        .unwrap_or(ink)
}

/// Tokens no theme sets: alpha steps of the inks, and the edge of each
/// presence dot fitted to the sidebar and the rail it sits on. The dot keeps
/// its hue; its 1 px edge is what reaches 3:1 against the background (WCAG
/// 1.4.11 counts a graphic's boundary).
pub fn computed(layer: &BTreeMap<&'static str, Rgba>) -> BTreeMap<String, Rgba> {
    let mut out = BTreeMap::new();
    let rail_ink = layer["rail-ink"];
    out.insert("rail-text".into(), softened(rail_ink, layer["frame"], 219));
    out.insert("rail-hover".into(), rail_ink.with_alpha(31));
    // The lit round button is rail-ink with an accent-deep icon; on a rail
    // whose ink is dark, the icon takes whatever reads on it.
    let on = layer["accent-deep"];
    out.insert(
        "rail-on-ink".into(),
        if contrast(on, rail_ink) >= LARGE + 0.05 {
            on
        } else {
            ink_on(rail_ink)
        },
    );
    out.insert("rail-ring".into(), rail_ink.with_alpha(46));
    out.insert(
        "accent-ink-2".into(),
        softened(layer["accent-ink"], layer["accent"], 217),
    );
    for (name, c) in STATUS {
        out.insert(
            format!("sidebar-{name}-edge"),
            reach(c, layer["sidebar"], LARGE),
        );
        out.insert(format!("rail-{name}-edge"), reach(c, layer["frame"], LARGE));
        // The logo's presence dot sits on the workspace tile: its hue, moved
        // only as far as it takes to read there (a green dot on Jade's green
        // tile was all but invisible).
        let dot = if name == "green" {
            "mark-dot".to_string()
        } else {
            format!("mark-dot-{name}")
        };
        out.insert(dot, mark_dot(c, layer["ws-tile"]));
    }
    out
}

/// The logo's dot on its tile. A dot of another hue reads on any tile
/// (green on a coral tile stays the drawn green); only a dot close in
/// hue to its tile and close in lightness too (green on Jade) is moved to
/// 3:1, keeping its hue. "Close in hue" is under 50 degrees in OKLCH:
/// a red dot on a red tile is moved, a green one is not.
fn mark_dot(dot: Rgba, tile: Rgba) -> Rgba {
    let (d, t) = (dot.oklch(), tile.oklch());
    let grey = |o: &Oklch| o.c < 0.03;
    let hue_gap = {
        let g = (d.h - t.h).rem_euclid(360.0);
        g.min(360.0 - g)
    };
    let alike = if grey(&d) || grey(&t) {
        grey(&d) && grey(&t)
    } else {
        hue_gap < 50.0
    };
    if alike && contrast(dot, tile) < 2.0 {
        reach(dot, tile, LARGE)
    } else {
        dot
    }
}

// ---------------------------------------------------------------------------
// The mode layer
// ---------------------------------------------------------------------------

struct ModeColours {
    list: Rgba,
}

fn mode_colours(shade: Shade) -> ModeColours {
    let raw = mode_raw(shade);
    let get = |k: &str| {
        raw.iter()
            .find(|(n, _)| *n == k)
            .and_then(|(_, v)| Rgba::parse_css(v))
            .expect("mode colour")
    };
    ModeColours { list: get("list") }
}

/// The fixed part of each mode. Light is the shell as it was drawn, with
/// `ink-4` darkened to reach AA; the dots' edges are fitted in
/// [`mode_layer`].
fn mode_raw(shade: Shade) -> &'static [(&'static str, &'static str)] {
    match shade {
        Shade::Light => &[
            ("list", "#f6f5f6"),
            ("surface", "#ffffff"),
            ("surface-raised", "#ffffff"),
            ("surface-2", "#fafafa"),
            ("line", "#e3e0e3"),
            ("line-strong", "#c9c4c9"),
            ("line-hover", "#a9a5a9"),
            ("ink", "#1d1c1d"),
            ("ink-2", "#454245"),
            ("ink-3", "#616061"),
            ("ink-4", "#747274"),
            ("hover", "#f8f8f8"),
            ("hover-2", "#f4f4f4"),
            ("control", "#f0f0f0"),
            ("file-hover", "#f2f2f2"),
            ("btn-hover", "#f6f6f6"),
            ("item-hover", "#efedef"),
            ("selected", "#fdf8e6"),
            ("tint", "#f4f0f5"),
            ("chip", "#ece7ee"),
            ("chip-ink", "#3e3540"),
            ("amber-bg", "#fbf0cf"),
            ("amber-ink", "#7c5a07"),
            ("red-bg", "#fbe3e3"),
            ("red-ink", "#a5161f"),
            ("red-line", "#f2b8bb"),
            ("green-bg", "#e4f2dd"),
            ("green-ink", "#2f6b22"),
            ("green-line", "#c3e3b6"),
            ("flag-bg", "#fff6e0"),
            ("flag-ink", "#8a5a00"),
            ("flag-line", "#f1dca6"),
            ("ok-bg", "#eaf5ef"),
            ("ok-ink", "#1d6b48"),
            ("ok-line", "#b9dfcb"),
            ("warn-bg", "#fdf3dc"),
            ("warn-ink", "#5c4511"),
            ("warn-line", "#f1dfb0"),
            ("note-bg", "#fdf6e3"),
            ("banner-ink", "#7a1219"),
            ("warn-icon", "#a1620a"),
            ("send", "#2e7a55"),
            ("send-ink", "#ffffff"),
            ("focus", "#2b5d91"),
            ("menu-hover", "#2b5d91"),
            ("menu-hover-ink", "#ffffff"),
            ("focus-line", "#868686"),
            ("panel-ring", "rgba(0, 0, 0, 0.12)"),
            ("sidebar-line", "rgba(0, 0, 0, 0.08)"),
            ("rule", "rgba(0, 0, 0, 0.1)"),
            ("scrim", "rgba(29, 28, 29, 0.5)"),
            ("av-ink", "#ffffff"),
            ("av-operator", "#b94f6b"),
            ("av-lead", "#6b6a6b"),
            ("tone-1", "#846f90"),
            ("tone-2", "#5e9ca0"),
            ("tone-3", "#d0905c"),
            ("tone-4", "#5f9a63"),
            ("tone-5", "#d97a6c"),
            ("tone-6", "#6f8fb8"),
            ("tone-7", "#b07aa1"),
            ("tone-8", "#8f8a52"),
            // Robot avatars: the tile behind each robot, its screen's light,
            // and the status line's icons (agents.html).
            // Sage, ice, sand and lilac: one per agent everywhere it appears,
            // and none of them near red, which means blocked (C3).
            ("robot-tile-1", "#d5e5cb"),
            ("robot-tile-2", "#d3e6ef"),
            ("robot-tile-3", "#ede0bf"),
            ("robot-tile-4", "#dfd7f1"),
            ("robot-glow-warm", "#ffe7a8"),
            ("robot-glow-ice", "#8ff3ff"),
            ("robot-glow-mint", "#a8ffd8"),
            ("robot-glow-lilac", "#e4d4ff"),
            ("robot-glow-amber", "#ffc15c"),
            ("robot-off", "#6a6a6a"),
            // Seasonal decorations (season.rs): snow over the frame, the
            // bulbs of the lights in the robots' accents, their wire.
            ("festive-snow", "#ffffff"),
            ("festive-wire", "#4a4238"),
            ("festive-bulb-1", "#f2b84b"),
            ("festive-bulb-2", "#e86a50"),
            ("festive-bulb-3", "#8fd3b0"),
            ("festive-bulb-4", "#9cc8ee"),
            ("st-term", "#1d1c1d"),
            ("st-term-ink", "#ffffff"),
            ("st-web", "#3aa6c9"),
            ("st-orbit", "#c45ac9"),
            ("st-lens", "rgba(255, 255, 255, 0.6)"),
            // Chart series: identity only, checked for colour-blind separation
            // against this surface.
            ("series-1", "#2a78d6"),
            ("series-2", "#eb6834"),
            ("series-3", "#1baf7a"),
            ("series-4", "#eda100"),
            ("series-5", "#e87ba4"),
            ("series-6", "#008300"),
            ("series-7", "#6250d6"),
            ("series-8", "#e34948"),
        ],
        Shade::Dark => &[
            ("list", "#1d2024"),
            ("surface", "#1a1d21"),
            ("surface-raised", "#25282d"),
            ("surface-2", "#222529"),
            ("line", "#2f3237"),
            ("line-strong", "#45484e"),
            ("line-hover", "#63666c"),
            ("ink", "#e8e8e9"),
            ("ink-2", "#c9cacc"),
            ("ink-3", "#a7a9ac"),
            ("ink-4", "#919398"),
            ("hover", "#212428"),
            ("hover-2", "#2a2d32"),
            ("control", "#2c2f34"),
            ("file-hover", "#2a2d32"),
            ("btn-hover", "#2a2d32"),
            ("item-hover", "#24272b"),
            ("selected", "#332d1c"),
            ("tint", "#25232a"),
            ("chip", "#302d35"),
            ("chip-ink", "#ddd8e0"),
            ("amber-bg", "#3b3014"),
            ("amber-ink", "#f2c661"),
            ("red-bg", "#3f1b1e"),
            ("red-ink", "#f4999d"),
            ("red-line", "#6e2c31"),
            ("green-bg", "#1c3320"),
            ("green-ink", "#94d38f"),
            ("green-line", "#2f5a35"),
            ("flag-bg", "#3a2e12"),
            ("flag-ink", "#f0c36a"),
            ("flag-line", "#6a5220"),
            ("ok-bg", "#16302a"),
            ("ok-ink", "#82d8b3"),
            ("ok-line", "#2b5b4a"),
            ("warn-bg", "#3a3016"),
            ("warn-ink", "#f1d9a0"),
            ("warn-line", "#5b4a20"),
            ("note-bg", "#332c1a"),
            ("banner-ink", "#f8bcbf"),
            ("warn-icon", "#e8a317"),
            ("send", "#2f8059"),
            ("send-ink", "#ffffff"),
            ("focus", "#5aa7e6"),
            ("menu-hover", "#2b5d91"),
            ("menu-hover-ink", "#ffffff"),
            ("focus-line", "#8a8d92"),
            ("panel-ring", "rgba(255, 255, 255, 0.08)"),
            ("sidebar-line", "rgba(255, 255, 255, 0.06)"),
            ("rule", "rgba(255, 255, 255, 0.1)"),
            ("scrim", "rgba(0, 0, 0, 0.6)"),
            ("av-ink", "#ffffff"),
            ("av-operator", "#b04a65"),
            ("av-lead", "#6b6a6b"),
            ("tone-1", "#846f90"),
            ("tone-2", "#4f8a8e"),
            ("tone-3", "#b5784a"),
            ("tone-4", "#55895a"),
            ("tone-5", "#c46a5d"),
            ("tone-6", "#5f7fa8"),
            ("tone-7", "#9c6a8e"),
            ("tone-8", "#7d7846"),
            ("robot-tile-1", "#4a6b4c"),
            ("robot-tile-2", "#386476"),
            ("robot-tile-3", "#6f5d2e"),
            ("robot-tile-4", "#5a4e82"),
            ("robot-glow-warm", "#ffe7a8"),
            ("robot-glow-ice", "#8ff3ff"),
            ("robot-glow-mint", "#a8ffd8"),
            ("robot-glow-lilac", "#e4d4ff"),
            ("robot-glow-amber", "#ffc15c"),
            ("robot-off", "#6a6a6a"),
            ("festive-snow", "#f1ede6"),
            ("festive-wire", "#5a5248"),
            ("festive-bulb-1", "#f2b84b"),
            ("festive-bulb-2", "#e86a50"),
            ("festive-bulb-3", "#8fd3b0"),
            ("festive-bulb-4", "#9cc8ee"),
            ("st-term", "#0d0e10"),
            ("st-term-ink", "#ffffff"),
            ("st-web", "#5cbfdf"),
            ("st-orbit", "#d684da"),
            ("st-lens", "rgba(29, 32, 36, 0.6)"),
            // Chart series: identity only, checked for colour-blind separation
            // against this surface.
            ("series-1", "#3987e5"),
            ("series-2", "#d95926"),
            ("series-3", "#199e70"),
            ("series-4", "#c98500"),
            ("series-5", "#d55181"),
            ("series-6", "#008300"),
            ("series-7", "#9085e9"),
            ("series-8", "#e66767"),
        ],
    }
}

/// Every mode-layer token for `shade`, as CSS, including the presence
/// hues, their fitted edges and the two shadows.
pub fn mode_layer(shade: Shade) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = mode_raw(shade)
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    let list = mode_colours(shade).list;
    for (name, c) in STATUS {
        out.insert(name.to_string(), c.css());
        out.insert(format!("{name}-edge"), reach(c, list, LARGE).css());
    }
    let (pop, tools) = match shade {
        Shade::Light => (
            "0 0 0 1px rgba(29, 28, 29, 0.13), 0 18px 48px rgba(0, 0, 0, 0.35)",
            "0 1px 3px rgba(0, 0, 0, 0.08)",
        ),
        Shade::Dark => (
            "0 0 0 1px rgba(255, 255, 255, 0.1), 0 18px 48px rgba(0, 0, 0, 0.6)",
            "0 1px 3px rgba(0, 0, 0, 0.4)",
        ),
    };
    out.insert("pop-shadow".into(), pop.into());
    out.insert("tools-shadow".into(), tools.into());
    out.insert(
        "color-scheme".into(),
        match shade {
            Shade::Light => "light",
            Shade::Dark => "dark",
        }
        .into(),
    );
    out
}

// ---------------------------------------------------------------------------
// Contrast pairs
// ---------------------------------------------------------------------------

/// One text-on-background pair: `fg` over the stack `bg` (an opaque base
/// with translucent washes on top), with its minimum and where it shows.
#[derive(Debug, Clone, Copy)]
pub struct Pair {
    pub fg: &'static str,
    pub bg: &'static [&'static str],
    pub min: f64,
    pub what: &'static str,
    /// Opacity the shell draws `fg` at (`opacity` in workspace.css), 255 = none.
    pub opacity: u8,
}

const fn p(fg: &'static str, bg: &'static [&'static str], min: f64, what: &'static str) -> Pair {
    pa(fg, bg, min, what, 255)
}

/// A pair whose foreground the shell draws at `opacity`.
const fn pa(
    fg: &'static str,
    bg: &'static [&'static str],
    min: f64,
    what: &'static str,
    opacity: u8,
) -> Pair {
    Pair {
        fg,
        bg,
        min,
        what,
        opacity,
    }
}

/// Every pair the shell draws, theme and mode layer alike.
pub const PAIRS: &[Pair] = &[
    p("rail-text", &["frame"], BODY, "rail labels"),
    p(
        "rail-ink",
        &["frame", "rail-active"],
        BODY,
        "active rail destination",
    ),
    p("rail-ink", &["rail-tile"], LARGE, "rail button icons"),
    p(
        "rail-ink",
        &["rail-tile-hover"],
        LARGE,
        "rail icons on hover",
    ),
    p("rail-ink", &["ws-tile"], LARGE, "rail logo"),
    p("rail-on-ink", &["rail-ink"], LARGE, "lit rail button"),
    p("accent-deep", &["badge"], BODY, "unread badge"),
    p("sidebar-ink", &["sidebar"], BODY, "sidebar rows"),
    p(
        "sidebar-ink",
        &["sidebar", "sidebar-hover"],
        BODY,
        "row under the pointer",
    ),
    p("sidebar-ink-2", &["sidebar"], BODY, "sections and counters"),
    p(
        "sidebar-ink-2",
        &["sidebar", "sidebar-hover"],
        BODY,
        "counter on the row under the pointer",
    ),
    p("accent-ink", &["accent"], BODY, "selected channel"),
    p(
        "accent-ink-2",
        &["accent"],
        BODY,
        "selected channel counter",
    ),
    p(
        "accent-ink",
        &["accent-deep"],
        BODY,
        "primary button and counters",
    ),
    p(
        "accent-ink",
        &["accent-hover"],
        BODY,
        "primary button under the pointer",
    ),
    p("accent", &["accent-soft"], LARGE, "pressed button"),
    p("accent", &["surface"], LARGE, "active tab"),
    p("accent-text", &["list"], BODY, "unread pill"),
    p("accent-text", &["surface"], BODY, "accent as text"),
    p("link", &["surface"], BODY, "links"),
    p("link", &["list"], BODY, "links in lists"),
    // `.hb-row-hash` is drawn at 85% opacity; a glyph, so 3:1.
    pa("sidebar-ink", &["sidebar"], LARGE, "channel hash", 217),
    pa(
        "sidebar-ink",
        &["sidebar", "sidebar-hover"],
        LARGE,
        "hash under the pointer",
        217,
    ),
    pa(
        "accent-ink",
        &["accent"],
        LARGE,
        "selected channel hash",
        217,
    ),
    p(
        "sidebar-green-edge",
        &["sidebar"],
        LARGE,
        "green dot edge on the sidebar",
    ),
    p(
        "sidebar-amber-edge",
        &["sidebar"],
        LARGE,
        "amber dot edge on the sidebar",
    ),
    p(
        "sidebar-red-edge",
        &["sidebar"],
        LARGE,
        "red dot edge on the sidebar",
    ),
    p(
        "sidebar-grey-edge",
        &["sidebar"],
        LARGE,
        "grey dot edge on the sidebar",
    ),
    p(
        "rail-green-edge",
        &["frame"],
        LARGE,
        "green dot edge on the rail",
    ),
    p(
        "rail-amber-edge",
        &["frame"],
        LARGE,
        "amber dot edge on the rail",
    ),
    p(
        "rail-red-edge",
        &["frame"],
        LARGE,
        "red dot edge on the rail",
    ),
    p(
        "rail-grey-edge",
        &["frame"],
        LARGE,
        "grey dot edge on the rail",
    ),
    p("ink", &["surface"], BODY, "text"),
    p("ink-2", &["surface"], BODY, "secondary text"),
    p("ink-3", &["surface"], BODY, "times and metadata"),
    p("ink-4", &["surface"], BODY, "paths and hints"),
    p("ink-4", &["surface-2"], BODY, "path on a file card"),
    p("ink", &["list"], BODY, "text in lists"),
    p("ink-2", &["list"], BODY, "preview in lists"),
    p("ink-3", &["list"], BODY, "times in lists"),
    p("ink", &["surface-raised"], BODY, "menus and dialogs"),
    p(
        "ink-3",
        &["surface-raised"],
        BODY,
        "secondary text in dialogs",
    ),
    p("ink", &["selected"], BODY, "selected message"),
    p("ink-2", &["tint"], BODY, "info strip"),
    p("chip-ink", &["chip"], BODY, "label"),
    p("amber-ink", &["amber-bg"], BODY, "amber label"),
    p("red-ink", &["red-bg"], BODY, "red label"),
    p("green-ink", &["green-bg"], BODY, "green label"),
    p("flag-ink", &["flag-bg"], BODY, "protocol notice"),
    p("ok-ink", &["ok-bg"], BODY, "verified"),
    p("warn-ink", &["warn-bg"], BODY, "warning strip"),
    p("warn-ink", &["note-bg"], BODY, "menu note"),
    p("banner-ink", &["red-bg"], BODY, "error banner"),
    p("red-ink", &["surface"], BODY, "error"),
    p("send-ink", &["send"], BODY, "send button"),
    p("menu-hover-ink", &["menu-hover"], BODY, "menu option"),
    p("warn-icon", &["surface"], LARGE, "warning icon"),
    p("focus", &["surface"], LARGE, "focus ring"),
    p("green-edge", &["list"], LARGE, "green dot edge"),
    p("amber-edge", &["list"], LARGE, "amber dot edge"),
    p("red-edge", &["list"], LARGE, "red dot edge"),
    p("grey-edge", &["list"], LARGE, "grey dot edge"),
];

/// Every colour token for one shade: theme layer, computed and mode layer.
pub fn palette(spec: &ThemeSpec, shade: Shade) -> BTreeMap<String, Rgba> {
    let layer = layer(spec, shade);
    let mut out: BTreeMap<String, Rgba> =
        layer.iter().map(|(k, v)| ((*k).to_string(), *v)).collect();
    out.extend(computed(&layer));
    for (k, v) in mode_layer(shade) {
        if let Some(c) = Rgba::parse_css(&v) {
            out.entry(k).or_insert(c);
        }
    }
    out
}

/// The first pair below its minimum, with its ratio.
pub fn check(palette: &BTreeMap<String, Rgba>) -> Result<(), (Pair, f64)> {
    for pair in PAIRS {
        let mut stack = pair.bg.iter().map(|k| palette[*k]);
        let mut bg = stack.next().expect("pair base");
        if bg.a != 255 {
            bg = bg.over(CANVAS);
        }
        for wash in stack {
            bg = wash.over(bg);
        }
        let fg = palette[pair.fg];
        let alpha = (u32::from(fg.a) * u32::from(pair.opacity) / 255) as u8;
        let fg = fg.with_alpha(alpha).over(bg);
        let ratio = contrast(fg, bg);
        if ratio < pair.min {
            return Err((*pair, ratio));
        }
    }
    Ok(())
}

/// Resolve both shades and check every pair; the theme is whole or refused.
pub fn resolve(spec: &ThemeSpec) -> Result<ResolvedTheme, ThemeError> {
    let mut out = BTreeMap::new();
    for shade in Shade::BOTH {
        let full = palette(spec, shade);
        if let Err((pair, ratio)) = check(&full) {
            let written = |k: &str| match shade {
                Shade::Light => spec.light.contains_key(k),
                Shade::Dark => spec.dark.contains_key(k),
            };
            let on = pair.bg.join(" + ");
            let (fix, fix_code) = if written(pair.fg) {
                (
                    format!(
                        "change `{}` so it reads over `{on}`, or remove it and harbard derives it",
                        pair.fg
                    ),
                    "theme.low_contrast.fix_fg",
                )
            } else {
                (
                    format!(
                        "change `{}` (or `{}`), or remove it and harbard derives it",
                        pair.bg[0], pair.fg
                    ),
                    "theme.low_contrast.fix_bg",
                )
            };
            let section = if shade == Shade::Dark { "[dark] " } else { "" };
            return Err(ThemeError::new(
                spec.source.clone(),
                "low_contrast",
                format!(
                    "insufficient contrast in {} mode: {section}`{}` on `{on}` ({}) gives {:.2}:1 and needs {}:1",
                    shade.label(),
                    pair.fg,
                    pair.what,
                    ratio,
                    pair.min
                ),
                fix,
            )
            .coded(
                match shade {
                    Shade::Light => "theme.low_contrast.light",
                    Shade::Dark => "theme.low_contrast.dark",
                },
                fix_code,
                params([
                    ("section", section.into()),
                    ("fg", pair.fg.into()),
                    ("on", on.as_str().into()),
                    ("first_bg", pair.bg[0].into()),
                    ("what", pair.what.into()),
                    ("what_key", pair.what.replace(' ', "_").into()),
                    ("ratio", format!("{ratio:.2}").into()),
                    ("min", pair.min.to_string().into()),
                ]),
            )
            .key(pair.fg));
        }
        let layer = layer(spec, shade);
        let mut tokens: BTreeMap<String, String> = layer
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.css()))
            .collect();
        for (k, v) in computed(&layer) {
            tokens.insert(k, v.css());
        }
        out.insert(shade, tokens);
    }
    let light = out.remove(&Shade::Light).unwrap_or_default();
    let dark = out.remove(&Shade::Dark).unwrap_or_default();
    let mut theme = ResolvedTheme {
        id: spec.id.clone(),
        label: spec.label.clone(),
        builtin: spec.builtin,
        file: (!spec.builtin).then(|| spec.source.clone()),
        light,
        dark,
        share: String::new(),
        season: spec.season,
    };
    theme.share = file::share_string(&theme);
    Ok(theme)
}

/// Every token of `theme` for one shade as `(name, css)`: theme layer in
/// key order, then the computed tokens, then the mode layer. For light
/// [`DEFAULT_THEME`] this is the token block `workspace.css` carries.
pub fn tokens(theme: &ResolvedTheme, shade: Shade) -> Vec<(String, String)> {
    let layer = match shade {
        Shade::Light => &theme.light,
        Shade::Dark => &theme.dark,
    };
    let mut out: Vec<(String, String)> = THEME_KEYS
        .iter()
        .filter_map(|k| layer.get(*k).map(|v| ((*k).to_string(), v.clone())))
        .collect();
    out.extend(
        layer
            .iter()
            .filter(|(k, _)| theme_key(k).is_none())
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    out.extend(mode_layer(shade));
    out
}

// ---------------------------------------------------------------------------
// Built-in themes
// ---------------------------------------------------------------------------

struct Builtin {
    id: &'static str,
    label: &'static str,
    light: &'static [(&'static str, &'static str)],
    dark: &'static [(&'static str, &'static str)],
    season: Option<&'static str>,
}

/// Every built-in names a frame and an accent and derives the family. The
/// first three come from the robots: the warm ink and the coral, mustard and
/// sage of their accents.
const BUILTINS: &[Builtin] = &[
    // The ink of the wordmark on the cream banner, with the coral robot.
    Builtin {
        id: "ink",
        label: "Ink",
        light: &[("frame", "#2a241d"), ("accent", "#c24f38")],
        dark: &[],
        season: None,
    },
    // The navy robot's overalls, with the coral one's helmet.
    Builtin {
        id: "harbour",
        label: "Harbour",
        light: &[("frame", "#1d2b45"), ("accent", "#c24f38")],
        dark: &[],
        season: None,
    },
    // The sage robot, deepened, with the mustard one.
    Builtin {
        id: "sage",
        label: "Sage",
        light: &[("frame", "#26392f"), ("accent", "#b8860f")],
        dark: &[],
        season: None,
    },
    // A deep aubergine frame with a plum accent, every light key written out
    // as the shell was first drawn; the dark ones derive.
    Builtin {
        id: "melanzane",
        label: "Melanzane",
        light: &[
            ("frame", "#3f0e40"),
            ("frame-2", "#4a154b"),
            ("rail-tile", "#5a2a5b"),
            ("rail-tile-hover", "#6d3b6e"),
            ("rail-active", "#ffffff38"),
            ("rail-ink", "#ffffff"),
            ("ws-tile", "#8a3f93"),
            ("sidebar", "#cdbfce"),
            ("sidebar-hover", "#3f0e4014"),
            ("sidebar-ink", "#2b232c"),
            ("sidebar-ink-2", "#4d434e"),
            ("accent", "#7c2e86"),
            ("accent-deep", "#4a154b"),
            ("accent-hover", "#611f69"),
            ("accent-ink", "#ffffff"),
            ("accent-soft", "#f1e8f3"),
            ("accent-text", "#4a154b"),
            ("link", "#7a2f86"),
            ("badge", "#dcc0e0"),
        ],
        dark: &[],
        season: None,
    },
    Builtin {
        id: "clementine",
        label: "Clementine",
        light: &[("frame", "#5a2610"), ("accent", "#c2410c")],
        dark: &[],
        season: None,
    },
    Builtin {
        id: "jade",
        label: "Jade",
        light: &[("frame", "#0d3b2e"), ("accent", "#0f7a56")],
        dark: &[],
        season: None,
    },
    Builtin {
        id: "lagoon",
        label: "Lagoon",
        light: &[("frame", "#0b3352"), ("accent", "#236e9e")],
        dark: &[],
        season: None,
    },
    Builtin {
        id: "indigo",
        label: "Indigo",
        light: &[("frame", "#1e1b4b"), ("accent", "#4f46e5")],
        dark: &[],
        season: None,
    },
    Builtin {
        id: "graphite",
        label: "Graphite",
        light: &[("frame", "#1d1f22"), ("accent", "#4f5560")],
        dark: &[],
        season: None,
    },
    // The builder's pick: the cold green-blue of a Norwegian fjord, the
    // register the shell's name comes from.
    Builtin {
        id: "fjord",
        label: "Fjord",
        light: &[("frame", "#0e2e35"), ("accent", "#0e7480")],
        dark: &[],
        season: None,
    },
    // Christmas: pine needles and cranberry, applied by its season
    // (`season::SEASONS`) over whatever the operator chose.
    Builtin {
        id: "christmas",
        label: "Christmas",
        light: &[("frame", "#1b3a2c"), ("accent", "#b3303d")],
        dark: &[],
        season: Some("christmas"),
    },
];

fn builtin_spec(b: &Builtin) -> ThemeSpec {
    let map = |pairs: &[(&'static str, &'static str)]| {
        pairs
            .iter()
            .map(|(k, v)| {
                (
                    theme_key(k).expect("built-in key"),
                    Rgba::parse_hex(v).expect("built-in colour"),
                )
            })
            .collect()
    };
    ThemeSpec {
        id: b.id.into(),
        label: b.label.into(),
        builtin: true,
        source: format!("builtin:{}", b.id),
        light: map(b.light),
        dark: map(b.dark),
        season: b.season,
    }
}

/// The built-in themes in picker order, the seasonal ones last.
pub fn builtins() -> Vec<ThemeSpec> {
    BUILTINS.iter().map(builtin_spec).collect()
}

pub fn builtin(id: &str) -> Option<ThemeSpec> {
    BUILTINS.iter().find(|b| b.id == id).map(builtin_spec)
}

pub fn is_builtin(id: &str) -> bool {
    BUILTINS.iter().any(|b| b.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passes(id: &str, shade: Shade) {
        let spec = builtin(id).expect("built-in");
        let full = palette(&spec, shade);
        if let Err((pair, ratio)) = check(&full) {
            panic!(
                "{id} {shade:?}: {} on {:?} is {ratio:.2}, needs {}",
                pair.fg, pair.bg, pair.min
            );
        }
        resolve(&spec).expect("resolves");
    }

    macro_rules! contrast_tests {
        ($($name:ident: $id:literal, $shade:expr;)*) => {
            $(
                #[test]
                fn $name() {
                    passes($id, $shade);
                }
            )*
        };
    }

    contrast_tests! {
        ink_light_passes_every_contrast_pair: "ink", Shade::Light;
        ink_dark_passes_every_contrast_pair: "ink", Shade::Dark;
        harbour_light_passes_every_contrast_pair: "harbour", Shade::Light;
        harbour_dark_passes_every_contrast_pair: "harbour", Shade::Dark;
        sage_light_passes_every_contrast_pair: "sage", Shade::Light;
        sage_dark_passes_every_contrast_pair: "sage", Shade::Dark;
        melanzane_light_passes_every_contrast_pair: "melanzane", Shade::Light;
        melanzane_dark_passes_every_contrast_pair: "melanzane", Shade::Dark;
        christmas_light_passes_every_contrast_pair: "christmas", Shade::Light;
        christmas_dark_passes_every_contrast_pair: "christmas", Shade::Dark;
        clementine_light_passes_every_contrast_pair: "clementine", Shade::Light;
        clementine_dark_passes_every_contrast_pair: "clementine", Shade::Dark;
        jade_light_passes_every_contrast_pair: "jade", Shade::Light;
        jade_dark_passes_every_contrast_pair: "jade", Shade::Dark;
        lagoon_light_passes_every_contrast_pair: "lagoon", Shade::Light;
        lagoon_dark_passes_every_contrast_pair: "lagoon", Shade::Dark;
        indigo_light_passes_every_contrast_pair: "indigo", Shade::Light;
        indigo_dark_passes_every_contrast_pair: "indigo", Shade::Dark;
        graphite_light_passes_every_contrast_pair: "graphite", Shade::Light;
        graphite_dark_passes_every_contrast_pair: "graphite", Shade::Dark;
        fjord_light_passes_every_contrast_pair: "fjord", Shade::Light;
        fjord_dark_passes_every_contrast_pair: "fjord", Shade::Dark;
    }

    /// `cargo test -p harbard-core print_tokens -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_tokens() {
        for spec in builtins() {
            let theme = resolve(&spec).unwrap();
            for shade in Shade::BOTH {
                println!("/* {} {:?} */", spec.id, shade);
                for (k, v) in tokens(&theme, shade) {
                    println!("  --hb-{k}: {v};");
                }
            }
        }
    }

    #[test]
    fn the_default_is_a_chosen_builtin_and_retired_ids_are_gone() {
        let default = builtin(DEFAULT_THEME).expect("default built-in");
        assert!(default.season.is_none(), "the default is not seasonal");
        for (old, new) in RETIRED {
            assert!(!is_builtin(old), "{old}");
            assert!(is_builtin(new), "{new}");
            assert_ne!(*new, DEFAULT_THEME, "a renamed theme is never the default");
        }
    }

    #[test]
    fn melanzane_light_keeps_the_drawn_theme_layer() {
        let spec = builtin("melanzane").unwrap();
        let light = layer(&spec, Shade::Light);
        assert_eq!(light["frame"].css(), "#3f0e40");
        assert_eq!(light["accent"].css(), "#7c2e86");
        assert_eq!(light["rail-active"].css(), "rgba(255, 255, 255, 0.22)");
        assert_eq!(light["sidebar-hover"].css(), "rgba(63, 14, 64, 0.08)");
        assert_eq!(light.len(), THEME_KEYS.len());
    }

    /// The logo's dot must read on the workspace tile of every built-in:
    /// another hue, or 3:1. Jade's green tile got a mint dot; Ink keeps
    /// the drawn green and moves the red off its coral tile.
    #[test]
    fn the_logo_dot_reads_on_every_workspace_tile() {
        for spec in builtins() {
            let theme = resolve(&spec).unwrap();
            for shade in Shade::BOTH {
                let t: BTreeMap<String, String> = tokens(&theme, shade).into_iter().collect();
                let tile = Rgba::parse_css(&t["ws-tile"]).unwrap();
                for (name, drawn) in STATUS {
                    let key = if name == "green" {
                        "mark-dot".to_string()
                    } else {
                        format!("mark-dot-{name}")
                    };
                    let dot = Rgba::parse_css(&t[&key]).unwrap();
                    if dot != drawn {
                        assert!(
                            contrast(dot, tile) >= LARGE,
                            "{} {shade:?} {key} {} on {}",
                            spec.id,
                            t[&key],
                            t["ws-tile"]
                        );
                    } else {
                        assert_eq!(dot, mark_dot(drawn, tile));
                    }
                }
            }
        }
        let jade = resolve(&builtin("jade").unwrap()).unwrap();
        let t: BTreeMap<String, String> = tokens(&jade, Shade::Light).into_iter().collect();
        let (dot, tile) = (
            Rgba::parse_css(&t["mark-dot"]).unwrap(),
            Rgba::parse_css(&t["ws-tile"]).unwrap(),
        );
        assert!(
            contrast(dot, tile) >= LARGE,
            "jade's dot {} on {}",
            t["mark-dot"],
            t["ws-tile"]
        );
        let ink = resolve(&builtin("ink").unwrap()).unwrap();
        let t: BTreeMap<String, String> = tokens(&ink, Shade::Light).into_iter().collect();
        assert_eq!(t["mark-dot"], "#2bac76");
        assert_ne!(t["mark-dot-red"], "#d4424a");
    }

    /// The counts and the `#` sit inside `.hb-row`, which gets
    /// `sidebar-hover` under the pointer; the `#` is drawn at 85% opacity
    /// (`.hb-row-hash`), also on the selected row's accent.
    #[test]
    fn sidebar_counts_and_hashes_read_on_a_hovered_or_selected_row() {
        let mut low = Vec::new();
        for spec in builtins() {
            for shade in Shade::BOTH {
                let p = palette(&spec, shade);
                let hovered = p["sidebar-hover"].over(p["sidebar"]);
                let checks = [
                    (
                        "count on hover",
                        p["sidebar-ink-2"].over(hovered),
                        hovered,
                        BODY,
                    ),
                    (
                        "hash",
                        p["sidebar-ink"].with_alpha(217).over(p["sidebar"]),
                        p["sidebar"],
                        LARGE,
                    ),
                    (
                        "hash on hover",
                        p["sidebar-ink"].with_alpha(217).over(hovered),
                        hovered,
                        LARGE,
                    ),
                    (
                        "hash selected",
                        p["accent-ink"].with_alpha(217).over(p["accent"]),
                        p["accent"],
                        LARGE,
                    ),
                ];
                for (what, fg, bg, min) in checks {
                    let r = contrast(fg, bg);
                    if r < min {
                        low.push(format!("{} {shade:?} {what}: {r:.2}", spec.id));
                    }
                }
            }
        }
        assert!(low.is_empty(), "below AA: {low:?}");
    }

    #[test]
    fn every_builtin_has_a_contrast_test_per_mode() {
        let ids: Vec<String> = builtins().into_iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            [
                "ink",
                "harbour",
                "sage",
                "melanzane",
                "clementine",
                "jade",
                "lagoon",
                "indigo",
                "graphite",
                "fjord",
                "christmas"
            ]
        );
    }

    /// A two-colour draft with a mid-tone
    /// frame was refused on a pair whose both sides harbard derived (white
    /// on the rail's active wash). Derived tokens must always pass.
    #[test]
    fn two_colour_drafts_with_mid_tone_frames_resolve() {
        for frame in [
            "#aa3355", "#2b7bb9", "#777777", "#c2410c", "#3a7d44", "#8a5a00",
        ] {
            let mut spec = ThemeSpec {
                id: "borrador".into(),
                label: "borrador".into(),
                source: "borrador".into(),
                ..ThemeSpec::default()
            };
            spec.light.insert("frame", Rgba::parse_hex(frame).unwrap());
            spec.light
                .insert("accent", Rgba::parse_hex("#7c2e86").unwrap());
            if let Err(e) = resolve(&spec) {
                panic!("{frame}: {e}");
            }
        }
    }

    #[test]
    fn a_key_list_and_its_docs_agree() {
        let documented: Vec<&str> = KEY_DOCS.iter().map(|(k, _)| *k).collect();
        assert_eq!(documented, THEME_KEYS);
    }

    #[test]
    fn a_written_colour_is_refused_not_moved() {
        let mut spec = builtin("ink").unwrap();
        spec.source = "/tmp/bajo.toml".into();
        spec.light
            .insert("accent", Rgba::parse_hex("#ffd000").unwrap());
        spec.light.insert("accent-ink", WHITE);
        let err = resolve(&spec).unwrap_err();
        assert_eq!(err.code, "low_contrast");
        assert_eq!(err.key.as_deref(), Some("accent-ink"));
        assert!(err.message.contains("accent-ink` on `accent`"), "{err}");
        assert!(err.message.contains(":1"), "names the ratio: {err}");
    }

    #[test]
    fn a_minimal_spec_derives_every_key_in_both_shades() {
        let mut spec = ThemeSpec {
            id: "mio".into(),
            label: "mio".into(),
            source: "mio.toml".into(),
            ..ThemeSpec::default()
        };
        spec.light
            .insert("frame", Rgba::parse_hex("#203040").unwrap());
        spec.light
            .insert("accent", Rgba::parse_hex("#c03070").unwrap());
        let theme = resolve(&spec).expect("derived theme passes");
        for key in THEME_KEYS {
            assert!(
                theme.light.contains_key(key) && theme.dark.contains_key(key),
                "{key}"
            );
        }
    }
}
