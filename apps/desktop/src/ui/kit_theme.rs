//! GPUI Kit's components painted with our `--hb-*` tokens and Lato.
//!
//! Kit's Input, Button, Popover, Dialog, Menu, Tooltip, Switch and Scrollbar
//! read `gpui_component::Theme`. [`init`] fills it from [`crate::theme::Theme`]
//! at startup and again whenever our theme changes (a theme write, the mode,
//! the system appearance), so a Kit component looks like the console's
//! `.hb-input`, `.hb-btn`, `.hb-pop` and `.hb-modal`.
//!
//! Kit paints a dialog with its `background` (our `surface`, which inputs
//! need) and a border. The console's modals are `surface-raised` with the pop
//! shadow and no border: wrap a dialog in [`hb_dialog`] to get that.

use crate::theme::Theme as Hb;
use crate::ui::{self, control, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyView, App, AppContext, BoxShadow, Context, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, div, px,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::{Theme as Kit, ThemeMode, WindowExt as _};
use std::cell::Cell;

/// The radii (`DESIGN.md` R1), by the names the kit and older code use.
pub const RADIUS_SM: f32 = crate::ui::radius::SM;
pub const RADIUS: f32 = crate::ui::radius::BASE;
pub const RADIUS_MD: f32 = crate::ui::radius::MD;
pub const RADIUS_LG: f32 = crate::ui::radius::LG;

pub fn init(cx: &mut App) {
    apply(cx);
    cx.observe_global::<Hb>(apply).detach();
}

fn apply(cx: &mut App) {
    let hb = cx.global::<Hb>().clone();
    let mode = if hb.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    // A mode change loads Kit's registered theme over the colours, so switch
    // first and paint ours in a second edit.
    if Kit::global(cx).mode != mode {
        Kit::change(mode, None, cx);
    }
    Kit::update(cx, |kit| {
        paint(kit, &hb);
        kit.font_family = crate::ui::SANS.into();
        kit.font_size = px(crate::ui::size::UI);
        kit.mono_font_family = crate::ui::MONO.into();
        kit.mono_font_size = px(size::SMALL);
        kit.radius = px(RADIUS);
        kit.radius_lg = px(RADIUS_LG);
        kit.shadow = true;
    });
}

fn paint(kit: &mut Kit, hb: &Hb) {
    let c = |name: &str| hb.c(name);
    let k = &mut kit.colors;

    k.background = c("surface");
    k.foreground = c("ink");
    k.border = c("line");
    k.input = c("line-strong");
    k.ring = c("focus");
    k.caret = c("ink");
    k.selection = c("focus").opacity(0.3);
    k.overlay = c("scrim");
    k.window_border = c("panel-ring");

    k.muted = c("control");
    k.muted_foreground = c("ink-3");
    k.skeleton = c("hover-2");

    // `.hb-pop-item:hover` and the Finder's selected hit: the blue menu wash.
    k.accent = c("menu-hover");
    k.accent_foreground = c("menu-hover-ink");

    k.popover = c("surface-raised");
    k.popover_foreground = c("ink");

    // `.hb-btn-primary`.
    k.primary = c("accent-deep");
    k.primary_hover = c("accent-hover");
    k.primary_active = c("accent-hover");
    k.primary_foreground = c("accent-ink");
    // `.hb-btn`: a surface button with a strong edge.
    k.secondary = c("surface");
    k.secondary_hover = c("btn-hover");
    k.secondary_active = c("control");
    k.secondary_foreground = c("ink");
    k.button = c("surface");
    k.button_hover = c("btn-hover");
    k.button_active = c("control");
    k.button_foreground = c("ink");
    k.button_primary = c("accent-deep");
    k.button_primary_hover = c("accent-hover");
    k.button_primary_active = c("accent-hover");
    k.button_primary_foreground = c("accent-ink");
    k.button_secondary = c("surface");
    k.button_secondary_hover = c("btn-hover");
    k.button_secondary_active = c("control");
    k.button_secondary_foreground = c("ink");

    status(
        &mut k.danger,
        &mut k.danger_hover,
        &mut k.danger_active,
        &mut k.danger_foreground,
        c("red"),
        c("accent-ink"),
    );
    status(
        &mut k.success,
        &mut k.success_hover,
        &mut k.success_active,
        &mut k.success_foreground,
        c("send"),
        c("send-ink"),
    );
    status(
        &mut k.warning,
        &mut k.warning_hover,
        &mut k.warning_active,
        &mut k.warning_foreground,
        c("amber"),
        c("ink"),
    );
    status(
        &mut k.info,
        &mut k.info_hover,
        &mut k.info_active,
        &mut k.info_foreground,
        c("focus"),
        c("menu-hover-ink"),
    );
    k.button_danger = k.danger;
    k.button_danger_hover = k.danger_hover;
    k.button_danger_active = k.danger_active;
    k.button_danger_foreground = k.danger_foreground;
    k.button_success = k.success;
    k.button_success_hover = k.success_hover;
    k.button_success_active = k.success_active;
    k.button_success_foreground = k.success_foreground;
    k.button_warning = k.warning;
    k.button_warning_hover = k.warning_hover;
    k.button_warning_active = k.warning_active;
    k.button_warning_foreground = k.warning_foreground;
    k.button_info = k.info;
    k.button_info_hover = k.info_hover;
    k.button_info_active = k.info_active;
    k.button_info_foreground = k.info_foreground;

    k.link = c("link");
    k.link_hover = c("link");
    k.link_active = c("accent-text");

    k.list = c("surface");
    k.list_hover = c("hover");
    k.list_active = c("selected");
    k.list_active_border = c("accent");
    k.list_even = c("surface-2");
    k.list_head = c("surface-2");

    k.table = c("surface");
    k.table_hover = c("hover");
    k.table_active = c("selected");
    k.table_active_border = c("accent");
    k.table_even = c("surface-2");
    k.table_head = c("surface-2");
    k.table_head_foreground = c("ink-3");
    k.table_foot = c("surface-2");
    k.table_foot_foreground = c("ink-3");
    k.table_row_border = c("line");

    k.tab = c("surface");
    k.tab_active = c("surface");
    k.tab_active_foreground = c("ink");
    k.tab_foreground = c("ink-3");
    k.tab_bar = c("surface");
    k.tab_bar_segmented = c("control");

    k.scrollbar = Hsla::transparent_black();
    k.scrollbar_thumb = c("line-strong");
    k.scrollbar_thumb_hover = c("line-hover");

    k.switch = c("line-strong");
    k.switch_thumb = c("surface");
    k.slider_bar = c("accent");
    k.slider_thumb = c("surface");
    k.progress_bar = c("accent");

    k.sidebar = c("sidebar");
    k.sidebar_foreground = c("sidebar-ink");
    k.sidebar_accent = c("sidebar-hover");
    k.sidebar_accent_foreground = c("sidebar-ink");
    k.sidebar_border = c("sidebar-line");
    k.sidebar_primary = c("accent");
    k.sidebar_primary_foreground = c("accent-ink");

    k.title_bar = c("frame");
    k.title_bar_border = c("frame");
    k.status_bar = c("frame");
    k.status_bar_border = c("frame");
    k.group_box = c("surface-2");
    k.group_box_foreground = c("ink");
    k.description_list_label = c("surface-2");
    k.description_list_label_foreground = c("ink-3");
    k.accordion = c("surface");
    k.drag_border = c("focus");
    k.drop_target = c("focus").opacity(0.15);
    k.chart_grid = c("line");

    k.red = c("red");
    k.red_light = c("red-bg");
    k.green = c("green");
    k.green_light = c("green-bg");
    k.blue = c("focus");
    k.blue_light = c("focus").opacity(0.2);
    k.yellow = c("amber");
    k.yellow_light = c("amber-bg");
    k.magenta = c("st-orbit");
    k.magenta_light = c("chip");
    k.cyan = c("st-web");
    k.cyan_light = c("chip");
}

fn status(
    bg: &mut Hsla,
    hover: &mut Hsla,
    active: &mut Hsla,
    fg: &mut Hsla,
    color: Hsla,
    ink: Hsla,
) {
    *bg = color;
    *hover = color.opacity(0.9);
    *active = color.opacity(0.8);
    *fg = ink;
}

/// A Kit dialog dressed as the console's `.hb-modal` / `.hb-small-dialog`:
/// raised surface, 12px corners, the pop shadow, no border.
pub fn hb_dialog(dialog: Dialog, cx: &App) -> Dialog {
    let hb = cx.global::<Hb>();
    dialog
        .bg(hb.c("surface-raised"))
        .border_0()
        .rounded(px(RADIUS_LG))
        .shadow(hb.pop_shadow())
}

/// A Kit button that is only an icon (a row's or a header's "⋯"): the
/// app's own hover and pressed washes (C8). The Kit's ghost button takes
/// the menu's selection colour instead, which is a menu's, not a button's.
pub fn quiet_icon(button: Button, cx: &App) -> Button {
    use gpui_kit::component::button::ButtonCustomVariant;
    let hb = cx.global::<Hb>();
    let clear = gpui::transparent_black();
    button.custom(
        ButtonCustomVariant::new(cx)
            .color(clear)
            .foreground(hb.c("ink-3"))
            .hover(hb.c("hover"))
            .active(hb.c("control")),
    )
}

/// The primary action of a dialog: the height of the buttons beside it
/// (`control::MD`), 14/700, on `accent-deep`.
pub fn dialog_primary(button: Button) -> Button {
    button
        .primary()
        .h(px(control::MD))
        .px(px(space::LG))
        .rounded(px(radius::BASE))
        .text_size(px(size::UI))
        .font_weight(weight::STRONG)
}

/// Any other button of a dialog: 32 px, 14/700, a surface with a strong edge.
pub fn dialog_button(button: Button) -> Button {
    button
        .h(px(control::MD))
        .px(px(space::LG))
        .rounded(px(radius::BASE))
        .text_size(px(size::UI))
        .font_weight(weight::STRONG)
}

// ---------------------------------------------------------------------------
// Focus ring and tooltips (F1, K2, K3)
// ---------------------------------------------------------------------------

/// F1: reachable with Tab, and ringed in `focus` while the keyboard holds it.
/// `inset` for rows (the ring stays inside the row), outside for controls.
pub trait FocusRing: InteractiveElement + Sized {
    fn focus_ring(self, theme: &Hb, inset: bool) -> Self {
        let ring = ui::focus_ring(theme, inset);
        self.tab_index(0)
            .focus_visible(move |s| s.shadow(vec![ring]))
    }
}

impl<E: InteractiveElement + Sized> FocusRing for E {}

thread_local! {
    /// The shell's own popovers that are open, one bit each.
    static OPEN_MENUS: Cell<u8> = const { Cell::new(0) };
}

/// The shell's popovers, for [`mark_menu`].
pub const MENU_WORKSPACE: u8 = 1;
pub const MENU_CREATE: u8 = 2;
pub const MENU_USER: u8 = 4;

/// A shell popover opened or closed.
pub fn mark_menu(menu: u8, open: bool) {
    let now = OPEN_MENUS.get();
    OPEN_MENUS.set(if open { now | menu } else { now & !menu });
}

/// Something floats over the window: a dialog, the Finder, the lightbox or
/// one of the shell's menus. No tooltip of the page shows meanwhile.
pub fn overlay_open(window: &mut Window, cx: &mut App) -> bool {
    OPEN_MENUS.get() != 0
        || window.has_active_dialog(cx)
        || window.has_active_sheet(cx)
        || crate::chat::attachments::lightbox_open(cx)
}

/// The floating shadow of a tooltip: the pop shadow's ring with a short drop.
fn tip_shadow(theme: &Hb) -> Vec<BoxShadow> {
    let mut shadow = theme.pop_shadow();
    if let Some(drop) = shadow.last_mut() {
        drop.offset.y = px(space::XS);
        drop.blur_radius = px(space::LG);
    }
    shadow
}

/// A tooltip: the name of the thing and, after it, its keys (K2). The one
/// tooltip of the app: every screen's `tip` ends here.
struct Tip {
    label: SharedString,
    keys: Vec<&'static str>,
    /// Its element sits inside a dialog, a popover or the lightbox, so an
    /// open one does not hide it.
    inside: bool,
}

impl Render for Tip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // An open menu, popover, drop-down or dialog owns the pointer: the
        // page under it says nothing, and a button does not name itself
        // over what it opened.
        if !self.inside
            && (overlay_open(window, cx)
                || gpui_kit::component::GlobalState::is_in_deferred_context(cx))
        {
            return div();
        }
        let theme = cx.global::<Hb>();
        div().child(
            div()
                .m(px(space::MD))
                .flex()
                .items_center()
                .gap(px(space::MD))
                .max_w(px(TIP_MAX))
                .px(px(space::MD))
                .py(px(space::XS))
                .rounded(px(radius::BASE))
                .bg(theme.c("surface-raised"))
                .shadow(tip_shadow(theme))
                .child(ui::text(
                    self.label.clone(),
                    size::SMALL,
                    weight::REGULAR,
                    theme.c("ink"),
                ))
                .when(!self.keys.is_empty(), |d| {
                    d.child(ui::kbd_keys(self.keys.iter().copied(), theme))
                }),
        )
    }
}

/// A tooltip is a name, not a paragraph: longer text wraps at this width.
const TIP_MAX: f32 = 360.;

fn tip_view(
    label: SharedString,
    keys: &[&'static str],
    inside: bool,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static + use<> {
    let keys = keys.to_vec();
    move |_, cx| {
        let (label, keys) = (label.clone(), keys.clone());
        cx.new(|_| Tip {
            label,
            keys,
            inside,
        })
        .into()
    }
}

/// `.tooltip(tip(label))`: a tooltip with the thing's name.
pub fn tip(label: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    tip_view(label.into(), &[], false)
}

/// `.tooltip(tip_keys(label, &["⌘", "K"]))`: the name and its keys.
pub fn tip_keys<L: Into<SharedString>>(
    label: L,
    keys: &[&'static str],
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static + use<L> {
    tip_view(label.into(), keys, false)
}

/// The same for something inside a dialog, a popover or the lightbox.
pub fn tip_inside<L: Into<SharedString>>(
    label: L,
    keys: &[&'static str],
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static + use<L> {
    tip_view(label.into(), keys, true)
}
