//! `GET /api/v1/theme` and the `theme-*` actions of
//! `POST /api/v1/workspace/<action>`: the console's theme and mode.
//!
//! The state is read from `workspace.json` and `{home}/themes`
//! (`harbard_core::theme::state`) and kept in memory with the last accepted
//! tokens of every custom theme, so a file broken by an edit stays applied
//! as it was while its refusal is reported.
//!
//! A watcher thread stats the desk file and the theme files once a second
//! and re-reads only when that fingerprint moves; the console polls
//! `?since=<revision>` and gets a two-field answer until something changed.
//! The fingerprint also moves every quarter of an hour, so a season starts
//! at the operator's midnight without anything written. Nothing here reads
//! outside the harbard home but the system's time zone files
//! (`harbard_core::theme::zone`).
//!
//! `theme-season` (`auto`, `on`, `off`) and `theme-decorations` store the
//! seasonal choices; neither touches the chosen theme.

use super::fleet_routes::ApiError;
use super::workspace_route::with_desk;
use harbard_core::Store;
use harbard_core::theme::colour::Rgba;
use harbard_core::theme::file::{self, themes_dir};
use harbard_core::theme::season::Choice;
use harbard_core::theme::state::{self, State};
use harbard_core::theme::{Mode, ResolvedTheme, ThemeError, ThemeSpec, resolve};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// How often the watcher stats the folder. The console polls at 1.5 s, so
/// an edit shows within about three seconds.
pub const WATCH_EVERY: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Cache {
    root: Option<PathBuf>,
    fingerprint: Option<u64>,
    body: Arc<Value>,
    revision: String,
    last_good: BTreeMap<String, ResolvedTheme>,
    /// Refusals already logged, so a broken file is logged once.
    logged: Vec<String>,
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

fn revision_of(body: &Value) -> String {
    let mut h = DefaultHasher::new();
    body.to_string().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// The current state, re-read only when the fingerprint moved.
fn current(store: &Store) -> (Arc<Value>, String) {
    let root = store.root();
    let fp = state::fingerprint(root);
    let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
    if c.root.as_deref() != Some(root) {
        *c = Cache {
            root: Some(root.to_path_buf()),
            ..Cache::default()
        };
    }
    if c.fingerprint == Some(fp) {
        return (Arc::clone(&c.body), c.revision.clone());
    }
    let s: State = state::read(root, &mut c.last_good);
    let fresh: Vec<String> = s.errors.iter().map(ToString::to_string).collect();
    for e in fresh.iter().filter(|e| !c.logged.contains(e)) {
        eprintln!(
            "{}",
            json!({ "component": "harbard-serve", "event": "theme_refused", "fields": { "reason": e } })
        );
    }
    c.logged = fresh;
    let mut body = serde_json::to_value(&s).unwrap_or_else(|e| json!({ "error": e.to_string() }));
    let revision = revision_of(&body);
    if let Some(o) = body.as_object_mut() {
        o.insert("revision".into(), json!(revision));
    }
    c.body = Arc::new(body);
    c.revision = revision;
    c.fingerprint = Some(fp);
    (Arc::clone(&c.body), c.revision.clone())
}

/// Forget the fingerprint so the next read re-reads (after a write).
fn invalidate() {
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .fingerprint = None;
}

/// `GET /api/v1/theme[?since=<revision>]`.
pub fn theme_body(store: &Store, since: Option<String>) -> Value {
    let (body, revision) = current(store);
    if since.as_deref() == Some(revision.as_str()) {
        return json!({ "revision": revision, "unchanged": true });
    }
    (*body).clone()
}

/// The one theme watcher: a stat per second, a re-read on change.
pub fn start_watcher(store: Store, shutdown: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        while !shutdown.load(Ordering::Relaxed) {
            let _ = current(&store);
            std::thread::sleep(WATCH_EVERY);
        }
    });
}

fn refused(err: &ThemeError) -> ApiError {
    let status = match err.code {
        "missing_theme" => 404,
        "exists" => 409,
        _ => 422,
    };
    ApiError::new(status, "theme_refused", err.to_string())
        .with("refusal", serde_json::to_value(err).unwrap_or(Value::Null))
}

