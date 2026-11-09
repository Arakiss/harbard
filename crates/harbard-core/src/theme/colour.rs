//! Colours for the console themes: parsing, CSS output, WCAG contrast and
//! OKLCH steps.
//!
//! OKLCH is used for derivation because a lightness step there looks like
//! the same step whatever the hue, which is what lets two colours from the
//! operator (frame and accent) produce a whole family that reads as one.

/// An sRGB colour with 8-bit alpha. Opaque unless a token is a translucent
/// wash laid over another surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

pub const WHITE: Rgba = Rgba::rgb(255, 255, 255);
pub const BLACK: Rgba = Rgba::rgb(0, 0, 0);

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Rgba { r, g, b, a: 255 }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Rgba { a, ..self }
    }

    /// `#rgb`, `#rrggbb` or `#rrggbbaa`, case-insensitive. Nothing else.
    pub fn parse_hex(raw: &str) -> Option<Self> {
        let hex = raw.strip_prefix('#')?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        match hex.len() {
            3 => {
                let d = |i: usize| u8::from_str_radix(&hex[i..=i], 16).ok().map(|v| v * 17);
                Some(Rgba::rgb(d(0)?, d(1)?, d(2)?))
            }
            6 => Some(Rgba::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Rgba {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: byte(6)?,
            }),
            _ => None,
        }
    }

    /// A CSS colour as the token blocks write them: hex or `rgba(r, g, b, a)`.
    pub fn parse_css(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.starts_with('#') {
            return Self::parse_hex(raw);
        }
        let inner = raw.strip_prefix("rgba(")?.strip_suffix(')')?;
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() != 4 {
            return None;
        }
        let c = |s: &str| s.parse::<u8>().ok();
        let a: f64 = parts[3].parse().ok()?;
        if !(0.0..=1.0).contains(&a) {
            return None;
        }
        Some(Rgba {
            r: c(parts[0])?,
            g: c(parts[1])?,
            b: c(parts[2])?,
            a: (a * 255.0).round() as u8,
        })
    }

    /// `#rrggbb`, or `#rrggbbaa` when translucent. The share string's form.
    pub fn hex(&self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }

    /// The CSS the console receives: hex when opaque, else `rgba()` with the
    /// alpha to two decimals, the way the hand-written tokens were written.
    pub fn css(&self) -> String {
        if self.a == 255 {
            return self.hex();
        }
        let a = f64::from(self.a) / 255.0;
        let mut alpha = format!("{a:.2}");
        while alpha.ends_with('0') {
            alpha.pop();
        }
        if alpha.ends_with('.') {
            alpha.pop();
        }
        format!("rgba({}, {}, {}, {alpha})", self.r, self.g, self.b)
    }

    /// This colour laid over an opaque `bg` (source-over).
    pub fn over(&self, bg: Rgba) -> Rgba {
        if self.a == 255 {
            return *self;
        }
        let a = f64::from(self.a) / 255.0;
        let mix = |f: u8, b: u8| (f64::from(f) * a + f64::from(b) * (1.0 - a)).round() as u8;
        Rgba::rgb(mix(self.r, bg.r), mix(self.g, bg.g), mix(self.b, bg.b))
    }

    /// WCAG 2 relative luminance of the opaque colour.
    pub fn luminance(&self) -> f64 {
        let lin = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    pub fn oklch(&self) -> Oklch {
        Oklch::from_rgb(*self)
    }
}

/// WCAG 2 contrast ratio between two opaque colours, 1 to 21.
pub fn contrast(a: Rgba, b: Rgba) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Lightness 0..1, chroma, hue in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

fn to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

impl Oklch {
    pub fn new(l: f64, c: f64, h: f64) -> Self {
        Oklch {
            l: l.clamp(0.0, 1.0),
            c: c.max(0.0),
            h: h.rem_euclid(360.0),
        }
    }

    pub fn from_rgb(c: Rgba) -> Self {
        let (r, g, b) = (to_linear(c.r), to_linear(c.g), to_linear(c.b));
        let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
        let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
        let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;
        let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
        let ll = 0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s;
        let aa = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
        let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
        Oklch::new(ll, (aa * aa + bb * bb).sqrt(), bb.atan2(aa).to_degrees())
    }

    /// Linear sRGB, possibly outside the gamut.
    fn linear(&self) -> (f64, f64, f64) {
        let (a, b) = (
            self.c * self.h.to_radians().cos(),
            self.c * self.h.to_radians().sin(),
        );
        let l = self.l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
        let m = self.l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
        let s = self.l - 0.089_484_177_5 * a - 1.291_485_548 * b;
        let (l, m, s) = (l * l * l, m * m * m, s * s * s);
        (
            4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
            -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
            -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s,
        )
    }

