//! Boring storage: append-only JSONL daily segments under `~/.harbard/events/`.
//!
//! Writes are a single `write_all` of a `<= MAX_LINE_BYTES` line to an
//! `O_APPEND` handle, which POSIX makes atomic for regular files. Reads skip-and-count corrupt lines and are never
//! fatal: a deleted store is a clean slate, not a panic.

use crate::clock::now_ms;
use crate::event::{Event, EventKind, MAX_LINE_BYTES, RecipientKind};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Default retention in days for lazy GC on `hello`.
pub const DEFAULT_RETENTION_DAYS: u64 = 14;

/// Which segments a read should scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// Today and yesterday only (bounded; the `ls` default).
    Recent,
    /// Every segment on disk (aged segments are compacted, not wiped).
    All,
}

/// The event store rooted at a harbard home directory.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

/// Outcome of a best-effort read: parsed events plus skip-and-count tallies.
#[derive(Debug, Default, Clone)]
pub struct ReadResult {
    pub events: Vec<Event>,
    pub total_lines: u64,
    pub corrupt_lines: u64,
    pub unknown_events: u64,
    pub segments_read: u64,
}

impl Store {
    /// Open (without creating) a store at an explicit root.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    /// Resolve the store home: `HARBARD_HOME` if set, else `~/.harbard`.
    pub fn resolve() -> Self {
        if let Ok(v) = std::env::var("HARBARD_HOME")
            && !v.trim().is_empty()
        {
            return Store::at(v);
        }
        let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
        Store::at(home.join(".harbard"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn events_dir(&self) -> PathBuf {
        self.root.join("events")
    }

    pub fn cursors_dir(&self) -> PathBuf {
        self.root.join("cursors")
    }

    /// Append one event as a JSONL line via an `O_APPEND` single write.
    ///
    /// Line/text bounds are enforced by [`Event::to_line`] before the write;
    /// the write itself is one `write_all` so a `<= MAX_LINE_BYTES` line lands
    /// atomically under concurrent writers (A5).
    pub fn append(&self, event: &Event) -> std::io::Result<()> {
        let dir = self.events_dir();
        fs::create_dir_all(&dir)?;
        let mut line = event
            .to_line()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()))?;
        line.push('\n');
        debug_assert!(line.len() <= MAX_LINE_BYTES);
        let path = dir.join(segment_name(ms_to_ymd(now_ms())));
        let mut f = OpenOptions::new().create(true).append(true).open(path)?;
        f.write_all(line.as_bytes())?;
        // Best-effort derived index — never fails the append (JSONL is authority).
        if event.event == EventKind::Note {
            crate::fts::index_note(self, event);
        }
        Ok(())
    }

    /// List segment paths in ascending date order (file order across days).
    pub fn segments(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = match fs::read_dir(self.events_dir()) {
            Ok(rd) => rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("events-") && n.ends_with(".jsonl"))
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        out.sort();
        out
    }

    /// Read events across the requested window, skip-and-counting corruption.
    /// Never fatal: unreadable or missing segments are simply skipped.
    pub fn read(&self, window: Window) -> ReadResult {
        let mut segs = self.segments();
        if window == Window::Recent {
            let today = ymd_int(ms_to_ymd(now_ms()));
            let yesterday = ymd_int(ms_to_ymd(now_ms().saturating_sub(86_400_000)));
            segs.retain(|p| segment_ymd(p).is_some_and(|d| d == today || d == yesterday));
        }
        let mut result = ReadResult::default();
        for seg in segs {
            if read_segment(&seg, &mut result).is_ok() {
                result.segments_read += 1;
            }
        }
        result
    }

    /// Read only what was appended since a known position — the hot path's
    /// read (F6).
    ///
    /// `read(Window::Recent)` parses today's and yesterday's segments in full
    /// on every call. That is fine at a turn boundary and ruinous before every
    /// tool call: an adversarial review measured 12.8 ms at 152 KB and 852 ms
    /// at 11.8 MB, and the throttle bounds how OFTEN that happens, never what
    /// it costs. Segments are append-only, so a byte offset into the newest one
    /// is a valid resume point and the work becomes proportional to what is
    /// new rather than to the whole day.
    ///
    /// Returns the events after `from`, plus the position to resume from next
    /// time. A caller whose remembered segment no longer exists (a new day, a
    /// GC) gets the full recent window instead — correctness first, speed when
    /// the position still holds.
    pub fn read_since(&self, from: Option<&ReadPosition>) -> (ReadResult, Option<ReadPosition>) {
        let newest = self.segments().into_iter().next_back();
        let Some(newest) = newest else {
            return (ReadResult::default(), None);
        };
        let name = newest
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = fs::metadata(&newest).map(|m| m.len()).unwrap_or(0);

        let resume = ReadPosition {
            segment: name.clone(),
            offset: size,
        };

        match from {
            // Same segment, and the file has not been truncated under us:
            // parse only the tail.
            Some(p) if p.segment == name && p.offset <= size => {
                let mut result = ReadResult::default();
                if read_segment_from(&newest, p.offset, &mut result).is_ok() {
                    result.segments_read = 1;
                }
                (result, Some(resume))
            }
            // First run, a day boundary, or a truncation: fall back to the
            // honest full read rather than silently skipping events.
            _ => (self.read(Window::Recent), Some(resume)),
        }
    }

    /// Retention pass. Nothing is removed: the log is the only record of when a
    /// room was opened and by whom, and `channels.json` holds only who is
    /// subscribed now. Kept as a no-op so callers need no change. Always returns 0.
    pub fn gc(&self, _retention_days: u64, _now: u64) -> u64 {
        0
    }

    /// The retired rescue mirror, kept only so existing installs can still
    /// point at the file. Nothing writes here since retention became
    /// permanent: the segments under `events/` are now the whole corpus, so
    /// there is nothing left to rescue them from.
    pub fn signal_path(&self) -> PathBuf {
        self.root.join("signal.jsonl")
    }

    /// Bounded reverse page for the History archive feed.
    ///
    /// Walks daily segments newest→oldest and stops after `limit + 1` matches —
    /// never materializes `Window::All` then slices. File order is authoritative.
    ///
    /// - `snapshot_newest_id`: freeze the browse upper bound (first page sets it).
    /// - `before`: exclusive older cursor; unknown/expired → [`HistoryError::CursorExpired`].
    pub fn read_history_page(
        &self,
        limit: usize,
        before: Option<&str>,
        snapshot_newest_id: Option<&str>,
        filter: &HistoryFilter,
    ) -> Result<HistoryPage, HistoryError> {
        let limit = limit.max(1);
        let mut segs = self.segments();
        segs.reverse(); // newest segment first

        let mut collected: Vec<Event> = Vec::with_capacity(limit.saturating_add(1));
        let mut has_more = false;
        let mut seen_snapshot = snapshot_newest_id.is_none();
        let mut collecting = before.is_none();
        let mut found_before = before.is_none();
        let mut found_snapshot = snapshot_newest_id.is_none();
        let mut live_newest: Option<String> = None;

        for seg in &segs {
            let mut page = ReadResult::default();
            if read_segment(seg, &mut page).is_err() {
                continue;
            }
            // Within a segment, file order is oldest→newest; reverse for browse.
            for ev in page.events.into_iter().rev() {
                if live_newest.is_none() {
                    live_newest = Some(ev.id.clone());
                }
                if !seen_snapshot && let Some(snap) = snapshot_newest_id {
                    if ev.id == snap {
                        seen_snapshot = true;
                        found_snapshot = true;
                    } else {
                        continue; // newer than frozen snapshot
                    }
                }
                if !collecting && let Some(b) = before {
                    if ev.id == b {
                        collecting = true;
                        found_before = true;
                        continue; // exclusive
                    }
                    continue;
                }
                if !filter.matches(&ev) {
                    continue;
                }
                if collected.len() >= limit {
                    has_more = true;
                    break;
                }
                collected.push(ev);
            }
            if has_more {
                break;
            }
        }

        if let Some(b) = before
            && !found_before
        {
            return Err(HistoryError::CursorExpired {
                kind: "before",
                id: b.to_string(),
            });
        }
        if let Some(snap) = snapshot_newest_id
            && !found_snapshot
        {
            return Err(HistoryError::CursorExpired {
                kind: "snapshot",
                id: snap.to_string(),
            });
        }

        let snapshot = snapshot_newest_id
            .map(str::to_string)
            .or_else(|| collected.first().map(|e| e.id.clone()))
            .or(live_newest);

        let newest_id = collected.first().map(|e| e.id.clone());
        let oldest_id = collected.last().map(|e| e.id.clone());
        let returned = collected.len();

        Ok(HistoryPage {
            events: collected,
            oldest_id,
            newest_id,
            has_more,
            returned,
            snapshot_newest_id: snapshot,
            retention_days: DEFAULT_RETENTION_DAYS,
        })
    }
}

/// Optional filters for [`Store::read_history_page`].
#[derive(Debug, Clone, Default)]
pub struct HistoryFilter {
    pub kinds: Option<Vec<EventKind>>,
    pub channel: Option<String>,
    pub session: Option<String>,
    pub harness: Option<String>,
    /// Match notes addressed to this session id (`to.kind == session`).
    pub to_session: Option<String>,
    /// Match notes addressed to this harness (`to.kind == harness`).
    pub to_harness: Option<String>,
}

impl HistoryFilter {
    fn matches(&self, ev: &Event) -> bool {
        match &self.kinds {
            Some(kinds) if !kinds.is_empty() => {
                if !kinds.contains(&ev.event) {
                    return false;
                }
            }
            // The operator's feedback and rules are meta, not conversation:
            // listed only when a caller asks for them by kind.
            _ if matches!(ev.event, EventKind::Feedback | EventKind::Rule) => return false,
            _ => {}
        }
        if let Some(session) = &self.session
            && ev.session != *session
        {
            return false;
        }
        if let Some(harness) = &self.harness
            && ev.harness != *harness
        {
            return false;
        }
        if let Some(channel) = &self.channel {
            let hit = ev
                .to
                .as_ref()
                .is_some_and(|t| t.kind == RecipientKind::Channel && t.value == *channel);
            if !hit {
                return false;
            }
        }
        if self.to_session.is_some() || self.to_harness.is_some() {
            let hit = ev.to.as_ref().is_some_and(|t| {
                (self.to_session.is_some()
                    && t.kind == RecipientKind::Session
                    && self.to_session.as_deref() == Some(t.value.as_str()))
                    || (self.to_harness.is_some()
                        && t.kind == RecipientKind::Harness
                        && self.to_harness.as_deref() == Some(t.value.as_str()))
            });
            if !hit {
                return false;
            }
        }
        true
    }
}

/// One page of the History archive feed (newest-first).
#[derive(Debug, Clone)]
pub struct HistoryPage {
    pub events: Vec<Event>,
    pub oldest_id: Option<String>,
    pub newest_id: Option<String>,
    pub has_more: bool,
    pub returned: usize,
    pub snapshot_newest_id: Option<String>,
    pub retention_days: u64,
}

/// Cursor / snapshot failures for archive pagination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    CursorExpired { kind: &'static str, id: String },
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HistoryError::CursorExpired { kind, id } => {
                write!(f, "cursor_expired:{kind}:{id}")
            }
        }
    }
}

