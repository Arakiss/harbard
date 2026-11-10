//! Preferences › Appearance wording (`lib/theme.ts`, `season-section.tsx`):
//! theme names, the picker's swatch, refusals, the season's dates.

use crate::i18n::{I18n, Locale, serve_text, t, tv};
use crate::model::Params;
use crate::model::theme::{ThemeEntry, ThemeRefusal, ThemeState};
use gpui::{App, Hsla, SharedString};

/// The id serve gives the unsaved custom theme while its colours are edited.
pub const DRAFT_ID: &str = "personalizado";

/// The built-in themes by id; serve names them in English whatever the language.
fn builtin_key(id: &str) -> Option<&'static str> {
    Some(match id {
        "ink" => "preferences.themeInk",
        "harbour" => "preferences.themeHarbour",
        "sage" => "preferences.themeSage",
        "melanzane" => "preferences.themeMelanzane",
        "christmas" => "preferences.themeChristmas",
        "clementine" => "preferences.themeClementine",
        "jade" => "preferences.themeJade",
        "lagoon" => "preferences.themeLagoon",
        "indigo" => "preferences.themeIndigo",
        "graphite" => "preferences.themeGraphite",
        "fjord" => "preferences.themeFjord",
        _ => return None,
    })
}

/// A theme's name on screen: a built-in one translated, a user's as written.
pub fn theme_label(entry: &ThemeEntry, cx: &App) -> SharedString {
    if entry.id == DRAFT_ID && !entry.builtin {
        return t(cx, "preferences.themeCustomDraft");
    }
    match entry.builtin.then(|| builtin_key(&entry.id)).flatten() {
        Some(key) => t(cx, key),
        None => entry.label.clone().into(),
    }
}

/// The themes the picker offers: every one but the seasonal ones.
pub fn picker_themes(themes: &[ThemeEntry]) -> Vec<&ThemeEntry> {
    themes.iter().filter(|t| t.season.is_none()).collect()
}

/// A custom theme name the picker may save: what serve accepts.
pub fn valid_theme_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= 40
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Strip the accent from a Latin letter (the console's NFD + combining-mark strip).
fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' | 'Á' | 'À' | 'Ä' | 'Â' | 'Ã' | 'Å' => 'a',
        'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' | 'Ó' | 'Ò' | 'Ö' | 'Ô' | 'Õ' => 'o',
        'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
        'ñ' | 'Ñ' => 'n',
        'ç' | 'Ç' => 'c',
        other => other,
    }
}

/// A display name to a file name: "Mi tema" → "mi-tema".
pub fn slugify(label: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in label.chars().map(fold).flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
    }
    out.chars()
        .take(40)
        .collect::<String>()
        .trim_end_matches('-')
        .to_string()
}

/// The swatch a picker card draws, in a shade.
#[derive(Clone, Copy, Debug)]
pub struct Swatch {
    pub frame: Hsla,
    pub sidebar: Hsla,
    pub accent: Hsla,
    pub ink: Hsla,
    pub rail_ink: Hsla,
}

pub fn swatch(entry: &ThemeEntry, dark: bool) -> Swatch {
    let tokens = if dark { &entry.dark } else { &entry.light };
    let c = |k: &str| {
        tokens
            .get(k)
            .and_then(|v| crate::theme::parse_color(v))
            .unwrap_or(gpui::hsla(0., 0., 0.5, 1.))
    };
    Swatch {
        frame: c("frame"),
        sidebar: c("sidebar"),
        accent: c("accent"),
        ink: c("sidebar-ink"),
        rail_ink: c("rail-ink"),
    }
}

/// `snake_case` → `PascalCase`, the console's contrast-pair keys.
fn pascal(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// A refusal's params, with the contrast pair's description in this language.
fn refusal_vars(r: &ThemeRefusal, cx: &App) -> Option<Params> {
    let mut params = r.params.clone()?;
    if let Some(what) = params
        .get("what_key")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    {
        let key = format!("serve.pair{}", pascal(&what));
        let fallback = params
            .get("what")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| what.clone());
        let text = cx
            .global::<I18n>()
            .get(&key)
            .map(str::to_string)
            .unwrap_or(fallback);
        params.insert("what".into(), serde_json::Value::String(text));
    }
    Some(params)
}

