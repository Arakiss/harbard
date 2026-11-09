//! Theme files and share strings.
//!
//! A custom theme is `{home}/themes/<name>.toml`: `frame` and `accent` at
//! least, any other theme key optionally, and an optional `[dark]` table of
//! overrides. The format is a strict subset of TOML (comments, one
//! `[dark]` header, `key = "#hex"` lines), parsed here so every refusal can
//! name the file, the line, the key and the fix.
//!
//! Files whose name starts with `_` are documentation and are not loaded:
//! that is where the generated example lives.
//!
//! A share string is one line, `harbard-theme:v1:<name>:<hex>,...`: the 19
//! light keys of [`THEME_KEYS`] in order, then the same 19 for dark, every
//! value resolved. Importing it writes a file that resolves to the same
//! tokens, whatever the derivation does in a later version.

use super::colour::Rgba;
use super::{
    KEY_DOCS, ResolvedTheme, THEME_KEYS, ThemeError, ThemeSpec, WASH_KEYS, builtin, builtins,
    is_builtin, resolve, theme_key,
};
use crate::coded::params;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SHARE_PREFIX: &str = "harbard-theme:v1:";
/// The `file` of a refusal that came from a pasted share code, not a path.
pub const SHARE_SOURCE: &str = "share code";
/// The generated example, documentation only.
pub const EXAMPLE_FILE: &str = "_example.toml";
/// The example's old name, kept only to adopt a file an older build wrote
/// (the operator may have edited it) instead of leaving two.
const OLD_EXAMPLE_FILE: &str = "_ejemplo.toml";
/// A theme file is a few hundred bytes; anything past this is not one.
const MAX_FILE_BYTES: u64 = 64 * 1024;
const NAME_MAX: usize = 40;

/// `{home}/themes`.
pub fn themes_dir(root: &Path) -> PathBuf {
    root.join("themes")
}

/// A custom theme name: lowercase ASCII letters, digits and hyphens.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_MAX
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
}

fn name_error(source: &str, name: &str) -> Option<ThemeError> {
    if !valid_name(name) {
        return Some(ThemeError::new(
            source,
            "bad_name",
            format!("`{name}` is not a valid theme name"),
            format!(
                "use unaccented lowercase letters, digits and hyphens, up to {NAME_MAX} characters, e.g. `my-theme`"
            ),
        )
        .coded(
            "theme.bad_name.invalid",
            "theme.bad_name.invalid.fix",
            params([("name", name.into()), ("max", NAME_MAX.into())]),
        ));
    }
    if is_builtin(name) {
        return Some(
            ThemeError::new(
                source,
                "bad_name",
                format!("`{name}` is the name of a built-in theme"),
                format!("choose another name, e.g. `my-{name}`"),
            )
            .coded(
                "theme.bad_name.builtin",
                "theme.bad_name.builtin.fix",
                params([("name", name.into())]),
            ),
        );
    }
    None
}

fn closest_key(raw: &str) -> Option<&'static str> {
    fn distance(a: &str, b: &str) -> usize {
        let b: Vec<char> = b.chars().collect();
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        for (i, ca) in a.chars().enumerate() {
            let mut cur = vec![i + 1];
            for (j, cb) in b.iter().enumerate() {
                let cost = usize::from(ca != *cb);
                cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
            }
            prev = cur;
        }
        prev[b.len()]
    }
    let normal = raw.replace('_', "-").to_lowercase();
    THEME_KEYS
        .iter()
        .copied()
        .map(|k| (distance(&normal, k), k))
        .filter(|(d, _)| *d <= 3)
        .min()
        .map(|(_, k)| k)
}

/// Strip a trailing `# comment` that sits outside the quoted value.
fn value_part(rest: &str) -> Option<(String, &str)> {
    let rest = rest.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = &rest[1..];
    let end = body.find(quote)?;
    let value = body[..end].to_string();
    let after = body[end + 1..].trim();
    Some((value, after))
}

