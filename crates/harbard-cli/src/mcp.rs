//! Thin MCP stdio facade over Harbard protocol v1.
//!
//! Newline-delimited JSON-RPC (MCP stdio transport, 2025-11-25). No async
//! runtime — tools shell out to this same binary so the CLI remains the
//! single authority for bus verbs. MCP is a host adapter, not a second bus.

use crate::cli::Common;
use crate::machine;
use harbard_core::protocol::{PROTOCOL_VERSION, ProtocolDepth, capabilities, catalog};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};

/// Negotiated MCP protocol version we speak.
const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

/// Run the MCP stdio loop until stdin closes. Exit 0 on clean EOF.
pub fn cmd_mcp(common: &Common) -> i32 {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout().lock();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return 0,
            Ok(_) => {}
            Err(e) => {
                eprintln!("harbard mcp: read error: {e}");
                return 1;
            }
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("harbard mcp: bad JSON: {e}");
                continue;
            }
        };

        // Notifications have no id — ignore unknowns quietly.
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(json!({}));

        if id.is_none() {
            // notifications/initialized, etc.
            continue;
        }

        let result = match method {
            "initialize" => Ok(initialize_result(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => call_tool(common, &params),
            "resources/list" => Ok(json!({ "resources": resources() })),
            "resources/read" => read_resource(&params),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            _ => Err(rpc_error(-32601, format!("method not found: {method}"))),
        };

        let response = match result {
            Ok(result) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            }),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": error,
            }),
        };
        if let Err(e) = write_message(&mut stdout, &response) {
            eprintln!("harbard mcp: write error: {e}");
            return 1;
        }
    }
}

fn write_message(out: &mut impl Write, msg: &Value) -> std::io::Result<()> {
    let mut s = serde_json::to_string(msg).unwrap_or_else(|_| "{}".into());
    // MCP stdio: messages MUST NOT contain embedded newlines.
    s.retain(|c| c != '\n' && c != '\r');
    out.write_all(s.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

fn rpc_error(code: i32, message: String) -> Value {
    json!({ "code": code, "message": message })
}

fn initialize_result(params: &Value) -> Value {
    let client_version = params
        .get("protocolVersion")
        .and_then(|v| v.as_str())
        .unwrap_or(MCP_PROTOCOL_VERSION);
    // Speak the client's version when we know it; otherwise our preferred.
    let version = match client_version {
        "2024-11-05" | "2025-03-26" | "2025-06-18" | "2025-11-25" | "2026-07-28" => client_version,
        _ => MCP_PROTOCOL_VERSION,
    };
    json!({
        "protocolVersion": version,
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": { "listChanged": false },
        },
        "serverInfo": {
            "name": "harbard",
            "version": env!("CARGO_PKG_VERSION"),
            "title": "Harbard fleet bus",
        },
        "instructions": format!(
            "Harbard protocol {PROTOCOL_VERSION}. Fleet coordination: presence, \
    speech acts (intent/note/ack), writer leases. One writer per checkout; \
    parallel agents use git worktree. Prefer these tools over inventing bus grammar."
        ),
    })
}

/// Tool description from the catalog capability summary (M5b parity).
fn cap_summary(id: &str, fallback: &str) -> String {
    capabilities()
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.summary.to_string())
        .unwrap_or_else(|| fallback.to_string())
}

