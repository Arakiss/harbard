//! The rail (`shell.tsx` `Rail`): the living logo and the workspace tiles,
//! the four destinations plus Search, then Create and the operator's avatar
//! at the foot. The workspace menu and the Create popover open from here.
//!
//! With one workspace the logo is the workspace menu's button. With several,
//! the logo heads the rail, the tiles switch workspace (where you are first
//! and largest), and the menu opens from the Home sidebar's workspace name
//! ([`super::open_workspace_menu`]).

use super::brand::LivingMark;
use super::emblem::{emblem, palettes_for};
use super::menu::{
    Action, MenuItem, menu_rows, menu_step, pop_head, pop_item, pop_panel, pop_title,
};
use super::update::UpdateStore;
use super::user_menu::UserMenu;
use crate::desk::{Connection, Desk};
use crate::i18n::{t, tv};
use crate::model::WorkspaceTile;
use crate::nav::{self, Destination, Nav, Route};
use crate::theme::Theme;
use crate::ui::icon;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, AppContext, Context, Entity, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, Point, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point, px,
};
use std::rc::Rc;
use std::time::{Duration, Instant};

struct WsMenu {
    at: Point<Pixels>,
    /// The keyboard's item, once a key was used.
    kb: Option<usize>,
}

pub struct Rail {
    mark: Entity<LivingMark>,
    user: Entity<UserMenu>,
    ws_menu: Option<WsMenu>,
    create: bool,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    /// A press outside closed a popover just now: the click on its own
    /// button must not open it again.
    closed_at: Option<Instant>,
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
            create: false,
            focus: cx.focus_handle(),
            restore: None,
            closed_at: None,
        }
    }

    fn just_closed(&self) -> bool {
        self.closed_at
            .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
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
            at: at.unwrap_or(point(px(80.), px(12.))),
            kb: None,
        });
        self.restore = window.focused(cx);
        self.focus.focus(window, cx);
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
        let multi = tiles.len() > 1;
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
            run,
        };
        if multi {
            for tile in tiles.iter().filter(|t| !t.current) {
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
            items.push(item(
                "read",
                t(cx, "shell.markAllRead"),
                "check-check",
                Rc::new(|_, cx| super::read_all(cx)),
            ));
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
            k @ ("up" | "down" | "home" | "end") => menu.kb = menu_step(menu.kb, items.len(), k),
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

/// `.hb-rail-badge`: the badge pill with the frame's 2px cut-out.
fn rail_badge(text: String, small: bool, theme: &Theme) -> impl IntoElement {
    let (h, min_w, pad, size, top, right) = if small {
        (16., 16., 4., 10., -5., -7.)
    } else {
        (18., 18., 5., 11., -6., -9.)
    };
    div()
        .absolute()
        .top(px(top))
        .right(px(right))
        .min_w(px(min_w))
        .h(px(h))
        .px(px(pad))
        .rounded(px(h / 2.))
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.c("badge"))
        .text_color(theme.c("accent-deep"))
        .text_size(px(size))
        .font_weight(FontWeight::BLACK)
        .shadow(ring(theme.c("frame"), 2.))
        .child(text)
}

fn ring(color: gpui::Hsla, width: f32) -> Vec<gpui::BoxShadow> {
    vec![gpui::BoxShadow {
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(width),
        inset: false,
    }]
}

/// A 10px corner dot with the frame's cut-out (`.hb-ws-dot`, `.hb-ws-working`).
fn corner_dot(color: gpui::Hsla, top: bool, theme: &Theme) -> impl IntoElement {
    div()
        .absolute()
        .right(px(-3.))
        .when(top, |d| d.top(px(-3.)))
        .when(!top, |d| d.bottom(px(-3.)))
        .size(px(10.))
        .rounded_full()
        .bg(color)
        .shadow(ring(theme.c("frame"), 2.))
}

impl Render for Rail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let multi = tiles.len() > 1;
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
                let (size, radius) = if cur { (36., 9.) } else { (28., 7.) };
                let badge = tile_badge(tile);
                let id = tile.id.clone();
                let letters = custom_tile(tile).then(|| tile.tile.clone());
                div()
                    .id(SharedString::from(format!("hb-ws-tile-{}", tile.id)))
                    .occlude()
                    .relative()
                    .size(px(size))
                    .rounded(px(radius))
                    .when(!cur, |d| {
                        d.opacity(0.86).hover(|s| s.opacity(1.)).cursor_pointer()
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
                                .border(px(1.5))
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
                                .text_size(px(19.))
                                .font_weight(FontWeight::BLACK)
                                .text_color(rail_ink)
                                .child(l),
                        )
                    })
                    .map(|d| match badge {
                        Some(TileBadge::Count(n)) => d.child(rail_badge(n, !cur, &theme)),
                        Some(TileBadge::Dot) => d.child(corner_dot(theme.c("badge"), true, &theme)),
                        None => d,
                    })
                    .when(!cur && tile.working > 0, |d| {
                        d.child(corner_dot(theme.c("green-ink"), false, &theme))
                    })
            });
            // The brand and the tiles are two children of the rail: its 6px gap sits between them.
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .h(px(48.))
                        .mb(px(8.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.mark.clone()),
                )
                .child(
                    div()
                        .w(px(44.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(8.))
                        .pb(px(12.))
                        .mb(px(12.))
                        .border_b_1()
                        .border_color(theme.c("rail-ring"))
                        .children(tile_els),
                )
                .into_any_element()
        } else {
            let open = self.ws_menu.is_some();
            let mark = self.mark.clone();
            div()
                .id("hb-ws-brand")
                .occlude()
                .relative()
                .mb(px(10.))
                .size(px(48.))
                .rounded(px(12.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(open, |d| d.bg(rail_hover))
                .hover(move |s| s.bg(rail_hover))
                .on_hover(move |on, _, cx| mark.update(cx, |m, cx| m.set_hover(*on, cx)))
                .on_click(cx.listener(|r, _, window, cx| {
                    if r.just_closed() {
                        return;
                    }
                    r.open_ws_menu(None, window, cx);
                }))
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
                .w(px(72.))
                .pt(px(2.))
                .pb(px(8.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(3.))
                .cursor_pointer()
                .on_click(move |_, _, cx| nav::navigate(route.clone(), cx))
                .child(
                    div()
                        .relative()
                        .size(px(42.))
                        .rounded(px(10.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(active, |e| {
                            e.bg(theme.c("rail-active"))
                                .border_1()
                                .border_color(theme.c("rail-ring"))
                        })
                        .when(!active, |e| {
                            e.group_hover(group.clone(), move |s| s.bg(rail_hover))
                        })
                        .child(icon(d.icon, 24., rail_ink))
                        .when(d.badge > 0, |e| {
                            e.child(rail_badge(
                                if d.badge > 99 {
                                    "99+".into()
                                } else {
                                    d.badge.to_string()
                                },
                                false,
                                &theme,
                            ))
                        }),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::BOLD)
                        .line_height(gpui::relative(1.46))
                        .text_color(if active {
                            rail_ink
                        } else {
                            theme.c("rail-text")
                        })
                        .child(d.label),
                )
        });

        // 5. Create.
        let create_on = self.create;
        let create = div()
            .id("hb-create")
            .occlude()
            .mt(px(6.))
            .size(px(38.))
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
            .on_click(cx.listener(|r, _, _, cx| {
                if r.just_closed() {
                    return;
                }
                r.create = !r.create;
                r.ws_menu = None;
                cx.notify();
            }))
            .child(icon("plus", 20., theme.c("rail-on-ink")));

        let vh = window.viewport_size().height;
        let create_pop = self.create.then(|| {
            let item = |id: &'static str, label: SharedString, to: Route| {
                pop_item(id, false, &theme)
                    .child(label)
                    .on_click(cx.listener(move |r, _, _, cx| {
                        r.create = false;
                        nav::navigate(to.clone(), cx);
                        cx.notify();
                    }))
            };
            deferred(
                anchored()
                    .anchor(Anchor::BottomLeft)
                    .position(point(px(80.), vh - px(70.)))
                    .snap_to_window()
                    .child(
                        pop_panel(&theme)
                            .id("hb-create-pop")
                            .occlude()
                            .on_mouse_down_out(cx.listener(|r, _, _, cx| {
                                r.create = false;
                                r.closed_at = Some(Instant::now());
                                cx.notify();
                            }))
                            .on_key_down(cx.listener(|r, ev: &KeyDownEvent, _, cx| {
                                if ev.keystroke.key == "escape" {
                                    r.create = false;
                                    cx.notify();
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
                    ),
            )
            .with_priority(1)
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
            deferred(
                anchored().position(menu.at).snap_to_window().child(
                    pop_panel(&theme)
                        .id("hb-ws-menu")
                        .occlude()
                        .track_focus(&self.focus)
                        .on_key_down(cx.listener(Self::on_ws_key))
                        .on_mouse_down_out(cx.listener(|r, _, window, cx| {
                            r.closed_at = Some(Instant::now());
                            r.close_ws_menu(window, cx);
                        }))
                        .child(pop_head("harbard", status, &theme))
                        .children(rows),
                ),
            )
            .with_priority(1)
        });

        super::drag_band()
            .w(px(78.))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .pt(px(4.))
            .pb(px(14.))
            .text_color(rail_ink)
            .child(head)
            .children(dest_els)
            .child(div().flex_1())
            .child(create)
            .child(self.user.clone())
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
