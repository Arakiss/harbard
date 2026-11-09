Synthetic Claude Code transcripts for the team view tests.

The shape (record types, keys, the `<teammate-message>` envelope, the
`api-request*` records that open with their type) follows what Claude Code
2.1.3xx writes under `~/.claude/projects/<project>/<session>/subagents/`.
Every string is invented; no real transcript text is copied here.

- `lead.jsonl`: the lead session transcript.
- `agent-a1.*`: teammate `notes`. It sent START, then BLOCKED, and ended its
  turn without an answer.
- `agent-a2.*`: teammate `dates`. It wrote in `/work/demo` while `notes` did,
  sent the same unlabelled message twice, and is still inside a tool call.
