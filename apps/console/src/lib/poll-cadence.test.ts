import { expect, test } from "bun:test";
import { HIDDEN_DESKTOP_MS, VISIBLE_ACTIVITY_MS, activityPoll } from "./poll-cadence";

// The Dock's counts (needs and directs) and the logo come from the faces'
// slice (`?only=activity`). A browser tab stops it while hidden; the
// desktop shell, whose window is hidden whenever it is closed, keeps a
// slower pull of the same slice so the badge and the bounce stay live.
test("a visible shell pulls the slice every 2 s, in a browser or the desktop", () => {
  expect(activityPoll(true, false)).toEqual({ fetch: true, delay: VISIBLE_ACTIVITY_MS });
  expect(activityPoll(true, true)).toEqual({ fetch: true, delay: VISIBLE_ACTIVITY_MS });
  expect(VISIBLE_ACTIVITY_MS).toBe(2_000);
});

test("a hidden browser tab stops; the hidden desktop shell keeps a 10 s pull", () => {
  expect(activityPoll(false, false).fetch).toBe(false);
  expect(activityPoll(false, true)).toEqual({ fetch: true, delay: HIDDEN_DESKTOP_MS });
  expect(HIDDEN_DESKTOP_MS).toBe(10_000);
});

test("the hidden desktop poll uses the slice, and the full view stays visible-only", async () => {
  const src = await Bun.file(new URL("../components/workspace/workspace-context.tsx", import.meta.url)).text();
  const slice = src.slice(src.indexOf("fetchActivity(controller.signal)") - 600, src.indexOf("fetchActivity(controller.signal)"));
  expect(slice).toContain("activityPoll(");
  const full = src.slice(src.indexOf("fetchWorkspace(controller.signal)") - 600, src.indexOf("fetchWorkspace(controller.signal)"));
  expect(full).not.toContain("activityPoll(");
  expect(full).toContain('document.visibilityState !== "visible"');
});

// The strip runs its own WorkspaceProvider in the desktop's second
// window. It does not report to the Dock, so it must not add a hidden pull.
test("only the main shell keeps the hidden desktop pull; the strip does not", async () => {
  const layout = await Bun.file(new URL("../app/(workspace)/layout.tsx", import.meta.url)).text();
  const strip = await Bun.file(new URL("../app/(strip)/pulse/layout.tsx", import.meta.url)).text();
  expect(layout).toMatch(/<WorkspaceProvider[^>]*hiddenDesktopPoll/);
  expect(strip).not.toContain("hiddenDesktopPoll");
  const ctx = await Bun.file(new URL("../components/workspace/workspace-context.tsx", import.meta.url)).text();
  expect(ctx).toContain('hiddenDesktopPoll && desktopViewBridge().kind === "desktop"');
});
