//! Local FTS5 index over note text — same log, better index.
//!
//! The JSONL bus remains authority. This SQLite file under `fts/notes.sqlite`
//! is disposable: delete it and the next rebuild (doctor / first search /
//! post-gc) reconstructs from segments. Never a second store of truth.
//!
//! Indexing never fails an append. Search failures fall back to callers.

use crate::alias;
use crate::event::{Event, EventKind};
use crate::store::{Store, Window};
use rusqlite::{Connection, OptionalExtension, params};
use std::fs;
use std::path::PathBuf;

const SCHEMA_VERSION: u32 = 1;
const DEFAULT_LIMIT: usize = 16;
const MAX_LIMIT: usize = 64;

/// One search hit. Bodies are truncated for wire/CLI; `id` is the event ULID.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SearchHit {
    pub id: String,
    pub ts: String,
    pub text: String,
    pub session: String,
    pub harness: String,
    pub alias: Option<String>,
    pub note_kind: String,
    pub to_kind: Option<String>,
    pub to_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FtsStats {
    pub path: String,
    pub present: bool,
    pub schema_version: Option<u32>,
    pub indexed_notes: u64,
    pub newest_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RebuildStats {
    pub indexed: u64,
    pub skipped: u64,
}

#[derive(Debug)]
pub enum FtsError {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    EmptyQuery,
}

impl std::fmt::Display for FtsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FtsError::Io(e) => write!(f, "{e}"),
            FtsError::Sql(e) => write!(f, "{e}"),
            FtsError::EmptyQuery => write!(f, "empty search query"),
        }
    }
}

impl From<std::io::Error> for FtsError {
    fn from(e: std::io::Error) -> Self {
        FtsError::Io(e)
    }
}

impl From<rusqlite::Error> for FtsError {
    fn from(e: rusqlite::Error) -> Self {
        FtsError::Sql(e)
    }
}

pub fn db_path(store: &Store) -> PathBuf {
    store.root().join("fts").join("notes.sqlite")
}

/// Best-effort index of one note after a successful append. Never panics.
pub fn index_note(store: &Store, event: &Event) {
    if event.event != EventKind::Note {
        return;
    }
    let aliases = alias::read(store);
    let alias = alias::get(&aliases, &event.session);
    let _ = with_conn(store, |conn| {
        ensure_schema(conn)?;
        upsert_note(conn, event, alias.as_deref())?;
        Ok(())
    });
}

/// Drop and rebuild from every note still on the JSONL bus.
///
/// Rebuilds into a sibling temp file then renames into place so a concurrent
/// reader (serve / doctor) never sees a half-written index or a deleted WAL.
pub fn rebuild(store: &Store) -> Result<RebuildStats, FtsError> {
    let path = db_path(store);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("sqlite.tmp.{}", std::process::id()));
    let tmp_s = tmp.display().to_string();
    let _ = fs::remove_file(&tmp);
    let _ = fs::remove_file(format!("{tmp_s}-wal"));
    let _ = fs::remove_file(format!("{tmp_s}-shm"));

    let aliases = alias::read(store);
    let read = store.read(Window::All);
    let mut indexed = 0u64;
    let mut skipped = 0u64;

    {
        let conn = Connection::open(&tmp)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;",
        )?;
        ensure_schema(&conn)?;
        let tx = conn.unchecked_transaction()?;
        for ev in &read.events {
            if ev.event != EventKind::Note {
                continue;
            }
            let a = alias::get(&aliases, &ev.session);
            match upsert_note(&tx, ev, a.as_deref()) {
                Ok(()) => indexed += 1,
                Err(_) => skipped += 1,
            }
        }
        set_meta(&tx, "schema_version", &SCHEMA_VERSION.to_string())?;
        set_meta(&tx, "indexed_notes", &indexed.to_string())?;
        if let Some(last) = read
            .events
            .iter()
            .rev()
            .find(|e| e.event == EventKind::Note)
        {
            set_meta(&tx, "newest_id", &last.id)?;
        }
        tx.commit()?;
        // Checkpoint so the rename does not depend on -wal sidecar presence.
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    }

    fs::rename(&tmp, &path)?;
    let _ = fs::remove_file(format!("{}-wal", path.display()));
    let _ = fs::remove_file(format!("{}-shm", path.display()));
    Ok(RebuildStats { indexed, skipped })
}

/// Open or create the index; rebuild when missing or schema-mismatched.
pub fn ensure(store: &Store) -> Result<FtsStats, FtsError> {
    let path = db_path(store);
    if !path.exists() {
        rebuild(store)?;
        return Ok(stats(store));
    }
    let version = with_conn(store, |conn| {
        ensure_schema(conn)?;
        Ok(meta_u32(conn, "schema_version"))
    })?;
    if version != Some(SCHEMA_VERSION) {
        rebuild(store)?;
    }
    Ok(stats(store))
}

pub fn stats(store: &Store) -> FtsStats {
    let path = db_path(store);
    if !path.exists() {
        return FtsStats {
            path: path.display().to_string(),
            present: false,
            schema_version: None,
            indexed_notes: 0,
            newest_id: None,
        };
    }
    with_conn(store, |conn| {
        let indexed = conn
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as u64;
        Ok(FtsStats {
            path: path.display().to_string(),
            present: true,
            schema_version: meta_u32(conn, "schema_version"),
            indexed_notes: indexed,
            newest_id: meta_string(conn, "newest_id"),
        })
    })
    .unwrap_or(FtsStats {
        path: path.display().to_string(),
        present: false,
        schema_version: None,
        indexed_notes: 0,
        newest_id: None,
    })
}