/// Where a hot-path reader left off: a segment name and a byte offset into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadPosition {
    pub segment: String,
    pub offset: u64,
}

/// Read one segment into `result`, tallying corrupt/unknown lines. Best-effort.
fn read_segment(path: &Path, result: &mut ReadResult) -> std::io::Result<()> {
    read_segment_from(path, 0, result)
}

/// Parse one segment starting at `offset`.
///
/// The offset always lands on a line boundary because it is only ever the size
/// of a file whose writes are whole `O_APPEND` lines — so no partial line can
/// be observed at that position.
fn read_segment_from(path: &Path, offset: u64, result: &mut ReadResult) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    if offset > 0 {
        file.seek(SeekFrom::Start(offset))?;
    }
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = read_until_newline(&mut reader, &mut buf)?;
        if n == 0 {
            break;
        }
        // Strip trailing CR/LF.
        while matches!(buf.last(), Some(b'\n' | b'\r')) {
            buf.pop();
        }
        if buf.is_empty() {
            continue; // blank line: not counted
        }
        result.total_lines += 1;
        if buf.len() + 1 > MAX_LINE_BYTES {
            result.corrupt_lines += 1;
            continue;
        }
        let Ok(text) = std::str::from_utf8(&buf) else {
            result.corrupt_lines += 1;
            continue;
        };
        match serde_json::from_str::<Event>(text) {
            Ok(ev) if ev.event == EventKind::Unknown => result.unknown_events += 1,
            Ok(ev) => result.events.push(ev),
            Err(_) => result.corrupt_lines += 1,
        }
    }
    Ok(())
}