    fn in_gamut(&self) -> bool {
        let (r, g, b) = self.linear();
        let ok = |v: f64| (-1e-6..=1.0 + 1e-6).contains(&v);
        ok(r) && ok(g) && ok(b)
    }

    /// Back to sRGB. Out-of-gamut colours keep their lightness and hue and
    /// lose chroma until they fit, so a step never changes the hue family.
    pub fn to_rgb(&self) -> Rgba {
        let mut fit = *self;
        if !fit.in_gamut() {
            let (mut lo, mut hi) = (0.0, self.c);
            for _ in 0..24 {
                let mid = (lo + hi) / 2.0;
                fit.c = mid;
                if fit.in_gamut() {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            fit.c = lo;
        }
        let (r, g, b) = fit.linear();
        let byte = |v: f64| (from_linear(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
        Rgba::rgb(byte(r), byte(g), byte(b))
    }

    pub fn with_l(self, l: f64) -> Self {
        Oklch::new(l, self.c, self.h)
    }
}

/// `fg` moved in lightness until it reaches `min` against `bg`: the
/// smallest move that passes, darker or lighter. Used only for derived
/// tokens; a colour the operator wrote is never moved, it is refused
/// instead. Without any passing step it returns black or white, whichever
/// reads better, and the caller's contrast check reports the pair.
pub fn reach(fg: Rgba, bg: Rgba, min: f64) -> Rgba {
    let target = min + 0.05;
    if contrast(fg, bg) >= target {
        return fg;
    }
    let base = fg.oklch();
    for i in 1..=100 {
        let d = f64::from(i) * 0.01;
        let mut found = [base.l - d, base.l + d]
            .into_iter()
            .filter(|l| (0.0..=1.0).contains(l))
            .map(|l| base.with_l(l).to_rgb())
            .filter(|c| contrast(*c, bg) >= target);
        if let Some(c) = found.next() {
            return c;
        }
    }
    if contrast(WHITE, bg) >= contrast(BLACK, bg) {
        WHITE
    } else {
        BLACK
    }
}

/// White or the shell's near-black ink, whichever reads better on `bg`;
/// pure black when neither of those reaches AA (a mid-grey).
pub fn ink_on(bg: Rgba) -> Rgba {
    let dark = Rgba::rgb(0x1d, 0x1c, 0x1d);
    let best = if contrast(WHITE, bg) >= contrast(dark, bg) {
        WHITE
    } else {
        dark
    };
    if contrast(best, bg) < 4.5 && contrast(BLACK, bg) > contrast(best, bg) {
        BLACK
    } else {
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parses_the_three_forms_and_nothing_else() {
        assert_eq!(
            Rgba::parse_hex("#3f0e40"),
            Some(Rgba::rgb(0x3f, 0x0e, 0x40))
        );
        assert_eq!(Rgba::parse_hex("#FFF"), Some(WHITE));
        assert_eq!(Rgba::parse_hex("#ffffff38"), Some(WHITE.with_alpha(0x38)));
        for bad in ["3f0e40", "#3f0e4", "#gg0000", "#3f0e40ff00", "", "#", "red"] {
            assert_eq!(Rgba::parse_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn css_round_trips_the_hand_written_forms() {
        for raw in [
            "#3f0e40",
            "rgba(255, 255, 255, 0.22)",
            "rgba(63, 14, 64, 0.08)",
        ] {
            let c = Rgba::parse_css(raw).unwrap();
            assert_eq!(c.css(), raw);
        }
    }

    #[test]
    fn contrast_matches_the_wcag_reference_values() {
        assert!((contrast(WHITE, BLACK) - 21.0).abs() < 1e-9);
        // #767676 on white is the well-known 4.54:1.
        let grey = Rgba::parse_hex("#767676").unwrap();
        assert!((contrast(grey, WHITE) - 4.54).abs() < 0.01);
    }

    #[test]
    fn oklch_round_trips_srgb() {
        for hex in [
            "#3f0e40", "#7c2e86", "#cdbfce", "#1264a3", "#2bac76", "#ffffff", "#000000",
        ] {
            let c = Rgba::parse_hex(hex).unwrap();
            assert_eq!(c.oklch().to_rgb(), c, "{hex}");
        }
    }

    #[test]
    fn reach_moves_a_derived_colour_until_it_passes() {
        let sidebar = Rgba::parse_hex("#cdbfce").unwrap();
        let green = Rgba::parse_hex("#2bac76").unwrap();
        let fitted = reach(green, sidebar, 3.0);
        assert!(contrast(fitted, sidebar) >= 3.0);
        assert!(
            fitted.luminance() < green.luminance(),
            "darker on a light bg"
        );
        let frame = Rgba::parse_hex("#3f0e40").unwrap();
        let red = Rgba::parse_hex("#d4424a").unwrap();
        assert!(contrast(reach(red, frame, 4.5), frame) >= 4.5);
    }
}
