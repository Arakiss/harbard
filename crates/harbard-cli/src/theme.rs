//! `harbard theme`: the console's theme and mode from the terminal.
//!
//! The choice lands in `workspace.json`, the file the console's preferences
//! write through serve. When a serve for this same home answers (at
//! `HARBARD_API`, by default `http://127.0.0.1:7419`), `set` and `mode` go
//! through it, so there is one writer; otherwise the CLI writes the file
//! itself under the same cross-process lock serve takes
//! (`DeskState::update`). An open console repaints without a reload. Every
//! command prints what it did; a refusal goes to stderr with exit 1.

use crate::cli::ThemeCmd;
use harbard_core::Store;
use harbard_core::theme::file::{self, EXAMPLE_FILE, themes_dir};
use harbard_core::theme::{Mode, PAIRS, state};
use harbard_core::workspace::DeskState;
use std::collections::BTreeMap;

fn refuse(message: impl std::fmt::Display) -> i32 {
    eprintln!("refused: {message}");
    1
}

/// Where the change was written, for the line the command prints.
enum Wrote {
    Serve(String),
    File(String),
}

impl std::fmt::Display for Wrote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Wrote::Serve(base) => write!(f, "serve ({base})"),
            Wrote::File(path) => write!(f, "{path}"),
        }
    }
}

/// The write through serve, when a serve for this home answers. `None`
/// when there is none (or it serves another home): the caller writes the
/// file. `Some(Err)` is serve's refusal.
fn via_serve(
    store: &Store,
    action: &str,
    body: serde_json::Value,
) -> Option<Result<String, String>> {
    let base = std::env::var("HARBARD_API")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", crate::serve::DEFAULT_PORT));
    if !(base.starts_with("http://127.0.0.1") || base.starts_with("http://localhost")) {
        return None;
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_millis(1500))
        .build();
    let token = crate::hub_dx::read_hub_token(store);
    let auth = |req: ureq::Request| match &token {
        Some(t) => req.set("authorization", &format!("Bearer {t}")),
        None => req,
    };
    let state: serde_json::Value = auth(agent.get(&format!("{base}/api/v1/theme")))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let ours = themes_dir(store.root()).display().to_string();
    if state.get("dir").and_then(|d| d.as_str()) != Some(ours.as_str()) {
        return None;
    }
    let url = format!("{base}/api/v1/workspace/{action}");
    match auth(agent.post(&url)).send_json(body) {
        Ok(_) => Some(Ok(base)),
        Err(ureq::Error::Status(_, resp)) => {
            let v: serde_json::Value = resp.into_json().unwrap_or_default();
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("serve refused the change")
                .to_string();
            Some(Err(msg))
        }
        Err(_) => None,
    }
}

fn save_desk(
    store: &Store,
    action: &str,
    body: serde_json::Value,
    f: impl FnOnce(&mut DeskState),
) -> Result<Wrote, String> {
    if let Some(done) = via_serve(store, action, body) {
        return done.map(Wrote::Serve);
    }
    let path = DeskState::path(store.root());
    DeskState::update(&path, f).map_err(|e| format!("{}: could not write: {e}", path.display()))?;
    Ok(Wrote::File(path.display().to_string()))
}

pub fn cmd_theme(store: &Store, action: &ThemeCmd) -> i32 {
    let dir = themes_dir(store.root());
    match action {
        ThemeCmd::List => {
            let s = state::read(store.root(), &mut BTreeMap::new());
            println!("Mode: {} ({})", s.mode.label(), s.mode.as_str());
            println!("Themes:");
            for entry in &s.themes {
                let t = &entry.theme;
                let mark = if t.id == s.theme { "*" } else { " " };
                let origin = match (&t.file, t.season) {
                    (Some(file), _) => file.clone(),
                    (None, Some(season)) => format!("built-in, painted by the {season} season"),
                    (None, None) => "built-in".into(),
                };
                println!("{mark} {:<14} {:<14} {origin}", t.id, t.label);
            }
            if let Some(season) = s.season.as_ref() {
                println!(
                    "Season: {} {} ({}, {} to {})",
                    season.id,
                    if season.active { "on" } else { "off" },
                    season.choice.as_str(),
                    season.from,
                    season.to
                );
            }
            if let Some(notice) = &s.notice {
                println!("Notice: {notice}");
            }
            if !s.errors.is_empty() {
                println!("Refused:");
                for e in &s.errors {
                    println!("  {e}");
                }
            }
            0
        }
        // Built-in ids changed spelling; an installed hook may still use the old one.
        ThemeCmd::Set { name } => {
            match file::find(&dir, harbard_core::old_spelling::theme_id(name)) {
                Err(e) => refuse(e),
                Ok(theme) => match save_desk(
                    store,
                    "theme-set",
                    serde_json::json!({ "theme": theme.id }),
                    |d| d.theme = Some(theme.id.clone()),
                ) {
                    Ok(path) => {
                        println!("Theme: {} ({}). Saved to {path}.", theme.label, theme.id);
                        0
                    }
                    Err(e) => refuse(e),
                },
            }
        }
        ThemeCmd::Mode { mode } => {
            let Some(mode) = Mode::parse(mode) else {
                return refuse(format!("`{mode}` is not claro, oscuro or sistema"));
            };
            match save_desk(
                store,
                "theme-mode",
                serde_json::json!({ "mode": mode.as_str() }),
                |d| d.mode = Some(mode.as_str().into()),
            ) {
                Ok(path) => {
                    println!(
                        "Mode: {} ({}). Saved to {path}.",
                        mode.label(),
                        mode.as_str()
                    );
                    0
                }
                Err(e) => refuse(e),
            }
        }
        ThemeCmd::Check { file: path } => match file::load(path) {
            Ok(theme) => {
                println!(
                    "ok: {}: the theme \"{}\" passes in light and in dark ({} contrast pairs per mode).",
                    path.display(),
                    theme.id,
                    PAIRS.len()
                );
                0
            }
            Err(e) => refuse(e),
        },
        ThemeCmd::Import { code, force } => {
            let mut spec = match file::parse_share(code) {
                Ok(spec) => spec,
                Err(e) => return refuse(e),
            };
            spec.source = dir.join(format!("{}.toml", spec.id)).display().to_string();
            match file::save(&dir, &spec, *force) {
                Ok(path) => {
                    println!(
                        "Imported: {} (theme \"{}\"). Choose it with `harbard theme set {}`.",
                        path.display(),
                        spec.id,
                        spec.id
                    );
                    0
                }
                Err(e) => refuse(e),
            }
        }
        ThemeCmd::Path => {
            let existed = dir.join(EXAMPLE_FILE).exists();
            match file::ensure_dir(&dir) {
                Ok(path) => {
                    println!("{}", path.display());
                    if !existed {
                        eprintln!("created with {EXAMPLE_FILE}, which documents every key");
                    }
                    0
                }
                Err(e) => refuse(format!("{}: {e}", dir.display())),
            }
        }
    }
}
