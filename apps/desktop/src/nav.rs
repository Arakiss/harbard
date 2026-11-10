//! Where the main window is: the console's routes as one enum.
//!
//! `navigate(route, cx)` moves there and re-renders every window; screens read
//! their parameters from `Nav::route(cx)`. Back and forward keep a short
//! history like the browser did.

use gpui::{App, Global};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// `/` (channel `None` = the default channel) and `/c/<id>`.
    Home {
        channel: Option<String>,
        thread: Option<String>,
        /// `?t=` a work item key.
        item: Option<String>,
    },
    Lobby,
    Threads {
        thread: Option<String>,
    },
    Deliveries,
    /// `/direct` and `/direct/<agent>` (`profile=1`, `direct=1` = watch live).
    Direct {
        agent: Option<String>,
        profile: bool,
        live: bool,
    },
    /// `tab` is needs (default), todo or threads.
    Activity {
        tab: String,
        sel: Option<String>,
    },
    Later,
    Search {
        q: String,
    },
    History {
        channel: Option<String>,
        q: String,
    },
    DeletedRoom {
        name: String,
    },
    Retro,
    Setup,
}

impl Default for Route {
    fn default() -> Self {
        Route::home()
    }
}

/// The rail's four destinations plus Search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Home,
    Direct,
    Activity,
    Later,
    Search,
}

impl Route {
    pub fn home() -> Route {
        Route::Home {
            channel: None,
            thread: None,
            item: None,
        }
    }

    pub fn channel(id: impl Into<String>) -> Route {
        let id = id.into();
        if id == "lobby" {
            return Route::Lobby;
        }
        Route::Home {
            channel: Some(id),
            thread: None,
            item: None,
        }
    }

    pub fn direct(agent: Option<String>) -> Route {
        Route::Direct {
            agent,
            profile: false,
            live: false,
        }
    }

    pub fn activity() -> Route {
        Route::Activity {
            tab: "needs".into(),
            sel: None,
        }
    }

    /// Which rail item is lit (`destinationOf`). History, Retro, Setup: none.
    pub fn destination(&self) -> Option<Destination> {
        match self {
            Route::Home { .. } | Route::Lobby | Route::Threads { .. } | Route::Deliveries => Some(Destination::Home),
            Route::Direct { .. } => Some(Destination::Direct),
            Route::Activity { .. } => Some(Destination::Activity),
            Route::Later => Some(Destination::Later),
            Route::Search { .. } => Some(Destination::Search),
            _ => None,
        }
    }

    /// Screens are kept per kind: a change of kind builds the screen anew.
    pub fn kind(&self) -> &'static str {
        match self {
            Route::Home { .. } => "home",
            Route::Lobby => "lobby",
            Route::Threads { .. } => "threads",
            Route::Deliveries => "deliveries",
            Route::Direct { .. } => "direct",
            Route::Activity { .. } => "activity",
            Route::Later => "later",
            Route::Search { .. } => "search",
            Route::History { .. } => "history",
            Route::DeletedRoom { .. } => "deleted-room",
            Route::Retro => "retro",
            Route::Setup => "setup",
        }
    }
}

#[derive(Default)]
pub struct Nav {
    pub route: Route,
    back: Vec<Route>,
    forward: Vec<Route>,
}

impl Global for Nav {}

impl Nav {
    pub fn route(cx: &App) -> &Route {
        &cx.global::<Nav>().route
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(Nav::default());
}

/// Go to `route`; the current one is pushed on the back stack.
pub fn navigate(route: Route, cx: &mut App) {
    let nav = cx.global_mut::<Nav>();
    if nav.route == route {
        return;
    }
    let previous = std::mem::replace(&mut nav.route, route);
    nav.back.push(previous);
    if nav.back.len() > 100 {
        nav.back.remove(0);
    }
    nav.forward.clear();
    cx.refresh_windows();
}

/// Change the current route's parameters without a history entry
/// (the console's `router.replace`).
pub fn replace(route: Route, cx: &mut App) {
    let nav = cx.global_mut::<Nav>();
    if nav.route != route {
        nav.route = route;
        cx.refresh_windows();
    }
}

pub fn back(cx: &mut App) {
    let nav = cx.global_mut::<Nav>();
    if let Some(prev) = nav.back.pop() {
        let cur = std::mem::replace(&mut nav.route, prev);
        nav.forward.push(cur);
        cx.refresh_windows();
    }
}

pub fn forward(cx: &mut App) {
    let nav = cx.global_mut::<Nav>();
    if let Some(next) = nav.forward.pop() {
        let cur = std::mem::replace(&mut nav.route, next);
        nav.back.push(cur);
        cx.refresh_windows();
    }
}
