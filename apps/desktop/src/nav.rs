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

    /// Read a console URL (`/c/harbard?thread=…`, `/direct/<key>?profile=1`,
    /// `/activity?tab=todo`, `/search?q=…`): deep links, notifications and the
    /// development `HARBARD_APP_ROUTE` all name places this way.
    pub fn parse(url: &str) -> Option<Route> {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        let param = |name: &str| -> Option<String> {
            query.split('&').find_map(|kv| {
                let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                (k == name).then(|| decode(v))
            })
        };
        let path = path.trim_end_matches('/');
        let route = match path {
            "" => Route::Home {
                channel: None,
                thread: param("thread"),
                item: param("t"),
            },
            "/lobby" => Route::Lobby,
            "/threads" => Route::Threads { thread: param("thread") },
            "/deliveries" => Route::Deliveries,
            "/direct" => Route::direct(None),
            "/activity" => Route::Activity {
                tab: param("tab").unwrap_or_else(|| "needs".into()),
                sel: param("sel"),
            },
            "/later" => Route::Later,
            "/search" => Route::Search { q: param("q").unwrap_or_default() },
            "/history" => Route::History {
                channel: param("channel"),
                q: param("q").unwrap_or_default(),
            },
            "/retro" => Route::Retro,
            "/setup" => Route::Setup,
            p => {
                if let Some(id) = p.strip_prefix("/c/") {
                    let mut r = Route::channel(decode(id));
                    if let Route::Home { thread, item, .. } = &mut r {
                        *thread = param("thread");
                        *item = param("t");
                    }
                    r
                } else if let Some(agent) = p.strip_prefix("/direct/") {
                    Route::Direct {
                        agent: Some(decode(agent)),
                        profile: param("profile").as_deref() == Some("1"),
                        live: param("direct").as_deref() == Some("1"),
                    }
                } else {
                    Route::DeletedRoom {
                        name: decode(p.strip_prefix("/history/rooms/")?),
                    }
                }
            }
        };
        Some(route)
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

/// Percent-decoding for one URL part (`+` is a space in a query).
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_urls_name_the_same_places() {
        assert_eq!(Route::parse("/"), Some(Route::home()));
        assert_eq!(Route::parse("/c/lobby"), Some(Route::Lobby));
        assert_eq!(
            Route::parse("/c/harbard?thread=01AB"),
            Some(Route::Home { channel: Some("harbard".into()), thread: Some("01AB".into()), item: None })
        );
        assert_eq!(
            Route::parse("/direct/session%3Aabc?profile=1"),
            Some(Route::Direct { agent: Some("session:abc".into()), profile: true, live: false })
        );
        assert_eq!(Route::parse("/search?q=two+words"), Some(Route::Search { q: "two words".into() }));
        assert_eq!(Route::parse("/activity?tab=todo"), Some(Route::Activity { tab: "todo".into(), sel: None }));
        assert_eq!(Route::parse("/nowhere"), None);
    }
}