fn tools() -> Vec<Value> {
    let name = cap_summary(
        "name",
        "Choose this session's permanent callsign: a fresh proper name with character.",
    );
    let intent = cap_summary("intent", "Declare what this session is doing now.");
    let note = cap_summary("note", "Leave a note on the bus.");
    let ack = cap_summary("ack", "Close a handoff addressed to you.");
    let brief = cap_summary("brief", "Bounded digest of live peers + unread notes.");
    let claim = cap_summary("claim", "Acquire the writer lease for this checkout.");
    let sense = cap_summary("sense", "List sessions with liveness (live/stale/ended).");
    let doctor = cap_summary("doctor", "Store health, leases, open resolutions.");
    let audit = cap_summary("audit", "Git checkout / worktree hygiene (read-only).");
    let trace = cap_summary("trace", "Reconstruct delivery for a note or session.");
    let intro = cap_summary("intro", "Say who you are in #lobby.");
    let feedback = cap_summary("feedback", "Tell harbard what got in your way.");
    let feedback_list = cap_summary("feedback.list", "Read what agents reported about harbard.");
    let channel_read = cap_summary("channel.read", "Read a room's history.");
    let channel_post = cap_summary("channel.post", "Write in a room.");
    vec![
        tool(
            "harbard_protocol",
            "Discover Harbard protocol v1 catalog (same document as GET /api/v1/protocol).",
            json!({
                "type": "object",
                "properties": {
                    "full": { "type": "boolean", "description": "Return the full card text" }
                }
            }),
        ),
        tool(
            "harbard_name",
            &name,
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": harbard_core::protocol::NAME_BRIEF },
                    "intro": { "type": "string", "maxLength": 280, "description": "Also say who you are in #lobby" }
                },
                "required": ["name"]
            }),
        ),
        tool(
            "harbard_intro",
            &intro,
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "maxLength": 280, "description": "Who you are, in a line" }
                },
                "required": ["text"]
            }),
        ),
        tool(
            "harbard_feedback",
            &feedback,
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "One sentence" },
                    "kind": { "type": "string", "enum": harbard_core::agent_feedback::KINDS },
                    "verb": { "type": "string", "description": "The harbard verb this is about, if any" }
                },
                "required": ["text", "kind"]
            }),
        ),
        tool(
            "harbard_feedback_list",
            &feedback_list,
            json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": harbard_core::agent_feedback::KINDS },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 50 }
                }
            }),
        ),
        tool(
            "harbard_channel_read",
            &channel_read,
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Room name, for example lobby" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 30 },
                    "before": { "type": "string", "description": "Older page: messages before this event id" }
                },
                "required": ["name"]
            }),
        ),
        tool(
            "harbard_channel_post",
            &channel_post,
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Room name, for example commons" },
                    "text": { "type": "string" },
                    "kind": { "type": "string", "enum": ["fyi", "warning", "ask"], "default": "fyi" },
                    "ref": { "type": "string", "description": "Parent message id" }
                },
                "required": ["name", "text"]
            }),
        ),
        tool(
            "harbard_intent",
            &intent,
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "One-line intent" }
                },
                "required": ["text"]
            }),
        ),
        tool(
            "harbard_note",
            &note,
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" },
                    "kind": {
                        "type": "string",
                        "enum": ["fyi", "warning", "ask", "handoff"],
                        "default": "fyi"
                    },
                    "to": {
                        "type": "string",
                        "description": "session:<id>|harness:<h>|repo:<path>|channel:<name>"
                    },
                    "ref": { "type": "string", "description": "Parent note id (one nest level)" }
                },
                "required": ["text"]
            }),
        ),
        tool(
            "harbard_ack",
            &ack,
            json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Note id or `last`"
                    },
                    "note": { "type": "string", "description": "Optional reply text" }
                },
                "required": ["target"]
            }),
        ),
        tool(
            "harbard_brief",
            &brief,
            json!({
                "type": "object",
                "properties": {
                    "touch": {
                        "type": "boolean",
                        "description": "Also heartbeat / advance brief mark"
                    }
                }
            }),
        ),
        tool(
            "harbard_claim",
            &claim,
            json!({
                "type": "object",
                "properties": {
                    "ttl_secs": { "type": "integer", "minimum": 1, "default": 900 }
                }
            }),
        ),
        tool(
            "harbard_ls",
            &sense,
            json!({
                "type": "object",
                "properties": {
                    "all": { "type": "boolean", "description": "Scan all segments" }
                }
            }),
        ),
        tool(
            "harbard_inbox",
            "Unread notes addressed to this session; advances the inbox cursor.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "harbard_doctor",
            &doctor,
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "harbard_dependency_open",
            "Register a durable dependency on one session's explicit resolution. The continuation is recorded text, never executed.",
            json!({"type": "object", "properties": {
                "to": {"type": "string", "pattern": "^session:.+", "description": "Exactly one responder session: session:<id>"},
                "condition": {"type": "string", "minLength": 1},
                "resume": {"type": "string", "minLength": 1, "description": "What to resume after consuming the resolution; not a command to execute"},
                "deadline_secs": {"type": "integer", "minimum": 1, "maximum": 604800}
            }, "required": ["to", "condition", "resume"]}),
        ),
        tool(
            "harbard_dependency_resolve",
            "Resolve an addressed dependency explicitly with an outcome and evidence.",
            json!({"type": "object", "properties": {
                "id": {"type": "string", "minLength": 1},
                "outcome": {"type": "string", "enum": ["satisfied", "failed"]},
                "evidence": {"type": "string", "minLength": 1}
            }, "required": ["id", "outcome", "evidence"]}),
        ),
        tool(
            "harbard_dependency_cancel",
            "Cancel your dependency with an explicit reason.",
            json!({"type": "object", "properties": {
                "id": {"type": "string", "minLength": 1},
                "reason": {"type": "string", "minLength": 1}
            }, "required": ["id", "reason"]}),
        ),
        tool(
            "harbard_dependency_consume",
            "Record that you consumed the dependency resolution. Does not execute the continuation.",
            json!({"type": "object", "properties": {"id": {"type": "string", "minLength": 1}}, "required": ["id"]}),
        ),
        tool(
            "harbard_dependency_show",
            "Read a durable dependency and its current resolution.",
            json!({"type": "object", "properties": {"id": {"type": "string", "minLength": 1}}, "required": ["id"]}),
        ),
        tool(
            "harbard_dependency_list",
            "List this session's dependencies, optionally including consumed history.",
            json!({"type": "object", "properties": {"all": {"type": "boolean", "default": false}}}),
        ),
        tool(
            "harbard_dependency_await",
            "Wait for one dependency for a bounded duration. Returns the durable state on satisfaction, failure, cancellation or timeout; never executes or consumes its continuation.",
            json!({"type": "object", "properties": {
                "id": {"type": "string", "minLength": 1},
                "timeout_secs": {"type": "integer", "minimum": 0, "maximum": 86400, "default": 300},
                "poll_ms": {"type": "integer", "minimum": 10, "maximum": 60000, "default": 250}
            }, "required": ["id"]}),
        ),
        tool(
            "harbard_trace",
            &trace,
            json!({
                "type": "object",
                "properties": {
                    "note": { "type": "string", "description": "Note id or prefix" },
                    "session": { "type": "string", "description": "Session id or prefix" }
                }
            }),
        ),
        tool(
            "harbard_audit",
            &audit,
            json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Compare worktrees to this ref (default upstream / origin/dev)"
                    },
                    "include_worktrees": {
                        "type": "boolean",
                        "description": "Include every linked worktree"
                    }
                }
            }),
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