/// Read bytes up to and including the next `\n`, capping the buffer so a
/// pathological unterminated segment cannot exhaust memory.
///
/// Scans the reader's buffer a chunk at a time. It used to pull one byte per
/// `read` call, which made every full-store reader (`ls`, `brief --touch`,
/// `stop-check`, `watch`) pay one call per byte of the whole log.
fn read_until_newline<R: BufRead>(reader: &mut R, buf: &mut Vec<u8>) -> std::io::Result<usize> {
    let mut read = 0;
    // 4x the line cap is generous headroom; beyond it the line is corrupt
    // regardless, and we stop growing the buffer.
    let hard_cap = MAX_LINE_BYTES * 4;
    loop {
        let available = match reader.fill_buf() {
            Ok(chunk) => chunk,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            break;
        }
        let (len, done) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (available.len(), false),
        };
        let room = hard_cap.saturating_sub(buf.len());
        buf.extend_from_slice(&available[..len.min(room)]);
        reader.consume(len);
        read += len;
        if done {
            break;
        }
    }
    Ok(read)
}

/// `(year, month, day)` for a millisecond instant, UTC.
fn ms_to_ymd(ms: u64) -> (i32, u8, u8) {
    let nanos = i128::from(ms) * 1_000_000;
    match time::OffsetDateTime::from_unix_timestamp_nanos(nanos) {
        Ok(dt) => {
            let d = dt.date();
            (d.year(), u8::from(d.month()), d.day())
        }
        Err(_) => (1970, 1, 1),
    }
}