fn text<'a>(body: &'a Value, name: &'static str) -> Result<&'a str, ApiError> {
    body.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::bad_request(name, format!("field `{name}` is required")))
}

fn colour(body: &Value, name: &'static str) -> Result<Rgba, ApiError> {
    let raw = text(body, name)?;
    Rgba::parse_hex(raw)
        .filter(|c| c.a == 255)
        .ok_or_else(|| ApiError::bad_request(name, format!("`{raw}` is not a #rrggbb colour")))
}

/// A draft from the picker: frame and accent, everything else derived.
fn draft(body: &Value, id: &str, source: &str) -> Result<ThemeSpec, ApiError> {
    let mut spec = ThemeSpec {
        id: id.to_string(),
        label: id.to_string(),
        builtin: false,
        source: source.to_string(),
        ..ThemeSpec::default()
    };
    spec.light.insert("frame", colour(body, "frame")?);
    spec.light.insert("accent", colour(body, "accent")?);
    Ok(spec)
}

fn set_theme(store: &Store, id: &str) -> Result<(), ApiError> {
    with_desk(store, |d| d.theme = Some(id.to_string()))
}

fn state_reply(store: &Store, extra: Value) -> Value {
    invalidate();
    let (body, _) = current(store);
    let mut out = (*body).clone();
    if let (Some(o), Some(e)) = (out.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    out
}

/// One `theme-*` workspace action. `open` is the OS opener.
pub fn write(
    store: &Store,
    action: &str,
    body: &Value,
    open: &mut dyn FnMut(&Path, &[&str]) -> std::io::Result<()>,
) -> Result<Value, ApiError> {
    let dir = themes_dir(store.root());
    match action {
        "theme-set" => {
            let id = text(body, "theme")?;
            file::find(&dir, id).map_err(|e| refused(&e))?;
            set_theme(store, id)?;
            Ok(state_reply(store, json!({ "ok": true })))
        }
        "theme-mode" => {
            let raw = text(body, "mode")?;
            let mode = Mode::parse(raw).ok_or_else(|| {
                ApiError::bad_request("mode", format!("`{raw}` is not claro, oscuro or sistema"))
            })?;
            with_desk(store, |d| d.mode = Some(mode.as_str().to_string()))?;
            Ok(state_reply(store, json!({ "ok": true })))
        }
        "theme-dock" => {
            let on = body
                .get("on")
                .and_then(Value::as_bool)
                .ok_or_else(|| ApiError::bad_request("on", "field `on` must be true or false"))?;
            // Stored only when off, so a desk that never chose stays default.
            with_desk(store, |d| d.dock_attention = (!on).then_some(false))?;
            Ok(state_reply(store, json!({ "ok": true })))
        }
        "theme-season" => {
            let raw = text(body, "season")?;
            let choice = Choice::parse(raw).ok_or_else(|| {
                ApiError::bad_request("season", format!("`{raw}` is not auto, on or off"))
            })?;
            // Stored only when not the default, like the Dock switch.
            with_desk(store, |d| {
                d.season = (choice != Choice::Auto).then(|| choice.as_str().to_string())
            })?;
            Ok(state_reply(store, json!({ "ok": true })))
        }
        "theme-decorations" => {
            let on = body
                .get("on")
                .and_then(Value::as_bool)
                .ok_or_else(|| ApiError::bad_request("on", "field `on` must be true or false"))?;
            with_desk(store, |d| d.decorations = (!on).then_some(false))?;
            Ok(state_reply(store, json!({ "ok": true })))
        }
        "theme-preview" => {
            let spec = draft(body, "personalizado", "custom theme")?;
            match resolve(&spec) {
                Ok(theme) => Ok(json!({ "ok": true, "theme": theme })),
                Err(e) => Ok(json!({ "ok": false, "refusal": e, "reason": e.to_string() })),
            }
        }
        "theme-save" => {
            let name = text(body, "name")?;
            let overwrite = body.get("overwrite").and_then(Value::as_bool) == Some(true);
            let path = dir.join(format!("{name}.toml"));
            let spec = draft(body, name, &path.display().to_string())?;
            let written = file::save(&dir, &spec, overwrite).map_err(|e| refused(&e))?;
            set_theme(store, name)?;
            Ok(state_reply(store, json!({ "ok": true, "path": written })))
        }
        "theme-import" => {
            let code = text(body, "code")?;
            let overwrite = body.get("overwrite").and_then(Value::as_bool) == Some(true);
            let mut spec = file::parse_share(code).map_err(|e| refused(&e))?;
            spec.source = dir.join(format!("{}.toml", spec.id)).display().to_string();
            let written = file::save(&dir, &spec, overwrite).map_err(|e| refused(&e))?;
            set_theme(store, &spec.id)?;
            Ok(state_reply(store, json!({ "ok": true, "path": written })))
        }
        "theme-open-folder" => {
            file::ensure_dir(&dir).map_err(|e| {
                ApiError::new(
                    500,
                    "store_failed",
                    format!("could not create {}: {e}", dir.display()),
                )
            })?;
            open(&dir, &[]).map_err(|e| {
                ApiError::new(500, "open_failed", format!("the OS opener failed: {e}"))
            })?;
            Ok(json!({ "ok": true, "path": dir }))
        }
        _ => Err(ApiError::new(
            404,
            "unknown_action",
            format!("no workspace action \"{action}\""),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harbard_core::workspace::DeskState;
    use std::sync::atomic::AtomicU64;

    static N: AtomicU64 = AtomicU64::new(0);
    /// The cache is process-wide; these tests take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    struct Home(PathBuf);
    impl Home {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "harbard-theme-route-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Home(p)
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn never(_: &Path, _: &[&str]) -> std::io::Result<()> {
        panic!("no opener in this test")
    }

    #[test]
    fn set_and_mode_write_the_desk_and_since_answers_unchanged() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let home = Home::new();
        let store = Store::at(&home.0);
        let first = theme_body(&store, None);
        assert_eq!(first["theme"], harbard_core::theme::DEFAULT_THEME);
        assert_eq!(first["mode"], "light");
        let rev = first["revision"].as_str().unwrap().to_string();
        assert_eq!(theme_body(&store, Some(rev.clone()))["unchanged"], true);

        let out = write(&store, "theme-set", &json!({ "theme": "jade" }), &mut never).unwrap();
        assert_eq!(out["theme"], "jade");
        write(
            &store,
            "theme-mode",
            &json!({ "mode": "system" }),
            &mut never,
        )
        .unwrap();
        let desk = DeskState::load_path(&DeskState::path(&home.0));
        assert_eq!(
            (desk.theme.as_deref(), desk.mode.as_deref()),
            (Some("jade"), Some("system"))
        );
        assert_ne!(theme_body(&store, Some(rev))["revision"], first["revision"]);

        let err = write(&store, "theme-set", &json!({ "theme": "nada" }), &mut never).unwrap_err();
        assert_eq!(err.status, 404);
        let err = write(
            &store,
            "theme-mode",
            &json!({ "mode": "night" }),
            &mut never,
        )
        .unwrap_err();
        assert_eq!(err.status, 400);
    }

    #[test]
    fn notify_en_el_dock_is_on_by_default_and_stored_with_the_theme() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let home = Home::new();
        let store = Store::at(&home.0);
        assert_eq!(theme_body(&store, None)["dock_attention"], true);
        let out = write(&store, "theme-dock", &json!({ "on": false }), &mut never).unwrap();
        assert_eq!(out["dock_attention"], false);
        let desk = DeskState::load_path(&DeskState::path(&home.0));
        assert_eq!(desk.dock_attention, Some(false));
        // The desktop shell reads the same key from the same file.
        let raw = std::fs::read_to_string(DeskState::path(&home.0)).unwrap();
        assert!(raw.contains("\"dock_attention\": false"), "{raw}");
        let out = write(&store, "theme-dock", &json!({ "on": true }), &mut never).unwrap();
        assert_eq!(out["dock_attention"], true);
        assert_eq!(
            DeskState::load_path(&DeskState::path(&home.0)).dock_attention,
            None
        );
        let err = write(&store, "theme-dock", &json!({ "on": "yes" }), &mut never).unwrap_err();
        assert_eq!(err.status, 400);
    }

    #[test]
    fn the_season_choice_and_decorations_are_stored_apart_from_the_theme() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let home = Home::new();
        let store = Store::at(&home.0);
        let first = theme_body(&store, None);
        assert_eq!(first["season"]["id"], "christmas");
        assert_eq!(first["season"]["choice"], "auto");
        assert_eq!(first["season"]["decorations"], true);

        write(
            &store,
            "theme-set",
            &json!({ "theme": "harbour" }),
            &mut never,
        )
        .unwrap();
        let out = write(
            &store,
            "theme-season",
            &json!({ "season": "on" }),
            &mut never,
        )
        .unwrap();
        assert_eq!(out["season"]["choice"], "on");
        assert_eq!(out["season"]["active"], true);
        assert_eq!(out["theme"], "harbour", "the choice is untouched");
        let out = write(
            &store,
            "theme-decorations",
            &json!({ "on": false }),
            &mut never,
        )
        .unwrap();
        assert_eq!(out["season"]["decorations"], false);
        let desk = DeskState::load_path(&DeskState::path(&home.0));
        assert_eq!(
            (
                desk.theme.as_deref(),
                desk.season.as_deref(),
                desk.decorations
            ),
            (Some("harbour"), Some("on"), Some(false))
        );

        let out = write(
            &store,
            "theme-season",
            &json!({ "season": "off" }),
            &mut never,
        )
        .unwrap();
        assert_eq!(out["season"]["active"], false);
        write(
            &store,
            "theme-season",
            &json!({ "season": "auto" }),
            &mut never,
        )
        .unwrap();
        write(
            &store,
            "theme-decorations",
            &json!({ "on": true }),
            &mut never,
        )
        .unwrap();
        let desk = DeskState::load_path(&DeskState::path(&home.0));
        assert_eq!(
            (desk.season, desk.decorations),
            (None, None),
            "defaults are not stored"
        );

        let err = write(
            &store,
            "theme-season",
            &json!({ "season": "always" }),
            &mut never,
        )
        .unwrap_err();
        assert_eq!(err.status, 400);
        let err = write(
            &store,
            "theme-decorations",
            &json!({ "on": "yes" }),
            &mut never,
        )
        .unwrap_err();
        assert_eq!(err.status, 400);
    }

    #[test]
    fn save_import_and_preview_go_through_the_same_checks() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let home = Home::new();
        let store = Store::at(&home.0);
        let ok = write(
            &store,
            "theme-preview",
            &json!({ "frame": "#12324a", "accent": "#2b7bb9" }),
            &mut never,
        )
        .unwrap();
        assert_eq!(ok["ok"], true);
        let low = write(
            &store,
            "theme-preview",
            &json!({ "frame": "#ffffff", "accent": "#fefefe" }),
            &mut never,
        )
        .unwrap();
        assert_eq!(low["ok"], false, "{low}");
        assert_eq!(low["refusal"]["code"], "low_contrast");

        let saved = write(
            &store,
            "theme-save",
            &json!({ "name": "mar", "frame": "#12324a", "accent": "#2b7bb9" }),
            &mut never,
        )
        .unwrap();
        assert_eq!(saved["theme"], "mar");
        assert!(home.0.join("themes/mar.toml").exists());
        assert!(home.0.join("themes/_example.toml").exists());

        let code = saved["themes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == "mar")
            .unwrap()["share"]
            .as_str()
            .unwrap()
            .replace(":mar:", ":mar-2:");
        let imported = write(&store, "theme-import", &json!({ "code": code }), &mut never).unwrap();
        assert_eq!(imported["theme"], "mar-2");
        let err = write(
            &store,
            "theme-import",
            &json!({ "code": "harbard-theme:v1:x:#000" }),
            &mut never,
        )
        .unwrap_err();
        assert_eq!(err.status, 422);
        assert_eq!(err.body()["refusal"]["code"], "bad_code");
    }

    #[test]
    fn open_folder_creates_it_and_hands_it_to_the_opener() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let home = Home::new();
        let store = Store::at(&home.0);
        let mut opened: Vec<PathBuf> = Vec::new();
        let mut open = |p: &Path, args: &[&str]| {
            assert!(args.is_empty());
            opened.push(p.to_path_buf());
            Ok(())
        };
        write(&store, "theme-open-folder", &json!({}), &mut open).unwrap();
        assert_eq!(opened, vec![home.0.join("themes")]);
        assert!(home.0.join("themes/_example.toml").exists());
    }
}
