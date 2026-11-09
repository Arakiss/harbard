//! A5: two concurrent writers × 1000 events each must produce zero torn or
//! interleaved lines. Each writer holds its own `O_APPEND` handle (the
//! multi-process condition), so this exercises the atomicity the store relies on
//! for `<= MAX_LINE_BYTES` lines on POSIX.

use harbard_core::event::MAX_LINE_BYTES;
use harbard_core::{Event, EventKind, NoteKind, Role, SCHEMA_VERSION, Store, Window};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_home() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("harbard-stress-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn event(writer: u32, i: u32) -> Event {
    Event {
        v: SCHEMA_VERSION,
        // Distinct, well-formed 26-char ULIDs per event (from_parts is fine).
        id: ulid_like(writer, i),
        ts: "2026-07-15T10:00:00Z".to_string(),
        event: EventKind::Note,
        session: format!("writer-{writer}"),
        harness: "codex".to_string(),
        host: "host".to_string(),
        cwd: "/tmp/repo".to_string(),
        repo: Some("/tmp/repo".to_string()),
        repo_key: None,
        branch: Some("main".to_string()),
        role: Role::Main,
        parent: None,
        note_ref: None,
        text: format!("writer {writer} event {i} — synthetic payload for the stress test"),
        to: None,
        kind: NoteKind::Fyi,
    }
}

fn ulid_like(writer: u32, i: u32) -> String {
    // Deterministic, unique, valid Crockford-base32 26-char id.
    let n = (u64::from(writer) << 32) | u64::from(i);
    let alphabet = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut s = [b'0'; 26];
    let mut v = n;
    for slot in s.iter_mut().rev() {
        *slot = alphabet[(v % 32) as usize];
        v /= 32;
    }
    String::from_utf8(s.to_vec()).unwrap()
}

#[test]
fn concurrent_writers_produce_no_torn_lines() {
    let home = temp_home();
    let store = Store::at(&home);
    // Ensure the segment dir exists before writers race on the file.
    std::fs::create_dir_all(store.events_dir()).unwrap();

    let per_writer = 1000u32;
    let handles: Vec<_> = (0..2u32)
        .map(|w| {
            let store = store.clone();
            std::thread::spawn(move || {
                for i in 0..per_writer {
                    store.append(&event(w, i)).expect("append");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("writer thread");
    }

    let r = store.read(Window::All);
    assert_eq!(r.corrupt_lines, 0, "no torn/interleaved lines");
    assert_eq!(
        r.events.len() as u32,
        2 * per_writer,
        "every event landed intact"
    );
    assert_eq!(r.total_lines, u64::from(2 * per_writer));
    // Every line is well within the atomic-write bound.
    for e in &r.events {
        assert!(e.to_line().unwrap().len() < MAX_LINE_BYTES);
    }
    let _ = std::fs::remove_dir_all(&home);
}
