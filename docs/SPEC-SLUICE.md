# The sluice: `deliver` and `integrate`

## Why

A writer lease answers "who may touch this checkout right now" by making
everyone else wait. That suits a shared working tree. Parallel agents should
not share one: Git can hold as many copies of a tree as they need.

The scarce resource is the right to move `main`, and a queue fits it. The sluice
takes a commit and integrates it if it survives the checks.

    agent A ──deliver(sha)──┐
    agent B ──deliver(sha)──┼──▶ integrate ──▶ candidate on current main
    agent C ──deliver(sha)──┘                        │
                                                     ├─ checks pass → main moves to THAT sha
                                                     └─ checks fail → main untouched, receipt says why

Nobody waits to start. One thing at a time is integrated, and the SHA that
lands is the SHA that was checked, with no rebuild in between.

## Behaviours

| # | Behaviour | How it is proven |
|---|---|---|
| B1 | `deliver` records repo, delivered SHA, target branch, the base it was built on, and the check command | receipt carries all five; `deliveries` lists it |
| B2 | `deliver` refuses a SHA that does not exist in that repo | non-zero exit, no receipt written |
| B3 | `deliver` refuses a target branch that does not exist | non-zero exit, no receipt written |
| B4 | `deliver` is append-only: two deliveries from one agent both queue | both appear, oldest first |
| B5 | `integrate` builds a candidate by merging the delivered SHA onto the CURRENT tip of the target branch | candidate's parents are the tip and the delivered SHA |
| B6 | The checks run against the candidate rather than the delivered SHA in isolation | the check's cwd is the candidate worktree |
| B7 | Checks pass → the target branch moves to the exact candidate that was checked | branch tip == candidate SHA from the receipt |
| B8 | Checks fail → the target branch does not move | tip unchanged; receipt records the failure and the check output tail |
| B9 | A merge conflict is a rejection and never a crash | receipt says `conflict`, branch unchanged, queue advances |
| B10 | An interrupted integrate leaves no half-state | a killed run leaves the branch unchanged and the delivery still queued |
| B11 | Every outcome is a durable receipt: who, what SHA, onto what base, which candidate, which checks, what verdict | `harbard deliveries --json` replays them |
| B12 | Integration never runs two at once for one repo | the second caller is refused with the service lease and is not queued in memory |

## Limits

- No policies (who may deliver to which branch).
- No remote push. `integrate` moves a local branch; pushing is a separate decision.
- No ordering beyond arrival: no priorities, dependencies between deliveries, or batching.
- No conflict resolution. A conflict is reported and rejected.

## Storage

`~/.harbard/sluice/<repo-hash>/queue.jsonl`: one line per delivery, append-only.
`~/.harbard/sluice/<repo-hash>/receipts.jsonl`: one line per integration attempt.

Both are append-only so the queue survives a crash mid-integration and every
landing keeps its evidence.
