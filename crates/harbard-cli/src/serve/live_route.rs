//! `GET /api/v1/live/<agent>?after=<rev>&prose=0|1`: the private live view
//! of one agent ("See it live"), and the hover card's fresh steps.
//!
//! Pull-only and local, like `/team`: it answers the console and nobody
//! else, and nothing is written or sent anywhere. Only an agent the team
//! projection already holds can be opened, and only its own transcript is
//! read, the file the projection reads (`harbard_core::team`), under the
//! Claude projects root. A key is a transcript id, never a path.
//!
//! serve keeps a log only for the few agents a console has open, and each
//! request reads just the bytes appended since the previous one. The answer
//! is a delta after the cursor `after`; see `harbard_core::live`.

use super::team_route;
use harbard_core::clock;
use harbard_core::live::LiveLog;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Agents whose logs serve keeps at once; the least recently asked goes.
/// Hovering over a few cards in a row must not drop the view that is open.
pub const OPEN_MAX: usize = 8;

/// One open agent. The table's lock is held only to find or add an entry;
/// reading the transcript (a first open can take most of a second) holds
/// only that entry's own lock, so other agents' polls are not held up.
struct Open {
    key: String,
    path: PathBuf,
    log: Arc<Mutex<LiveLog>>,
    used_ms: u64,
}

fn open_logs() -> &'static Mutex<Vec<Open>> {
    static OPEN: OnceLock<Mutex<Vec<Open>>> = OnceLock::new();
    OPEN.get_or_init(|| Mutex::new(Vec::new()))
}