fn alpha_error(source: &str, key: &'static str, value: &str) -> ThemeError {
    ThemeError::new(
        source,
        "bad_colour",
        format!("`{value}` has alpha and `{key}` is painted over the page"),
        "write `#rrggbb`; only `rail-active` and `sidebar-hover` accept `#rrggbbaa`",
    )
    .coded(
        "theme.bad_colour.alpha",
        "theme.bad_colour.alpha.fix",
        params([("value", value.into()), ("key", key.into())]),
    )
    .key(key)
}

/// Parse one theme file. `id` is the file stem; `source` names the file in
/// errors.
pub fn parse(id: &str, source: &str, text: &str) -> Result<ThemeSpec, ThemeError> {
    if let Some(err) = name_error(source, id) {
        return Err(err);
    }
    let mut spec = ThemeSpec {
        id: id.to_string(),
        label: id.to_string(),
        builtin: false,
        source: source.to_string(),
        ..ThemeSpec::default()
    };
    let mut dark = false;
    let mut seen_dark = false;
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') {
            let header = trimmed.split('#').next().unwrap_or_default().trim();
            if header == "[dark]" {
                if seen_dark {
                    return Err(ThemeError::new(
                        source,
                        "duplicate_key",
                        "the `[dark]` table appears twice",
                        "keep a single `[dark]` table at the end of the file",
                    )
                    .coded(
                        "theme.duplicate_key.dark_table",
                        "theme.duplicate_key.dark_table.fix",
                        params([]),
                    )
                    .at(line));
                }
                dark = true;
                seen_dark = true;
                continue;
            }
            return Err(ThemeError::new(
                source,
                "unknown_section",
                format!("a theme has no table `{header}`"),
                "the only table allowed is `[dark]`",
            )
            .coded(
                "theme.unknown_section",
                "theme.unknown_section.fix",
                params([("header", header.into())]),
            )
            .at(line));
        }
        let Some((key, rest)) = trimmed.split_once('=') else {
            return Err(ThemeError::new(
                source,
                "syntax",
                format!("cannot understand the line `{trimmed}`"),
                "each line is `key = \"#rrggbb\"`, a `#` comment or `[dark]`",
            )
            .coded(
                "theme.syntax.line",
                "theme.syntax.line.fix",
                params([("line", trimmed.into())]),
            )
            .at(line));
        };
        let key = key.trim().trim_matches('"');
        let Some(canonical) = theme_key(key) else {
            let (fix, fix_code, suggestion) = match closest_key(key) {
                Some(k) => (
                    format!("did you mean `{k}`? The valid keys are in {EXAMPLE_FILE}"),
                    "theme.unknown_key.fix_suggest",
                    k,
                ),
                None => (
                    format!("remove it; the valid keys are in {EXAMPLE_FILE}"),
                    "theme.unknown_key.fix",
                    "",
                ),
            };
            return Err(ThemeError::new(
                source,
                "unknown_key",
                format!("`{key}` is not a theme key"),
                fix,
            )
            .coded(
                "theme.unknown_key",
                fix_code,
                params([
                    ("key", key.into()),
                    ("suggestion", suggestion.into()),
                    ("example", EXAMPLE_FILE.into()),
                ]),
            )
            .at(line)
            .key(key));
        };
        let Some((value, after)) = value_part(rest) else {
            return Err(ThemeError::new(
                source,
                "bad_colour",
                format!("the value `{}` is not quoted", rest.trim()),
                format!("write `{canonical} = \"#rrggbb\"`"),
            )
            .coded(
                "theme.bad_colour.unquoted",
                "theme.bad_colour.unquoted.fix",
                params([("value", rest.trim().into()), ("key", canonical.into())]),
            )
            .at(line)
            .key(canonical));
        };
        if !after.is_empty() && !after.starts_with('#') {
            return Err(ThemeError::new(
                source,
                "syntax",
                format!("unexpected `{after}` after the value"),
                "keep only `key = \"#rrggbb\"` and, if you like, a `#` comment",
            )
            .coded(
                "theme.syntax.trailing",
                "theme.syntax.trailing.fix",
                params([("after", after.into())]),
            )
            .at(line)
            .key(canonical));
        }
        let Some(colour) = Rgba::parse_hex(value.trim()) else {
            return Err(ThemeError::new(
                source,
                "bad_colour",
                format!("`{value}` is not a hexadecimal colour"),
                "write `#rrggbb` (or `#rrggbbaa` with alpha), e.g. \"#3f0e40\"",
            )
            .coded(
                "theme.bad_colour.hex",
                "theme.bad_colour.hex.fix",
                params([("value", value.as_str().into())]),
            )
            .at(line)
            .key(canonical));
        };
        if colour.a != 255 && !WASH_KEYS.contains(&canonical) {
            return Err(alpha_error(source, canonical, &value).at(line));
        }
        let table = if dark {
            &mut spec.dark
        } else {
            &mut spec.light
        };
        if table.insert(canonical, colour).is_some() {
            return Err(ThemeError::new(
                source,
                "duplicate_key",
                format!("`{canonical}` appears twice"),
                "keep one line per key",
            )
            .coded(
                "theme.duplicate_key.key",
                "theme.duplicate_key.key.fix",
                params([("key", canonical.into())]),
            )
            .at(line)
            .key(canonical));
        }
    }
    for required in ["frame", "accent"] {
        if !spec.light.contains_key(required) {
            return Err(ThemeError::new(
                source,
                "missing_key",
                format!("`{required}` is missing and is required"),
                format!("add `{required} = \"#rrggbb\"` before `[dark]`"),
            )
            .coded(
                "theme.missing_key",
                "theme.missing_key.fix",
                params([("key", required.into())]),
            )
            .key(required));
        }
    }
    Ok(spec)
}

