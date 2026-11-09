//! MCP stdio facade: initialize + tools/list + protocol resource + note tool.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_home() -> std::path::PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("harbard-mcp-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn bin() -> std::path::PathBuf {
    std::env::var_os("CARGO_BIN_EXE_harbard")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let mut p = std::env::current_exe().unwrap();
            p.pop();
            if p.ends_with("deps") {
                p.pop();
            }
            p.push("harbard");
            p
        })
}

fn rpc(
    child_stdin: &mut impl Write,
    child_stdout: &mut impl BufRead,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    writeln!(child_stdin, "{}", req).unwrap();
    child_stdin.flush().unwrap();
    let mut line = String::new();
    child_stdout.read_line(&mut line).unwrap();
    serde_json::from_str(line.trim()).unwrap_or_else(|e| panic!("bad response {line:?}: {e}"))
}

#[test]
fn mcp_initialize_lists_tools_and_serves_protocol_resource() {
    let home = tmp_home();
    let mut child = Command::new(bin())
        .args([
            "mcp",
            "--home",
            home.to_str().unwrap(),
            "--session",
            "mcp-test",
            "--harness",
            "other",
            "--cwd",
            home.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn harbard mcp");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let init = rpc(
        &mut stdin,
        &mut stdout,
        1,
        "initialize",
        serde_json::json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "harbard-test", "version": "0" }
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "harbard");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    // notifications/initialized — no response expected
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    stdin.flush().unwrap();

    let tools = rpc(
        &mut stdin,
        &mut stdout,
        2,
        "tools/list",
        serde_json::json!({}),
    );
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"harbard_note"));
    assert!(names.contains(&"harbard_protocol"));
    assert!(names.contains(&"harbard_claim"));

    let res = rpc(
        &mut stdin,
        &mut stdout,
        3,
        "resources/read",
        serde_json::json!({ "uri": "harbard://protocol/v1" }),
    );
    let text = res["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(text.contains("harbard/protocol/v1"));
    assert!(text.contains("contested_checkout"));

    let proto = rpc(
        &mut stdin,
        &mut stdout,
        4,
        "tools/call",
        serde_json::json!({
            "name": "harbard_protocol",
            "arguments": { "full": false }
        }),
    );
    let body = proto["result"]["content"][0]["text"].as_str().unwrap();
    assert!(body.contains("correction_codes"));

    let intent = rpc(
        &mut stdin,
        &mut stdout,
        5,
        "tools/call",
        serde_json::json!({
            "name": "harbard_intent",
            "arguments": { "text": "mcp facade smoke" }
        }),
    );
    assert_eq!(intent["result"]["isError"], false);
    let intent_text = intent["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        intent_text.contains("intent") || intent_text.contains("receipt"),
        "got {intent_text}"
    );

    // Broadcast handoff must surface as a tool error payload (protocol rule).
    let bad = rpc(
        &mut stdin,
        &mut stdout,
        6,
        "tools/call",
        serde_json::json!({
            "name": "harbard_note",
            "arguments": { "text": "nope", "kind": "handoff" }
        }),
    );
    assert_eq!(bad["result"]["isError"], true);
    let bad_text = bad["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        bad_text.contains("handoff_target_cardinality") || bad_text.contains("broadcast_handoff"),
        "got {bad_text}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn protocol_json_exposes_stable_v1_contract() {
    let home = tmp_home();
    let out = Command::new(bin())
        .args(["protocol", "--json", "--home", home.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], "harbard/protocol/v1");
    let codes = v["correction_codes"].as_array().unwrap();
    for need in [
        "intent_stale",
        "ack_pending",
        "contested_checkout",
        "lease_held",
        "lease_unprotected",
        "broadcast_handoff",
        "handoff_target_cardinality",
        "channel_no_recipients",
        "channel_unused",
        "hooks_unhealthy",
    ] {
        assert!(
            codes.iter().any(|c| c == need),
            "missing correction code {need} in {codes:?}"
        );
    }
    assert!(v["capabilities"].as_array().unwrap().len() >= 5);
    assert!(v["rules"].as_array().unwrap().len() >= 4);
}