fn ymd_int((y, m, d): (i32, u8, u8)) -> i64 {
    i64::from(y) * 10_000 + i64::from(m) * 100 + i64::from(d)
}

fn segment_name((y, m, d): (i32, u8, u8)) -> String {
    format!("events-{y:04}{m:02}{d:02}.jsonl")
}

/// Extract the `YYYYMMDD` integer from a segment path, if it is one.
fn segment_ymd(path: &Path) -> Option<i64> {
    let name = path.file_name()?.to_str()?;
    let digits = name.strip_prefix("events-")?.strip_suffix(".jsonl")?;
    if digits.len() != 8 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok()
}

/// Best-effort home directory without pulling a crate: `$HOME` on unix.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventKind, Role, SCHEMA_VERSION};
    use crate::test_util::TempHome;

    fn ev(id: &str, kind: EventKind, session: &str, text: &str) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: id.to_string(),
            ts: "2026-07-15T10:00:00Z".to_string(),
            event: kind,
            session: session.to_string(),
            harness: "codex".to_string(),
            host: "h".to_string(),
            cwd: "/tmp".to_string(),
            repo: None,
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: text.to_string(),
            to: None,
            kind: crate::event::NoteKind::Fyi,
        }
    }

    #[test]
    fn append_then_read_round_trips() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        store
            .append(&ev("01AAA", EventKind::Hello, "s1", ""))
            .unwrap();
        store
            .append(&ev("01BBB", EventKind::Intent, "s1", "wiring hooks"))
            .unwrap();
        let r = store.read(Window::All);
        assert_eq!(r.events.len(), 2);
        assert_eq!(r.corrupt_lines, 0);
    }

    #[test]
    fn corrupt_and_unknown_lines_are_skipped_and_counted() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        store
            .append(&ev("01AAA", EventKind::Note, "s1", "ok"))
            .unwrap();
        // Hand-append a corrupt line and an unknown-event line (E6).
        let dir = store.events_dir();
        let seg = std::fs::read_dir(&dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let mut f = OpenOptions::new().append(true).open(&seg).unwrap();
        f.write_all(b"{ this is not json\n").unwrap();
        f.write_all(b"{\"v\":1,\"id\":\"z\",\"ts\":\"t\",\"event\":\"reaction\",\"session\":\"s2\",\"harness\":\"codex\",\"host\":\"h\",\"cwd\":\"/\",\"role\":\"main\",\"text\":\"\"}\n").unwrap();
        let r = store.read(Window::All);
        assert_eq!(r.events.len(), 1, "only the valid event survives");
        assert_eq!(r.corrupt_lines, 1);
        assert_eq!(r.unknown_events, 1);
    }

    #[test]
    fn missing_store_is_clean_slate() {
        let home = TempHome::new();
        let store = Store::at(home.path().join("does-not-exist"));
        let r = store.read(Window::All);
        assert_eq!(r.events.len(), 0);
        assert_eq!(r.corrupt_lines, 0);
    }

    #[test]
    fn chunked_line_reader_matches_byte_at_a_time_reading() {
        // The byte-at-a-time reader it replaced, kept as the reference.
        fn reference(bytes: &[u8]) -> Vec<(usize, Vec<u8>)> {
            let cap = MAX_LINE_BYTES * 4;
            let mut out = Vec::new();
            let mut i = 0;
            while i < bytes.len() {
                let (mut read, mut buf) = (0, Vec::new());
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    read += 1;
                    if buf.len() < cap {
                        buf.push(b);
                    }
                    if b == b'\n' {
                        break;
                    }
                }
                out.push((read, buf));
            }
            out
        }
        let mut input = b"short\n\n{\"a\":1}\r\n".to_vec();
        input.extend(std::iter::repeat_n(b'x', MAX_LINE_BYTES * 5));
        input.extend(b"\nafter the giant\nno trailing newline");
        // A 7-byte buffer splits nearly every line across refills.
        let mut reader = BufReader::with_capacity(7, input.as_slice());
        let mut got = Vec::new();
        loop {
            let mut buf = Vec::new();
            let n = read_until_newline(&mut reader, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            got.push((n, buf));
        }
        assert_eq!(got, reference(&input));
        assert_eq!(got[3].1.len(), MAX_LINE_BYTES * 4, "giant line is capped");
    }

    #[test]
    fn gc_never_prunes_however_old_the_segment_is() {
        // Retention is permanent. An empty 26-year-old segment is the easiest
        // thing to justify deleting, and it is still kept: the rule has no
        // exception to argue about later.
        let home = TempHome::new();
        let store = Store::at(home.path());
        fs::create_dir_all(store.events_dir()).unwrap();
        let ancient = store.events_dir().join("events-20000101.jsonl");
        fs::write(&ancient, b"").unwrap();
        let now = now_ms();
        let today = segment_name(ms_to_ymd(now));
        fs::write(store.events_dir().join(&today), b"").unwrap();
        let removed = store.gc(DEFAULT_RETENTION_DAYS, now);
        assert_eq!(removed, 0, "nothing is ever removed");
        assert!(ancient.exists(), "the ancient segment survives");
        assert!(store.events_dir().join(&today).exists());
    }

    #[test]
    fn history_page_newest_first_and_before_cursor() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        for (id, text) in [
            ("01AAA", "a"),
            ("01BBB", "b"),
            ("01CCC", "c"),
            ("01DDD", "d"),
        ] {
            store.append(&ev(id, EventKind::Note, "s1", text)).unwrap();
        }
        let page = store
            .read_history_page(2, None, None, &HistoryFilter::default())
            .unwrap();
        assert_eq!(page.returned, 2);
        assert!(page.has_more);
        assert_eq!(page.events[0].id, "01DDD");
        assert_eq!(page.events[1].id, "01CCC");
        assert_eq!(page.snapshot_newest_id.as_deref(), Some("01DDD"));

        let older = store
            .read_history_page(2, Some("01CCC"), Some("01DDD"), &HistoryFilter::default())
            .unwrap();
        assert_eq!(older.events[0].id, "01BBB");
        assert_eq!(older.events[1].id, "01AAA");
        assert!(!older.has_more);
    }

    #[test]
    fn history_leaves_out_feedback_and_rules_unless_asked() {
        // The operator's taps and rules are meta, not conversation: a feed
        // that lists every kind would print their JSON as a message.
        let home = TempHome::new();
        let store = Store::at(home.path());
        store
            .append(&ev("01AAA", EventKind::Note, "s1", "a"))
            .unwrap();
        store
            .append(&ev("01BBB", EventKind::Feedback, "s1", "{}"))
            .unwrap();
        store
            .append(&ev("01CCC", EventKind::Rule, "s1", "{}"))
            .unwrap();
        let page = store
            .read_history_page(10, None, None, &HistoryFilter::default())
            .unwrap();
        let ids: Vec<&str> = page.events.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["01AAA"]);
        let asked = HistoryFilter {
            kinds: Some(vec![EventKind::Rule]),
            ..HistoryFilter::default()
        };
        let page = store.read_history_page(10, None, None, &asked).unwrap();
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].id, "01CCC");
    }

    #[test]
    fn history_page_expired_before_returns_error() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        store
            .append(&ev("01AAA", EventKind::Note, "s1", "a"))
            .unwrap();
        let err = store
            .read_history_page(10, Some("01MISSING"), None, &HistoryFilter::default())
            .unwrap_err();
        assert!(matches!(err, HistoryError::CursorExpired { .. }));
    }

    #[test]
    fn history_page_snapshot_ignores_newer_appends() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        store
            .append(&ev("01AAA", EventKind::Note, "s1", "a"))
            .unwrap();
        store
            .append(&ev("01BBB", EventKind::Note, "s1", "b"))
            .unwrap();
        let snap = "01BBB";
        store
            .append(&ev("01CCC", EventKind::Note, "s1", "c"))
            .unwrap();
        let page = store
            .read_history_page(10, None, Some(snap), &HistoryFilter::default())
            .unwrap();
        assert_eq!(page.events[0].id, "01BBB");
        assert!(!page.events.iter().any(|e| e.id == "01CCC"));
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;
    use crate::test_util::TempHome;

    #[test]
    fn read_since_returns_only_what_is_new() {
        // The hot path's contract: cost proportional to what arrived, not to
        // the size of the day.
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mk = |i: u64| Event {
            v: crate::event::SCHEMA_VERSION,
            id: format!("01KYJ{i:021}"),
            ts: crate::clock::rfc3339_utc(1_000 + i),
            event: EventKind::Note,
            session: "s".into(),
            harness: "codex".into(),
            host: "h".into(),
            cwd: "/tmp".into(),
            repo: None,
            repo_key: None,
            branch: None,
            role: crate::event::Role::Main,
            parent: None,
            note_ref: None,
            text: format!("n{i}"),
            to: None,
            kind: crate::event::NoteKind::Fyi,
        };

        for i in 0..5 {
            store.append(&mk(i)).unwrap();
        }
        // First call has no position: it reads the window and reports where to
        // resume.
        let (first, pos) = store.read_since(None);
        assert_eq!(first.events.len(), 5);
        let pos = pos.expect("a resume position");

        // Nothing new: no events, and the position holds.
        let (quiet, pos2) = store.read_since(Some(&pos));
        assert!(quiet.events.is_empty(), "nothing was appended");
        assert_eq!(pos2.as_ref(), Some(&pos));

        // Two more: only those come back.
        store.append(&mk(5)).unwrap();
        store.append(&mk(6)).unwrap();
        let (fresh, _) = store.read_since(Some(&pos));
        assert_eq!(fresh.events.len(), 2, "only the tail is parsed");
        assert_eq!(fresh.events[0].text, "n5");
    }

    #[test]
    fn a_stale_position_falls_back_to_the_full_read() {
        // A day boundary or a GC invalidates the remembered segment. Silently
        // skipping would lose events; re-reading is the safe direction.
        let home = TempHome::new();
        let store = Store::at(home.path());
        store
            .append(&Event {
                v: crate::event::SCHEMA_VERSION,
                id: "01KYJ000000000000000000001".into(),
                ts: crate::clock::rfc3339_utc(1_000),
                event: EventKind::Note,
                session: "s".into(),
                harness: "codex".into(),
                host: "h".into(),
                cwd: "/tmp".into(),
                repo: None,
                repo_key: None,
                branch: None,
                role: crate::event::Role::Main,
                parent: None,
                note_ref: None,
                text: "only".into(),
                to: None,
                kind: crate::event::NoteKind::Fyi,
            })
            .unwrap();

        let ghost = ReadPosition {
            segment: "events-19990101.jsonl".into(),
            offset: 999_999,
        };
        let (result, _) = store.read_since(Some(&ghost));
        assert_eq!(
            result.events.len(),
            1,
            "unknown segment re-reads the window"
        );
    }
}