/// Read and parse `path`, refusing what is not a small regular file.
pub fn read(path: &Path) -> Result<ThemeSpec, ThemeError> {
    let source = path.display().to_string();
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    let meta = fs::symlink_metadata(path).map_err(|e| {
        ThemeError::new(
            &source,
            "read_failed",
            format!("the file cannot be read: {e}"),
            "check the path",
        )
        .coded(
            "theme.read_failed.read",
            "theme.read_failed.path.fix",
            params([("error", e.to_string().into())]),
        )
    })?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err(ThemeError::new(
            &source,
            "read_failed",
            "not a theme file (a link, a folder or too large)",
            "use a regular .toml file of a few hundred bytes",
        )
        .coded(
            "theme.read_failed.not_a_file",
            "theme.read_failed.not_a_file.fix",
            params([]),
        ));
    }
    let text = fs::read_to_string(path).map_err(|e| {
        ThemeError::new(
            &source,
            "read_failed",
            format!("the file cannot be read: {e}"),
            "save it as UTF-8 text",
        )
        .coded(
            "theme.read_failed.read",
            "theme.read_failed.encoding.fix",
            params([("error", e.to_string().into())]),
        )
    })?;
    parse(&id, &source, &text)
}

/// Parse and resolve a file: what `harbard theme check` and the watcher run.
pub fn load(path: &Path) -> Result<ResolvedTheme, ThemeError> {
    resolve(&read(path)?)
}

/// Whether the themes folder is a symbolic link. Such a folder is never
/// read: it would let serve (a launchd agent) reach into iCloud Drive or
/// ~/Documents, where macOS asks for permission.
pub fn linked(dir: &Path) -> bool {
    fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink())
}

/// The refusal for a linked themes folder.
pub fn linked_error(dir: &Path) -> ThemeError {
    ThemeError::new(
        dir.display().to_string(),
        "linked_dir",
        "the themes folder is a link and harbard does not follow it",
        "use a regular folder at that path; to sync, copy the .toml files or share the code",
    )
    .coded("theme.linked_dir", "theme.linked_dir.fix", params([]))
}

