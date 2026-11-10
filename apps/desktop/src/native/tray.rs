//! The menu-bar item: the logo's shape as a template image, the count of
//! live agents beside it, a left click that shows or hides the main window,
//! and a menu (right click) with the seven items the old shell had.

use super::Cmd;
use super::dock::TrayMark;
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

/// The labels the menu needs, already in the operator's language.
pub struct Labels {
    pub open: String,
    pub mute: String,
    pub show_strip: String,
    pub close_strip: String,
    pub summon: String,
    pub updates: String,
    pub quit: String,
}

pub struct Tray {
    icon: TrayIcon,
    open: MenuItem,
    mute: MenuItem,
    show_strip: MenuItem,
    close_strip: MenuItem,
    summon: MenuItem,
    updates: MenuItem,
    quit: MenuItem,
    mark: Option<TrayMark>,
    count: Option<usize>,
}

/// A bundled PNG as a tray icon.
fn icon(path: &str) -> Option<Icon> {
    let bytes = crate::assets::Assets::local(path)?;
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes.as_ref()));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    Icon::from_rgba(buf, info.width, info.height).ok()
}

impl Tray {
    /// Build the item. Clicks and menu picks arrive on `tx`.
    pub fn new(labels: &Labels, summon_ok: bool, tx: async_channel::Sender<Cmd>) -> Option<Tray> {
        let open = MenuItem::with_id("open", &labels.open, true, None);
        let mute = MenuItem::with_id("mute", &labels.mute, true, None);
        let show_strip = MenuItem::with_id("show_strip", &labels.show_strip, true, None);
        let close_strip = MenuItem::with_id("close_strip", &labels.close_strip, true, None);
        let summon = MenuItem::with_id("summon", &labels.summon, summon_ok, None);
        let updates = MenuItem::with_id("updates", &labels.updates, true, None);
        let quit = MenuItem::with_id("quit", &labels.quit, true, None);
        let menu = Menu::new();
        for item in [&open, &mute, &show_strip, &close_strip, &summon, &updates, &quit] {
            let _ = menu.append(item);
        }
        let menu_tx = tx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let cmd = match e.id.0.as_str() {
                "open" => Cmd::OpenOverview,
                "mute" => Cmd::ToggleMute,
                "show_strip" => Cmd::ShowStrip,
                "close_strip" => Cmd::CloseStrip,
                "summon" => Cmd::ToggleMain,
                "updates" => Cmd::Updates,
                "quit" => Cmd::Quit,
                _ => return,
            };
            let _ = menu_tx.send_blocking(cmd);
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = tx.send_blocking(Cmd::ToggleMain);
            }
        }));
        let icon = TrayIconBuilder::new()
            .with_id("fleet")
            .with_icon(icon("tray/tray.png")?)
            .with_icon_as_template(true)
            .with_title("")
            .with_tooltip("")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()
            .ok()?;
        Some(Tray {
            icon,
            open,
            mute,
            show_strip,
            close_strip,
            summon,
            updates,
            quit,
            mark: None,
            count: None,
        })
    }

    /// The logo's shape, set only when it changes.
    pub fn set_mark(&mut self, mark: TrayMark) {
        if self.mark == Some(mark) {
            return;
        }
        self.mark = Some(mark);
        if let Some(i) = icon(mark.path()) {
            let _ = self.icon.set_icon_with_as_template(Some(i), true);
        }
    }

    /// The live count beside the mark (blank at 0) and the tooltip.
    pub fn set_count(&mut self, live: usize, tooltip: &str) {
        if self.count == Some(live) {
            return;
        }
        self.count = Some(live);
        self.icon
            .set_title(Some(if live > 0 { live.to_string() } else { String::new() }));
        let _ = self.icon.set_tooltip(Some(tooltip));
    }

    /// New words (a language change, Mute ↔ Unmute).
    pub fn relabel(&mut self, labels: &Labels, summon_ok: bool) {
        self.open.set_text(&labels.open);
        self.mute.set_text(&labels.mute);
        self.show_strip.set_text(&labels.show_strip);
        self.close_strip.set_text(&labels.close_strip);
        self.summon.set_text(&labels.summon);
        self.summon.set_enabled(summon_ok);
        self.updates.set_text(&labels.updates);
        self.quit.set_text(&labels.quit);
    }

    /// Forget the count so the next one is written (a language change
    /// rewords the tooltip).
    pub fn reset_count(&mut self) {
        self.count = None;
    }
}