#[cfg(test)]
mod signal_tests {
    use super::*;
    use crate::test_util::TempHome;

    fn ev(id: &str, kind: EventKind, operator: bool, text: &str) -> Event {
        Event {
            v: crate::event::SCHEMA_VERSION,
            id: id.into(),
            ts: crate::clock::rfc3339_utc(1_000),
            event: kind,
            session: if operator {
                "operator@h".into()
            } else {
                "s1".into()
            },
            harness: if operator {
                crate::event::HARNESS_HUMAN.into()
            } else {
                "codex".into()
            },
            host: "h".into(),
            cwd: "/tmp".into(),
            repo: None,
            repo_key: None,
            branch: None,
            role: if operator {
                crate::event::Role::Operator
            } else {
                crate::event::Role::Main
            },
            parent: None,
            note_ref: None,
            text: text.into(),
            to: None,
            kind: crate::event::NoteKind::Fyi,
        }
    }

    #[test]
    fn nothing_the_bus_ever_recorded_is_deleted() {
        // The bus is a permanent corpus, so this test states the whole
        // retention policy: every event that was ever appended is still there
        // after a GC pass, whatever its kind and however old the segment.
        //
        // `join`/`leave` are the ones this used to drop, on the grounds that
        // `channels.json` already holds the membership. The snapshot answers
        // "who is subscribed now"; only these events answer "when was this
        // room opened and did anyone ever speak in it".
        let home = TempHome::new();
        let store = Store::at(home.path());
        let dir = store.events_dir();
        fs::create_dir_all(&dir).unwrap();

        let old = dir.join("events-20200101.jsonl");
        let mut f = fs::File::create(&old).unwrap();
        let mut join = ev("01E", EventKind::Join, false, "");
        join.to = Some(crate::event::Recipient {
            kind: crate::event::RecipientKind::Channel,
            value: "ghosts".into(),
        });
        let mut leave = ev("01F", EventKind::Leave, false, "");
        leave.to = Some(crate::event::Recipient {
            kind: crate::event::RecipientKind::Channel,
            value: "ghosts".into(),
        });
        for e in [
            ev("01A", EventKind::Note, true, "this is noise, stop"),
            ev("01B", EventKind::Note, false, "routine agent chatter"),
            ev("01C", EventKind::Ack, false, "01A - taken on"),
            ev("01D", EventKind::Hello, false, ""),
            join,
            leave,
        ] {
            writeln!(f, "{}", e.to_line().unwrap()).unwrap();
        }
        drop(f);
        let before = fs::read_to_string(&old).unwrap();

        for _ in 0..5 {
            assert_eq!(store.gc(1, 4_000_000_000_000), 0, "nothing is removed");
        }

        assert!(old.exists());
        let after = fs::read_to_string(&old).unwrap();
        assert_eq!(after, before, "the segment is not rewritten either");
        for needle in [
            "this is noise, stop",
            "routine agent chatter",
            "taken on",
            "\"hello\"",
            "\"join\"",
            "\"leave\"",
        ] {
            assert!(after.contains(needle), "{needle} must survive GC");
        }
    }

    #[test]
    fn a_segment_of_nothing_but_scaffolding_also_survives() {
        // The old policy deleted this file outright. It is the case that
        // matters most for later analysis: a room somebody opened and nobody
        // ever used leaves exactly this trace and nothing else.
        let home = TempHome::new();
        let store = Store::at(home.path());
        let dir = store.events_dir();
        fs::create_dir_all(&dir).unwrap();

        let old = dir.join("events-20200101.jsonl");
        let mut join = ev("01J", EventKind::Join, false, "");
        join.to = Some(crate::event::Recipient {
            kind: crate::event::RecipientKind::Channel,
            value: "ghosts".into(),
        });
        fs::write(&old, format!("{}\n", join.to_line().unwrap())).unwrap();

        assert_eq!(store.gc(1, 4_000_000_000_000), 0);
        assert!(old.exists(), "the only record that #ghosts existed");
        assert!(fs::read_to_string(&old).unwrap().contains("\"join\""));
    }
}