/// The theme files in `dir`, sorted by name, examples skipped. A missing
/// or linked folder is an empty list.
pub fn theme_files(dir: &Path) -> Vec<PathBuf> {
    if linked(dir) {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            name.ends_with(".toml") && !name.starts_with('_') && !name.starts_with('.')
        })
        .collect();
    out.sort();
    out
}

/// Built-ins, then every custom file: each resolved or refused.
pub fn catalogue(dir: &Path) -> Vec<Result<ResolvedTheme, ThemeError>> {
    let mut out: Vec<Result<ResolvedTheme, ThemeError>> = builtins().iter().map(resolve).collect();
    out.extend(theme_files(dir).iter().map(|p| load(p)));
    out
}

/// A theme by id: built-in, or `{dir}/<id>.toml`.
pub fn find(dir: &Path, id: &str) -> Result<ResolvedTheme, ThemeError> {
    if let Some(spec) = builtin(id) {
        return resolve(&spec);
    }
    if !valid_name(id) {
        return Err(name_error(id, id).unwrap_or_else(|| {
            ThemeError::new(id, "bad_name", "invalid name", "use `harbard theme list`").coded(
                "theme.bad_name.fallback",
                "theme.bad_name.fallback.fix",
                params([]),
            )
        }));
    }
    // The name as the folder lists it, not as the volume resolves it: on a
    // case-insensitive volume `mar.toml` "exists" when the file is
    // `Mar.toml`, which the catalogue refuses.
    let wanted = format!("{id}.toml");
    let path = dir.join(&wanted);
    let listed = theme_files(dir);
    if !listed
        .iter()
        .any(|p| p.file_name().and_then(|n| n.to_str()) == Some(wanted.as_str()))
    {
        let near = listed
            .iter()
            .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
            .find(|n| n.eq_ignore_ascii_case(&wanted));
        let fix = near.map_or_else(
            || "see the names with `harbard theme list`".to_string(),
            |n| format!("the file is `{n}`: rename it to `{wanted}`"),
        );
        let fix_code = if near.is_some() {
            "theme.missing_theme.fix_case"
        } else {
            "theme.missing_theme.fix"
        };
        return Err(ThemeError::new(
            path.display().to_string(),
            "missing_theme",
            format!("there is no theme `{id}`"),
            fix,
        )
        .coded(
            "theme.missing_theme",
            fix_code,
            params([
                ("id", id.into()),
                ("file", near.unwrap_or_default().into()),
                ("wanted", wanted.as_str().into()),
            ]),
        ));
    }
    load(&path)
}

// ---------------------------------------------------------------------------
// Share strings
// ---------------------------------------------------------------------------

/// `harbard-theme:v1:<id>:<19 light hex>,<19 dark hex>`.
pub fn share_string(theme: &ResolvedTheme) -> String {
    let hex = |tokens: &BTreeMap<String, String>, key: &str| {
        tokens
            .get(key)
            .and_then(|v| Rgba::parse_css(v))
            .map(|c| c.hex())
            .unwrap_or_default()
    };
    let values: Vec<String> = THEME_KEYS
        .iter()
        .map(|k| hex(&theme.light, k))
        .chain(THEME_KEYS.iter().map(|k| hex(&theme.dark, k)))
        .collect();
    format!("{SHARE_PREFIX}{}:{}", theme.id, values.join(","))
}