fn resources() -> Vec<Value> {
    vec![json!({
        "uri": "harbard://protocol/v1",
        "name": "Harbard protocol v1 catalog",
        "description": "Capabilities, rules, correction codes, and compact card",
        "mimeType": "application/json",
    })]
}

fn read_resource(params: &Value) -> Result<Value, Value> {
    let uri = params
        .get("uri")
        .and_then(|u| u.as_str())
        .ok_or_else(|| rpc_error(-32602, "resources/read requires uri".into()))?;
    if uri != "harbard://protocol/v1" {
        return Err(rpc_error(-32002, format!("resource not found: {uri}")));
    }
    let body = catalog(ProtocolDepth::Compact);
    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": "application/json",
            "text": serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into()),
        }]
    }))
}

fn call_tool(common: &Common, params: &Value) -> Result<Value, Value> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| rpc_error(-32602, "tools/call requires name".into()))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let operation =
        tool_operation(name).ok_or_else(|| rpc_error(-32602, format!("unknown tool: {name}")))?;
    let (text, is_error) = machine::invoke_operation(common, operation, &args)
        .map_err(|message| rpc_error(-32602, message))?;
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    }))
}

fn tool_operation(name: &str) -> Option<&str> {
    match name {
        "harbard_dependency_open" => Some("dependency.open"),
        "harbard_dependency_resolve" => Some("dependency.resolve"),
        "harbard_dependency_cancel" => Some("dependency.cancel"),
        "harbard_dependency_consume" => Some("dependency.consume"),
        "harbard_dependency_show" => Some("dependency.show"),
        "harbard_dependency_list" => Some("dependency.list"),
        "harbard_dependency_await" => Some("dependency.await"),
        "harbard_channel_read" => Some("channel.read"),
        "harbard_channel_post" => Some("channel.post"),
        "harbard_feedback_list" => Some("feedback.list"),
        _ => name.strip_prefix("harbard_"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_cover_protocol_verbs() {
        let listed = tools();
        let names: Vec<&str> = listed
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        for need in [
            "harbard_protocol",
            "harbard_name",
            "harbard_intent",
            "harbard_note",
            "harbard_ack",
            "harbard_brief",
            "harbard_claim",
            "harbard_ls",
            "harbard_inbox",
            "harbard_doctor",
            "harbard_trace",
            "harbard_audit",
            "harbard_dependency_open",
            "harbard_dependency_resolve",
            "harbard_dependency_cancel",
            "harbard_dependency_consume",
            "harbard_dependency_show",
            "harbard_dependency_list",
            "harbard_dependency_await",
            "harbard_intro",
            "harbard_feedback",
            "harbard_feedback_list",
            "harbard_channel_read",
            "harbard_channel_post",
        ] {
            assert!(names.contains(&need), "missing {need}");
        }
    }

    #[test]
    fn initialize_advertises_harbard() {
        let v = initialize_result(&json!({ "protocolVersion": "2025-11-25" }));
        assert_eq!(v["protocolVersion"], "2025-11-25");
        assert_eq!(v["serverInfo"]["name"], "harbard");
        assert!(v["instructions"].as_str().unwrap().contains("protocol"));
    }

    #[test]
    fn resource_catalog_is_v1() {
        let v = read_resource(&json!({ "uri": "harbard://protocol/v1" })).unwrap();
        let text = v["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains(PROTOCOL_VERSION));
        assert!(text.contains("contested_checkout"));
    }

    #[test]
    fn every_mcp_tool_maps_to_a_cli_operation() {
        let listed = tools();
        let names = listed
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        for name in names {
            let operation = tool_operation(name).unwrap();
            assert!(
                machine::OPERATION_IDS.contains(&operation),
                "MCP operation {operation} must exist in the CLI"
            );
        }
    }

    #[test]
    fn dependency_tools_keep_cli_bounds_and_explicit_outcomes() {
        let listed = tools();
        let schema =
            |name: &str| &listed.iter().find(|tool| tool["name"] == name).unwrap()["inputSchema"];
        let wait = schema("harbard_dependency_await");
        assert_eq!(wait["properties"]["timeout_secs"]["maximum"], 86_400);
        assert_eq!(wait["properties"]["timeout_secs"]["default"], 300);
        assert_eq!(wait["properties"]["poll_ms"]["minimum"], 10);
        assert_eq!(wait["properties"]["poll_ms"]["maximum"], 60_000);
        assert_eq!(wait["properties"]["poll_ms"]["default"], 250);
        assert_eq!(
            schema("harbard_dependency_resolve")["properties"]["outcome"]["enum"],
            json!(["satisfied", "failed"])
        );
        assert!(
            schema("harbard_note")["properties"]["kind"]["enum"]
                .as_array()
                .unwrap()
                .contains(&json!("ask"))
        );
    }
}
