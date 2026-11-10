//! Message attachments (`components/workspace/attachments.tsx` and
//! `MessageAttachments` in `components/huginn/view.tsx`): images as
//! thumbnails with a lightbox, a before/after pair side by side or under a
//! slider, Markdown reports in a pane on the right, diffs with their colours,
//! logs and text in monospace with "Show everything".
//!
//! Every byte comes from serve's attachments route, read once per id on a
//! background task and kept in a bounded cache. Text is drawn as text:
//! Markdown goes through `derive::markdown`, which yields data.
//!
//! - [`message_attachments`]: the block under a message.
//! - [`overlays`]: the lightbox and the report pane; mount once at a root.
//! - [`report_pane`]: the pane as a column, for a screen that lays it out.
//! - [`overlay_open`], [`close_top`]: for a screen's own Escape handling.

mod blocks;
mod md;
mod overlay;

use crate::api;
use crate::model::{Attachment, Message};
use gpui::{
    AnyElement, App, AppContext, Entity, EntityId, FocusHandle, Global, Image, ImageFormat,
    IntoElement, SharedString, Subscription, Window,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A message's attachments block (`MessageAttachments` + `Attachments`), or
/// None when it has none. `whole` (a thread pane) keeps its own state apart
/// from the same message drawn in a channel.
pub fn message_attachments(
    msg: &Message,
    whole: bool,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let _ = (window, cx);
    // html is copied by `attach` but never drawn; it opens as a file card.
    let drawn = |k: &str| {
        matches!(
            k,
            "image" | "markdown" | "diff" | "text" | "json" | "huginn"
        )
    };
    if !msg.attachments.iter().any(|a| drawn(&a.kind)) {
        return None;
    }
    Some(
        blocks::MessageAttachments {
            msg: msg.clone(),
            whole,
        }
        .into_any_element(),
    )
}

/// The lightbox and the report pane, drawn above everything; mount once at a
/// screen or shell root. The report pane is drawn here as the console drew
/// it (over the right side), unless the same view also calls [`report_pane`].
pub fn overlays(window: &mut Window, cx: &mut App) -> AnyElement {
    overlay::overlays(window, cx)
}

/// The report pane as a right-hand column (`aside.hb-report-pane`), when one
/// is open. While a view that calls this is alive, [`overlays`] draws no
/// floating pane; the view calls [`undock`] when it is released.
pub fn report_pane(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    overlay::docked_pane(window, cx)
}

/// A view that laid the report pane out is gone: the pane floats again.
pub fn undock(view: EntityId, cx: &mut App) {
    let s = store(cx);
    s.mounts.retain(|m| *m != view);
    if s.docked.remove(&view) && s.report.is_some() {
        tell_mounts(cx);
    }
}

/// Whether a lightbox or report pane is open (screens skip Esc-marks-read then).
pub fn overlay_open(cx: &App) -> bool {
    cx.try_global::<Store>()
        .is_some_and(|s| s.lightbox.is_some() || s.report.is_some())
}

/// Close the topmost one; true when something closed (Esc).
pub fn close_top(cx: &mut App) -> bool {
    match top(cx) {
        Some(top) => {
            close(top, None, cx);
            true
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// The store: fetched files, the open overlays, the views embedded per id.
// ---------------------------------------------------------------------------

/// Most bytes kept in memory across fetched attachments.
const CACHE_BYTES: usize = 96 * 1024 * 1024;
/// Most attachments kept.
const CACHE_FILES: usize = 400;
/// Most embedded views kept alive.
const CACHE_VIEWS: usize = 120;
/// A failed read is tried again after this long.
const RETRY_AFTER: Duration = Duration::from_secs(30);

/// One attachment's bytes, as far as they have arrived.
#[derive(Clone)]
pub(crate) enum File {
    Loading,
    Failed(Instant),
    Text(SharedString),
    /// The image and its size in pixels.
    Image(Arc<Image>, u32, u32),
}

/// Re-renders the element that asked for a file when it arrives.
pub(crate) struct Watch;

/// What is on top, to close exactly that one.
#[derive(Clone, PartialEq, Debug)]
enum Top {
    Lightbox(String),
    Report(String),
}

#[derive(Default)]
struct Store {
    files: HashMap<String, File>,
    /// Arrived ids, oldest first, for eviction.
    order: VecDeque<String>,
    bytes: usize,
    waiters: HashMap<String, Vec<gpui::WeakEntity<Watch>>>,
    lightbox: Option<Attachment>,
    report: Option<Attachment>,
    /// Focus before the lightbox opened, given back when it closes.
    focus_before: Option<FocusHandle>,
    lightbox_focus: Option<FocusHandle>,
    report_scroll: gpui::ScrollHandle,
    /// The parsed report, by attachment id.
    parsed: Option<(String, Arc<Vec<crate::derive::markdown::Block>>)>,
    /// Views that draw the overlays; told when one opens or closes.
    mounts: Vec<EntityId>,
    /// Views that lay the report pane out as a column.
    docked: HashSet<EntityId>,
    views: HashMap<(String, bool), Entity<crate::huginn::HuginnView>>,
    views_order: VecDeque<(String, bool)>,
    escape: Option<Subscription>,
}

impl Global for Store {}

fn store(cx: &mut App) -> &mut Store {
    cx.default_global::<Store>()
}

/// `GET /attachments/<id segments, each encoded>`.
fn attachment_path(id: &str) -> String {
    let segments: Vec<String> = id.split('/').map(api::enc).collect();
    format!("/attachments/{}", segments.join("/"))
}

/// An attachment's bytes: from the cache, else fetched once in the
/// background; `watch` is notified when they arrive.
pub(crate) fn file(att: &Attachment, watch: &Entity<Watch>, cx: &mut App) -> File {
    let image = att.kind == "image";
    let s = store(cx);
    match s.files.get(&att.id) {
        Some(File::Loading) => {
            s.waiters
                .entry(att.id.clone())
                .or_default()
                .push(watch.downgrade());
            return File::Loading;
        }
        Some(File::Failed(at)) if at.elapsed() < RETRY_AFTER => return File::Failed(*at),
        Some(File::Failed(_)) | None => {}
        Some(hit) => return hit.clone(),
    }
    s.files.insert(att.id.clone(), File::Loading);
    s.waiters
        .entry(att.id.clone())
        .or_default()
        .push(watch.downgrade());
    let id = att.id.clone();
    let path = attachment_path(&id);
    cx.spawn(async move |cx| {
        let got = cx
            .background_spawn(async move { api::get_bytes(&path) })
            .await;
        cx.update(|cx| arrived(id, got.ok(), image, cx));
    })
    .detach();
    File::Loading
}

fn arrived(id: String, bytes: Option<Vec<u8>>, image: bool, cx: &mut App) {
    let (file, size) = match bytes {
        None => (File::Failed(Instant::now()), 0),
        Some(bytes) if image => match image_format(&bytes).zip(image_size(&bytes)) {
            Some((format, (w, h))) => {
                let n = bytes.len();
                (
                    File::Image(
                        Arc::new(Image::from_bytes(format, bytes)),
                        w.max(1),
                        h.max(1),
                    ),
                    n,
                )
            }
            None => (File::Failed(Instant::now()), 0),
        },
        Some(bytes) => {
            let n = bytes.len();
            (
                File::Text(String::from_utf8_lossy(&bytes).into_owned().into()),
                n,
            )
        }
    };
    let s = store(cx);
    let done = !matches!(file, File::Failed(_));
    s.files.insert(id.clone(), file);
    if done {
        s.order.push_back(id.clone());
        s.bytes += size;
    }
    let mut evicted = Vec::new();
    while (s.bytes > CACHE_BYTES || s.order.len() > CACHE_FILES) && s.order.len() > 1 {
        let Some(old) = s.order.pop_front() else {
            break;
        };
        if let Some(f) = s.files.remove(&old) {
            s.bytes = s.bytes.saturating_sub(file_bytes(&f));
            if let File::Image(img, ..) = f {
                evicted.push(img);
            }
        }
    }
    let waiters = s.waiters.remove(&id).unwrap_or_default();
    for img in evicted {
        img.remove_asset(cx);
    }
    for w in waiters {
        if let Some(w) = w.upgrade() {
            w.update(cx, |_, cx| cx.notify());
        }
    }
}

fn file_bytes(f: &File) -> usize {
    match f {
        File::Text(t) => t.len(),
        File::Image(img, ..) => img.bytes().len(),
        _ => 0,
    }
}

/// The format of an image from its first bytes.
fn image_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        Some(ImageFormat::Jpeg)
    } else if bytes.starts_with(b"GIF8") {
        Some(ImageFormat::Gif)
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(ImageFormat::Webp)
    } else {
        None
    }
}

/// Width and height in pixels, read from the header of a PNG, GIF, JPEG or
/// WebP, so the layout knows the size before the image is decoded.
fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32);
    let le16 = |i: usize| Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as u32);
    let le24 = |i: usize| {
        Some(u32::from_le_bytes([
            *b.get(i)?,
            *b.get(i + 1)?,
            *b.get(i + 2)?,
            0,
        ]))
    };
    let be32 = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    match image_format(b)? {
        ImageFormat::Png => Some((be32(16)?, be32(20)?)),
        ImageFormat::Gif => Some((le16(6)?, le16(8)?)),
        ImageFormat::Webp => match b.get(12..16)? {
            b"VP8 " => Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF)),
            b"VP8L" => {
                let v = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
                Some(((v & 0x3FFF) + 1, ((v >> 14) & 0x3FFF) + 1))
            }
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        },
        ImageFormat::Jpeg => {
            let mut i = 2;
            while i + 9 < b.len() {
                if b[i] != 0xFF {
                    i += 1;
                    continue;
                }
                let marker = b[i + 1];
                if marker == 0xFF {
                    i += 1;
                    continue;
                }
                let len = be16(i + 2)? as usize;
                let sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
                if sof {
                    return Some((be16(i + 7)?, be16(i + 5)?));
                }
                i += 2 + len;
            }
            None
        }
        _ => None,
    }
}

