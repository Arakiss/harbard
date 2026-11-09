//! Clara's review of A2, R1 (red on a597c12): the "Was it you?" card is not
//! dismissed by a bare POST. A real serve on port 0
//! over a throwaway home; HARBARD_HUB names dead port 9.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{Value, json};

const KEY: &str = "check-card-fixture-not-a-secret";

fn send(port: u16, method: &str, path: &str, body: &str) -> (u16, Value) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAuthorization: Bearer {KEY}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    let (head, rest) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(rest).unwrap_or(Value::Null))
}

/// R1: the "Was it you?" card is the stated backstop against a process that
/// scripts the confirmation; the same process dismisses it with one bare
/// POST (Keep = resolve), with no confirmation at all.
#[test]
fn r1_the_fuiste_tu_card_cannot_be_dismissed_by_a_bare_post() {
    let home = std::env::temp_dir().join(format!("harbard-check-card-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("os")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .env("HOME", home.join("os"))
        .env("PATH", "/usr/bin:/bin")
        .env("HARBARD_HUB", "http://127.0.0.1:9")
        .arg("--home")
        .arg(home.join("store"))
        .args(["serve", "--port", "0", "--token", KEY])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port = serde_json::from_str::<Value>(&line).unwrap()["port"]
        .as_u64()
        .unwrap() as u16;

    // The scripted confirmation: ask, then send the code back.
    let (_, asked) = send(
        port,
        "POST",
        "/api/v1/workspace/memory-sink-allow",
        r#"{"on":true}"#,
    );
    let code = asked["confirm"]["id"].as_str().expect("a code").to_string();
    let (s, on) = send(
        port,
        "POST",
        "/api/v1/workspace/memory-sink-allow",
        &json!({"on": true, "confirm": code}).to_string(),
    );
    assert_eq!((s, on["allowed"].clone()), (200, json!(true)), "{on}");

    let (_, ws) = send(port, "GET", "/api/v1/workspace", "");
    let card = ws["cards"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "check")
        .expect("the check card")["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Keep, scripted.
    let (s, out) = send(
        port,
        "POST",
        "/api/v1/workspace/resolve",
        &json!({"card": card}).to_string(),
    );
    let (_, ws) = send(port, "GET", "/api/v1/workspace", "");
    let still_open = ws["cards"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["id"] == card.as_str() && c["resolved_ms"].is_null());
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        still_open,
        "one bare POST dismissed the check card ({s} {out}); saving to the memory sink stays on and Te necesita is empty"
    );
}
