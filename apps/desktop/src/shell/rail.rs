//! The rail (`shell.tsx` `Rail`): the living logo and the workspace tiles,
//! the four destinations plus Search, then Create and the operator's avatar
//! at the foot. The workspace menu and the Create popover open from here.
//!
//! With one workspace the logo is the workspace menu's button. With several,
//! the logo heads the rail, the tiles switch workspace (where you are first
//! and largest), and the menu opens from the Home sidebar's workspace name
//! ([`super::open_workspace_menu`]). However many there are, the tiles
//! scroll in their own column when the window is too short for them, and
//! the menu scrolls inside the window; "New workspace…" is in the menu.

use super::brand::LivingMark;
use super::decor;
use super::emblem::{emblem, palettes_for};
use super::menu::{
    Action, MenuItem, float, menu_rows, menu_step, pop_head, pop_item, pop_panel, pop_title,
};
use super::update::UpdateStore;
use super::user_menu::UserMenu;
use crate::derive::workspaces::{is_multi, switch_targets};
use crate::desk::{Connection, Desk};
use crate::i18n::{t, tv};
use crate::model::WorkspaceTile;
use crate::nav::{self, Destination, Nav, Route};
use crate::theme::Theme;
use crate::ui::kit_theme::{self, FocusRing as _};
use crate::ui::{self, avatar_size, control, icon, icon_size, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, AppContext, Context, Entity, FocusHandle, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Pixels, Point, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, div, point, px,
};
use std::rc::Rc;

struct WsMenu {
    at: Point<Pixels>,
    /// The keyboard's item, once a key was used.
    kb: Option<usize>,
}

pub struct Rail {
    mark: Entity<LivingMark>,
    user: Entity<UserMenu>,
    ws_menu: Option<WsMenu>,
    /// The workspace menu's rows, which scroll when they outgrow the window.
    ws_scroll: ScrollHandle,
    create: bool,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
}

impl Rail {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe(&UpdateStore::get(cx), |_, _, cx| cx.notify())
            .detach();
        cx.observe_global::<Nav>(|_, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        Rail {
            mark: cx.new(|cx| LivingMark::new(window, cx)),
            user: cx.new(|cx| UserMenu::new(window, cx)),
            ws_menu: None,
            ws_scroll: ScrollHandle::new(),
            create: false,
            focus: cx.focus_handle(),
            restore: None,
        }
    }