/// A share string back to a spec with every key written out.
pub fn parse_share(code: &str) -> Result<ThemeSpec, ThemeError> {
    let code = code.trim();
    let bad = |message: String, code: &'static str, values: crate::coded::Params| {
        ThemeError::new(
            SHARE_SOURCE,
            "bad_code",
            message,
            "copy the whole code with \"Copy code to share\"; it starts with harbard-theme:v1:",
        )
        .coded(code, "theme.bad_code.fix", values)
    };
    let Some(rest) = code.strip_prefix(SHARE_PREFIX) else {
        return Err(bad(
            "does not start with `harbard-theme:v1:`".into(),
            "theme.bad_code.prefix",
            params([]),
        ));
    };
    let Some((name, list)) = rest.split_once(':') else {
        return Err(bad(
            "the theme name is missing".into(),
            "theme.bad_code.name",
            params([]),
        ));
    };
    // A built-in's own code parses (the round trip); writing it under that
    // name is refused by [`save`].
    if !valid_name(name)
        && let Some(err) = name_error(SHARE_SOURCE, name)
    {
        return Err(err);
    }
    let values: Vec<&str> = list.split(',').map(str::trim).collect();
    if values.len() != THEME_KEYS.len() * 2 {
        return Err(bad(
            format!(
                "it has {} colours and a v1 code carries {}",
                values.len(),
                THEME_KEYS.len() * 2
            ),
            "theme.bad_code.count",
            params([
                ("count", values.len().into()),
                ("expected", (THEME_KEYS.len() * 2).into()),
            ]),
        ));
    }
    let mut spec = ThemeSpec {
        id: name.to_string(),
        label: name.to_string(),
        builtin: false,
        source: SHARE_SOURCE.into(),
        ..ThemeSpec::default()
    };
    for (i, raw) in values.iter().enumerate() {
        let key = THEME_KEYS[i % THEME_KEYS.len()];
        let colour = Rgba::parse_hex(raw).ok_or_else(|| {
            bad(
                format!(
                    "`{raw}` (position {}, `{key}`) is not a hexadecimal colour",
                    i + 1
                ),
                "theme.bad_code.colour",
                params([
                    ("value", (*raw).into()),
                    ("position", (i + 1).into()),
                    ("key", key.into()),
                ]),
            )
            .key(key)
        })?;
        if colour.a != 255 && !WASH_KEYS.contains(&key) {
            let mut err = alpha_error(SHARE_SOURCE, key, raw);
            err.message = format!("{} (position {})", err.message, i + 1);
            err.message_code = "theme.bad_code.alpha";
            err.params.insert("position".into(), (i + 1).into());
            return Err(err);
        }
        if i < THEME_KEYS.len() {
            spec.light.insert(key, colour);
        } else {
            spec.dark.insert(key, colour);
        }
    }
    Ok(spec)
}

// ---------------------------------------------------------------------------
// Writing files
// ---------------------------------------------------------------------------

/// The header every generated file carries. Documents every key.
pub fn header() -> String {
    let mut out = String::from(
        "# A harbard theme. It applies a few seconds after saving, with no reload.\n\
         #\n\
         # The theme's name is the file's (lowercase letters, digits and hyphens).\n\
         # Required: `frame` and `accent`. The rest are optional: when missing,\n\
         # harbard derives them from those two. Colours in hexadecimal, \"#rrggbb\",\n\
         # or \"#rrggbbaa\" with alpha. The optional [dark] table changes keys in\n\
         # dark mode only; what it leaves out is derived for dark.\n\
         #\n\
         # A theme that does not read well (under 4.5:1 for text, 3:1 for icons and\n\
         # presence dots) is refused whole, with the pair and the contrast, and\n\
         # the previous one stays applied. Check it with `harbard theme check`.\n\
         #\n\
         # Keys:\n",
    );
    for (key, doc) in KEY_DOCS {
        out.push_str(&format!("#   {key:<16} {doc}\n"));
    }
    out.push_str("#\n");
    out
}

/// The text of a theme file for `spec`: header, light keys, `[dark]`.
pub fn to_toml(spec: &ThemeSpec) -> String {
    let mut out = header();
    for key in THEME_KEYS {
        if let Some(c) = spec.light.get(key) {
            out.push_str(&format!("{key} = \"{}\"\n", c.hex()));
        }
    }
    if !spec.dark.is_empty() {
        out.push_str("\n[dark]\n");
        for key in THEME_KEYS {
            if let Some(c) = spec.dark.get(key) {
                out.push_str(&format!("{key} = \"{}\"\n", c.hex()));
            }
        }
    }
    out
}