/// The embedded view of one `.huginn` attachment, made once and kept.
pub(crate) fn huginn_view(
    att: &Attachment,
    msg: &Message,
    whole: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<crate::huginn::HuginnView> {
    let key = (att.id.clone(), whole);
    if let Some(v) = store(cx).views.get(&key) {
        return v.clone();
    }
    let view = cx.new(|cx| crate::huginn::HuginnView::new(att.clone(), msg.clone(), window, cx));
    let s = store(cx);
    s.views.insert(key.clone(), view.clone());
    s.views_order.push_back(key);
    while s.views_order.len() > CACHE_VIEWS {
        if let Some(old) = s.views_order.pop_front() {
            s.views.remove(&old);
        }
    }
    view
}

/// The file names a view shows itself, so they are not drawn twice; None
/// while the view is still being read.
pub(crate) fn files_named_by(
    view: &Entity<crate::huginn::HuginnView>,
    cx: &App,
) -> Option<HashSet<String>> {
    let view = view.read(cx);
    (!view.loading()).then(|| view.files_named().into_iter().collect())
}

// ---------------------------------------------------------------------------
// Opening and closing
// ---------------------------------------------------------------------------

fn top(cx: &App) -> Option<Top> {
    let s = cx.try_global::<Store>()?;
    if let Some(a) = &s.lightbox {
        return Some(Top::Lightbox(a.id.clone()));
    }
    s.report.as_ref().map(|a| Top::Report(a.id.clone()))
}

fn tell_mounts(cx: &mut App) {
    for id in store(cx).mounts.clone() {
        cx.notify(id);
    }
}

/// One image, full size, over the window.
pub(crate) fn open_lightbox(att: &Attachment, window: &mut Window, cx: &mut App) {
    let before = window.focused(cx);
    let focus = cx.focus_handle();
    let s = store(cx);
    s.lightbox = Some(att.clone());
    s.focus_before = before;
    s.lightbox_focus = Some(focus.clone());
    window.focus(&focus, cx);
    tell_mounts(cx);
}

/// A Markdown report in the pane on the right; one at a time.
pub(crate) fn open_report(att: &Attachment, cx: &mut App) {
    let s = store(cx);
    if s.report.as_ref().is_some_and(|r| r.id == att.id) {
        return;
    }
    s.report = Some(att.clone());
    s.report_scroll = gpui::ScrollHandle::new();
    tell_mounts(cx);
}

/// Close `which` if it is still open.
fn close(which: Top, window: Option<&mut Window>, cx: &mut App) {
    let s = store(cx);
    let refocus = match &which {
        Top::Lightbox(id) if s.lightbox.as_ref().is_some_and(|a| &a.id == id) => {
            s.lightbox = None;
            s.lightbox_focus = None;
            s.focus_before.take()
        }
        Top::Report(id) if s.report.as_ref().is_some_and(|a| &a.id == id) => {
            s.report = None;
            s.parsed = None;
            None
        }
        _ => return,
    };
    if let (Some(handle), Some(window)) = (refocus, window) {
        window.focus(&handle, cx);
    }
    tell_mounts(cx);
}

/// Escape closes the top overlay, like the console's window listener. The
/// close waits until the key has gone through, so a screen's own Escape
/// still sees the overlay open and does not mark anything read.
fn listen_for_escape(cx: &mut App) {
    if store(cx).escape.is_some() {
        return;
    }
    let sub = cx.intercept_keystrokes(|ev, window, cx| {
        if ev.keystroke.key != "escape" {
            return;
        }
        if let Some(which) = top(cx) {
            let handle = window.window_handle();
            cx.defer(move |cx| {
                let closed =
                    handle.update(cx, |_, window, cx| close(which.clone(), Some(window), cx));
                if closed.is_err() {
                    close(which, None, cx);
                }
            });
        }
    });
    store(cx).escape = Some(sub);
}

/// The word for serve's stored label (`shownLabel`).
pub(crate) fn shown_label(label: &str, cx: &App) -> SharedString {
    match label {
        "before" => crate::i18n::t(cx, "attachments.labelBefore"),
        "after" => crate::i18n::t(cx, "attachments.labelAfter"),
        "report" => crate::i18n::t(cx, "attachments.labelReport"),
        "code" => crate::i18n::t(cx, "attachments.labelCode"),
        other => other.to_string().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_encode_each_segment() {
        assert_eq!(
            attachment_path("01ABC/a b.png"),
            "/attachments/01ABC/a%20b.png"
        );
    }

    #[test]
    fn image_sizes_come_from_the_header() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((640, 480)));
        let gif = b"GIF89a\x20\x03\x58\x02rest";
        assert_eq!(image_size(gif), Some((800, 600)));
        // A JPEG: SOI, an APP0 segment, then SOF0 with height 300, width 400.
        let jpg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01,
            0x2C, 0x01, 0x90, 0x03,
        ];
        assert_eq!(image_size(&jpg), Some((400, 300)));
        assert_eq!(image_size(b"not an image"), None);
    }
}