/// Full-text search over note body (+ session / harness / alias columns).
pub fn search(store: &Store, query: &str, limit: usize) -> Result<Vec<SearchHit>, FtsError> {
    let match_expr = match_query(query).ok_or(FtsError::EmptyQuery)?;
    let limit = limit.clamp(1, MAX_LIMIT);
    let _ = ensure(store)?;
    with_conn(store, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, ts, body, session, harness, alias, note_kind, to_kind, to_value \
             FROM notes \
             WHERE notes MATCH ?1 \
             ORDER BY rank \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![match_expr, limit as i64], |row| {
            Ok(SearchHit {
                id: row.get(0)?,
                ts: row.get(1)?,
                text: row.get(2)?,
                session: row.get(3)?,
                harness: row.get(4)?,
                alias: empty_to_none(row.get::<_, String>(5)?),
                note_kind: row.get(6)?,
                to_kind: empty_to_none(row.get::<_, String>(7)?),
                to_value: empty_to_none(row.get::<_, String>(8)?),
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

pub fn default_limit() -> usize {
    DEFAULT_LIMIT
}

/// Build an FTS5 MATCH expression from free text. Returns None when nothing
/// searchable remains after sanitization.
pub fn match_query(raw: &str) -> Option<String> {
    let mut tokens = Vec::new();
    for part in raw.split_whitespace() {
        let cleaned: String = part
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
            .collect();
        if cleaned.len() < 2 {
            continue;
        }
        // Quote + prefix: migration ids and partial words both work.
        tokens.push(format!("\"{}\"*", cleaned.replace('"', "")));
    }
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

fn empty_to_none(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

fn with_conn<T>(
    store: &Store,
    f: impl FnOnce(&Connection) -> Result<T, FtsError>,
) -> Result<T, FtsError> {
    let path = db_path(store);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(&path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;",
    )?;
    f(&conn)
}

fn ensure_schema(conn: &Connection) -> Result<(), FtsError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
         );
         CREATE VIRTUAL TABLE IF NOT EXISTS notes USING fts5(
            body,
            session,
            harness,
            alias,
            id UNINDEXED,
            ts UNINDEXED,
            note_kind UNINDEXED,
            to_kind UNINDEXED,
            to_value UNINDEXED,
            tokenize = 'unicode61 remove_diacritics 2'
         );",
    )?;
    Ok(())
}

fn upsert_note(conn: &Connection, event: &Event, alias: Option<&str>) -> Result<(), FtsError> {
    // FTS5: delete by rowid (UNINDEXED equality in DELETE WHERE is unreliable).
    if let Ok(rowid) = conn.query_row(
        "SELECT rowid FROM notes WHERE id = ?1",
        params![event.id],
        |r| r.get::<_, i64>(0),
    ) {
        conn.execute("DELETE FROM notes WHERE rowid = ?1", params![rowid])?;
    }
    let (to_kind, to_value) = match &event.to {
        Some(t) => (t.kind.as_str().to_string(), t.value.clone()),
        None => (String::new(), String::new()),
    };
    conn.execute(
        "INSERT INTO notes (body, session, harness, alias, id, ts, note_kind, to_kind, to_value)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            event.text,
            event.session,
            event.harness,
            alias.unwrap_or(""),
            event.id,
            event.ts,
            event.kind.as_str(),
            to_kind,
            to_value,
        ],
    )?;
    set_meta(conn, "newest_id", &event.id)?;
    Ok(())
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<(), FtsError> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn meta_string(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
        r.get(0)
    })
    .optional()
    .ok()
    .flatten()
}

fn meta_u32(conn: &Connection, key: &str) -> Option<u32> {
    meta_string(conn, key)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{NoteKind, Role, SCHEMA_VERSION as EV_SCHEMA};
    use crate::test_util::TempHome;

    fn store(home: &TempHome) -> Store {
        Store::at(home.path())
    }

    fn note(home: &TempHome, id: &str, text: &str, session: &str) -> Event {
        let ev = Event {
            v: EV_SCHEMA,
            id: id.into(),
            ts: "2026-07-31T10:00:00Z".into(),
            event: EventKind::Note,
            session: session.into(),
            harness: "cursor".into(),
            host: "test".into(),
            cwd: "/tmp".into(),
            repo: None,
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            text: text.into(),
            to: None,
            kind: NoteKind::Fyi,
            note_ref: None,
        };
        store(home).append(&ev).unwrap();
        ev
    }

    #[test]
    fn match_query_sanitizes_and_prefixes() {
        assert_eq!(
            match_query("0178 migration!"),
            Some("\"0178\"* \"migration\"*".into())
        );
        assert!(match_query("a").is_none());
        assert!(match_query("@@@").is_none());
    }

    #[test]
    fn rebuild_and_search_finds_note_body() {
        let home = TempHome::new();
        let s = store(&home);
        note(
            &home,
            "01TESTNOTE00000000000001",
            "the 0178 migration failed upstream",
            "sess-a",
        );
        note(
            &home,
            "01TESTNOTE00000000000002",
            "unrelated channel hygiene",
            "sess-b",
        );
        let stats = rebuild(&s).unwrap();
        assert_eq!(stats.indexed, 2);
        let hits = search(&s, "0178 migration", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "01TESTNOTE00000000000001");
        assert!(hits[0].text.contains("0178"));
    }

    #[test]
    fn index_on_append_is_searchable_without_explicit_rebuild() {
        let home = TempHome::new();
        let s = store(&home);
        note(
            &home,
            "01TESTNOTE00000000000003",
            "atomic authorize-resume cerco",
            "sess-c",
        );
        let hits = search(&s, "authorize-resume", 8).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session, "sess-c");
    }
}
