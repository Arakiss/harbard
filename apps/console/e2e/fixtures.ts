import { spawn, type ChildProcess } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { test as base, type Page } from "@playwright/test";

const APP_DIR = dirname(dirname(fileURLToPath(import.meta.url)));
const MOCK_PORT = Number(process.env.HARBARD_MOCK_PORT ?? 7419);
const MOCK_BASE = `http://127.0.0.1:${MOCK_PORT}`;

async function sleep(ms: number): Promise<void> {
  await new Promise((r) => setTimeout(r, ms));
}

/**
 * Owns the mock serve process for a worker. Tests switch scenarios in place, and
 * B5 uses stop()/start() to prove the disconnected → recover cycle honestly, by
 * killing the process rather than faking an error.
 */
export class MockController {
  private proc: ChildProcess | null = null;

  async start(fixture = "fleet"): Promise<void> {
    if (this.proc) return;
    this.proc = spawn("bun", ["run", "mock/serve.ts"], {
      cwd: APP_DIR,
      env: { ...process.env, PORT: String(MOCK_PORT), HARBARD_FIXTURE: fixture },
      stdio: "ignore",
    });
    await this.waitReady(true);
  }

  async stop(): Promise<void> {
    if (!this.proc) return;
    const proc = this.proc;
    this.proc = null;
    proc.kill("SIGTERM");
    await this.waitReady(false).catch(() => undefined);
    await sleep(150);
  }

  /** Reload the in-memory store from a named fixture without restarting. */
  async reset(scenario: string): Promise<void> {
    const res = await fetch(`${MOCK_BASE}/__control/reset?scenario=${scenario}`, {
      method: "POST",
    });
    if (!res.ok) throw new Error(`reset ${scenario} -> ${res.status}`);
  }

  /** Append an event and broadcast it on the SSE stream. */
  async append(event: Record<string, unknown>): Promise<void> {
    const res = await fetch(`${MOCK_BASE}/__control/append`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(event),
    });
    if (!res.ok) throw new Error(`append -> ${res.status}`);
  }

  /** Append to the store without an SSE frame, simulating a lost leader. */
  async appendSilently(event: Record<string, unknown>): Promise<void> {
    const res = await fetch(`${MOCK_BASE}/__control/append-silent`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(event),
    });
    if (!res.ok) throw new Error(`append-silent -> ${res.status}`);
  }

  private async waitReady(up: boolean, timeoutMs = 8000): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      try {
        const res = await fetch(`${MOCK_BASE}/__control/ready`);
        if (up && res.ok) return;
        if (!up) {
          // Any response means still up; keep waiting for it to go dark.
          await sleep(100);
          continue;
        }
      } catch {
        if (!up) return; // connection refused == down
      }
      await sleep(100);
    }
    if (up) throw new Error("mock did not come up in time");
    throw new Error("mock did not go down in time");
  }
}

/**
 * Open the fleet rail before the first paint.
 *
 * Two preferences, because the rail needs both to render: it is closed by
 * default since `51252a1`, AND it is suppressed entirely on Harbor, which is
 * the default view. Every spec that addresses a `[data-channel=…]` row needs
 * the rail, and none of them said so, so fourteen of them waited 30s for an
 * element that was never going to exist — a DOM change that `just ci` cannot
 * see, because the e2e suite is not in the push gate.
 *
 * Seeding the same two keys the operator's own toggles write keeps the test on
 * the real code path: no test-only prop, no second way to open the rail.
 *
 * Call before `page.goto("/")` — `addInitScript` runs before the app's own
 * effects read the keys.
 */
export async function openRail(page: Page, view = "all"): Promise<void> {
  await page.addInitScript((startView) => {
    localStorage.setItem("harbard.rail", "open");
    localStorage.setItem("harbard.view", startView as string);
  }, view);
}

/** Twenty live sessions; contention puts the longest checkout in the glance budget. */
export async function busyFleet(mock: MockController) {
  await mock.reset("empty");
  const longCheckout = "/work/international-reconciliation-and-provider-delivery-verification-service";
  for (let index = 0; index < 19; index += 1) {
    const repo = index === 18 ? longCheckout : `/work/mission-${String(index).padStart(2, "0")}`;
    const common = { session: `busy-session-${index}`, harness: index % 2 ? "codex" : "claude-code", cwd: repo, repo, branch: "feat/verify-provider-delivery-and-preserve-existing-worktree-ownership" };
    await mock.append({ ...common, event: "name", text: `mission-owner-${String(index).padStart(2, "0")}` });
    await mock.append({ ...common, event: "intent", text: "Verify the provider delivery receipt against the served revision, preserve current ownership, and report the remaining acceptance evidence before closing this task." });
  }
  await mock.append({ event: "intent", session: "busy-independent-writer", harness: "codex", cwd: longCheckout, repo: longCheckout, branch: "feat/parallel-verification", text: "Independent writer checking the same checkout; ownership must be reviewed before another edit." });
  return { longCheckout, sessions: 20, checkouts: 19 };
}

export const test = base.extend<Record<never, never>, { mock: MockController }>({
  mock: [
    async ({}, use) => {
      const controller = new MockController();
      await controller.start("fleet");
      await use(controller);
      await controller.stop();
    },
    { scope: "worker" },
  ],
});

export const expect = test.expect;
export { MOCK_BASE, MOCK_PORT };
