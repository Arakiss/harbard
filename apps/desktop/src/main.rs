//! harbard desktop: the native fleet console.
//!
//! One process: GPUI draws the window, the hub thread reads the bus log
//! directly, and writes go to serve on loopback.

// Screens land one by one; parts of the base wait for their first caller.
#![allow(dead_code)]

mod api;
mod assets;
mod bench;
mod chat;
mod derive;
mod desk;
mod drive;
mod hub;
mod huginn;
mod i18n;
mod model;
mod nav;
mod prefs;
mod screens;
mod serve_text;
mod shell;
mod theme;
mod ui;

use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size};

fn main() {
    if std::env::args().any(|a| a == "--bench") {
        bench::run();
        return;
    }
    gpui_kit::application().with_assets(assets::Assets).run(|cx: &mut App| {
        gpui_kit::init(cx);
        assets::load_fonts(cx);
        theme::init(cx);
        ui::kit_theme::init(cx);
        i18n::init(cx);
        nav::init(cx);
        screens::secondary::open_route_from_env(cx);
        desk::Desk::init(cx);
        shell::init(cx);
        open_main(cx);
        drive::init(cx);
        cx.activate(true);
    });
}

fn open_main(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("harbard".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(18.))),
        }),
        window_min_size: Some(size(px(1080.), px(680.))),
        app_id: Some("com.harbard.desktop".into()),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| shell::Workspace::new(window, cx)))
        .expect("open the main window");
}
