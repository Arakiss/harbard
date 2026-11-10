import { MOCK_BASE, expect, test } from "./fixtures";
import type { Page } from "@playwright/test";

/** The mock's theme: a chosen theme, a mode, the season choice and the operator's local date. */
async function themeAt(params: Record<string, string>): Promise<void> {
  const res = await fetch(`${MOCK_BASE}/__control/theme?${new URLSearchParams(params)}`, { method: "POST" });
  if (!res.ok) throw new Error(`theme control -> ${res.status}`);
}

async function openPreferences(page: Page): Promise<void> {
  await page.getByTestId("me-button").click();
  await page.getByText("Preferences", { exact: true }).click();
}

test.describe("the Christmas season", () => {
  test.beforeEach(async ({ mock }) => {
    await mock.reset("fleet");
  });
  // The mock is shared by every spec of the worker: no theme left behind.
  test.afterEach(async () => {
    await fetch(`${MOCK_BASE}/__control/theme`, { method: "POST" });
  });

  test("in December it paints Christmas over the chosen theme, and Preferences turns it off, on and undecorated", async ({ page }) => {
    await themeAt({ mode: "light", theme: "harbour", season: "auto", decorations: "on", today: "2026-12-24" });
    await page.goto("/lobby");
    const root = page.locator(".hb").first();
    await expect(root).toHaveAttribute("data-hb-season", "christmas");
    await expect(root).toHaveAttribute("data-hb-theme", "christmas");
    await expect(page.getByTestId("season-decor")).toBeAttached();
    await expect(page.getByTestId("season-sleigh")).toBeAttached();
    await expect(page.locator('.hb-bot[data-season="christmas"]').first()).toBeAttached();

    await openPreferences(page);
    const section = page.getByTestId("season-section");
    await expect(section.getByLabel("Automatic", { exact: true })).toBeChecked();
    await expect(section).toContainText("from December 1 to January 6");

    await section.getByLabel("Off", { exact: true }).click();
    await expect(root).not.toHaveAttribute("data-hb-season", /.+/);
    await expect(root).toHaveAttribute("data-hb-theme", "harbour");
    await expect(page.getByTestId("season-decor")).toHaveCount(0);
    await expect(page.locator('.hb-bot[data-season="christmas"]')).toHaveCount(0);
    // The chosen theme in the picker never moved.
    await expect(page.locator('[data-theme-card="harbour"]')).toHaveAttribute("aria-checked", "true");
    await expect(page.locator('[data-theme-card="christmas"]')).toHaveCount(0);

    await section.getByLabel("On", { exact: true }).click();
    await expect(root).toHaveAttribute("data-hb-season", "christmas");
    await page.getByTestId("season-decorations").click();
    await expect(root).not.toHaveAttribute("data-hb-decor", /.*/);
    await expect(page.getByTestId("season-decor")).toHaveCount(0);
    await expect(root).toHaveAttribute("data-hb-theme", "christmas");
  });

  test("outside its dates Automatic paints nothing, and On paints it anyway", async ({ page }) => {
    await themeAt({ mode: "dark", theme: "sage", season: "auto", today: "2026-11-10" });
    await page.goto("/lobby");
    const root = page.locator(".hb").first();
    await expect(root).toHaveAttribute("data-hb-theme", "sage");
    await expect(root).not.toHaveAttribute("data-hb-season", /.+/);

    await themeAt({ mode: "dark", season: "on" });
    await page.reload();
    await expect(root).toHaveAttribute("data-hb-season", "christmas");
    await expect(root).toHaveAttribute("data-hb-theme", "christmas");
  });

  test("with reduced motion only the lights stay, standing still", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await themeAt({ mode: "light", theme: "ink", season: "auto", today: "2026-12-24" });
    await page.goto("/lobby");
    await expect(page.locator(".hb").first()).toHaveAttribute("data-hb-decor", "");
    const bulb = page.locator(".hb-festive-bulb").first();
    await expect(bulb).toBeVisible();
    expect(await bulb.evaluate((el) => getComputedStyle(el).animationName)).toBe("none");
    await expect(page.locator(".hb-festive-snow").first()).toBeHidden();
    await expect(page.locator(".hb-festive-sky")).toBeHidden();
    await expect(page.locator(".hb-festive-snow").first()).toBeHidden();
    await expect(page.locator(".hb-festive-bulb").first()).toBeVisible();
  });

  test("without reduced motion the snow falls and nothing takes the pointer", async ({ page }) => {
    await themeAt({ mode: "light", theme: "ink", season: "auto", today: "2026-12-24" });
    await page.goto("/lobby");
    const flake = page.locator(".hb-festive-snow-layer").first();
    expect(await flake.evaluate((el) => getComputedStyle(el).animationName)).toBe("hb-festive-fall");
    for (const sel of [".hb-festive-snow", ".hb-festive-lights", ".hb-festive-sky"]) {
      expect(await page.locator(sel).first().evaluate((el) => getComputedStyle(el).pointerEvents)).toBe("none");
    }
  });

  test("the decorations settle after two minutes untouched or without focus, and the next input wakes them", async ({ page }) => {
    await page.clock.install();
    await themeAt({ mode: "light", theme: "ink", season: "auto", today: "2026-12-24" });
    await page.goto("/lobby");
    const root = page.locator(".hb").first();
    await expect(root).toHaveAttribute("data-hb-decor", "");
    await expect(root).not.toHaveAttribute("data-hb-decor-still", /.*/);

    await page.clock.fastForward(121_000);
    await expect(root).toHaveAttribute("data-hb-decor-still", "");
    await expect(page.locator(".hb-festive-snow").first()).toBeHidden();
    await expect(page.locator(".hb-festive-bulb").first()).toBeVisible();
    expect(await page.locator(".hb-festive-lights").first().evaluate((el) => getComputedStyle(el).animationName)).toBe("none");
    await expect(page.locator(".hb-festive-sky")).toBeHidden();

    await page.mouse.move(400, 300);
    await page.mouse.move(420, 320);
    await expect(root).not.toHaveAttribute("data-hb-decor-still", /.*/);

    await page.evaluate(() => {
      Object.defineProperty(document, "hasFocus", { value: () => false, configurable: true });
      window.dispatchEvent(new Event("blur"));
    });
    await expect(root).toHaveAttribute("data-hb-decor-still", "");
  });
});