/// A transcript id: `[A-Za-z0-9_-]{1,128}`. Anything else (a slash, a dot,
/// an escape) is refused before any lookup.
pub fn valid_key(key: &str) -> bool {
    (1..=128).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `?only=progress` answers the hover card: the progress alone, no steps
/// and no prose.
pub fn live_body(
    key: &str,
    after: Option<String>,
    prose: Option<String>,
    only: Option<String>,
) -> (u16, Value) {
    live_with(
        key,
        after,
        prose,
        only,
        &team_route::transcript_of,
        open_logs(),
    )
}

/// The read with the lookup and the log table injected (tests).
fn live_with(
    key: &str,
    after: Option<String>,
    prose: Option<String>,
    only: Option<String>,
    lookup: &dyn Fn(&str) -> Option<PathBuf>,
    table: &Mutex<Vec<Open>>,
) -> (u16, Value) {
    if !valid_key(key) {
        return (400, json!({ "error": "not an agent key" }));
    }
    let Some(path) = lookup(key) else {
        return (
            404,
            json!({ "error": "no agent with that key in the team view" }),
        );
    };
    let after = after.and_then(|a| a.trim().parse::<u64>().ok());
    let with_prose = prose.as_deref() != Some("0");
    let now = clock::now_ms();
    let log = {
        let mut open = table.lock().unwrap_or_else(|e| e.into_inner());
        match open.iter_mut().find(|o| o.key == key && o.path == path) {
            Some(o) => {
                o.used_ms = now;
                Arc::clone(&o.log)
            }
            None => {
                open.retain(|o| o.key != key);
                if open.len() >= OPEN_MAX
                    && let Some(oldest) = open
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, o)| o.used_ms)
                        .map(|(i, _)| i)
                {
                    open.remove(oldest);
                }
                let log = Arc::new(Mutex::new(LiveLog::new()));
                open.push(Open {
                    key: key.to_string(),
                    path: path.clone(),
                    log: Arc::clone(&log),
                    used_ms: now,
                });
                log
            }
        }
    };
    let mut log = log.lock().unwrap_or_else(|e| e.into_inner());
    log.refresh(Path::new(&path));
    let mut delta = log.delta(after, with_prose);
    drop(log);
    if only.as_deref() == Some("progress") {
        delta.steps.clear();
        delta.prose.clear();
    }
    let mut body =
        serde_json::to_value(&delta).unwrap_or_else(|e| json!({ "error": e.to_string() }));
    if let Value::Object(map) = &mut body {
        map.insert("key".into(), json!(key));
        map.insert("now_ms".into(), json!(now));
    }
    (200, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harbard_core::team::{TeamProjector, TeamRoots, WINDOW_DEFAULT_MS};
    use serde_json::json;
    use std::io::Write;
    use std::time::{Duration, Instant};

    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    struct TempHome(PathBuf);
    impl TempHome {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "hb-live-{}-{}",
                std::process::id(),
                N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            TempHome(p)
        }
    }
    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SID: &str = "11111111-2222-3333-4444-555555555555";

    fn iso(ms: u64) -> String {
        harbard_core::clock::rfc3339_utc(ms)
    }

    fn record(typ: &str, ms: u64, message: Value) -> Value {
        json!({"type": typ, "timestamp": iso(ms), "sessionId": SID, "cwd": "/w",
               "gitBranch": "main", "message": message})
    }

    fn append(path: &Path, records: &[Value]) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for r in records {
            writeln!(f, "{}", serde_json::to_string(r).unwrap()).unwrap();
        }
    }

    /// A home with one lead session and one teammate `a1`.
    fn fixture(home: &Path) -> PathBuf {
        let session = home.join(".claude/projects/-w").join(SID);
        let sub = session.join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(
            sub.join("agent-a1.meta.json"),
            r#"{"name":"directo","taskKind":"in_process_teammate","agentType":"general-purpose"}"#,
        )
        .unwrap();
        std::fs::write(
            home.join(".claude/projects/-w")
                .join(format!("{SID}.jsonl")),
            "",
        )
        .unwrap();
        sub.join("agent-a1.jsonl")
    }

    #[test]
    fn a_key_is_a_transcript_id_never_a_path() {
        let table = Mutex::new(Vec::new());
        let seen = std::cell::RefCell::new(Vec::new());
        let lookup = |k: &str| {
            seen.borrow_mut().push(k.to_string());
            None
        };
        for bad in [
            "",
            "../../Documents/x",
            "a/b",
            "a.b",
            "%2e%2e",
            "a b",
            &"x".repeat(129),
        ] {
            let (status, _) = live_with(bad, None, None, None, &lookup, &table);
            assert_eq!(status, 400, "{bad:?}");
        }
        assert!(seen.borrow().is_empty(), "a bad key reached the lookup");
        let (status, _) = live_with("a1", None, None, None, &lookup, &table);
        assert_eq!(status, 404);
    }

    /// The live view reads the one transcript the projection holds for that
    /// agent, under the Claude projects root, and nothing under the
    /// TCC-protected folders.
    #[test]
    fn the_live_view_opens_only_a_transcript_the_projection_read() {
        let tmp = TempHome::new();
        let path = fixture(&tmp.0);
        let now = harbard_core::clock::now_ms();
        append(
            &path,
            &[record(
                "user",
                now - 60_000,
                json!({"role":"user","content":"<teammate-message teammate_id=\"team-lead\">Go.</teammate-message>"}),
            )],
        );
        let mut projector = TeamProjector::new(TeamRoots {
            home: tmp.0.clone(),
            claude_projects: tmp.0.join(".claude/projects"),
        });
        assert_eq!(projector.transcript_of("a1"), None, "nothing before a pass");
        projector.project(now, WINDOW_DEFAULT_MS);
        let found = projector.transcript_of("a1").unwrap();
        assert_eq!(found, path);
        assert!(found.starts_with(tmp.0.join(".claude/projects")));
        assert_eq!(projector.transcript_of("a2"), None);
        assert_eq!(projector.transcript_of("../a1"), None);
        let home = tmp.0.display().to_string();
        assert!(!harbard_core::workspace::protected(
            &found.display().to_string(),
            &home
        ));
    }

    #[test]
    fn deltas_carry_only_what_changed_and_a_new_log_resets_the_client() {
        let tmp = TempHome::new();
        let path = fixture(&tmp.0);
        let now = harbard_core::clock::now_ms();
        append(
            &path,
            &[record(
                "assistant",
                now - 9_000,
                json!({"id":"m1","role":"assistant","stop_reason":"tool_use","content":[
                    {"type":"thinking","thinking":"THINKING-SENTINEL"},
                    {"type":"text","text":"Voy a compilar."},
                    {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo build --release"}}]}),
            )],
        );
        let table = Mutex::new(Vec::new());
        let lookup = |_: &str| Some(path.clone());
        let (status, first) = live_with("a1", None, None, None, &lookup, &table);
        assert_eq!(status, 200);
        assert_eq!(first["reset"], true);
        assert_eq!(first["steps"][0]["detail"], "cargo build");
        assert_eq!(first["steps"][0]["status"], "running");
        assert_eq!(first["prose"][0]["text"], "Voy a compilar.");
        assert!(!first.to_string().contains("THINKING-SENTINEL"));
        let rev = first["rev"].as_u64().unwrap();

        // Nothing new: an empty delta.
        let (_, same) = live_with("a1", Some(rev.to_string()), None, None, &lookup, &table);
        assert_eq!(same["reset"], false);
        assert_eq!(same["steps"].as_array().unwrap().len(), 0);
        assert_eq!(same["prose"].as_array().unwrap().len(), 0);

        append(
            &path,
            &[record(
                "user",
                now - 2_000,
                json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"t1",
                    "content":"OUTPUT-SENTINEL\n    Finished `release` profile","is_error":false}]}),
            )],
        );
        let (_, next) = live_with(
            "a1",
            Some(rev.to_string()),
            Some("0".into()),
            None,
            &lookup,
            &table,
        );
        let steps = next["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1, "{next}");
        assert_eq!(steps[0]["status"], "ok");
        assert_eq!(steps[0]["n"], 0);
        assert!(next["prose"].as_array().unwrap().is_empty(), "prose=0");
        assert!(!next.to_string().contains("OUTPUT-SENTINEL"));
        assert_eq!(next["progress"]["tool_calls"], 1);

        // A cursor from a log serve no longer holds: everything again.
        let fresh = Mutex::new(Vec::new());
        let (_, again) = live_with("a1", Some(rev.to_string()), None, None, &lookup, &fresh);
        assert_eq!(again["reset"], true);
        assert_eq!(again["steps"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn only_a_few_logs_are_kept_and_the_oldest_goes() {
        let tmp = TempHome::new();
        let path = fixture(&tmp.0);
        let table = Mutex::new(Vec::new());
        let lookup = |_: &str| Some(path.clone());
        for i in 0..(OPEN_MAX + 3) {
            let (status, _) = live_with(&format!("k{i}"), None, None, None, &lookup, &table);
            assert_eq!(status, 200);
            std::thread::sleep(Duration::from_millis(2));
        }
        let open = table.lock().unwrap();
        assert_eq!(open.len(), OPEN_MAX);
        assert!(!open.iter().any(|o| o.key == "k0"));
        assert!(open.iter().any(|o| o.key == format!("k{}", OPEN_MAX + 2)));
    }

    /// A transcript change is in the next answer, and a poll that finds
    /// nothing new is cheap: the console polls every second, so a change
    /// reaches the open view within 2 s.
    #[test]
    fn an_appended_record_is_in_the_next_poll_and_an_idle_poll_is_cheap() {
        let tmp = TempHome::new();
        let path = fixture(&tmp.0);
        let now = harbard_core::clock::now_ms();
        // A long transcript: 2,000 finished calls.
        let mut records = Vec::new();
        for i in 0..2_000 {
            let id = format!("t{i}");
            records.push(record(
                "assistant",
                now - 600_000 + i,
                json!({"role":"assistant","stop_reason":"tool_use","content":[
                    {"type":"tool_use","id":id,"name":"Read","input":{"file_path":format!("/w/f{i}.rs")}}]}),
            ));
            records.push(record(
                "user",
                now - 600_000 + i,
                json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":"x"}]}),
            ));
        }
        append(&path, &records);
        let table = Mutex::new(Vec::new());
        let lookup = |_: &str| Some(path.clone());
        let (_, first) = live_with("a1", None, Some("1".into()), None, &lookup, &table);
        let mut rev = first["rev"].as_u64().unwrap();
        let mut idle = Duration::ZERO;
        for _ in 0..20 {
            let t = Instant::now();
            let (_, d) = live_with("a1", Some(rev.to_string()), None, None, &lookup, &table);
            idle += t.elapsed();
            rev = d["rev"].as_u64().unwrap();
        }
        let per_idle = idle / 20;
        assert!(
            per_idle < Duration::from_millis(20),
            "idle poll {per_idle:?}"
        );
        append(
            &path,
            &[record(
                "assistant",
                now,
                json!({"role":"assistant","stop_reason":"tool_use","content":[
                    {"type":"tool_use","id":"new","name":"Edit","input":{"file_path":"/w/src/live.rs"}}]}),
            )],
        );
        let t = Instant::now();
        let (_, d) = live_with("a1", Some(rev.to_string()), None, None, &lookup, &table);
        let took = t.elapsed();
        let steps = d["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 1, "{d}");
        assert_eq!(steps[0]["detail"], "live.rs");
        assert_eq!(steps[0]["state"], "editing");
        assert!(took < Duration::from_millis(50), "{took:?}");
        eprintln!("live poll: idle {per_idle:?}, with one new record {took:?}");
    }

    /// The hover card asks for the progress alone: no steps and
    /// no prose, even on a first answer.
    #[test]
    fn the_card_gets_the_progress_alone() {
        let tmp = TempHome::new();
        let path = fixture(&tmp.0);
        let now = harbard_core::clock::now_ms();
        append(
            &path,
            &[record(
                "assistant",
                now - 9_000,
                json!({"id":"m1","role":"assistant","stop_reason":"tool_use","content":[
                    {"type":"text","text":"PROSE-SENTINEL"},
                    {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo build"}}]}),
            )],
        );
        let table = Mutex::new(Vec::new());
        let lookup = |_: &str| Some(path.clone());
        let (status, card) = live_with("a1", None, None, Some("progress".into()), &lookup, &table);
        assert_eq!(status, 200);
        assert!(card["steps"].as_array().unwrap().is_empty());
        assert!(card["prose"].as_array().unwrap().is_empty());
        assert_eq!(card["progress"]["tool_calls"], 1);
        assert_eq!(card["progress"]["steps"][0]["detail"], "cargo build");
        assert!(!card.to_string().contains("PROSE-SENTINEL"));
    }

    /// Reading one agent's transcript holds only that agent's log, never
    /// the table: another agent's poll is answered meanwhile.
    #[test]
    fn a_slow_read_of_one_agent_does_not_hold_up_another() {
        let tmp = TempHome::new();
        let path = fixture(&tmp.0);
        let table = Mutex::new(Vec::new());
        let lookup = |_: &str| Some(path.clone());
        live_with("slow", None, None, None, &lookup, &table);
        let slow = {
            let open = table.lock().unwrap();
            Arc::clone(&open.iter().find(|o| o.key == "slow").unwrap().log)
        };
        // The slow agent's read is stuck on its own log, as a first open of
        // a huge transcript would be.
        let stuck = slow.lock().unwrap();
        let (done, answered) = std::sync::mpsc::channel();
        std::thread::scope(|s| {
            s.spawn(|| live_with("slow", None, None, None, &lookup, &table));
            std::thread::sleep(Duration::from_millis(50));
            let done = done.clone();
            let (table, lookup) = (&table, &lookup);
            s.spawn(move || {
                let (status, _) = live_with("other", None, None, None, lookup, table);
                let _ = done.send(status);
            });
            let got = answered.recv_timeout(Duration::from_millis(500));
            drop(stuck);
            assert_eq!(got, Ok(200), "another agent's poll waited on the slow read");
        });
    }
}