/// The example: a minimal theme with every optional key commented out at
/// Aubergine's value.
pub fn example() -> String {
    let aubergine = builtin("aubergine").expect("aubergine");
    let mut out = header();
    out.push_str("# This file starts with _ and is not loaded: copy it as my-theme.toml.\n\n");
    for key in THEME_KEYS {
        let value = aubergine.light.get(key).map(Rgba::hex).unwrap_or_default();
        if key == "frame" || key == "accent" {
            out.push_str(&format!("{key} = \"{value}\"\n"));
        } else {
            out.push_str(&format!("# {key} = \"{value}\"\n"));
        }
    }
    out.push_str("\n# [dark]\n# frame = \"#2c0b2d\"\n# accent = \"#8a3a95\"\n");
    out
}

/// Create `dir` and the example in it when missing. Returns the folder.
pub fn ensure_dir(dir: &Path) -> std::io::Result<PathBuf> {
    if linked(dir) {
        return Err(std::io::Error::other(linked_error(dir).to_string()));
    }
    fs::create_dir_all(dir)?;
    let sample = dir.join(EXAMPLE_FILE);
    let old_sample = dir.join(OLD_EXAMPLE_FILE);
    if !sample.exists() && old_sample.is_file() {
        fs::rename(&old_sample, &sample)?;
    }
    if !sample.exists() {
        write_atomic(&sample, &example())?;
    }
    Ok(dir.to_path_buf())
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

/// Write `spec` as `{dir}/<id>.toml` after resolving it (a refused theme
/// is never written). An existing different file is refused unless
/// `overwrite`.
pub fn save(dir: &Path, spec: &ThemeSpec, overwrite: bool) -> Result<PathBuf, ThemeError> {
    let path = dir.join(format!("{}.toml", spec.id));
    let source = path.display().to_string();
    if let Some(err) = name_error(&source, &spec.id) {
        return Err(err);
    }
    let mut checked = spec.clone();
    checked.source = source.clone();
    resolve(&checked)?;
    let text = to_toml(spec);
    if path.exists() && !overwrite {
        let same = fs::read_to_string(&path)
            .map(|t| t == text)
            .unwrap_or(false);
        if !same {
            return Err(ThemeError::new(
                &source,
                "exists",
                format!("there is already a theme `{}`", spec.id),
                "choose another name or overwrite it (`--force` in the CLI)",
            )
            .coded(
                "theme.exists",
                "theme.exists.fix",
                params([("id", spec.id.as_str().into())]),
            ));
        }
    }
    let io = |e: std::io::Error| {
        ThemeError::new(
            &source,
            "read_failed",
            format!("could not write: {e}"),
            "check the permissions of the themes folder",
        )
        .coded(
            "theme.read_failed.write",
            "theme.read_failed.write.fix",
            params([("error", e.to_string().into())]),
        )
    };
    ensure_dir(dir).map_err(io)?;
    write_atomic(&path, &text).map_err(io)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "harbard-themes-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
        fn write(&self, name: &str, text: &str) -> PathBuf {
            let p = self.0.join(name);
            fs::write(&p, text).unwrap();
            p
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_minimal_file_with_frame_and_accent_loads() {
        let dir = Dir::new();
        let p = dir.write(
            "mar.toml",
            "frame = \"#12324a\"\naccent = '#2b7bb9' # azul\n",
        );
        let theme = load(&p).expect("loads");
        assert_eq!(theme.id, "mar");
        assert_eq!(theme.light["frame"], "#12324a");
        assert_eq!(theme.light["accent"], "#2b7bb9");
        assert!(theme.dark.contains_key("sidebar"), "dark derived");
    }

    #[test]
    fn an_unknown_key_is_refused_with_the_file_line_key_and_fix() {
        let dir = Dir::new();
        let p = dir.write(
            "x.toml",
            "frame = \"#12324a\"\naccent = \"#2b7bb9\"\nsidebar_hover = \"#000000\"\n",
        );
        let err = load(&p).unwrap_err();
        assert_eq!(err.code, "unknown_key");
        assert_eq!(err.line, Some(3));
        assert_eq!(err.key.as_deref(), Some("sidebar_hover"));
        assert!(err.file.ends_with("x.toml"));
        assert!(err.fix.contains("sidebar-hover"), "{err}");
        assert!(
            err.to_string().contains("x.toml:3: `sidebar_hover`"),
            "{err}"
        );
    }

    #[test]
    fn a_bad_hex_is_refused_with_the_key() {
        let dir = Dir::new();
        let p = dir.write("x.toml", "frame = \"#12324\"\naccent = \"#2b7bb9\"\n");
        let err = load(&p).unwrap_err();
        assert_eq!(
            (err.code, err.line, err.key.as_deref()),
            ("bad_colour", Some(1), Some("frame"))
        );
        assert!(err.fix.contains("#rrggbb"));
    }

    #[test]
    fn a_low_contrast_file_is_refused_with_the_pair_and_ratio() {
        let dir = Dir::new();
        let p = dir.write(
            "x.toml",
            "frame = \"#3f0e40\"\naccent = \"#ffd000\"\naccent-ink = \"#ffffff\"\n",
        );
        let err = load(&p).unwrap_err();
        assert_eq!(err.code, "low_contrast");
        assert_eq!(err.key.as_deref(), Some("accent-ink"));
        assert!(err.message.contains("`accent-ink` on `accent`"), "{err}");
        assert!(err.message.contains("needs 4.5:1"), "{err}");
    }

    #[test]
    fn missing_required_keys_and_stray_tables_are_refused() {
        let dir = Dir::new();
        let p = dir.write("x.toml", "frame = \"#12324a\"\n");
        assert_eq!(load(&p).unwrap_err().code, "missing_key");
        let p = dir.write(
            "y.toml",
            "frame = \"#12324a\"\naccent = \"#2b7bb9\"\n[light]\n",
        );
        assert_eq!(load(&p).unwrap_err().code, "unknown_section");
        let p = dir.write(
            "z.toml",
            "frame = \"#12324a\"\nframe = \"#12324a\"\naccent = \"#2b7bb9\"\n",
        );
        assert_eq!(load(&p).unwrap_err().code, "duplicate_key");
        let p = dir.write("jade.toml", "frame = \"#12324a\"\naccent = \"#2b7bb9\"\n");
        assert_eq!(load(&p).unwrap_err().code, "bad_name");
    }

    #[test]
    fn dark_overrides_apply_only_in_dark() {
        let dir = Dir::new();
        let p = dir.write(
            "night.toml",
            "frame = \"#12324a\"\naccent = \"#2b7bb9\"\n\n[dark]\nsidebar = \"#101418\"\n",
        );
        let theme = load(&p).unwrap();
        assert_eq!(theme.dark["sidebar"], "#101418");
        assert_ne!(theme.light["sidebar"], "#101418");
    }

    /// `.hb` paints the frame straight over the console's dark
    /// body (#181b1f), so a translucent base measured over white could pass
    /// and paint unreadable. Alpha is for the two washes only.
    #[test]
    fn a_translucent_base_colour_is_refused_in_files_and_codes() {
        let err = parse(
            "hueco",
            "hueco.toml",
            "frame = \"#ffffff00\"\naccent = \"#0088ff\"\n",
        )
        .and_then(|spec| resolve(&spec).map(|_| spec))
        .unwrap_err();
        assert_eq!(
            (err.code, err.key.as_deref(), err.line),
            ("bad_colour", Some("frame"), Some(1))
        );
        assert!(
            err.fix.contains("rail-active") && err.fix.contains("sidebar-hover"),
            "{err}"
        );
        let ok = parse(
            "lavado",
            "lavado.toml",
            "frame = \"#12324a\"\naccent = \"#2b7bb9\"\nsidebar-hover = \"#00000014\"\n",
        );
        assert!(ok.is_ok(), "a wash may carry alpha");
        let code = resolve(&builtin("jade").unwrap())
            .unwrap()
            .share
            .replace(":jade:", ":hueco:");
        let first = code
            .split(':')
            .nth(3)
            .unwrap()
            .split(',')
            .next()
            .unwrap()
            .to_string();
        let code = code.replacen(&first, "#ffffff00", 1);
        assert_eq!(
            parse_share(&code).unwrap_err().key.as_deref(),
            Some("frame")
        );
    }

    /// On a case-insensitive volume `mar.toml` "exists" when the
    /// file is `Mar.toml`, which the catalogue refuses. `find` must agree
    /// with the catalogue.
    #[test]
    fn find_matches_the_file_name_exactly_like_the_catalogue() {
        let dir = Dir::new();
        dir.write("Mar.toml", "frame = \"#12324a\"\naccent = \"#2b7bb9\"\n");
        let err = find(&dir.0, "mar").unwrap_err();
        assert_eq!(err.code, "missing_theme", "{err}");
        assert!(err.fix.contains("Mar.toml"), "names the file it saw: {err}");
        dir.write("lago.toml", "frame = \"#12324a\"\naccent = \"#2b7bb9\"\n");
        assert_eq!(find(&dir.0, "lago").unwrap().id, "lago");
    }

    #[test]
    fn the_share_string_round_trips_to_the_same_tokens() {
        for spec in builtins() {
            let theme = resolve(&spec).unwrap();
            let mut imported = parse_share(&theme.share).expect("parses");
            assert_eq!(imported.id, spec.id);
            imported.builtin = true;
            imported.label = spec.label.clone();
            imported.source = spec.source.clone();
            let again = resolve(&imported).expect("resolves");
            assert_eq!(again.light, theme.light, "{}", spec.id);
            assert_eq!(again.dark, theme.dark, "{}", spec.id);
            assert_eq!(again.share, theme.share);
        }
    }

    #[test]
    fn an_imported_code_written_to_a_file_loads_back_the_same_tokens() {
        let dir = Dir::new();
        let source = resolve(&builtin("jade").unwrap()).unwrap();
        let code = source.share.replace(":jade:", ":mi-jade:");
        let spec = parse_share(&code).unwrap();
        let path = save(&dir.0, &spec, false).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.light, source.light);
        assert_eq!(loaded.dark, source.dark);
        assert_eq!(loaded.share, code);
    }

    #[test]
    fn broken_share_strings_are_refused() {
        for code in [
            "harbard-theme:v2:x:#000000",
            "harbard-theme:v1:x:#000000,#ffffff",
            "harbard-theme:v1:Mi Tema:#000000",
        ] {
            assert!(parse_share(code).is_err(), "{code}");
        }
        let good = resolve(&builtin("lagoon").unwrap()).unwrap().share;
        let broken = good.replacen("#", "#zz", 1).replace(":lagoon:", ":otra:");
        assert_eq!(parse_share(&broken).unwrap_err().code, "bad_code");
    }

    #[test]
    fn the_example_documents_every_key_and_is_never_loaded() {
        let dir = Dir::new();
        ensure_dir(&dir.0).unwrap();
        let text = fs::read_to_string(dir.0.join(EXAMPLE_FILE)).unwrap();
        for key in THEME_KEYS {
            assert!(text.contains(&format!("#   {key} ")), "{key} documented");
        }
        assert!(theme_files(&dir.0).is_empty(), "the example is skipped");
        // Its body is a valid minimal theme once copied.
        let copy = dir.write("copia.toml", &text);
        load(&copy).expect("the example parses");
    }

    #[test]
    fn save_refuses_to_overwrite_a_different_file() {
        let dir = Dir::new();
        let mut spec = parse(
            "mio",
            "mio.toml",
            "frame = \"#12324a\"\naccent = \"#2b7bb9\"\n",
        )
        .unwrap();
        save(&dir.0, &spec, false).unwrap();
        save(&dir.0, &spec, false).expect("same content is fine");
        spec.light
            .insert("accent", Rgba::parse_hex("#1f6f9f").unwrap());
        assert_eq!(save(&dir.0, &spec, false).unwrap_err().code, "exists");
        save(&dir.0, &spec, true).expect("overwrite");
    }
}
