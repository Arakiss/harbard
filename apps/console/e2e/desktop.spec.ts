import { test, expect } from "./fixtures";

declare global {
  interface Window {
    __HARBARD_PENDING_DESTINATION__?: unknown;
    __HARBARD_PENDING_SUGGESTION__?: unknown;
  }
}

/**
 * The desktop bridge on the current shell: the app leaves a
 * destination pending while the page loads, or dispatches `harbard:navigate`;
 * the shell goes to Direct or Activity, and refuses anything else without
 * leaving the global behind.
 */
test.describe("desktop — the main window follows the app's destinations", () => {
  test("a pending Direct destination is consumed on load", async ({ page, mock }) => {
    await mock.reset("fleet");
    await page.addInitScript(() => {
      window.__HARBARD_PENDING_DESTINATION__ = { view: "direct", session: "a08" };
    });
    await page.goto("/?shell=desktop");
    await expect(page).toHaveURL(/\/direct\/a08$/);
    expect(await page.evaluate(() => "__HARBARD_PENDING_DESTINATION__" in window)).toBe(false);
  });

  test("harbard:navigate opens an Activity item with its selection", async ({ page, mock }) => {
    await mock.reset("fleet");
    await page.goto("/?shell=desktop");
    await expect(page.locator(".hb-rail")).toBeVisible();
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("harbard:navigate", { detail: { view: "activity", note: "card:c-blocked" } })));
    await expect(page).toHaveURL(/\/activity\?sel=card%3Ac-blocked$/);
  });

  test("a destination of the removed console is refused and cleared", async ({ page, mock }) => {
    await mock.reset("fleet");
    await page.addInitScript(() => {
      window.__HARBARD_PENDING_DESTINATION__ = { view: "inbox", note: "n1" };
    });
    await page.goto("/?shell=desktop");
    await expect(page.locator(".hb-rail")).toBeVisible();
    await page.waitForTimeout(300);
    expect(new URL(page.url()).pathname).toBe("/");
    expect(await page.evaluate(() => "__HARBARD_PENDING_DESTINATION__" in window)).toBe(false);
  });

  test("the away suggestion shows as a card, pending or by event, and never switches by itself", async ({ page, mock }) => {
    await mock.reset("fleet");
    await page.addInitScript(() => {
      window.__HARBARD_PENDING_SUGGESTION__ = { state: "away" };
    });
    await page.goto("/?shell=desktop");
    const card = page.getByTestId("suggest-card");
    await expect(card).toContainText("Set your status to Away?");
    expect(await page.evaluate(() => "__HARBARD_PENDING_SUGGESTION__" in window)).toBe(false);
    await card.getByRole("button", { name: "Not now" }).click();
    await expect(card).toHaveCount(0);
    // Available is offered only to someone in Away: the mock's operator is Available.
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("harbard:suggest", { detail: { state: "available" } })));
    await page.waitForTimeout(200);
    await expect(card).toHaveCount(0);
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("harbard:suggest", { detail: { state: "away" } })));
    await expect(card).toBeVisible();
  });
});