    /// Open the workspace menu at `at` (window coordinates); `None` = under
    /// the logo tile. Opening it again closes it.
    pub fn open_ws_menu(
        &mut self,
        at: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ws_menu.is_some() {
            self.close_ws_menu(window, cx);
            return;
        }
        self.create = false;
        self.ws_menu = Some(WsMenu {
            at: at.unwrap_or(point(px(super::RAIL + space::HAIR), px(space::LG))),
            kb: None,
        });
        self.ws_scroll = ScrollHandle::new();
        self.restore = window.focused(cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Open or close the Create menu. Open, it holds the focus (Esc closes
    /// it, Tab walks its rows); closed, the focus goes back where it was.
    fn set_create(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.create == open {
            return;
        }
        self.create = open;
        if open {
            self.ws_menu = None;
            self.restore = window.focused(cx);
            self.focus.focus(window, cx);
        } else if let Some(f) = self.restore.take() {
            f.focus(window, cx);
        }
        cx.notify();
    }

    fn close_ws_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ws_menu = None;
        if let Some(f) = self.restore.take() {
            f.focus(window, cx);
        }
        cx.notify();
    }

    /// The workspace menu's items, in the console's order.
    fn ws_items(&self, cx: &App) -> Vec<MenuItem> {
        let desk = Desk::get(cx).read(cx);
        let tiles: &[WorkspaceTile] = desk.view.as_ref().map_or(&[], |v| &v.workspaces);
        let ws_name = tiles
            .iter()
            .find(|t| t.current)
            .map_or("harbard".to_string(), |t| t.name.clone());
        let update = UpdateStore::get(cx).read(cx);
        let mut items = Vec::new();
        let item = |id: &str, label: SharedString, icon: &'static str, run: Action| MenuItem {
            id: SharedString::from(id.to_string()),
            label,
            icon,
            dot: false,
            keys: &[],
            run,
        };
        for tile in switch_targets(tiles) {
            let label = if tile.working > 0 {
                tv(
                    cx,
                    "shell.switchToWorking",
                    &[("name", &tile.name), ("count", &tile.working.to_string())],
                )
            } else {
                tv(cx, "shell.switchTo", &[("name", &tile.name)])
            };
            let id = tile.id.clone();
            items.push(item(
                &format!("espacio-{}", tile.id),
                label,
                "arrow-left-right",
                Rc::new(move |_, cx| switch_to(id.clone(), cx)),
            ));
        }
        if desk.view.is_some() {
            items.push(item(
                "new-workspace",
                t(cx, "shell.newWorkspace"),
                "plus",
                Rc::new(crate::prefs::open_new_workspace),
            ));
        }
        items.push(item(
            "preferences",
            t(cx, "shell.preferences"),
            "settings",
            Rc::new(|w, cx| crate::prefs::open_preferences(None, w, cx)),
        ));
        items.push(item(
            "themes",
            t(cx, "shell.themes"),
            "palette",
            Rc::new(|w, cx| crate::prefs::open_preferences(Some("themes"), w, cx)),
        ));
        items.push(item(
            "shortcuts",
            t(cx, "shell.keyboardShortcuts"),
            "keyboard",
            Rc::new(super::dialogs::open_shortcuts),
        ));
        if !tiles.is_empty() {
            items.push(item(
                "espacio",
                tv(cx, "shell.workspaceSettingsFor", &[("name", &ws_name)]),
                "settings",
                Rc::new(crate::prefs::open_workspace_settings),
            ));
        }
        // The session retro: hard-coded in the console too.
        items.push(item(
            "retro",
            "Retro".into(),
            "lightbulb",
            Rc::new(|_, cx| nav::navigate(Route::Retro, cx)),
        ));
        if desk.view.is_some() {
            let mut read = item(
                "read",
                t(cx, "shell.markAllRead"),
                "check-check",
                Rc::new(|_, cx| super::read_all(cx)),
            );
            read.keys = &["⇧", "Esc"];
            items.push(read);
        }
        if !update.unsupported && update.view.is_some() {
            let available = update.available();
            let mut it = item(
                "update",
                if available {
                    t(cx, "shell.updateHarbard")
                } else {
                    t(cx, "shell.checkForUpdates")
                },
                "circle-arrow-up",
                Rc::new(crate::prefs::open_update_dialog),
            );
            it.dot = available;
            items.push(it);
        }
        items.push(item(
            "acerca",
            t(cx, "shell.aboutHarbard"),
            "info",
            Rc::new(super::dialogs::open_about),
        ));
        items.push(item(
            "strip",
            t(cx, "shell.openStrip"),
            "radar",
            Rc::new(|_, cx| open_strip(cx)),
        ));
        items
    }

    fn on_ws_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.ws_items(cx);
        let Some(menu) = self.ws_menu.as_mut() else {
            return;
        };
        match ev.keystroke.key.as_str() {
            k @ ("up" | "down" | "home" | "end") => {
                menu.kb = menu_step(menu.kb, items.len(), k);
                if let Some(i) = menu.kb {
                    self.ws_scroll.scroll_to_item(i);
                }
            }
            "escape" => self.close_ws_menu(window, cx),
            "enter" | "space" => {
                if let Some(item) = menu.kb.and_then(|i| items.get(i)) {
                    let run = item.run.clone();
                    self.close_ws_menu(window, cx);
                    run(window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

/// Switch workspace: remembered, re-read at once, and Home when elsewhere.
pub fn switch_to(id: String, cx: &mut App) {
    Desk::get(cx).update(cx, |d, cx| d.switch_workspace(Some(id), cx));
    if !matches!(
        Nav::route(cx),
        Route::Home {
            channel: None,
            thread: None,
            item: None
        }
    ) {
        nav::navigate(Route::home(), cx);
    }
}

/// "Open Strip": the desktop's Strip window.
fn open_strip(cx: &mut App) {
    crate::native::open_strip(cx);
}

struct Dest {
    dest: Destination,
    label: SharedString,
    icon: &'static str,
    route: Route,
    badge: u64,
}

/// What a tile's corner says: the open "Needs you" count, a dot for unread, or nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum TileBadge {
    Count(String),
    Dot,
}

pub fn tile_badge(tile: &WorkspaceTile) -> Option<TileBadge> {
    if tile.needs > 0 {
        return Some(TileBadge::Count(if tile.needs > 99 {
            "99+".into()
        } else {
            tile.needs.to_string()
        }));
    }
    (!tile.current && tile.unread > 0).then_some(TileBadge::Dot)
}

/// True when the operator typed the tile's text; by default it is the name's first letter.
pub fn custom_tile(tile: &WorkspaceTile) -> bool {
    let first = tile
        .name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "·".into());
    tile.tile != first
}

/// The rail's order: where you are first, then the others as the hub lists them.
pub fn rail_order(tiles: &[WorkspaceTile]) -> Vec<&WorkspaceTile> {
    tiles
        .iter()
        .filter(|t| t.current)
        .chain(tiles.iter().filter(|t| !t.current))
        .collect()
}

/// The brand's tile when it is the workspace menu's button.
const BRAND: f32 = 48.;
/// A workspace tile: where you are, and the others.
const TILE: f32 = control::LG;
const TILE_OTHER: f32 = control::SM;
/// A destination's column, and the tile behind its icon.
const DEST_WIDTH: f32 = 72.;
const DEST_TILE: f32 = control::LG;
/// A badge on the rail: 16 px inside the frame's 2 px cut-out.
const BADGE: f32 = 20.;
/// A corner dot: 8 px inside the same cut-out.
const CORNER_DOT: f32 = 12.;
/// The least the workspace menu shows before it scrolls: its head and a few rows.
const MENU_MIN_H: f32 = 160.;

/// `.hb-rail-badge`: the count with the frame's cut-out around it.
fn rail_badge(text: String, theme: &Theme) -> impl IntoElement {
    div()
        .absolute()
        .top(px(-space::SM))
        .right(px(-space::MD))
        .min_w(px(BADGE))
        .h(px(BADGE))
        .px(px(space::HAIR))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .border_2()
        .border_color(theme.c("frame"))
        .bg(theme.c("badge"))
        .child(ui::text(
            text,
            size::MICRO,
            weight::STRONG,
            theme.c("accent-deep"),
        ))
}

/// A corner dot with the frame's cut-out (`.hb-ws-dot`, `.hb-ws-working`).
fn corner_dot(color: gpui::Hsla, top: bool, theme: &Theme) -> impl IntoElement {
    div()
        .absolute()
        .right(px(-space::XS))
        .when(top, |d| d.top(px(-space::XS)))
        .when(!top, |d| d.bottom(px(-space::XS)))
        .size(px(CORNER_DOT))
        .rounded_full()
        .border_2()
        .border_color(theme.c("frame"))
        .bg(color)
}

/// What a workspace tile says under the pointer: its name, and who works there.
fn tile_tip(tile: &WorkspaceTile, cx: &App) -> SharedString {
    if tile.working > 0 {
        tv(
            cx,
            "shell.tileWorking",
            &[("name", &tile.name), ("count", &tile.working.to_string())],
        )
    } else {
        tile.name.clone().into()
    }
}

impl Render for Rail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        kit_theme::mark_menu(kit_theme::MENU_WORKSPACE, self.ws_menu.is_some());
        kit_theme::mark_menu(kit_theme::MENU_CREATE, self.create);
        // The rail's buttons report their boxes again below: snow keeps out of them.
        decor::forget_clear("rail-");
        let theme = cx.global::<Theme>().clone();
        let desk = Desk::get(cx).read(cx);
        let (directs, activity) = match (&desk.activity, &desk.view) {
            (Some(a), _) => (a.unread.directs, a.unread.activity),
            (None, Some(v)) => (v.unread.directs, v.unread.activity),
            _ => (0, 0),
        };
        let tiles: Vec<WorkspaceTile> = desk
            .view
            .as_ref()
            .map(|v| v.workspaces.clone())
            .unwrap_or_default();
        let multi = is_multi(&tiles);
        let connection = desk.connection();
        let active_agents = desk.view.as_ref().map_or(0, |v| {
            v.agents
                .iter()
                .filter(|a| !a.temporary && !a.lost && a.presence != "finished")
                .count()
        });
        let update_dot = UpdateStore::get(cx).read(cx).available();
        let current = Nav::route(cx).destination();
        let rail_ink = theme.c("rail-ink");
        let rail_hover = theme.c("rail-hover");

        // 1–2. The living logo and the workspace tiles.
        let head: AnyElement = if multi {
            let palettes = palettes_for(tiles.iter().map(|t| t.id.as_str()));
            let tile_els = rail_order(&tiles).into_iter().map(|tile| {
                let cur = tile.current;
                let (size, radius, letter) = if cur {
                    (TILE, radius::MD, size::UI)
                } else {
                    (TILE_OTHER, radius::BASE, size::META)
                };
                let badge = tile_badge(tile);
                let id = tile.id.clone();
                let letters = custom_tile(tile).then(|| tile.tile.clone());
                div()
                    .id(SharedString::from(format!("hb-ws-tile-{}", tile.id)))
                    .occlude()
                    .relative()
                    .flex_none()
                    .size(px(size))
                    .rounded(px(radius))
                    .child(decor::keep_clear(format!("rail-tile-{}", tile.id)))
                    .tooltip(kit_theme::tip(tile_tip(tile, cx)))
                    .when(!cur, |d| {
                        d.opacity(0.86)
                            .hover(|s| s.opacity(1.))
                            .cursor_pointer()
                            .focus_ring(&theme, false)
                    })
                    .on_click(move |_, _, cx| {
                        if !cur {
                            switch_to(id.clone(), cx);
                        }
                    })
                    .child(emblem(
                        &tile.id,
                        palettes.get(&tile.id).copied(),
                        size,
                        radius,
                    ))
                    .when(cur, |d| {
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded(px(radius))
                                .border_1()
                                .border_color(theme.c("rail-ring")),
                        )
                    })
                    .when_some(letters, |d, l| {
                        d.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(ui::text(l, letter, weight::STRONG, rail_ink)),
                        )
                    })
                    .map(|d| match badge {
                        Some(TileBadge::Count(n)) => d.child(rail_badge(n, &theme)),
                        Some(TileBadge::Dot) => d.child(corner_dot(theme.c("badge"), true, &theme)),
                        None => d,
                    })
                    .when(!cur && tile.working > 0, |d| {
                        d.child(corner_dot(theme.c("green-ink"), false, &theme))
                    })
            });
            // The brand and the tiles are two children of the rail: its gap sits between them.
            // The tiles give way first when the window is short: their column
            // shrinks and scrolls, the destinations under it keep their size.
            div()
                .flex()
                .flex_col()
                .items_center()
                .min_h_0()
                .gap(px(space::SM))
                .child(
                    div()
                        .flex_none()
                        .h(px(BRAND))
                        .mb(px(space::HAIR))
                        .flex()
                        .items_center()
                        .justify_center()
                        .relative()
                        .child(decor::keep_clear("rail-brand"))
                        .child(self.mark.clone()),
                )
                .child(
                    // As wide as the rail, so a tile's corner badge is not cut
                    // by the scroll; the room above and below is the badges'.
                    div()
                        .id("hb-ws-tiles")
                        .w(px(super::RAIL))
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(space::MD))
                        .py(px(space::SM))
                        .overflow_y_scroll()
                        .children(tile_els),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(DEST_TILE + space::MD))
                        .h(px(1.))
                        .mb(px(space::LG))
                        .bg(theme.c("rail-ring")),
                )
                .into_any_element()
        } else {
            let open = self.ws_menu.is_some();
            let mark = self.mark.clone();
            div()
                .id("hb-ws-brand")
                .occlude()
                .relative()
                .mb(px(space::MD))
                .size(px(BRAND))
                .rounded(px(radius::LG))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(open, |d| d.bg(rail_hover))
                .hover(move |s| s.bg(rail_hover))
                .focus_ring(&theme, false)
                .tooltip(kit_theme::tip(t(cx, "shell.harbardMenu")))
                .on_hover(move |on, _, cx| mark.update(cx, |m, cx| m.set_hover(*on, cx)))
                .on_click(cx.listener(|r, _, window, cx| r.open_ws_menu(None, window, cx)))
                .child(decor::keep_clear("rail-brand"))
                .child(self.mark.clone())
                .when(update_dot, |d| {
                    d.child(corner_dot(theme.c("amber"), true, &theme))
                })
                .into_any_element()
        };

        // 3. Destinations, then Search.
        let dests = [
            Dest {
                dest: Destination::Home,
                label: t(cx, "workspace.railHome"),
                icon: "house",
                route: Route::home(),
                badge: 0,
            },
            Dest {
                dest: Destination::Direct,
                label: t(cx, "workspace.railDirect"),
                icon: "messages-square",
                route: Route::direct(None),
                badge: directs,
            },
            Dest {
                dest: Destination::Activity,
                label: t(cx, "workspace.railActivity"),
                icon: "bell",
                route: Route::activity(),
                badge: activity,
            },
            Dest {
                dest: Destination::Later,
                label: t(cx, "workspace.railLater"),
                icon: "bookmark",
                route: Route::Later,
                badge: 0,
            },
            Dest {
                dest: Destination::Search,
                label: t(cx, "shell.search"),
                icon: "search",
                route: Route::Search { q: String::new() },
                badge: 0,
            },
        ];
        let dest_els = dests.into_iter().map(|d| {
            let active = current == Some(d.dest);
            let route = d.route.clone();
            let group: SharedString = format!("hb-dest-{:?}", d.dest).into();
            div()
                .id(group.clone())
                .occlude()
                .group(group.clone())
                .flex_none()
                .w(px(DEST_WIDTH))
                .pt(px(space::HAIR))
                .pb(px(space::SM))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(space::XS))
                .rounded(px(radius::MD))
                .cursor_pointer()
                .focus_ring(&theme, true)
                .on_click(move |_, _, cx| nav::navigate(route.clone(), cx))
                .relative()
                .child(decor::keep_clear(format!("rail-dest-{:?}", d.dest)))
                .child(
                    div()
                        .relative()
                        .size(px(DEST_TILE))
                        .rounded(px(radius::MD))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(active, |e| e.bg(theme.c("rail-active")))
                        .when(!active, |e| {
                            e.group_hover(group.clone(), move |s| s.bg(rail_hover))
                        })
                        .child(icon(d.icon, icon_size::RAIL, rail_ink))
                        .when(d.badge > 0, |e| {
                            e.child(rail_badge(
                                if d.badge > 99 {
                                    "99+".into()
                                } else {
                                    d.badge.to_string()
                                },
                                &theme,
                            ))
                        }),
                )
                .child(
                    ui::text(
                        d.label,
                        size::META,
                        if active {
                            weight::STRONG
                        } else {
                            weight::REGULAR
                        },
                        if active {
                            rail_ink
                        } else {
                            theme.c("rail-text")
                        },
                    )
                    .max_w_full()
                    .truncate(),
                )
        });

        // 5. Create.
        let create_on = self.create;
        let create = div()
            .id("hb-create")
            .occlude()
            .flex_none()
            .mt(px(space::SM))
            .size(px(control::LG))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rail_ink)
            .when(!create_on, |d| {
                let text = theme.c("rail-text");
                d.hover(move |s| s.bg(text))
            })
            .cursor_pointer()
            .focus_ring(&theme, false)
            .tooltip(kit_theme::tip(t(cx, "shell.create")))
            .on_click(cx.listener(|r, _, window, cx| {
                r.set_create(!r.create, window, cx);
            }))
            .relative()
            .child(decor::keep_clear("rail-create"))
            .child(icon("plus", icon_size::RAIL, theme.c("rail-on-ink")));

        let vh = window.viewport_size().height;
        let create_pop = self.create.then(|| {
            let item = |id: &'static str, label: SharedString, to: Route| {
                pop_item(id, false, &theme)
                    .child(label)
                    .on_click(cx.listener(move |r, _, window, cx| {
                        r.set_create(false, window, cx);
                        nav::navigate(to.clone(), cx);
                    }))
            };
            float(
                "hb-create-in",
                pop_panel(&theme)
                    .id("hb-create-pop")
                    .occlude()
                    .track_focus(&self.focus)
                    .on_mouse_down_out(
                        cx.listener(|r, _, window, cx| r.set_create(false, window, cx)),
                    )
                    .on_key_down(cx.listener(|r, ev: &KeyDownEvent, window, cx| {
                        if ev.keystroke.key == "escape" {
                            r.set_create(false, window, cx);
                        }
                    }))
                    .child(pop_title(t(cx, "shell.create"), &theme))
                    .child(item(
                        "hb-create-agent",
                        t(cx, "shell.messageAgent"),
                        Route::direct(None),
                    ))
                    .child(item(
                        "hb-create-channel",
                        t(cx, "shell.messageChannel"),
                        Route::home(),
                    )),
                // Beside the Create button, its foot on the button's.
                point(
                    px(super::RAIL + space::HAIR),
                    vh - px(space::LG + avatar_size::ROW + space::SM),
                ),
                Anchor::BottomLeft,
                window,
                cx,
            )
        });

        // The workspace menu.
        let ws_pop = self.ws_menu.as_ref().map(|menu| {
            let items = self.ws_items(cx);
            let status = workspace_status(connection, active_agents, cx);
            let me = cx.entity();
            let rows = menu_rows(&items, menu.kb, &theme, |i| {
                let me = me.clone();
                let run = items[i].run.clone();
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    me.update(cx, |r, cx| r.close_ws_menu(window, cx));
                    run(window, cx);
                })
            });
            // However many workspaces there are, the menu stays inside the
            // window: its rows scroll under the head.
            let room = (vh - menu.at.y - px(space::LG)).max(px(MENU_MIN_H));
            float(
                "hb-ws-menu-in",
                pop_panel(&theme)
                    .id("hb-ws-menu")
                    .occlude()
                    .max_h(room)
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(Self::on_ws_key))
                    .on_mouse_down_out(cx.listener(|r, _, window, cx| r.close_ws_menu(window, cx)))
                    .child(pop_head("harbard", status, &theme).flex_none())
                    .child(
                        div()
                            .id("hb-ws-menu-rows")
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .track_scroll(&self.ws_scroll)
                            .children(rows),
                    ),
                menu.at,
                Anchor::TopLeft,
                window,
                cx,
            )
        });

        super::drag_band()
            .w(px(super::RAIL))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(space::SM))
            .pt(px(space::XS))
            .pb(px(space::LG))
            .text_color(rail_ink)
            .child(head)
            .children(dest_els)
            .child(div().flex_1())
            .child(create)
            .child(div().flex_none().child(self.user.clone()))
            .children(create_pop)
            .children(ws_pop)
    }
}

