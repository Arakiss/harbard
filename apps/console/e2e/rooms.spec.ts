import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "./fixtures";

test.beforeEach(async ({ mock }) => {
  await mock.reset("fleet");
});

test("creating a room validates the name and opens the new room", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Create a room" }).click();
  const dialog = page.getByRole("dialog", { name: "Create a room" });
  const name = dialog.getByRole("textbox").first();
  await name.fill("bad!name");
  await expect(dialog.getByRole("alert")).toContainText("Only letters");
  await name.fill("commons");
  await expect(dialog.getByRole("alert")).toContainText("built-in");
  await name.fill("Release Notes");
  await expect(dialog).toContainText("It will be created as #release-notes");
  await dialog.getByPlaceholder("What is this room for?").fill("What goes out next");
  await dialog.getByRole("button", { name: "Create room" }).click();
  await expect(page).toHaveURL(/\/c\/release-notes$/);
  await expect(page.locator(".hb-head")).toContainText("What goes out next");
  await expect(page.locator('[data-channel-row="release-notes"]')).toBeVisible();
});

test("the create dialog works from the keyboard and Esc leaves it", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Create a room" }).focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Create a room" });
  await expect(dialog.getByRole("textbox").first()).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});

test("archiving moves a room to Archived and unarchiving brings it back", async ({ page }) => {
  await page.goto("/c/design-crit");
  await page.getByRole("button", { name: "Actions for #design-crit" }).click();
  await page.getByRole("menuitem", { name: "Archive" }).click();
  await expect(page.locator(".hb-room-strip")).toContainText("#design-crit is archived");
  await expect(page.getByText("This room is archived. Unarchive it to post here.")).toBeVisible();
  await page.getByRole("button", { name: "Archived (2)" }).click();
  await expect(page.locator('[data-channel-row="design-crit"]')).toBeVisible();
  await page.locator(".hb-room-strip").getByRole("button", { name: "Unarchive" }).click();
  await expect(page.locator(".hb-room-strip")).toHaveCount(0);
  await expect(page.locator(".hb-composer textarea")).toBeVisible();
});

test("deleting asks in the app, says the history is kept, and the room stays readable", async ({ page }) => {
  let browserDialog = false;
  page.on("dialog", () => (browserDialog = true));
  await page.goto("/c/parser-spike");
  await page.getByRole("button", { name: "Actions for #parser-spike" }).click();
  await page.getByRole("menuitem", { name: "Delete room" }).click();
  const dialog = page.getByRole("dialog", { name: "Delete #parser-spike?" });
  await expect(dialog.getByTestId("delete-kept")).toContainText("Nothing is erased");
  await expect(dialog.locator("[data-autofocus]")).toBeFocused();
  await dialog.getByRole("button", { name: "Delete room" }).click();
  await expect(page).toHaveURL(/\/history\/rooms\/parser-spike$/);
  await expect(page.getByText("we keep the old parser")).toBeVisible();
  await expect(page.locator(".hb-composer textarea")).toHaveCount(0);
  await page.goto("/history");
  await expect(page.getByRole("link", { name: /#parser-spike/ })).toBeVisible();
  await expect(page.getByRole("link", { name: /#launch-room/ })).toBeVisible();
  expect(browserDialog).toBe(false);
});

test("a built-in room offers no menu and says a post reaches no one's work", async ({ page }) => {
  await page.goto("/c/commons");
  await expect(page.getByRole("button", { name: "Actions for #commons" })).toHaveCount(0);
  await expect(page.locator(".hb-composer-wait")).toContainText("reaches no one's work");
});

test("a project channel can be archived but offers no delete", async ({ page }) => {
  await page.goto("/c/orchard");
  await page.getByRole("button", { name: "Actions for #orchard" }).click();
  await expect(page.getByRole("menuitem", { name: "Archive" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Delete room" })).toHaveCount(0);
});

test("scrolling up reaches the very first message of a long room", async ({ page }) => {
  await page.goto("/c/design-crit");
  const stream = page.locator(".hb-main .hb-stream");
  await expect(page.getByText("Design note 129")).toBeVisible();
  await expect(page.getByText("First line ever said in this room.")).toHaveCount(0);
  await expect(async () => {
    await stream.evaluate((el) => (el.scrollTop = 0));
    await expect(page.getByText("This is the very start of #design-crit.")).toBeVisible({ timeout: 500 });
  }).toPass({ timeout: 15_000 });
  await expect(page.getByText("First line ever said in this room.")).toBeVisible();
});

test("#feedback reads as an inbox with a chip per kind", async ({ page }) => {
  await page.goto("/c/feedback");
  const rows = page.locator(".hb-inbox-row");
  await expect(rows).toHaveCount(6);
  await expect(rows.first().locator(".hb-chip")).toHaveText("Bug");
  await expect(rows.first()).not.toContainText("[bug]");
  await expect(rows.first()).toContainText("harbard 0.41.9");
  await page.getByRole("button", { name: /^Praise/ }).click();
  await expect(rows).toHaveCount(1);
  await expect(rows.first().locator(".hb-chip")).toHaveText("Praise");
});

test("the Lobby shows everyone, introduced or not, with who they are", async ({ page }) => {
  await page.goto("/");
  await page.locator("[data-lobby-link]").click();
  await expect(page).toHaveURL(/\/lobby$/);
  const card = page.locator('[data-lobby-card="a02"]');
  await expect(card).toContainText("ledger");
  await expect(card).toContainText("Codex");
  await expect(card).toContainText("orchard");
  await expect(card).toContainText("Here now");
  await expect(card).toContainText("I make the numbers agree");
  await expect(card.locator(".hb-bot")).toBeVisible();
  const silent = page.locator('[data-lobby-card="a06"]');
  await expect(silent).toContainText("Has not introduced itself yet.");
  await expect(silent).toContainText("Gone quiet");
  await expect(page.locator('[data-lobby-card="outside1"]')).toContainText("sable");
  await expect(page.locator(".hb-lobby-feed .hb-composer-wait")).toContainText("reaches no one's work");
});

test("the Lobby's filter and lineup find an agent", async ({ page }) => {
  await page.goto("/lobby");
  await page.getByLabel("Find an agent").fill("ledger");
  await expect(page.locator("[data-lobby-card]")).toHaveCount(1);
  await page.getByLabel("Find an agent").fill("");
  await page.locator(".hb-lobby-stand", { hasText: "parser" }).click();
  await expect(page.locator('[data-lobby-card="a03"]')).toHaveAttribute("data-lit", "true");
  await expect(page.locator('[data-lobby-card="a03"] a')).toBeFocused();
});

test("under reduced motion the Lobby does not animate", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/lobby");
  const name = await page.locator(".hb-lobby-card").first().evaluate((el) => getComputedStyle(el).animationName);
  expect(name).toBe("none");
});

test("/c/lobby lands on the Lobby", async ({ page }) => {
  await page.goto("/c/lobby");
  await expect(page).toHaveURL(/\/lobby$/);
});

test("the Lobby has no serious accessibility violations", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/lobby");
  await expect(page.locator("[data-lobby-card]").first()).toBeVisible();
  const { violations } = await new AxeBuilder({ page }).include(".hb-lobby").withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => `${v.id}: ${v.nodes.map((n) => n.target).join(" | ")}`)).toEqual([]);
});
