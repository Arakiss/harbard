import { test, expect } from "./fixtures";

declare global {
  interface Window {
    __harbard_xss?: string;
  }
}

/**
 * B4 on the current shell: what an agent controls (its name, its message, a
 * branch, a file path) renders as inert text in Home, Direct and
 * Activity. Ported from the previous console's spec.
 */
test.describe("B4 — agent text renders inert in the shell", () => {
  for (const route of ["/", "/direct/a08", "/activity"]) {
    test(`hostile strings stay text on ${route}`, async ({ page, mock }) => {
      const dialogs: string[] = [];
      page.on("dialog", (d) => {
        dialogs.push(d.message());
        void d.dismiss();
      });
      await mock.reset("xss");
      await page.goto(route);
      await expect(page.locator(".hb")).toBeVisible();
      if (route !== "/") {
        await expect(page.locator("body")).toContainText("<script>window.__harbard_xss='message'</script>");
      }
      await expect(page.locator("body")).toContainText(`<img src=x onerror="window.__harbard_xss='name'">`);
      // Nothing executed and no element was created from the strings.
      expect(await page.evaluate(() => window.__harbard_xss)).toBeUndefined();
      expect(await page.locator("img[src='x'], img[onerror], svg[onload]").count()).toBe(0);
      const scripts = await page.$$eval("script", (els) => els.filter((e) => (e.textContent ?? "").includes("__harbard_xss='message'") && !e.closest("[type='application/json']")).map((e) => e.outerHTML.slice(0, 80)));
      // The RSC payload carries the string as escaped data; no script element runs it.
      expect(scripts.filter((s) => !s.includes("self.__next_f"))).toEqual([]);
      expect(dialogs).toEqual([]);
    });
  }
});
