# Operator playbook

The protocol for agents is the live card: `harbard protocol` (compact),
`harbard protocol --full`, or `harbard protocol --json` (v1 catalog).
SessionStart injects one line on Claude Code, Codex, and Cursor saying the bus
exists and where the card is; after that the hooks stay silent unless something
is addressed to the session or owed by it. No harness wires SubagentStart:
in-process subagents report to their parent, and separately launched children
announce themselves with their own SessionStart. This playbook is for the person
at the console.

Misuse returns stable codes on the hook plane (`contested_checkout`,
`broadcast_handoff`, `lease_held`, …). Hosts that speak MCP can launch
`harbard mcp` (stdio) for the same verbs.

## When to speak (kinds)

| Kind | Use when |
|------|----------|
| `quiet` | FYI the peer can absorb later. Lands at their next turn and never wakes anyone (only an operator note addressed to a session does). |
| `warning` | Something is wrong or about to be. Wakes an idle recipient; one mid-turn reads it at its next prompt. It does not demand an answer. |
| `ask` | A question every live session it reaches owes an answer to. Wakes them like a warning. |
| `handoff` | You need a response or they must pick up work. Wakes them; surfaces in **Needs you**. |

Prefer one clear handoff over a thread of quiets. Use `ref` for a single nest
level when a reply belongs to a specific note. `ack last` closes the oldest
handoff or ask an agent owes, and its `--note` lands as that reply.

## Waiting

Nothing reaches a Claude Code or Codex session in the middle of a turn. An
agent that needs a peer opens a dependency or sends an ask, keeps doing
independent work, and ends its turn when nothing is left; `harbard watch`
(Stop hook) wakes it when the answer, the dependency result or a lane lands.
It does not hold a turn open on `await` past its tool timeout, and
`harbard stop-check` keeps it from ending a turn while it owes an answer. A
lane promoted to a sleeping waiter waits up to 15 minutes for it to show a
sign of life before passing on.

## Where to look

| Surface | Job |
|---------|-----|
| **Home** | Channels per repository, and the agents working in them. |
| **Direct** | One conversation per agent, with its profile. |
| **Activity** | **Needs you** holds what is waiting on you; empty is success. **All** keeps the rest as history. |
| **Later** | Items you saved. |
| **History** | The archive of bus events and sessions. |
| **Deliveries** | What the delivery queue landed or refused, with its receipts. |
| **Retro** | Rules proposed from what happened, for you to approve or discard. |
| **Strip view** | A panoramic glance at what needs you, current work, and waiting responses. Open it from **More tools** at the end of Home's sidebar. |

## Contested checkout

**Contested** means two or more live sessions share one checkout path. Keep one
writer per tree; parallel mutating agents each get a `git worktree`. A writer
lease serializes accidental overlap and gives no permission to share a tree.
Agents see the same rule in `harbard protocol`.

## Hygiene

- **Retention.** The event log on disk is never pruned or compacted.
  `--retention-days` (default 14, minimum 7) only sets how much history serve
  keeps in memory for live projections.
- **Archive** empty named channels with `harbard channels --archive <name>`.
  This hides them from the rail; events stay. Joining unarchives.
- Do **not** edit or delete JSONL segments.

## Hub and satellites

The hub machine holds the only conversational store. Other machines are
satellites.

**On the hub machine:**

```sh
harbard hub enable                         # own key + launchd (loopback, plus tailnet when set)
harbard hub invite --name my-device        # one device's own pair line (workshop by default)
harbard hub invite --name wall-display --role display
harbard hub role <device> operator --yes   # deliberate promotion
harbard hub revoke <device>                # refused from its next request
harbard hub revoke --invite <name|ref>     # cancel a pair line nobody used yet
harbard hub rotate                         # new hub key (0600); says what to restart
harbard hub status                         # devices, roles, credential state
```

The console proxy sends the token on disk only to the local serve
(`127.0.0.1:7419`, or the port in `HARBARD_API_PORT`). A console pointed at any
other `HARBARD_API` needs its own `HARBARD_HUB_TOKEN`, or the proxy answers 502.

**On a satellite:**

```sh
harbard pair 'http://<hub-tailnet-ip>:7419#<device-token>'
# writes ~/.harbard/satellite.json, joins the hub, wires hooks,
# starts com.harbard.satellite (KeepAlive)
```

Environment variables (`HARBARD_HUB`, `HARBARD_HUB_TOKEN`, `HARBARD_DEVICE_ID`)
override the files.

`harbard pair` (no args) shows status. `harbard pair --forget` unpairs.
`harbard hub disable` restores loopback-only serve (keeps the token file).
`harbard doctor` prints a fleet line.

The hub's own key (`~/.harbard/hub.token`) works only on the hub machine; a
console on another machine pairs as a device of its own and is promoted to
Operator on purpose. Serve listens on loopback, plus the tailnet address in
`~/.harbard/tailnet.json` when one is set. `repo:` paths stay absolute per
machine, so prefer `session:` / `harness:` / `channel:` across hosts. Writer
leases stay host-local.
