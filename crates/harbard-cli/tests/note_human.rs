//! Notes the operator reads: a handoff to them, files cited in one step, and
//! the human-line checks a note gets before it is sent.
//!
//! A handoff to the human must not need one live session, sending a file
//! takes one step, and shorthand in the operator's channel gets a warning.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

const LEAD: &str = "44444444-4444-4444-8444-444444444444";
const PEER: &str = "55555555-5555-4555-8555-555555555555";

struct Home(PathBuf);

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Home {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("harbard-note-human-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let home = Home(d);
        for (s, h) in [(LEAD, "claude-code"), (PEER, "codex")] {
            let out = home.run(s, h, &["hello"]);
            assert!(out.status.success(), "{out:?}");
        }
        home
    }

    fn store(&self) -> PathBuf {
        self.0.join(".harbard")
    }

    fn run(&self, session: &str, harness: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", &self.0)
            .env("PATH", "/usr/bin:/bin")
            // A dead port: nothing here may reach a running serve.
            .env("HARBARD_API", "http://127.0.0.1:9")
            .args(args)
            .arg("--home")
            .arg(self.store())
            .args(["--cwd", self.0.to_str().unwrap()])
            .args(["--session", session, "--harness", harness])
            .output()
            .unwrap()
    }

    fn lead(&self, args: &[&str]) -> Output {
        self.run(LEAD, "claude-code", args)
    }

    fn file(&self, name: &str, body: &str) -> PathBuf {
        let p = self.0.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_handoff_to_the_human_is_accepted() {
    let home = Home::new();
    let out = home.lead(&[
        "note",
        "Handing over the form 303 signature: it is ready for you to review.",
        "--kind",
        "handoff",
        "--to",
        "harness:human",
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let v = json(&out);
    assert_eq!(v["kind"], "handoff", "{v}");
    assert_eq!(v["to"]["kind"], "harness", "{v}");
    assert_eq!(v["to"]["value"], "human", "{v}");
}

#[test]
fn a_handoff_to_a_crowd_or_to_nobody_is_still_refused() {
    let home = Home::new();
    for to in [Some("harness:codex"), Some("repo:/tmp"), None] {
        let mut args = vec!["note", "Toma esto.", "--kind", "handoff"];
        if let Some(to) = to {
            args.extend(["--to", to]);
        }
        let out = home.lead(&args);
        assert!(!out.status.success(), "{to:?} must be refused: {out:?}");
    }
}

#[test]
fn attach_copies_the_files_and_cites_them_in_the_note() {
    let home = Home::new();
    let report = home.file("report.md", "# Informe\n");
    let shot = home.file("before.png", "not really a png");
    let out = home.lead(&[
        "note",
        "You have the report and the screenshot.",
        "--to",
        "harness:human",
        "--attach",
        report.to_str().unwrap(),
        "--attach",
        shot.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    let v = json(&out);
    let text = v["text"].as_str().unwrap();
    let attachments = home.store().join("attachments");
    let cited: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with(attachments.to_str().unwrap()))
        .collect();
    assert_eq!(cited.len(), 2, "both paths cited: {text}");
    for path in &cited {
        assert!(Path::new(path).is_file(), "{path} was copied");
    }
    assert!(cited[0].ends_with("/report.md") && cited[1].ends_with("/before.png"));
    // The human line stays the first line; the paths ride the body.
    assert!(
        text.starts_with("You have the report and the screenshot.\n\n"),
        "{text}"
    );
}

#[test]
fn attach_refuses_before_anything_is_sent() {
    let home = Home::new();
    let bad = home.file("binario.exe", "x");
    let out = home.lead(&[
        "note",
        "Mira esto.",
        "--to",
        "harness:human",
        "--attach",
        bad.to_str().unwrap(),
    ]);
    assert!(!out.status.success(), "{out:?}");
    let events: String = std::fs::read_dir(home.store().join("events"))
        .unwrap()
        .flatten()
        .map(|e| std::fs::read_to_string(e.path()).unwrap_or_default())
        .collect();
    assert!(!events.contains("Mira esto."), "nothing was sent");
}

#[test]
fn body_file_puts_the_detail_after_the_human_line() {
    let home = Home::new();
    let body = home.file("detail.txt", "Detail for the lead: tests 29/29.\n");
    let out = home.lead(&[
        "note",
        "Step five is ready.",
        "--to",
        "harness:human",
        "--body-file",
        body.to_str().unwrap(),
        "--json",
    ]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        json(&out)["text"],
        "Step five is ready.\n\nDetail for the lead: tests 29/29."
    );
}

#[test]
fn a_long_first_line_for_the_human_is_warned_about_with_the_fix() {
    let home = Home::new();
    let long = "Step five is ready and also ".repeat(6);
    let out = home.lead(&["note", &long, "--to", "harness:human"]);
    assert!(out.status.success(), "a warning, never a refusal: {out:?}");
    let err = stderr(&out);
    assert!(err.contains("first line"), "{err}");
    assert!(err.contains("--body-file"), "{err}");

    let short = home.lead(&["note", "It is ready.", "--to", "harness:human"]);
    assert!(
        !stderr(&short).contains("--body-file"),
        "{}",
        stderr(&short)
    );
}

#[test]
fn shorthand_for_the_human_is_warned_about_before_it_is_sent() {
    let home = Home::new();
    let out = home.lead(&[
        "note",
        "rebased onto main fcf628c, and the gates are green",
        "--to",
        "harness:human",
    ]);
    assert!(out.status.success(), "{out:?}");
    let err = stderr(&out);
    let warn = err.find("plain language").expect("jargon warning");
    let receipt = err.find("note recorded").or_else(|| err.find("recorded"));
    assert!(
        receipt.is_none_or(|r| warn < r),
        "the warning comes before the receipt: {err}"
    );
    for why in ["sha", "word"] {
        assert!(err.contains(why), "{why}: {err}");
    }

    // A note between agents is not the operator's to read.
    let peer = home.lead(&[
        "note",
        "rebased onto main 1582702",
        "--to",
        &format!("session:{PEER}"),
    ]);
    assert!(
        !stderr(&peer).contains("plain language"),
        "{}",
        stderr(&peer)
    );
}

#[test]
fn the_note_operation_takes_attach_and_body_file() {
    let home = Home::new();
    let shot = home.file("after.png", "png");
    let body = home.file("detail.txt", "Detail.");
    let input = serde_json::json!({
        "text": "Done.",
        "to": "harness:human",
        "attach": [shot.to_str().unwrap()],
        "body_file": body.to_str().unwrap(),
    })
    .to_string();
    let out = home.lead(&["call", "note", "--input", &input]);
    assert!(out.status.success(), "{out:?}");
    let v = json(&out);
    let text = v["data"]["text"].as_str().unwrap();
    assert!(text.starts_with("Done.\n\nDetail.\n"), "{text}");
    assert!(text.ends_with("/after.png"), "{text}");
}
