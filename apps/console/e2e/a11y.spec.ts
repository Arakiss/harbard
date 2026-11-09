import AxeBuilder from "@axe-core/playwright";
import { test, expect } from "./fixtures";

/**
 * B8 on the current shell: no serious or critical axe
 * violations on Home, Direct and Activity, the rail and the avatar
 * menu from the keyboard, and nothing moving under reduced motion.
 */
const ROUTES = ["/", "/direct/a08", "/activity"];

test.describe("B8 — the shell is operable and accessible", () => {
  for (const route of ROUTES) {
    test(`no serious or critical axe violations on ${route}`, async ({ page, mock }) => {
      await mock.reset("fleet");
      await page.goto(route);
      await expect(page.locator(".hb-rail")).toBeVisible();
      const result = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"]).analyze();
      const serious = result.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
      expect(serious, JSON.stringify(serious.map((v) => ({ id: v.id, nodes: v.nodes.map((n) => n.target) })), null, 2)).toEqual([]);
    });
  }

  test("the rail and the avatar menu work from the keyboard", async ({ page, mock }) => {
    await mock.reset("fleet");
    await page.goto("/");
    await expect(page.locator(".hb-rail")).toBeVisible();
    let onDirect = false;
    for (let i = 0; i < 20 && !onDirect; i++) {
      await page.keyboard.press("Tab");
      onDirect = await page.evaluate(() => document.activeElement?.getAttribute("data-dest") === "direct");
    }
    expect(onDirect).toBe(true);
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/direct/);
    await page.getByTestId("me-button").focus();
    await page.keyboard.press("Enter");
    const menu = page.getByRole("dialog", { name: /^Menu for/ });
    await expect(menu).toBeVisible();
    await page.keyboard.press("End");
    await expect(page.locator(":focus")).toHaveText("Preferences");
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
    await expect(page.getByTestId("me-button")).toBeFocused();
  });

  test("nothing moves under prefers-reduced-motion", async ({ page, mock }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await mock.reset("fleet");
    await page.goto("/");
    await expect(page.locator(".hb-rail")).toBeVisible();
    expect(await page.evaluate(() => matchMedia("(prefers-reduced-motion: reduce)").matches)).toBe(true);
    await page.waitForTimeout(500);
    const running = await page.evaluate(() =>
      document.getAnimations().filter((a) => a.playState === "running" && (a.effect as KeyframeEffect | null)?.getTiming().iterations === Infinity).length,
    );
    expect(running).toBe(0);
  });
});
