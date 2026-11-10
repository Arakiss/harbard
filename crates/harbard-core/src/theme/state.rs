//! The operator's theme state: what `workspace.json` asks for, what the
//! themes folder holds, and what is actually applied.
//!
//! Shared by `harbard theme` and by serve's `/api/v1/theme`. Serve passes a
//! `last_good` map that outlives each read, so a theme file broken by an
//! edit keeps its previous tokens applied while the refusal is reported:
//! a theme is never applied half-way.

use super::file::{self, themes_dir};
use super::season::{self, SeasonState};
use super::zone;
use super::{DEFAULT_MODE, DEFAULT_THEME, Mode, ResolvedTheme, Shade, ThemeError, mode_layer};
use crate::coded::{Params, params};
use crate::profile::Profile;
use crate::workspace::DeskState;
use serde::Serialize;
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// One theme in the picker, with whether its tokens are a kept copy.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    #[serde(flatten)]
    pub theme: ResolvedTheme,
    /// The file is refused now; these are its last accepted tokens.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stored {
    pub theme: Option<String>,
    pub mode: Option<String>,
}

/// What the console needs to paint and to offer the picker.
#[derive(Debug, Clone, Serialize)]
pub struct State {
    pub version: u32,
    /// The applied theme id: the stored one, or the default when it cannot be.
    pub theme: String,
    pub mode: Mode,
    pub stored: Stored,
    pub themes: Vec<Entry>,
    /// Every refused file, and a bad stored mode or theme.
    pub errors: Vec<ThemeError>,
    /// Why `theme`/`mode` differ from what is stored, in one sentence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
    /// The console's code for `notice`, filled from `notice_params`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice_code: Option<&'static str>,
    #[serde(skip_serializing_if = "Params::is_empty")]
    pub notice_params: Params,
    pub dir: String,
    /// "Notify in the Dock", kept with the theme: on unless stored false.
    pub dock_attention: bool,
    /// The season, today or next: whether it paints now and its choice.
    /// While active the console paints `season.theme` over `theme`, which
    /// stays the operator's choice.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season: Option<SeasonState>,
    /// The mode layer, sent once for every theme.
    pub modes: BTreeMap<&'static str, BTreeMap<String, String>>,
}

pub const STATE_VERSION: u32 = 1;

/// Read the desk's choice and the folder, resolve everything.
pub fn read(root: &Path, last_good: &mut BTreeMap<String, ResolvedTheme>) -> State {
    read_at(root, last_good, crate::clock::now_ms())
}

