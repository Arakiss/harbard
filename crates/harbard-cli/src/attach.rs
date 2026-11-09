//! `harbard attach <path>... [--label before|after|report|code]`: copy
//! files an agent wants the operator to see into the harbard home, so the
//! console can show them.
//!
//! The copy lands in `{home}/attachments/<ulid>/`, with a `manifest.json`
//! naming each file and its label. The command prints the copied paths; an
//! agent cites them in its message, and the console renders them: images
//! inline, an `before`/`after` pair side by side, Markdown in a pane, a
//! diff with its colours, a log in monospace. HTML is copied too but never
//! rendered; it opens with the OS like any file card.
//!
//! This is what keeps serve away from the operator's private folders: serve
//! only ever reads attachments from this folder (`serve/workspace_route.rs`),
//! so a screenshot an agent left in `~/Documents` is never read by serve.
//! The agent's own CLI reads the source, with the agent's own permissions.
//!
//! Refused: a symlink, anything that is not a regular file, a type outside
//! [`TYPES`], a file over [`MAX_BYTES`], and a `.huginn` view that breaks the
//! catalog ([`crate::huginn`]): the agent hears about it here, while it can
//! still fix it, not from an operator who sees a broken view.

use crate::huginn;
use harbard_core::Store;
use harbard_core::safe_fs;
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

/// Largest file `attach` copies: 25 MB.
pub const MAX_BYTES: u64 = 25 * 1024 * 1024;
/// The folder's own manifest. A leading dot: [`valid_name`] never accepts
/// it, so an attached `manifest.json` can never be overwritten by it (L1).
pub const MANIFEST: &str = ".manifest.json";
/// Largest manifest read back.
pub const MANIFEST_MAX: u64 = 64 * 1024;
/// Longest file name kept.
const NAME_MAX: usize = 100;

/// Allowed extensions: `(extension, kind, content type served)`. `html` is
/// copied but never served; the console opens it with the OS.
pub const TYPES: [(&str, &str, &str); 13] = [
    ("png", "image", "image/png"),
    ("jpg", "image", "image/jpeg"),
    ("jpeg", "image", "image/jpeg"),
    ("webp", "image", "image/webp"),
    ("gif", "image", "image/gif"),
    ("md", "markdown", "text/plain; charset=utf-8"),
    ("html", "html", ""),
    ("txt", "text", "text/plain; charset=utf-8"),
    ("json", "json", "text/plain; charset=utf-8"),
    ("diff", "diff", "text/plain; charset=utf-8"),
    ("patch", "diff", "text/plain; charset=utf-8"),
    ("log", "text", "text/plain; charset=utf-8"),
    ("huginn", "huginn", "text/plain; charset=utf-8"),
];

/// The labels `--label` takes, as they are stored on each attachment.
pub const LABELS: [&str; 4] = ["before", "after", "report", "code"];

/// `{home}/attachments`.
pub fn root(store: &Store) -> PathBuf {
    store.root().join("attachments")
}

/// The kind and content type of a file name, by its extension.
pub fn kind_of(name: &str) -> Option<(&'static str, &'static str)> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    TYPES
        .iter()
        .find(|(e, _, _)| *e == ext)
        .map(|(_, k, c)| (*k, *c))
}

/// A label as typed or as an old manifest stored it, in its canonical form.
pub fn label_of(raw: &str) -> Option<&'static str> {
    // `attachment_label` exists only to read labels older builds stored.
    match harbard_core::old_spelling::attachment_label(raw.trim().to_lowercase().as_str()) {
        "before" => Some("before"),
        "after" => Some("after"),
        "report" => Some("report"),
        "code" => Some("code"),
        _ => None,
    }
}

/// True for a name `attach` could have written: ASCII letters, digits, `.`,
/// `_` and `-`, not starting with a dot, with an allowed extension.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_MAX
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && kind_of(name).is_some()
}

/// True for an attachment folder name: a ULID.
pub fn valid_dir(dir: &str) -> bool {
    ulid::Ulid::from_string(dir).is_ok() && dir.len() == 26
}

/// The source's file name, reduced to what [`valid_name`] accepts.
fn safe_name(src: &Path) -> Option<String> {
    let raw = src.file_name()?.to_string_lossy().into_owned();
    let mut name: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    while name.starts_with('.') {
        name.remove(0);
    }
    let (stem, ext) = name.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    let stem: String = stem.chars().take(NAME_MAX - ext.len() - 1).collect();
    let stem = if stem.is_empty() { "file".into() } else { stem };
    let name = format!("{stem}.{ext}");
    valid_name(&name).then_some(name)
}

