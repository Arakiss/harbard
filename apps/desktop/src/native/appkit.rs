//! The few AppKit calls GPUI does not make: the Dock tile, its badge and
//! bounce, hiding a single window without closing it, and the screens'
//! pixel sizes. Every call must run on the main thread; off it they do
//! nothing.

use super::dock::Tile;
use gpui::Window;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSImage, NSRequestUserAttentionType, NSScreen, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn app() -> Option<Retained<NSApplication>> {
    MainThreadMarker::new().map(NSApplication::sharedApplication)
}

/// `NSApplication.applicationIconImage`; `None` restores the bundle icon.
pub fn set_tile(tile: Option<&Tile>) {
    let Some(app) = app() else { return };
    match tile {
        // SAFETY: nil is documented to restore the bundle's icon.
        None => unsafe { app.setApplicationIconImage(None) },
        Some(tile) => {
            let Some(png) = crate::assets::Assets::local(&tile.path()) else {
                return;
            };
            let data = NSData::with_bytes(&png);
            if let Some(image) = NSImage::initWithData(<NSImage as objc2::AllocAnyThread>::alloc(), &data) {
                // SAFETY: a valid, retained image, on the main thread.
                unsafe { app.setApplicationIconImage(Some(&image)) };
            }
        }
    }
}

/// The red Dock badge; `None` clears it.
pub fn set_badge(label: Option<&str>) {
    let Some(app) = app() else { return };
    let label = label.map(NSString::from_str);
    app.dockTile().setBadgeLabel(label.as_deref());
}

/// One informational bounce of the Dock icon.
pub fn bounce() {
    if let Some(app) = app() {
        app.requestUserAttention(NSRequestUserAttentionType::InformationalRequest);
    }
}

/// harbard is the active app (one of its windows has focus).
pub fn app_active() -> bool {
    app().is_some_and(|a| a.isActive())
}

/// Low Power Mode is on.
pub fn low_power() -> bool {
    objc2_foundation::NSProcessInfo::processInfo().isLowPowerModeEnabled()
}

fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else {
        return None;
    };
    // SAFETY: the handle's view is the window's live content view.
    let view: &objc2_app_kit::NSView = unsafe { h.ns_view.cast().as_ref() };
    view.window()
}

/// Take the window off screen without closing it: its state stays.
pub fn hide(window: &Window) {
    if let Some(w) = ns_window(window) {
        w.orderOut(None);
    }
}

/// Bring the window back: out of the Dock if minimized, in front, focused.
pub fn show(window: &Window) {
    if let Some(w) = ns_window(window) {
        if w.isMiniaturized() {
            w.deminiaturize(None);
        }
        w.makeKeyAndOrderFront(None);
    }
    if let Some(app) = app() {
        app.activate();
    }
}

/// On screen and not minimized.
pub fn visible(window: &Window) -> bool {
    ns_window(window).is_some_and(|w| w.isVisible() && !w.isMiniaturized())
}

/// Show without taking focus (a doorbell may show the Strip; it never
/// steals the keyboard).
pub fn show_quietly(window: &Window) {
    if let Some(w) = ns_window(window) {
        w.orderFrontRegardless();
    }
}

/// Make the window borderless and lay it exactly over `frame` (AppKit
/// coordinates, bottom-left origin).
pub fn make_borderless_at(window: &Window, frame: NSRect) {
    if let Some(w) = ns_window(window) {
        w.setStyleMask(NSWindowStyleMask::Borderless);
        w.setFrame_display(frame, true);
    }
}

/// A screen as AppKit sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    /// AppKit frame (points, bottom-left origin).
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

impl Screen {
    /// Physical pixels.
    pub fn pixels(&self) -> (u32, u32) {
        (
            (self.w * self.scale).round() as u32,
            (self.h * self.scale).round() as u32,
        )
    }

    pub fn frame(&self) -> NSRect {
        NSRect::new(NSPoint::new(self.x, self.y), NSSize::new(self.w, self.h))
    }
}

pub fn screens() -> Vec<Screen> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Vec::new();
    };
    NSScreen::screens(mtm)
        .iter()
        .map(|s| {
            let f = s.frame();
            Screen {
                x: f.origin.x,
                y: f.origin.y,
                w: f.size.width,
                h: f.size.height,
                scale: s.backingScaleFactor(),
            }
        })
        .collect()
}