/// [`read`] at `now_ms`: the season depends on the operator's local date.
pub fn read_at(root: &Path, last_good: &mut BTreeMap<String, ResolvedTheme>, now_ms: u64) -> State {
    let desk = DeskState::load_path(&DeskState::path(root));
    let profile = Profile::load(root);
    let today = zone::local_date(profile.time_zone.as_deref(), now_ms);
    let dir = themes_dir(root);
    let mut themes: Vec<Entry> = Vec::new();
    let mut errors: Vec<ThemeError> = Vec::new();
    if file::linked(&dir) {
        errors.push(file::linked_error(&dir));
    }
    for item in file::catalogue(&dir) {
        match item {
            Ok(theme) => {
                if !theme.builtin {
                    last_good.insert(theme.id.clone(), theme.clone());
                }
                themes.push(Entry {
                    theme,
                    stale: false,
                });
            }
            Err(err) => {
                let id = Path::new(&err.file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                if let Some(kept) = last_good.get(&id) {
                    themes.push(Entry {
                        theme: kept.clone(),
                        stale: true,
                    });
                }
                errors.push(err);
            }
        }
    }
    // A file that disappeared is no longer kept.
    last_good.retain(|id, _| themes.iter().any(|e| &e.theme.id == id));

    let mut notice = None;
    let mut notice_code = None;
    let mut notice_params = Params::new();
    let mode = match desk.mode.as_deref() {
        None => DEFAULT_MODE,
        Some(raw) => Mode::parse(raw).unwrap_or_else(|| {
            errors.push(
                ThemeError::new(
                    DeskState::path(root).display().to_string(),
                    "bad_mode",
                    format!("the mode `{raw}` does not exist"),
                    "use `harbard theme mode claro|oscuro|sistema`",
                )
                .coded(
                    "theme.bad_mode",
                    "theme.bad_mode.fix",
                    params([("mode", raw.into())]),
                )
                .key("mode"),
            );
            notice = Some(format!(
                "The saved mode \"{raw}\" does not exist; using {}.",
                DEFAULT_MODE.label()
            ));
            notice_code = Some("theme.notice.bad_mode");
            notice_params = params([("mode", raw.into())]);
            DEFAULT_MODE
        }),
    };
    let wanted = desk
        .theme
        .clone()
        .unwrap_or_else(|| DEFAULT_THEME.to_string());
    let theme = if themes.iter().any(|e| e.theme.id == wanted) {
        wanted
    } else {
        let refusal = errors.iter().position(|e| {
            Path::new(&e.file).file_stem().and_then(|s| s.to_str()) == Some(wanted.as_str())
        });
        let why = refusal.map_or_else(
            || "not in the themes folder".to_string(),
            |i| errors[i].message.clone(),
        );
        notice = Some(format!(
            "The theme \"{wanted}\" cannot be applied ({why}); using the default theme."
        ));
        let mut values = params([("theme", wanted.as_str().into()), ("why", why.into())]);
        notice_code = Some(match refusal {
            Some(i) => {
                values.insert("refusal".into(), i.into());
                "theme.notice.refused"
            }
            None => "theme.notice.absent",
        });
        notice_params = values;
        DEFAULT_THEME.to_string()
    };
    let mut modes = BTreeMap::new();
    modes.insert("light", mode_layer(Shade::Light));
    modes.insert("dark", mode_layer(Shade::Dark));
    State {
        version: STATE_VERSION,
        theme,
        mode,
        stored: Stored {
            theme: desk.theme,
            mode: desk.mode,
        },
        themes,
        errors,
        notice,
        notice_code,
        notice_params,
        dir: dir.display().to_string(),
        dock_attention: desk.dock_attention.unwrap_or(true),
        season: season::state(today, desk.season.as_deref(), desk.decorations),
        modes,
    }
}

/// How often the state is re-read with nothing changed on disk, so a season
/// starts within this of the operator's midnight. A quarter of an hour
/// lands on every zone's midnight (offsets are whole quarters).
pub const DATE_EVERY_MS: u64 = 15 * 60 * 1000;

/// Changes whenever `workspace.json`, `profile.json` (its time zone) or any
/// theme file changes, and every [`DATE_EVERY_MS`]: the cheap stat that
/// tells the watcher a re-read is due. Reads no file contents.
pub fn fingerprint(root: &Path) -> u64 {
    fingerprint_at(root, crate::clock::now_ms())
}

pub fn fingerprint_at(root: &Path, now_ms: u64) -> u64 {
    let mut h = DefaultHasher::new();
    (now_ms / DATE_EVERY_MS).hash(&mut h);
    let stat = |p: &Path, h: &mut DefaultHasher| {
        p.display().to_string().hash(h);
        if let Ok(meta) = std::fs::metadata(p) {
            meta.len().hash(h);
            meta.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .hash(h);
        }
    };
    stat(&DeskState::path(root), &mut h);
    stat(&Profile::path(root), &mut h);
    for path in file::theme_files(&themes_dir(root)) {
        stat(&path, &mut h);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn root() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "harbard-theme-state-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(p.join("themes")).unwrap();
        p
    }

    fn set(root: &Path, theme: Option<&str>, mode: Option<&str>) {
        let mut desk = DeskState::load_path(&DeskState::path(root));
        desk.theme = theme.map(str::to_string);
        desk.mode = mode.map(str::to_string);
        desk.save_path(&DeskState::path(root)).unwrap();
    }

    /// 2026-11-10 12:00 and 2026-12-24 12:00 UTC.
    const NOVEMBER: u64 = 1_794_312_000_000;
    const CHRISTMAS_EVE: u64 = 1_798_113_600_000;

    #[test]
    fn nothing_stored_is_the_light_default() {
        let r = root();
        let s = read_at(&r, &mut BTreeMap::new(), NOVEMBER);
        assert_eq!((s.theme.as_str(), s.mode), (DEFAULT_THEME, Mode::Light));
        assert!(s.stored.theme.is_none() && s.notice.is_none());
        let season = s.season.unwrap();
        assert_eq!((season.id, season.active), ("christmas", false));
        let _ = fs::remove_dir_all(&r);
    }

    /// A desk that stored the old id of Melanzane opens on Melanzane, with no
    /// notice: it is a rename, not a refusal.
    #[test]
    fn a_stored_renamed_theme_reads_as_its_new_id() {
        let r = root();
        fs::write(
            DeskState::path(&r),
            r#"{"theme":"aubergine","mode":"dark"}"#,
        )
        .unwrap();
        let s = read_at(&r, &mut BTreeMap::new(), NOVEMBER);
        assert_eq!((s.theme.as_str(), s.mode), ("melanzane", Mode::Dark));
        assert!(s.notice.is_none(), "{:?}", s.notice);
        assert!(s.themes.iter().all(|e| e.theme.id != "aubergine"));
        let _ = fs::remove_dir_all(&r);
    }

    /// The season paints over the choice without changing it, and the
    /// operator's choice of `on`, `off` and decorations is read from the desk.
    #[test]
    fn the_season_is_active_in_its_window_and_leaves_the_choice_alone() {
        let r = root();
        set(&r, Some("jade"), Some("dark"));
        let s = read_at(&r, &mut BTreeMap::new(), CHRISTMAS_EVE);
        assert_eq!(s.theme, "jade", "the choice stays");
        let season = s.season.clone().unwrap();
        assert!(season.active && season.decorations);
        assert!(s.themes.iter().any(|e| e.theme.id == season.theme));

        let mut desk = DeskState::load_path(&DeskState::path(&r));
        desk.season = Some("off".into());
        desk.decorations = Some(false);
        desk.save_path(&DeskState::path(&r)).unwrap();
        let s = read_at(&r, &mut BTreeMap::new(), CHRISTMAS_EVE);
        let season = s.season.unwrap();
        assert!(!season.active && season.in_window && !season.decorations);
        assert_eq!(s.theme, "jade");
        let _ = fs::remove_dir_all(&r);
    }

    /// The profile's zone decides the date: at 23:30 UTC on 30 November it is
    /// already December in Tokyo, still November in Los Angeles.
    #[test]
    fn the_profile_time_zone_decides_the_first_day() {
        if !Path::new("/usr/share/zoneinfo/Asia/Tokyo").exists() {
            return;
        }
        let r = root();
        let at = 1_796_081_400_000;
        fs::write(Profile::path(&r), r#"{"time_zone":"Asia/Tokyo"}"#).unwrap();
        assert!(read_at(&r, &mut BTreeMap::new(), at).season.unwrap().active);
        fs::write(Profile::path(&r), r#"{"time_zone":"America/Los_Angeles"}"#).unwrap();
        assert!(!read_at(&r, &mut BTreeMap::new(), at).season.unwrap().active);
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn a_broken_edit_keeps_the_previous_tokens_applied() {
        let r = root();
        let path = r.join("themes/mio.toml");
        fs::write(&path, "frame = \"#12324a\"\naccent = \"#2b7bb9\"\n").unwrap();
        set(&r, Some("mio"), Some("dark"));
        let mut kept = BTreeMap::new();
        let good = read(&r, &mut kept);
        assert_eq!(good.theme, "mio");
        let tokens = good
            .themes
            .iter()
            .find(|e| e.theme.id == "mio")
            .unwrap()
            .theme
            .clone();

        for broken in [
            "frame = \"#12324a\"\naccent = \"#2b7bb9\"\nsidebar_hover = \"#000\"\n",
            "frame = \"#12324\"\naccent = \"#2b7bb9\"\n",
            "frame = \"#3f0e40\"\naccent = \"#ffd000\"\naccent-ink = \"#ffffff\"\n",
        ] {
            fs::write(&path, broken).unwrap();
            let s = read(&r, &mut kept);
            assert_eq!(s.theme, "mio", "still applied");
            let entry = s.themes.iter().find(|e| e.theme.id == "mio").unwrap();
            assert!(entry.stale);
            assert_eq!(entry.theme.light, tokens.light);
            assert_eq!(s.errors.len(), 1, "{broken}");
        }
        // Without a kept copy (a fresh serve), the shell falls back and says why.
        let s = read(&r, &mut BTreeMap::new());
        assert_eq!(s.theme, DEFAULT_THEME);
        assert!(s.notice.unwrap().contains("mio"));
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn a_bad_stored_mode_is_reported_not_fatal() {
        let r = root();
        set(&r, Some("jade"), Some("night"));
        let s = read(&r, &mut BTreeMap::new());
        assert_eq!((s.theme.as_str(), s.mode), ("jade", Mode::Light));
        assert_eq!(s.errors[0].code, "bad_mode");
        let _ = fs::remove_dir_all(&r);
    }

    /// A themes folder linked into iCloud or ~/Documents would
    /// make serve, a launchd agent, read a TCC-protected place every
    /// second. A linked folder is refused, never followed.
    #[test]
    fn a_linked_themes_folder_is_refused_and_never_read() {
        let r = root();
        let elsewhere = r.join("synced-elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(
            elsewhere.join("mio.toml"),
            "frame = \"#12324a\"\naccent = \"#2b7bb9\"\n",
        )
        .unwrap();
        fs::remove_dir_all(r.join("themes")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, r.join("themes")).unwrap();

        let s = read(&r, &mut BTreeMap::new());
        assert!(s.themes.iter().all(|e| e.theme.id != "mio"), "not followed");
        assert!(
            s.errors.iter().any(|e| e.code == "linked_dir"),
            "{:?}",
            s.errors
        );
        let before = fingerprint(&r);
        fs::write(
            elsewhere.join("mio.toml"),
            "frame = \"#22324a\"\naccent = \"#2b7bb9\"\n",
        )
        .unwrap();
        fs::write(
            elsewhere.join("otro.toml"),
            "frame = \"#22324a\"\naccent = \"#2b7bb9\"\n",
        )
        .unwrap();
        assert_eq!(before, fingerprint(&r), "the watcher does not look inside");
        assert!(crate::theme::file::ensure_dir(&r.join("themes")).is_err());
        assert!(crate::theme::file::theme_files(&r.join("themes")).is_empty());
        let _ = fs::remove_dir_all(&r);
    }

    #[test]
    fn the_fingerprint_moves_with_the_desk_and_the_files() {
        let r = root();
        let a = fingerprint_at(&r, NOVEMBER);
        fs::write(r.join("themes/mio.toml"), "frame = \"#12324a\"\n").unwrap();
        let b = fingerprint_at(&r, NOVEMBER);
        assert_ne!(a, b);
        fs::write(r.join("themes/_example.toml"), "# doc").unwrap();
        assert_eq!(
            b,
            fingerprint_at(&r, NOVEMBER),
            "the example is not watched"
        );
        assert_ne!(
            b,
            fingerprint_at(&r, NOVEMBER + DATE_EVERY_MS),
            "the date is"
        );
        fs::write(Profile::path(&r), r#"{"time_zone":"Asia/Tokyo"}"#).unwrap();
        assert_ne!(b, fingerprint_at(&r, NOVEMBER), "so is the profile's zone");
        let b = fingerprint_at(&r, NOVEMBER);
        set(&r, Some("mio"), None);
        assert_ne!(b, fingerprint_at(&r, NOVEMBER));
        let _ = fs::remove_dir_all(&r);
    }
}
