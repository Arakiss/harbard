//! `GET /api/v1/history/agents` and `GET /api/v1/history/agents/{key}`:
//! History, the sessions and agents of the last month
//! (`harbard_core::history`).
//!
//! Pull-only and cold, like `/history`: nothing here runs on the stream tick
//! or writes anything. The month is read once and kept for
//! [`FRESH_MS`]; every page and every conversation in that time is cut from
//! the same read, so paging through a month is a slice, not a new read. A
//! projector of its own reads the transcripts: the shell's keeps a day, and a
//! pass forgets the files outside its window.

use super::{query_param, workspace_route};
use harbard_core::history::{self, Item, Query};
use harbard_core::team::{TeamProjector, TeamRoots};
use harbard_core::workspace::WorkspaceView;
use harbard_core::{Store, Window, clock};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// How long one read of the month answers pages and conversations.
pub const FRESH_MS: u64 = 30_000;

struct Built {
    root: PathBuf,
    /// The workspace the month was read for (`None`: one workspace).
    workspace: Option<String>,
    at_ms: u64,
    view: WorkspaceView,
    items: Vec<Item>,
}

fn projector() -> &'static Mutex<TeamProjector> {
    static PROJECTOR: OnceLock<Mutex<TeamProjector>> = OnceLock::new();
    PROJECTOR.get_or_init(|| Mutex::new(TeamProjector::new(TeamRoots::from_env())))
}

fn cache() -> &'static Mutex<Option<Arc<Built>>> {
    static CACHE: OnceLock<Mutex<Option<Arc<Built>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// The month, read now or a moment ago. One reader at a time: a second
/// request waits for the first read instead of starting its own.
fn current(store: &Store, stale_secs: u64, ws: Option<&str>) -> Arc<Built> {
    let now = clock::now_ms();
    let placement = harbard_core::workspaces::Placement::load(store);
    let workspace = workspace_route::query_workspace(&placement, ws);
    let mut slot = cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(b) = slot.as_ref()
        && b.root == store.root()
        && b.workspace == workspace
        && now.saturating_sub(b.at_ms) < FRESH_MS
    {
        return Arc::clone(b);
    }
    let events = store.read(Window::All).events;
    let team = projector()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .project_archive(now);
    let gate = match &workspace {
        Some(id) => placement.gate_of(id),
        None => harbard_core::workspaces::Gate::Open,
    };
    let view = workspace_route::archive_view(store, &events, stale_secs, &team, now, &gate);
    let items = history::items(&view);
    let built = Arc::new(Built {
        root: store.root().to_path_buf(),
        workspace: workspace.clone(),
        at_ms: now,
        view,
        items,
    });
    *slot = Some(Arc::clone(&built));
    built
}

/// The `ws` of a query string, decoded.
fn ws_of(query: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("ws="))
        .map(decode)
}

/// A query value as the browser sent it: `+` is a space, `%XX` a byte.
/// Bytes that do not form UTF-8 are replaced, never trusted.
pub fn decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut i = 0;
    while i < bytes.len() {
        let pair = bytes
            .get(i + 1..i + 3)
            .and_then(|p| Some((hex(p[0])?, hex(p[1])?)));
        match (bytes[i], pair) {
            (b'%', Some((hi, lo))) => {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
            (b'+', _) => out.push(b' '),
            (b, _) => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The page a query string asks for.
pub fn query_of(query: &str) -> Query {
    let get = |k: &str| query_param(query, k).map(|v| decode(&v));
    Query {
        before: get("before").filter(|v| !v.is_empty()),
        limit: get("limit").and_then(|v| v.trim().parse().ok()),
        channel: get("channel").filter(|v| !v.trim().is_empty()),
        q: get("q").filter(|v| !v.trim().is_empty()),
    }
}

/// `GET /api/v1/history/agents?before=&limit=&channel=&q=`.
pub fn agents_body(store: &Store, stale_secs: u64, query: &str) -> Value {
    let ws = ws_of(query);
    let built = current(store, stale_secs, ws.as_deref());
    let page = history::page(&built.items, &query_of(query), built.view.now_ms);
    serde_json::to_value(&page).unwrap_or_else(|e| json!({ "error": e.to_string() }))
}

/// `GET /api/v1/history/agents/{key}`: one conversation, read-only when its
/// session ended.
pub fn conversation_body(
    store: &Store,
    stale_secs: u64,
    path: &str,
    query: &str,
) -> Result<Value, (u16, Value)> {
    let key = decode(path.strip_prefix("/api/v1/history/agents/").unwrap_or(""));
    let key = key.trim();
    if key.is_empty() || key.contains('/') {
        return Err((
            400,
            json!({ "error": "expected /api/v1/history/agents/{key}" }),
        ));
    }
    let ws = ws_of(query);
    let built = current(store, stale_secs, ws.as_deref());
    let Some(c) = history::conversation(&built.view, &built.items, key) else {
        return Err((404, json!({ "error": "unknown_agent", "key": key })));
    };
    serde_json::to_value(&c).map_err(|e| (500, json!({ "error": e.to_string() })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_value_is_decoded_like_a_form() {
        assert_eq!(decode("br%C3%BAjula+roja"), "brújula roja");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
        assert_eq!(decode("%FF"), "\u{FFFD}");
    }

    #[test]
    fn a_page_query_reads_every_field_and_drops_blanks() {
        let q = query_of("before=1794000000000%3Aa1&limit=25&channel=harbard&q=br%C3%BAjula");
        assert_eq!(q.before.as_deref(), Some("1794000000000:a1"));
        assert_eq!(q.limit, Some(25));
        assert_eq!(q.channel.as_deref(), Some("harbard"));
        assert_eq!(q.q.as_deref(), Some("brújula"));
        assert_eq!(query_of("channel=&q=+&limit=x"), Query::default());
    }

    #[test]
    fn a_conversation_path_without_a_key_is_refused_before_any_read() {
        // No store is read for a malformed path: the check comes first.
        let store = Store::at(std::path::Path::new("/nonexistent-harbard-history"));
        for path in [
            "/api/v1/history/agents/",
            "/api/v1/history/agents/a/b",
            "/api/v1/history/agents/%2F",
        ] {
            let (status, _) = conversation_body(&store, 900, path, "").unwrap_err();
            assert_eq!(status, 400, "{path}");
        }
    }
}
