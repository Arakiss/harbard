//! Bundled files: the app's own (fonts, robots, avatars, dictionaries) first,
//! then GPUI Kit's Lucide catalog (`icons/<name>.svg`).

use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[exclude = "*.txt"]
struct Local;

pub struct Assets;

impl Assets {
    /// One of the app's own files.
    pub fn local(path: &str) -> Option<Cow<'static, [u8]>> {
        Local::get(path).map(|f| f.data)
    }

    /// The app's own files under `prefix`.
    pub fn local_paths(prefix: &str) -> Vec<String> {
        Local::iter()
            .filter(|p| p.starts_with(prefix))
            .map(|p| p.to_string())
            .collect()
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        if let Some(data) = Self::local(path) {
            return Ok(Some(data));
        }
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out: Vec<SharedString> = Self::local_paths(path)
            .into_iter()
            .map(Into::into)
            .collect();
        out.extend(gpui_kit::assets::AllAssets.list(path)?);
        Ok(out)
    }
}

/// Register the bundled faces: Lato (the workspace), IBM Plex Mono (code)
/// and Schibsted Grotesk (headings outside the workspace).
pub fn load_fonts(cx: &gpui::App) {
    let fonts: Vec<Cow<'static, [u8]>> = Assets::local_paths("fonts/")
        .iter()
        .filter(|p| p.ends_with(".ttf"))
        .filter_map(|p| Assets::local(p))
        .collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("harbard: could not load the bundled fonts: {e}");
    }
}