/// "serve is responding · 4 active agents".
pub fn workspace_status(connection: Connection, active: usize, cx: &App) -> SharedString {
    let serve = match connection {
        Connection::Connected => t(cx, "shell.serveResponding"),
        Connection::Connecting => t(cx, "shell.serveConnecting"),
        Connection::Disconnected => t(cx, "shell.serveNotResponding"),
    };
    let agents = if active == 1 {
        t(cx, "shell.oneActiveAgent")
    } else {
        tv(cx, "shell.activeAgents", &[("count", &active.to_string())])
    };
    format!("{serve} · {agents}").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(name: &str, letters: &str, current: bool, needs: u64, unread: u64) -> WorkspaceTile {
        WorkspaceTile {
            id: name.to_lowercase(),
            name: name.into(),
            tile: letters.into(),
            current,
            needs,
            unread,
            ..Default::default()
        }
    }

    #[test]
    fn a_tile_says_needs_first_then_unread_elsewhere() {
        assert_eq!(
            tile_badge(&tile("A", "A", true, 3, 9)),
            Some(TileBadge::Count("3".into()))
        );
        assert_eq!(
            tile_badge(&tile("A", "A", false, 120, 0)),
            Some(TileBadge::Count("99+".into()))
        );
        assert_eq!(
            tile_badge(&tile("A", "A", false, 0, 2)),
            Some(TileBadge::Dot)
        );
        assert_eq!(tile_badge(&tile("A", "A", true, 0, 2)), None);
    }

    #[test]
    fn letters_count_as_custom_only_when_typed() {
        assert!(!custom_tile(&tile("personal", "P", true, 0, 0)));
        assert!(custom_tile(&tile("personal", "PW", true, 0, 0)));
        assert!(!custom_tile(&tile("  ¡hola", "H", true, 0, 0)));
    }

    #[test]
    fn the_current_tile_goes_first() {
        let tiles = vec![
            tile("A", "A", false, 0, 0),
            tile("B", "B", true, 0, 0),
            tile("C", "C", false, 0, 0),
        ];
        let order: Vec<&str> = rail_order(&tiles).iter().map(|t| t.name.as_str()).collect();
        assert_eq!(order, vec!["B", "A", "C"]);
    }
}
