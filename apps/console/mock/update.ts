/**
 * Mock `/api/v1/update` (docs/SPEC-UPDATES.md): a believable newer `main`
 * with its changelog, and a run that walks its five steps on a clock so the
 * banner, the dialog and its progress can be seen with `just dev`.
 *
 * `HARBARD_MOCK_UPDATE=current` starts up to date; `=fail` makes the run's
 * console step fail; `=dirty` reports a checkout with uncommitted changes.
 */

type StepId = "fetch" | "cli" | "serve" | "console" | "desktop";

const MODE = process.env.HARBARD_MOCK_UPDATE ?? "available";
const STEP_MS: Record<StepId, number> = { fetch: 1_500, cli: 4_000, serve: 1_500, console: 6_000, desktop: 2_000 };
const ORDER: StepId[] = ["fetch", "cli", "serve", "console", "desktop"];

const INSTALLED = "a955a72e0c4d4f1a9b2e7d3c5f8a6b1e2d3c4f5a";
const TARGET = "4be59701c9d84e2fa1b6c3d7e8f9a0b1c2d3e4f5";

const NOTES = `### Updates from the console

- The console says when a new version is out, shows what changed and installs it with one button.
- \`harbard update\` does the same from the terminal, and \`--check\` only looks.

### Teams

- Each paired device says which version it runs; those behind the hub are flagged.`;

const CHANGES = [
  { kind: "feat", scope: "console", subject: "a notice, the changelog and one button to update" },
  { kind: "feat", scope: "cli", subject: "harbard update checks, shows what changed and installs it" },
  { kind: "feat", scope: "serve", subject: "check for a newer version every six hours" },
  { kind: "feat", scope: "hub", subject: "devices report their version and commit" },
  { kind: "fix", scope: "desktop", subject: "the app reopens after an update replaced it" },
  { kind: "fix", scope: "core", subject: "never wait on a transcript root macOS has not cleared" },
  { kind: "perf", scope: "serve", subject: "the check runs off the request path" },
  { kind: "refactor", scope: null, subject: "one module owns the source checkout" },
  { kind: "docs", scope: null, subject: "updates, as built" },
  { kind: "test", scope: "update", subject: "the run survives a serve restart" },
  { kind: "build", scope: "cli", subject: "embed the source root at build time" },
].map((c, i) => ({ commit: `${(0xa1b2c3d + i * 0x1111).toString(16)}${"0".repeat(33)}`.slice(0, 40), date_ms: Date.now() - (i + 1) * 3_600_000, ...c }));

interface Step {
  id: StepId;
  state: "pending" | "running" | "done" | "failed" | "skipped";
  started_ms: number | null;
  finished_ms: number | null;
  detail: string | null;
}

let prefs = { channel: "main" as "main" | "releases", auto_check: true };
let installed = INSTALLED;
let checkedMs = Date.now() - 12 * 60_000;
let upToDate = MODE === "current";
let run: { id: string; started_ms: number; failAt: StepId | null } | null = null;
let runSnapshot: unknown = null;

function checkBody() {
  if (upToDate) {
    return {
      checked_ms: checkedMs,
      channel: prefs.channel,
      status: "current",
      error: null,
      target: { version: "0.3.0-alpha.9", commit: installed, tag: null, title: null, date_ms: Date.now() - 2 * 3_600_000 },
      behind: 0,
      notes: prefs.channel === "main" ? NOTES : null,
      changes: [],
      changes_truncated: false,
    };
  }
  const releases = prefs.channel === "releases";
  return {
    checked_ms: checkedMs,
    channel: prefs.channel,
    status: "available",
    error: null,
    target: releases
      ? { version: "0.3.0-alpha.10", commit: TARGET, tag: "v0.3.0-alpha.10", title: "Update with one button", date_ms: Date.now() - 50 * 60_000 }
      : { version: "0.3.0-alpha.9", commit: TARGET, tag: null, title: null, date_ms: Date.now() - 50 * 60_000 },
    behind: CHANGES.length,
    notes: NOTES,
    changes: CHANGES,
    changes_truncated: false,
  };
}

