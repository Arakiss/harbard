//! The console's dictionaries, unchanged: `assets/locales/{en,es}/<ns>.json`.
//!
//! A key is `ns.key` (`t("shell.menuOf")`). Holes are `{name}` and are filled
//! from the vars passed in. A missing key falls back to English, then to the
//! key itself, the console's order. Plurals are key pairs (`fooOne`/`fooMany`);
//! [`tn`] picks one by count.

use crate::assets::Assets;
use gpui::{App, Global, SharedString};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    En,
    Es,
}

impl Locale {
    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::Es => "es",
        }
    }

    pub fn parse(s: &str) -> Locale {
        if s.trim().to_ascii_lowercase().starts_with("es") {
            Locale::Es
        } else {
            Locale::En
        }
    }
}

pub struct I18n {
    pub locale: Locale,
    en: HashMap<String, String>,
    es: HashMap<String, String>,
}

impl Global for I18n {}

fn load(locale: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let prefix = format!("locales/{locale}/");
    for path in Assets::local_paths(&prefix) {
        let Some(ns) = path
            .strip_prefix(&prefix)
            .and_then(|f| f.strip_suffix(".json"))
        else {
            continue;
        };
        let Some(bytes) = Assets::local(&path) else {
            continue;
        };
        let Ok(serde_json::Value::Object(map)) =
            serde_json::from_slice::<serde_json::Value>(&bytes)
        else {
            continue;
        };
        flatten(ns, &serde_json::Value::Object(map), &mut out);
    }
    out
}

fn flatten(prefix: &str, v: &serde_json::Value, out: &mut HashMap<String, String>) {
    match v {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                flatten(&format!("{prefix}.{k}"), v, out);
            }
        }
        serde_json::Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        _ => {}
    }
}

/// The saved language: `desk.json` `locale`, else the system's, else English.
pub fn saved_locale() -> Locale {
    if let Some(l) = crate::desk::desk_prefs().locale {
        return Locale::parse(&l);
    }
    std::env::var("LANG")
        .map(|l| Locale::parse(&l))
        .unwrap_or(Locale::En)
}

pub fn init(cx: &mut App) {
    cx.set_global(I18n {
        locale: saved_locale(),
        en: load("en"),
        es: load("es"),
    });
}

pub fn set_locale(locale: Locale, cx: &mut App) {
    cx.global_mut::<I18n>().locale = locale;
    crate::desk::save_desk_prefs(|p| p.locale = Some(locale.code().to_string()));
    cx.refresh_windows();
}

impl I18n {
    pub fn get(&self, key: &str) -> Option<&str> {
        let primary = match self.locale {
            Locale::Es => &self.es,
            Locale::En => &self.en,
        };
        primary
            .get(key)
            .or_else(|| self.en.get(key))
            .map(String::as_str)
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
}

/// Fill `{name}` holes.
pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// Translate `ns.key`.
pub fn t(cx: &App, key: &str) -> SharedString {
    tv(cx, key, &[])
}

/// Translate `ns.key` and fill its holes.
pub fn tv(cx: &App, key: &str, vars: &[(&str, &str)]) -> SharedString {
    let i = cx.global::<I18n>();
    match i.get(key) {
        Some(s) => fill(s, vars).into(),
        None => key.to_string().into(),
    }
}

/// `ns.baseOne` / `ns.baseMany` by count, with `{count}` filled.
pub fn tn(cx: &App, base: &str, count: u64, vars: &[(&str, &str)]) -> SharedString {
    let key = if count == 1 {
        format!("{base}One")
    } else {
        format!("{base}Many")
    };
    let n = count.to_string();
    let mut all: Vec<(&str, &str)> = vars.to_vec();
    all.push(("count", &n));
    tv(cx, &key, &all)
}

/// A serve `*_code` with params, looked up in the dictionary, else serve's
/// literal text (the console's `serveText`).
pub fn serve_text(
    cx: &App,
    code: Option<&str>,
    params: Option<&crate::model::Params>,
    fallback: &str,
) -> SharedString {
    let Some(code) = code else {
        return fallback.to_string().into();
    };
    let i = cx.global::<I18n>();
    let Some(key) = crate::serve_text::SERVE_TEXT
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, k)| format!("serve.{k}"))
    else {
        return fallback.to_string().into();
    };
    let Some(template) = i.get(&key) else {
        return fallback.to_string().into();
    };
    let owned: Vec<(String, String)> = params
        .map(|p| {
            p.iter()
                .map(|(k, v)| {
                    let s = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), s)
                })
                .collect()
        })
        .unwrap_or_default();
    let vars: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    fill(template, &vars).into()
}
