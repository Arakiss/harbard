# Git hygiene audit contract

## Objective

`harbard audit` gives an operator one read-only view of every checkout that
belongs to the same Git repository. It identifies sibling worktrees explicitly
so a clean primary checkout cannot hide stale, divergent, or dirty linked work.

```text
harbard audit --repo <path> [--target <ref>] [--include-worktrees] [--json]
```

The command runs Git inspection commands only. It strips inherited `GIT_*`
control variables, sets `GIT_OPTIONAL_LOCKS=0`, disables fsmonitor and the
untracked cache, and suppresses configured filters, external diffs, and
textconv programs during working-tree inspection. Dirtiness is derived from
read-only diff plumbing plus untracked-file enumeration; `git status` is not
used. It never fetches, updates refs, refreshes the index, changes
configuration, acquires leases, starts a process supervisor, or writes to
Harbard's event store.

## Repository grouping

The repository identity is the absolute path returned by
`git rev-parse --git-common-dir`. With `--include-worktrees`, Harbard enumerates
`git worktree list --porcelain -z` and emits one result per checkout. The first
entry is marked `primary`; every other entry is a distinct linked worktree.
Without that flag, the command emits only the checkout containing `--repo`, but
still reports the shared Git identity and whether that checkout is primary.

## Comparison target

The comparison target is selected independently for every worktree:

1. The explicit `--target <ref>`, when provided.
2. The branch's semantic upstream as resolved through Git's fetch refspec,
   including a configured upstream whose ref has been deleted.
3. `origin/dev`, only when that commit resolves locally.
4. No target when none of the preceding choices resolves or applies.

Ahead and behind are computed from local objects with
`git rev-list --left-right --count <target>...HEAD`. No implicit fetch occurs.
A configured but unresolvable upstream sets `upstreamGone: true`. An explicit
missing target and any other worktree-level read failure are recorded in that
worktree's `errors` array without aborting sibling inspection.

## JSON contract

The top-level object contains:

- `requestedRepo`, `commonDir`, `requestedTarget`, and `includeWorktrees`.
- `worktrees`, one object per selected checkout.
- `errors`, for repository discovery or worktree enumeration failures.

Each worktree contains `root`, `primary`, `branch`, `detached`, `head`,
`upstream`, `upstreamGone`, `target`, `targetHead`, `ahead`, `behind`, `dirty`,
and `errors`. Nullable values mean the fact could not be derived locally. A
path outside Git produces a structured report, an empty `worktrees` array, and
a non-zero exit status. Once repository discovery succeeds, individual read
errors are nonfatal and the command exits successfully with those errors in the
report. On Unix, invalid UTF-8 path bytes are retained reversibly as `\\xNN`
sequences rather than replaced with the Unicode replacement character.

## Required behaviour

| ID | Priority | Behavior | Evidence |
|---|---|---|---|
| G1 | P0 | Primary and linked worktrees share `commonDir` but retain distinct roots and `primary` flags. | Temporary primary plus two linked worktrees. |
| G2 | P0 | A configured upstream whose remote-tracking ref was deleted sets `upstreamGone`. | Temporary bare remote and deleted branch/ref. |
| G3 | P0 | Behind and diverged worktrees report correct target-relative counts. | Temporary branch graph with target-only and branch-only commits. |
| G4 | P0 | Untracked or modified files set `dirty` only for their own worktree. | Dirty linked-worktree fixture. |
| G5 | P1 | A missing explicit target is a nonfatal per-worktree error. | Missing-ref CLI test. |
| G6 | P1 | A path outside Git returns structured JSON and a non-zero status. | Non-repository CLI test. |
| G7 | P0 | Inspection performs no fetch or Git/Harbard writes. | Command implementation review. |
| G9 | P0 | Inherited Git environment cannot retarget inspection or inject configuration, and inspection leaves the index byte-for-byte unchanged. | Hostile-environment and index immutability tests. |
| G10 | P0 | Fsmonitor, clean/smudge/process filters, external diffs, and textconv programs are never executed. | Sentinel-program regression test. |
| G11 | P1 | Custom fetch refspecs resolve the semantic upstream; trailing-space and non-UTF-8 checkout paths remain operable. | Custom-refspec and path regression tests. |
| G12 | P1 | A submodule reports its checkout root as primary rather than exposing internal module storage as a worktree root. | Submodule regression test. |