/// One file copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    pub path: PathBuf,
    pub name: String,
    pub kind: &'static str,
    pub bytes: u64,
}

/// Why a file was refused, in words the agent can quote to the operator.
fn check(src: &Path) -> Result<(String, &'static str, fs::Metadata), String> {
    let shown = src.display();
    let meta = fs::symlink_metadata(src).map_err(|e| format!("{shown}: cannot be read ({e})"))?;
    if meta.file_type().is_symlink() {
        return Err(format!("{shown}: is a symbolic link; attach the real file"));
    }
    if !meta.is_file() {
        return Err(format!("{shown}: not a regular file"));
    }
    if meta.len() > MAX_BYTES {
        return Err(format!(
            "{shown}: {} MB; the limit is 25 MB",
            meta.len().div_ceil(1024 * 1024)
        ));
    }
    let name = safe_name(src).ok_or_else(|| {
        format!("{shown}: type not supported (png, jpg, webp, gif, md, html, txt, json, diff, patch, log or huginn)")
    })?;
    let (kind, _) = kind_of(&name).expect("safe_name checks the type");
    Ok((name, kind, meta))
}

/// Copy `sources` into a new `{home}/attachments/<ulid>/`. Every source is
/// checked before anything is written; a copy that fails half-way removes
/// the folder it created.
pub fn attach(
    store: &Store,
    sources: &[PathBuf],
    label: Option<&'static str>,
) -> Result<Vec<Attached>, String> {
    if sources.is_empty() {
        return Err("nothing to attach".into());
    }
    let mut checked = Vec::new();
    for src in sources {
        checked.push((src, check(src)?));
    }
    let (dir, fd) = new_folder(store)?;
    let result = copy_all(&dir, &fd, &checked);
    if result.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    let out = result?;
    let manifest = json!({
        "v": 1,
        "label": label,
        "files": out.iter().map(|a| json!({"name": a.name, "kind": a.kind, "bytes": a.bytes})).collect::<Vec<_>>(),
    });
    if let Err(e) = safe_fs::replace_at(&fd, MANIFEST, manifest.to_string().as_bytes(), 0o600) {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("{MANIFEST}: {e}"));
    }
    Ok(out)
}

/// A new `{home}/attachments/<ulid>/`, created and held by descriptor: the
/// root must not be a symlink, and the folder is made inside it with
/// `mkdirat`, never re-resolved by path.
fn new_folder(store: &Store) -> Result<(PathBuf, OwnedFd), String> {
    let root = root(store);
    if fs::symlink_metadata(&root).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!(
            "{} is a symbolic link; harbard does not attach there",
            root.display()
        ));
    }
    fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    restrict(&root);
    let root_fd = safe_fs::open_dir(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    let id = ulid::Ulid::generate().to_string();
    let fd = safe_fs::mkdir_at(&root_fd, &id, 0o700)
        .map_err(|e| format!("{}/{id}: {e}", root.display()))?;
    Ok((root.join(id), fd))
}

