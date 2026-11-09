import { defineConfig, devices } from "@playwright/test";

/**
 * E2E against the mock serve. The Next server is built once and started on 3419
 * (production build — no HMR sockets, so every request is a clean GET, which row
 * B6 asserts). The mock is NOT managed here: a worker fixture owns its lifecycle
 * so a single test can kill and restart it (row B5). Tests run serially on one
 * mock port to mirror the real single-serve topology.
 */
/**
 * Both ports are overridable, because the suite otherwise cannot run at all
 * while a live `just up` stack is on the machine: the console holds 3419 and
 * the operator's real `harbard serve` holds 7419, which is the port the mock
 * has to bind. Killing the operator's stack to run a test is not a reasonable
 * price, so `E2E_PORT` and `HARBARD_MOCK_PORT` move the pair out of the way.
 */
const PORT = Number(process.env.E2E_PORT ?? 3419);
const MOCK_PORT = Number(process.env.HARBARD_MOCK_PORT ?? 7419);

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: [["list"], ["html", { open: "never", outputFolder: "playwright-report" }]],
  timeout: 30_000,
  expect: { timeout: 8_000 },
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    command: `bunx next start --hostname 127.0.0.1 --port ${PORT}`,
    url: `http://127.0.0.1:${PORT}`,
    env: {
      HARBARD_API: `http://127.0.0.1:${MOCK_PORT}`,
      // The mock is not the local serve: the proxy sends it only an explicit
      // token, never the one on disk (upstream.ts, `localServe`).
      HARBARD_HUB_TOKEN: "e2e-mock-fixture-not-a-secret",
      HARBARD_MOCK_PORT: String(MOCK_PORT),
    },
    // Never reuse: a live `just up` console on this port serves a build that
    // may have been rebuilt underneath it (inconsistent chunks, broken page)
    // and every spec fails with element-not-found. Loud EADDRINUSE beats
    // silent staleness — move the suite to
    // E2E_PORT rather than pointing it at whatever is already listening.
    reuseExistingServer: false,
    timeout: 60_000,
    stdout: "ignore",
    stderr: "pipe",
  },
});
