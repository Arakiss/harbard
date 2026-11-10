//! Which native features may touch the operator's desktop this run.
//!
//! The menu-bar item, the summon shortcut, the Dock tile, OS notifications
//! and the single-instance socket are shared with every other app on the
//! Mac. They are live only when the binary runs from inside a `.app` bundle
//! (the installed app), or when `HARBARD_APP_NATIVE` asks for them in a dev
//! build:
//!
//! - `HARBARD_APP_NATIVE=1` (or `all`): every feature;
//! - `HARBARD_APP_NATIVE=tray,dock`: only those, to try one at a time.
//!   Names: `single`, `tray`, `hotkey`, `dock`, `notify`.
//!
//! Anywhere else they are inert: notifications are written to stderr as
//! `would notify: …` and nothing else happens.

use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    SingleInstance,
    Tray,
    Hotkey,
    Dock,
    Notify,
}

impl Feature {
    pub const ALL: [Feature; 5] = [
        Feature::SingleInstance,
        Feature::Tray,
        Feature::Hotkey,
        Feature::Dock,
        Feature::Notify,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Feature::SingleInstance => "single",
            Feature::Tray => "tray",
            Feature::Hotkey => "hotkey",
            Feature::Dock => "dock",
            Feature::Notify => "notify",
        }
    }
}

/// What this run may do, decided once at launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gate {
    /// Running from inside a `.app` bundle.
    pub bundled: bool,
    enabled: [bool; 5],
}

impl Gate {
    /// Every feature from a bundle; from a dev build only what
    /// `HARBARD_APP_NATIVE` names.
    pub fn decide(bundled: bool, native_env: Option<&str>) -> Gate {
        let mut enabled = [bundled; 5];
        if !bundled && let Some(raw) = native_env {
            let raw = raw.trim().to_ascii_lowercase();
            let all = raw == "1" || raw == "all" || raw == "true";
            for (i, f) in Feature::ALL.iter().enumerate() {
                enabled[i] = all || raw.split(',').any(|w| w.trim() == f.name());
            }
        }
        Gate { bundled, enabled }
    }

    pub fn allows(&self, f: Feature) -> bool {
        self.enabled[Feature::ALL.iter().position(|x| *x == f).unwrap_or(0)]
    }

    /// A one-line summary for the launch log.
    pub fn describe(&self) -> String {
        let on: Vec<&str> = Feature::ALL
            .iter()
            .filter(|f| self.allows(**f))
            .map(|f| f.name())
            .collect();
        format!(
            "{} · native features: {}",
            if self.bundled { "bundled" } else { "dev build" },
            if on.is_empty() {
                "none (inert)".to_string()
            } else {
                on.join(", ")
            }
        )
    }
}

/// The run's gate.
pub fn gate() -> Gate {
    static GATE: OnceLock<Gate> = OnceLock::new();
    *GATE.get_or_init(|| {
        Gate::decide(
            in_app_bundle(),
            std::env::var("HARBARD_APP_NATIVE").ok().as_deref(),
        )
    })
}

pub fn allows(f: Feature) -> bool {
    gate().allows(f)
}

/// Whether the executable sits in `<Name>.app/Contents/MacOS/`.
pub fn in_app_bundle() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok())
        .is_some_and(|exe| bundle_of(&exe))
}

fn bundle_of(exe: &std::path::Path) -> bool {
    let mut parts = exe.ancestors().skip(1);
    let macos = parts.next();
    let contents = parts.next();
    let app = parts.next();
    macos
        .and_then(|p| p.file_name())
        .is_some_and(|n| n == "MacOS")
        && contents
            .and_then(|p| p.file_name())
            .is_some_and(|n| n == "Contents")
        && app.and_then(|p| p.extension()).is_some_and(|e| e == "app")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_dev_build_is_inert_unless_asked() {
        let g = Gate::decide(false, None);
        assert!(Feature::ALL.iter().all(|f| !g.allows(*f)));
        let g = Gate::decide(false, Some(""));
        assert!(Feature::ALL.iter().all(|f| !g.allows(*f)));
        let g = Gate::decide(false, Some("0"));
        assert!(Feature::ALL.iter().all(|f| !g.allows(*f)));
    }

    #[test]
    fn the_env_turns_on_everything_or_a_named_few() {
        let g = Gate::decide(false, Some("1"));
        assert!(Feature::ALL.iter().all(|f| g.allows(*f)));
        let g = Gate::decide(false, Some("tray, dock"));
        assert!(g.allows(Feature::Tray) && g.allows(Feature::Dock));
        assert!(
            !g.allows(Feature::Hotkey)
                && !g.allows(Feature::Notify)
                && !g.allows(Feature::SingleInstance)
        );
    }

    #[test]
    fn a_bundle_has_everything() {
        let g = Gate::decide(true, None);
        assert!(Feature::ALL.iter().all(|f| g.allows(*f)));
    }

    #[test]
    fn only_contents_macos_inside_a_dot_app_is_a_bundle() {
        assert!(bundle_of(Path::new(
            "/Applications/harbard.app/Contents/MacOS/harbard-desktop"
        )));
        assert!(!bundle_of(Path::new("/tmp/target/debug/harbard-app")));
        assert!(!bundle_of(Path::new("/tmp/x.app/MacOS/harbard-app")));
        assert!(!bundle_of(Path::new("/tmp/Contents/MacOS/harbard-app")));
    }
}