/// `{n}-{name}`, its stem cut so the whole stays within [`NAME_MAX`] (L2).
fn numbered(n: usize, name: &str) -> String {
    let prefix = format!("{n}-");
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let room = NAME_MAX.saturating_sub(prefix.len() + ext.len() + 1);
    let stem: String = stem.chars().take(room).collect();
    format!("{prefix}{stem}.{ext}")
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict(_: &Path) {}

fn copy_all(
    dir: &Path,
    fd: &OwnedFd,
    checked: &[(&PathBuf, (String, &'static str, fs::Metadata))],
) -> Result<Vec<Attached>, String> {
    let mut out: Vec<Attached> = Vec::new();
    let mut named_by_views: Vec<(String, Vec<String>)> = Vec::new();
    for (src, (name, kind, meta)) in checked {
        let mut unique = name.clone();
        let mut n = 2;
        while out.iter().any(|a| a.name == unique) {
            unique = numbered(n, name);
            n += 1;
        }
        let name = unique;
        let path = dir.join(&name);
        let (bytes, content) = copy_one(src, meta, fd, &name)?;
        if *kind == "huginn" {
            named_by_views.push((name.clone(), view_files(&name, &content)?));
        }
        out.push(Attached {
            path,
            name,
            kind,
            bytes,
        });
    }
    for (view, files) in &named_by_views {
        if let Some(missing) = files.iter().find(|f| out.iter().all(|a| &a.name != *f)) {
            return Err(format!(
                "{view}: shows {missing}, which is not attached; attach it in the same command"
            ));
        }
    }
    Ok(out)
}

/// The files a valid view names, or why the view is refused.
fn view_files(name: &str, content: &[u8]) -> Result<Vec<String>, String> {
    let refused = |issues: Vec<huginn::Issue>| {
        format!("{name}: vista rechazada. {}", huginn::describe(&issues))
    };
    huginn::validate(content).map_err(refused)
}

/// Copy one checked file. The opened file must be the one checked (same
/// device and inode), so a swap for a symlink between the check and the
/// open is refused; the size cap holds even if the file grows meanwhile.
fn copy_one(
    src: &Path,
    checked: &fs::Metadata,
    dir: &OwnedFd,
    name: &str,
) -> Result<(u64, Vec<u8>), String> {
    let shown = src.display();
    let mut open = OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Never through a symlink swapped in, never blocked on a FIFO.
        open.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    let file = open.open(src).map_err(|e| format!("{shown}: {e}"))?;
    let opened = file.metadata().map_err(|e| format!("{shown}: {e}"))?;
    if !opened.is_file() || !same_file(checked, &opened) {
        return Err(format!("{shown}: the file changed while attaching"));
    }
    let mut buf = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("{shown}: {e}"))?;
    if buf.len() as u64 > MAX_BYTES {
        return Err(format!("{shown}: over 25 MB"));
    }
    safe_fs::write_new_at(dir, name, &buf, 0o600).map_err(|e| format!("{name}: {e}"))?;
    Ok((buf.len() as u64, buf))
}

#[cfg(unix)]
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(_: &fs::Metadata, _: &fs::Metadata) -> bool {
    true
}

/// Write `bytes` under `name` into a new `{home}/attachments/<ulid>/`: what
/// the console's "Attach file" sends after the operator picked a file
/// in the OS dialog. Same types, cap and manifest as [`attach`].
pub fn attach_bytes(
    store: &Store,
    name: &str,
    bytes: &[u8],
    label: Option<&'static str>,
) -> Result<Attached, String> {
    let safe = safe_name(Path::new(name)).ok_or_else(|| {
        format!(
            "{name}: tipo no admitido (png, jpg, webp, gif, md, html, txt, json, diff, patch, log o huginn)"
        )
    })?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("{name}: over 25 MB"));
    }
    let (kind, _) = kind_of(&safe).expect("safe_name checks the type");
    if kind == "huginn" {
        view_files(&safe, bytes)?;
    }
    let (dir, fd) = new_folder(store)?;
    let path = dir.join(&safe);
    let manifest = json!({
        "v": 1,
        "label": label,
        "files": [{"name": safe, "kind": kind, "bytes": bytes.len()}],
    });
    let done = safe_fs::write_new_at(&fd, &safe, bytes, 0o600)
        .and_then(|()| safe_fs::replace_at(&fd, MANIFEST, manifest.to_string().as_bytes(), 0o600));
    if let Err(e) = done {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(Attached {
        path,
        name: safe,
        kind,
        bytes: bytes.len() as u64,
    })
}

/// `harbard attach`: print one copied path per line (JSON with `--json`).
pub fn cmd_attach(store: &Store, paths: &[PathBuf], label: Option<&str>, as_json: bool) -> i32 {
    let label = match label {
        Some(raw) => match label_of(raw) {
            Some(l) => Some(l),
            None => {
                eprintln!(
                    "refused: --label is one of {}, not {raw:?}",
                    LABELS.join(", ")
                );
                return 2;
            }
        },
        None => None,
    };
    match attach(store, paths, label) {
        Ok(out) => {
            if as_json {
                let files: Vec<_> = out
                    .iter()
                    .map(|a| json!({"path": a.path.display().to_string(), "name": a.name, "kind": a.kind, "bytes": a.bytes, "label": label}))
                    .collect();
                println!("{}", json!({ "attachments": files }));
            } else {
                for a in &out {
                    println!("{}", a.path.display());
                }
                eprintln!(
                    "harbard · cite this path in your message and the console will show it{}",
                    label.map(|l| format!(" as \"{l}\"")).unwrap_or_default()
                );
            }
            0
        }
        Err(e) => {
            eprintln!("refused: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Self {
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("harbard-attach-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const IMAGE_VIEW: &str =
        r#"{"root":"a","elements":{"a":{"type":"Image","props":{"file":"shot.png","alt":"x"}}}}"#;

    #[test]
    fn a_view_is_copied_with_its_kind() {
        let src = Tmp::new();
        let home = Tmp::new();
        let view = src.0.join("plan.huginn");
        fs::write(
            &view,
            r#"{"root":"a","elements":{"a":{"type":"Text","props":{"text":"x"}}}}"#,
        )
        .unwrap();
        let out = attach(&Store::at(&home.0), &[view], None).unwrap();
        assert_eq!(out[0].kind, "huginn");
    }

    #[test]
    fn a_view_that_breaks_the_catalog_is_refused_and_leaves_nothing_behind() {
        let src = Tmp::new();
        let home = Tmp::new();
        let store = Store::at(&home.0);
        let view = src.0.join("mala.huginn");
        fs::write(
            &view,
            r#"{"root":"a","elements":{"a":{"type":"Script","props":{}}}}"#,
        )
        .unwrap();
        let err = attach(&store, &[view], None).unwrap_err();
        assert!(err.contains("unknown_component"), "{err}");
        assert_eq!(fs::read_dir(root(&store)).unwrap().count(), 0);
    }

    #[test]
    fn a_view_showing_a_file_must_travel_with_it() {
        let src = Tmp::new();
        let home = Tmp::new();
        let store = Store::at(&home.0);
        let view = src.0.join("v.huginn");
        fs::write(&view, IMAGE_VIEW).unwrap();
        let err = attach(&store, std::slice::from_ref(&view), None).unwrap_err();
        assert!(err.contains("shot.png"), "{err}");
        let png = src.0.join("shot.png");
        fs::write(&png, b"\x89PNG fake").unwrap();
        assert_eq!(attach(&store, &[view, png], None).unwrap().len(), 2);
    }

    #[test]
    fn an_upload_of_a_broken_view_is_refused_too() {
        let home = Tmp::new();
        let err = attach_bytes(&Store::at(&home.0), "v.huginn", b"nope", None).unwrap_err();
        assert!(err.contains("invalid_json"), "{err}");
    }

    #[test]
    fn a_copy_lands_only_in_the_attachments_folder_with_its_manifest() {
        let src = Tmp::new();
        let home = Tmp::new();
        let store = Store::at(&home.0);
        let png = src.0.join("Captura de pantalla.PNG");
        fs::write(&png, b"\x89PNG fake").unwrap();
        let md = src.0.join("report.md");
        fs::write(&md, "# Informe").unwrap();
        let out = attach(&store, &[png.clone(), md], Some("before")).unwrap();
        assert_eq!(out.len(), 2);
        let dir = out[0].path.parent().unwrap();
        assert_eq!(dir.parent().unwrap(), root(&store));
        assert!(valid_dir(&dir.file_name().unwrap().to_string_lossy()));
        assert_eq!(out[0].name, "Captura-de-pantalla.png");
        assert_eq!(out[0].kind, "image");
        assert_eq!(fs::read(&out[0].path).unwrap(), b"\x89PNG fake");
        assert_eq!(out[1].kind, "markdown");
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(manifest["label"], "before");
        assert_eq!(manifest["files"][0]["name"], "Captura-de-pantalla.png");
        // The source is untouched, and nothing else was written beside it.
        assert!(png.exists());
        assert_eq!(fs::read_dir(&src.0).unwrap().count(), 2);
        // Everything harbard wrote is under {home}/attachments.
        let top: Vec<_> = fs::read_dir(&home.0)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(top, vec![std::ffi::OsString::from("attachments")]);
    }

    #[test]
    fn types_outside_the_list_are_refused_and_nothing_is_written() {
        let src = Tmp::new();
        let home = Tmp::new();
        let store = Store::at(&home.0);
        let ok = src.0.join("a.png");
        fs::write(&ok, b"x").unwrap();
        for bad in ["script.sh", "doc.pdf", "binario", "x.svg", ".env"] {
            let p = src.0.join(bad);
            fs::write(&p, b"x").unwrap();
            let err = attach(&store, &[ok.clone(), p], None).unwrap_err();
            assert!(err.contains("type not supported"), "{bad}: {err}");
        }
        assert!(!root(&store).exists(), "a refusal writes nothing");
        for good in [
            "a.jpg", "a.jpeg", "a.webp", "a.gif", "a.html", "a.txt", "a.json", "a.diff", "a.patch",
            "a.log",
        ] {
            assert!(kind_of(good).is_some(), "{good}");
        }
    }

    #[test]
    fn the_size_cap_is_25_mb() {
        let src = Tmp::new();
        let home = Tmp::new();
        let store = Store::at(&home.0);
        let big = src.0.join("big.log");
        let f = File::create(&big).unwrap();
        f.set_len(MAX_BYTES + 1).unwrap();
        let err = attach(&store, std::slice::from_ref(&big), None).unwrap_err();
        assert!(err.contains("25 MB"), "{err}");
        f.set_len(MAX_BYTES).unwrap();
        let out = attach(&store, &[big], None).unwrap();
        assert_eq!(out[0].bytes, MAX_BYTES);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_non_files_are_refused() {
        let src = Tmp::new();
        let home = Tmp::new();
        let store = Store::at(&home.0);
        let real = src.0.join("real.png");
        fs::write(&real, b"x").unwrap();
        let link = src.0.join("link.png");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let err = attach(&store, &[link], None).unwrap_err();
        assert!(err.contains("symbolic link"), "{err}");
        let dir = src.0.join("folder.png");
        fs::create_dir(&dir).unwrap();
        let err = attach(&store, &[dir], None).unwrap_err();
        assert!(err.contains("not a regular file"), "{err}");
        let err = attach(&store, &[src.0.join("missing.png")], None).unwrap_err();
        assert!(err.contains("cannot be read"), "{err}");
        // An attachments folder that is itself a symlink is refused too.
        fs::create_dir_all(src.0.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(src.0.join("elsewhere"), root(&store)).unwrap();
        let err = attach(&store, &[real], None).unwrap_err();
        assert!(err.contains("symbolic link"), "{err}");
        assert_eq!(fs::read_dir(src.0.join("elsewhere")).unwrap().count(), 0);
    }

    #[test]
    fn bytes_from_the_console_land_in_the_folder_with_the_same_rules() {
        let home = Tmp::new();
        let store = Store::at(&home.0);
        let a = attach_bytes(&store, "Mi captura.PNG", b"png", Some("before")).unwrap();
        assert!(a.path.starts_with(root(&store)));
        assert_eq!(
            (a.name.as_str(), a.kind, a.bytes),
            ("Mi-captura.png", "image", 3)
        );
        let manifest = fs::read_to_string(a.path.parent().unwrap().join(MANIFEST)).unwrap();
        assert!(manifest.contains("\"before\""));
        // A name never escapes the folder; a bad type writes nothing.
        let b = attach_bytes(&store, "../../etc/x.md", b"#", None).unwrap();
        assert!(b.path.starts_with(root(&store)) && b.name == "x.md");
        let before = fs::read_dir(root(&store)).unwrap().count();
        assert!(attach_bytes(&store, "run.sh", b"x", None).is_err());
        assert_eq!(fs::read_dir(root(&store)).unwrap().count(), before);
    }

    /// An attached `manifest.json` survives, and two long
    /// names stay servable.
    #[test]
    fn a_file_named_manifest_survives_and_long_duplicates_fit() {
        let src = Tmp::new();
        let home = Tmp::new();
        let store = Store::at(&home.0);
        fs::write(src.0.join("manifest.json"), b"{\"mine\":true}").unwrap();
        let out = attach(&store, &[src.0.join("manifest.json")], None).unwrap();
        assert_eq!(fs::read(&out[0].path).unwrap(), b"{\"mine\":true}");
        let long = format!("{}.md", "x".repeat(97));
        let (a, b) = (Tmp::new(), Tmp::new());
        fs::write(a.0.join(&long), b"a").unwrap();
        fs::write(b.0.join(&long), b"b").unwrap();
        let out = attach(&store, &[a.0.join(&long), b.0.join(&long)], None).unwrap();
        for f in &out {
            assert!(
                f.name.len() <= NAME_MAX && valid_name(&f.name),
                "{}",
                f.name
            );
        }
        assert_ne!(out[0].name, out[1].name);
    }

    #[test]
    fn labels_names_and_folders_validate() {
        assert_eq!(label_of("after"), Some("after"));
        assert_eq!(label_of("Código"), Some("code"));
        assert_eq!(label_of("otro"), None);
        // Exists only for old manifests: their label keeps its meaning.
        assert_eq!(label_of("antes"), Some("before"));
        assert_eq!(label_of("después"), Some("after"));
        assert_eq!(label_of("informe"), Some("report"));
        assert_eq!(label_of("codigo"), Some("code"));
        assert!(valid_name("before.png"));
        for bad in ["../x.png", "a/b.png", ".png", "a.exe", "", "á.png"] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert!(valid_dir("01K9Z8Y7X6W5V4T3S2R1Q0P9N8"));
        assert!(!valid_dir(".."));
        assert!(!valid_dir("01K9Z8Y7X6W5V4T3S2R1Q0P9N8/.."));
    }
}
