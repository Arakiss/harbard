# Memory sink

harbard can hand an approved rule to an external program, so a rule the
operator wants to keep can also land in whatever memory tool they use. It is
off by default and there is no built-in sink.

## Turning it on

1. Name one executable, by absolute path. Either:
   - the `HARBARD_MEMORY_SINK` environment variable of `harbard serve`, or
   - `"memory_sink_command"` inside `config` in `~/.harbard/workspace.json`.

   The variable wins when both are set. The value is a path, not a command
   line: no arguments, no shell, no `PATH` lookup. A relative path or a file
   that is not executable counts as not configured. No HTTP route can set it.
2. In Preferences, allow saving to external memory. This asks for confirmation
   and raises a "Was this you?" card.
3. In the Retro, tick the save box on the rule you approve. harbard stores the
   rule first; the sink runs only afterwards, and only for that rule.

## What the sink receives

One JSON object on stdin, then end of file:

```json
{
  "version": 1,
  "kind": "decision",
  "rule_id": "01J8ZQ4Y7V3K",
  "key": "rm_variable",
  "title": "Guard rm",
  "rule": "Quote the variable in every rm",
  "evidence": "Stalled three times this week"
}
```

`rule` and `title` are the stored text. `evidence` is the Retro's own, with
hidden characters removed. `rule_id` is stable, so a sink can ignore a repeat.

The process gets only `HOME`, `PATH`, `USER` and `LANG`. It has 15 seconds;
after that its whole process group is killed. harbard reads the first 64 KiB
of stdout and of stderr.

## What the sink answers

One JSON object on stdout, and nothing else:

| Answer | Meaning |
| --- | --- |
| `{ "version": 1, "saved": true, "id": "..." }` | Saved. `id` is optional. |
| `{ "version": 1, "saved": false, "reason": "locked" }` | The sink needs an unlock. harbard never unlocks anything. |
| `{ "version": 1, "saved": false, "reason": "..." }` | Refused, for that machine-readable reason. |

An answer with no `version` is read as version 1; any other version is not
read at all. A refusal counts whatever the exit code. `saved: true` counts only
with exit code 0. Anything else (no JSON, a crash, the deadline) leaves the
rule in harbard only, and the console shows why. A failed exit shows the first
line of stderr, so keep secrets out of it.

Whatever the sink does, the rule stays approved in harbard.