/** The run as of now: each step's window follows the previous one. */
function runBody(now: number) {
  if (!run) return runSnapshot;
  const steps: Step[] = [];
  let t = run.started_ms;
  let failed = false;
  let finished: number | null = null;
  for (const id of ORDER) {
    const end = t + STEP_MS[id];
    if (failed) {
      steps.push({ id, state: "pending", started_ms: null, finished_ms: null, detail: null });
    } else if (id === "desktop" && now >= t) {
      // In a browser there is no app to reopen: the step says so and ends the run.
      steps.push({ id, state: "skipped", started_ms: t, finished_ms: t, detail: "not_open" });
      finished = t;
    } else if (now < t) {
      steps.push({ id, state: "pending", started_ms: null, finished_ms: null, detail: null });
    } else if (now < end) {
      steps.push({ id, state: "running", started_ms: t, finished_ms: null, detail: null });
    } else if (run.failAt === id) {
      steps.push({ id, state: "failed", started_ms: t, finished_ms: end, detail: "next build exited with status 1" });
      failed = true;
      finished = end;
    } else {
      steps.push({ id, state: "done", started_ms: t, finished_ms: end, detail: null });
      if (id === "desktop") finished = end;
    }
    t = end;
  }
  const state = failed ? "failed" : finished !== null ? "done" : "running";
  const log = [
    "harbard update — fetching origin",
    `Updating ${INSTALLED.slice(0, 7)}..${TARGET.slice(0, 7)}`,
    "Fast-forward",
    " 49 files changed, 5243 insertions(+), 213 deletions(-)",
    "   Compiling harbard-core v0.3.0-alpha.9",
    "   Compiling harbard-cli v0.3.0-alpha.9",
    "    Finished `release` profile [optimized] target(s) in 19.99s",
    "installed: ~/.cargo/bin/harbard",
    "harbard update — restarting serve",
    "harbard update — building console and desktop before replacement",
    ...(failed ? ["error: next build failed", "Type error: Cannot find name 'x'."] : []),
  ].slice(0, 3 + steps.filter((s) => s.state !== "pending").length * 2);
  const body = {
    id: run.id,
    state,
    started_ms: run.started_ms,
    finished_ms: state === "running" ? null : finished,
    from: INSTALLED,
    to: TARGET,
    error: failed ? { code: "step_failed", message: "the console step failed" } : null,
    steps,
    log_tail: log,
    notes: NOTES,
    changes: CHANGES,
    changes_truncated: false,
  };
  if (state === "done") {
    installed = TARGET;
    upToDate = true;
  }
  if (state !== "running") {
    runSnapshot = body;
    run = null;
  }
  return body;
}

export function updateBody() {
  const now = Date.now();
  const r = runBody(now);
  return {
    installed: { version: "0.3.0-alpha.9", commit: installed },
    prefs,
    source: { path: "/Users/operator/Projects/harbard", problem: MODE === "dirty" ? "dirty" : null },
    check: checkBody(),
    run: r ?? null,
  };
}

/** `POST /api/v1/update/<action>`: status and body, the way serve answers. */
export async function updateWrite(action: string, req: Request): Promise<{ status: number; body: unknown }> {
  if (action === "check") {
    await new Promise((r) => setTimeout(r, 900));
    checkedMs = Date.now();
    return { status: 200, body: updateBody() };
  }
  if (action === "prefs") {
    const patch = (await req.json().catch(() => ({}))) as Partial<typeof prefs>;
    if (patch.channel === "main" || patch.channel === "releases") {
      prefs = { ...prefs, channel: patch.channel };
      checkedMs = Date.now();
    }
    if (typeof patch.auto_check === "boolean") prefs = { ...prefs, auto_check: patch.auto_check };
    return { status: 200, body: updateBody() };
  }
  if (action === "install") {
    if (run) return { status: 409, body: { error: "busy", message: "an update is already running" } };
    if (MODE === "dirty") return { status: 409, body: { error: "dirty", message: "the checkout has uncommitted changes" } };
    run = { id: `run-${Date.now()}`, started_ms: Date.now(), failAt: MODE === "fail" ? "console" : null };
    runSnapshot = null;
    return { status: 202, body: updateBody() };
  }
  return { status: 404, body: { error: "not_found", message: `no update action ${action}` } };
}
