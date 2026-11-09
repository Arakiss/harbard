import { test, expect } from "./fixtures";

/**
 * First-run Setup: brand-first welcome, detect, explain harnesses, wire.
 * "Set up harbard" (`/setup`) always shows it.
 *
 * The screenshots here are evidence, so they wait for the entry animations
 * first. Every setup block carries `animation: … both`, which holds the
 * element at its `from` keyframe — opacity 0 — until the animation runs, and
 * Playwright's visibility check ignores opacity. Without the settle the assert
 * passes and the capture is a black rectangle with a header on it.
 */
const settle = (page: import("@playwright/test").Page) =>
  page.waitForFunction(() =>
    document
      .getAnimations()
      // The rail's living logo loops forever; the wizard's entries do not.
      .filter((a) => (a.effect as KeyframeEffect | null)?.getTiming().iterations !== Infinity)
      .every((a) => a.playState === "finished" || a.playState === "idle"),
  );
test.describe("setup — first-run harness wiring", () => {
  test("welcome → scan → choose with harness blurbs", async ({ page, mock }) => {
    await mock.reset("fleet");
    await page.goto("/setup");
    await expect(page.getByRole("heading", { name: "harbard", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Search this Mac" })).toBeVisible();
    await settle(page);
    await page.screenshot({ path: "e2e/screenshots/setup-welcome.png" });

    await page.getByRole("button", { name: "Search this Mac" }).click();

    await expect(
      page.getByRole("heading", { name: /on this Mac|None yet/i }),
    ).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole("checkbox", { name: /Claude Code/ })).toBeVisible();
    await expect(page.getByRole("checkbox", { name: /Codex/ })).toBeVisible();
    await expect(page.getByRole("checkbox", { name: /Cursor/ })).toBeVisible();
    await expect(
      page.getByText(/so Claude receives the notes on the bus/i),
    ).toBeVisible();
    await settle(page);
    await page.screenshot({ path: "e2e/screenshots/setup-choose.png" });
  });
});
