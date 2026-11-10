import { test, expect } from "./fixtures";

test.describe("language", () => {
  test("the console starts in English, and Preferences switches it to Spanish for good", async ({ page, mock }) => {
    await mock.reset("fleet");
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("lang", "en");

    await page.getByTestId("me-button").click();
    await page.getByText("Preferences", { exact: true }).click();
    // Language lives on the General page; Appearance opens first.
    await page.locator('[data-prefs-page="general"]').click();
    await expect(page.getByTestId("language-en")).toBeChecked();

    await page.getByTestId("language-es").click();
    await expect(page.locator("html")).toHaveAttribute("lang", "es");

    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("lang", "es");
    await page.getByTestId("me-button").click();
    await expect(page.getByText("Preferencias", { exact: true })).toBeVisible();
  });
});
