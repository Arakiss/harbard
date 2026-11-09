//! The shell's menus and notifications, from per-feature JSON dictionaries.
//!
//! The console owns the language choice and tells the shell through
//! `desk_set_locale`; the shell keeps a copy in `desk.json` so the menus are
//! right before the first page has loaded.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Locale {
    #[default]
    En,
    Es,
}

impl Locale {
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Self::En),
            "es" => Some(Self::Es),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Es => "es",
        }
    }
}

type Table = HashMap<String, String>;

const SOURCES: [(&str, &str); 4] = [
    ("en", include_str!("../locales/en/menu.json")),
    ("en", include_str!("../locales/en/notifications.json")),
    ("es", include_str!("../locales/es/menu.json")),
    ("es", include_str!("../locales/es/notifications.json")),
];

/// Every key across the shell's features, by language. The sources are compiled
/// in, so a malformed one is a build defect the tests below catch.
fn tables() -> &'static HashMap<&'static str, Table> {
    static TABLES: OnceLock<HashMap<&'static str, Table>> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut all: HashMap<&'static str, Table> = HashMap::new();
        for (code, raw) in SOURCES {
            let table: Table = serde_json::from_str(raw).expect("shell dictionary is valid JSON");
            all.entry(code).or_default().extend(table);
        }
        all
    })
}

/// The text for `key`, falling back to English and then to the key itself.
pub fn tr(locale: Locale, key: &str) -> String {
    let all = tables();
    all.get(locale.code())
        .and_then(|table| table.get(key))
        .or_else(|| all.get(Locale::En.code()).and_then(|table| table.get(key)))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// `tr` with `{name}` holes filled.
pub fn tr_with(locale: Locale, key: &str, vars: &[(&str, &str)]) -> String {
    vars.iter().fold(tr(locale, key), |text, (name, value)| {
        text.replace(&format!("{{{name}}}"), value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn holes(text: &str) -> BTreeSet<String> {
        text.split('{')
            .skip(1)
            .filter_map(|rest| rest.split_once('}').map(|(name, _)| name.to_string()))
            .collect()
    }

    #[test]
    fn both_languages_have_the_same_keys_and_holes() {
        let all = tables();
        let (en, es) = (&all["en"], &all["es"]);
        assert_eq!(
            en.keys().collect::<BTreeSet<_>>(),
            es.keys().collect::<BTreeSet<_>>()
        );
        for (key, text) in en {
            assert_eq!(holes(text), holes(&es[key]), "{key}");
        }
    }

    #[test]
    fn an_unknown_language_is_refused_and_english_is_the_default() {
        assert_eq!(Locale::parse("fr"), None);
        assert_eq!(Locale::default(), Locale::En);
    }

    #[test]
    fn holes_are_filled_in_the_chosen_language() {
        let text = tr_with(Locale::Es, "batchMore", &[("count", "3")]);
        assert_eq!(text, "+3 más");
    }
}