fn refusal_message(r: &ThemeRefusal, cx: &App) -> SharedString {
    serve_text(
        cx,
        r.message_code.as_deref(),
        refusal_vars(r, cx).as_ref(),
        &r.message,
    )
}

/// One line for a refusal, as the CLI prints it.
pub fn describe_refusal(r: &ThemeRefusal, cx: &App) -> String {
    let file: SharedString = match r.file.as_str() {
        "share code" => t(cx, "preferences.refusalShareCode"),
        "custom theme" => t(cx, "preferences.refusalCustomTheme"),
        other => other.to_string().into(),
    };
    let line = r.line.map(|l| format!(":{l}")).unwrap_or_default();
    let key = r
        .key
        .as_ref()
        .map(|k| format!("`{k}`: "))
        .unwrap_or_default();
    let fix = serve_text(
        cx,
        r.fix_code.as_deref(),
        refusal_vars(r, cx).as_ref(),
        &r.fix,
    );
    let fix = tv(cx, "preferences.refusalFix", &[("fix", &fix)]);
    format!("{file}{line}: {key}{}. {fix}", refusal_message(r, cx))
}

/// Why the applied theme or mode differs from the stored one.
pub fn theme_notice(state: &ThemeState, cx: &App) -> Option<SharedString> {
    let notice = state.notice.as_ref()?;
    let mut params = state.notice_params.clone();
    if let Some(p) = params.as_mut()
        && let Some(ix) = p.get("refusal").and_then(|v| v.as_u64())
        && let Some(refused) = state.errors.get(ix as usize)
    {
        p.insert(
            "why".into(),
            serde_json::Value::String(refusal_message(refused, cx).to_string()),
        );
    }
    Some(serve_text(
        cx,
        state.notice_code.as_deref(),
        params.as_ref(),
        notice,
    ))
}

/// A season's name by id; one serve adds later falls back to its id.
pub fn season_name(id: &str, cx: &App) -> SharedString {
    match id {
        "christmas" => t(cx, "preferences.seasonChristmas"),
        other => other.to_string().into(),
    }
}

const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_ES: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];

/// `MM-DD` as "1 December" / "1 de diciembre".
pub fn season_day(mmdd: &str, locale: Locale) -> String {
    let mut parts = mmdd.split('-').map(|p| p.parse::<usize>().unwrap_or(1));
    let month = parts.next().unwrap_or(1).clamp(1, 12);
    let day = parts.next().unwrap_or(1);
    match locale {
        Locale::Es => format!("{day} de {}", MONTHS_ES[month - 1]),
        Locale::En => format!("{day} {}", MONTHS_EN[month - 1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_follow_the_console() {
        assert_eq!(slugify("Mi tema"), "mi-tema");
        assert_eq!(slugify("  Árbol  de Navidad!! "), "arbol-de-navidad");
        assert_eq!(slugify("---"), "");
        assert_eq!(slugify(&"a".repeat(60)).len(), 40);
    }

    #[test]
    fn names_serve_accepts() {
        assert!(valid_theme_name("mi-tema"));
        assert!(valid_theme_name("9lives"));
        assert!(!valid_theme_name("-lead"));
        assert!(!valid_theme_name(""));
        assert!(!valid_theme_name("Upper"));
        assert!(!valid_theme_name(&"a".repeat(41)));
    }

    #[test]
    fn season_days_in_both_languages() {
        assert_eq!(season_day("12-01", Locale::En), "1 December");
        assert_eq!(season_day("01-06", Locale::Es), "6 de enero");
    }

    #[test]
    fn pair_keys_are_pascal() {
        assert_eq!(pascal("rail_logo"), "RailLogo");
        assert_eq!(pascal("text"), "Text");
    }

    #[test]
    fn the_picker_leaves_out_seasons() {
        let a = ThemeEntry {
            id: "ink".into(),
            ..Default::default()
        };
        let b = ThemeEntry {
            id: "christmas".into(),
            season: Some("christmas".into()),
            ..Default::default()
        };
        let all = [a, b];
        let ids: Vec<_> = picker_themes(&all).iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["ink"]);
    }
}
