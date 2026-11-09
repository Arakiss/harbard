import { expect, test } from "bun:test";
import {
  desktopViewBridge,
  desktopViewHref,
  openDesktopView,
  shellRoute,
  updateDestination,
  validMainDestination,
} from "./desktop-view";

test("browser links keep the two explicit web destinations", async () => {
  expect(desktopViewHref("overview")).toBe("/?shell=desktop");
  expect(desktopViewHref("strip")).toBe("/pulse?shell=desktop");
  expect(desktopViewBridge({ navigator: { userAgent: "Browser" } })).toEqual({
    kind: "browser",
  });
  expect(await openDesktopView("overview", { kind: "browser" })).toBe(
    "browser",
  );
});

test("only Direct and Activity are destinations: the previous console's views are gone", () => {
  for (const view of ["overview", "inbox", "messages", "timeline", "history", "dependencies"]) {
    expect(validMainDestination({ view })).toBe(false);
  }
  expect(validMainDestination({ view: "direct", session: "a1" })).toBe(true);
  expect(validMainDestination({ view: "activity", note: "card:1" })).toBe(true);
});

test("main destinations preserve exact identifiers in the route and the native payload", async () => {
  const destination = { view: "activity" as const, note: "card:x /?&+\"" };
  const url = new URL(desktopViewHref("overview", destination), "https://example.test");
  expect(url.pathname).toBe("/activity");
  expect(url.searchParams.get("sel")).toBe(destination.note);
  const calls: unknown[] = [];
  await openDesktopView("overview", { kind: "desktop", invoke: async (...args) => { calls.push(args); } }, destination);
  expect(calls).toEqual([["desk_open_main", { destination }]]);
});

test("invalid destinations cannot reach native commands", async () => {
  expect(validMainDestination({ view: "activity", note: "a\ncommand" })).toBe(false);
  expect(validMainDestination({ view: "activity", note: "" })).toBe(false);
  expect(validMainDestination({ view: "activity", extra: "reload" })).toBe(false);
  expect(validMainDestination({ view: "unknown" })).toBe(false);
  expect(validMainDestination({ view: { toString: () => "activity" } })).toBe(false);
  expect(validMainDestination({ view: "direct", dependency: "d1" })).toBe(false);
  let calls = 0;
  await expect(openDesktopView("overview", { kind: "desktop", invoke: async () => { calls++; } },
    { view: "activity", note: "x".repeat(513) })).rejects.toThrow("Invalid main");
  expect(calls).toBe(0);
});

test("native views invoke only their fixed command once", async () => {
  const calls: string[] = [];
  const bridge = desktopViewBridge({
    __TAURI__: {
      core: {
        invoke: async (command) => {
          calls.push(command);
        },
      },
    },
  });
  expect(await openDesktopView("overview", bridge)).toBe("opened");
  expect(await openDesktopView("strip", bridge)).toBe("opened");
  expect(calls).toEqual(["desk_open_main", "desk_open_strip"]);
});

test("the internal Tauri bridge follows the same routing contract", async () => {
  const calls: string[] = [];
  const bridge = desktopViewBridge({
    __TAURI_INTERNALS__: {
      invoke: async (command) => {
        calls.push(command);
      },
    },
  });
  await openDesktopView("strip", bridge);
  expect(calls).toEqual(["desk_open_strip"]);
});

test("an unavailable native bridge does not fall back to navigating the current window", async () => {
  const bridge = desktopViewBridge({ __TAURI__: {} });
  expect(bridge.kind).toBe("desktop");
  await expect(openDesktopView("overview", bridge)).rejects.toThrow(
    "Window controls are unavailable",
  );
});

test("a rejected or partially completed native command is never retried or replaced with navigation", async () => {
  let calls = 0;
  await expect(
    openDesktopView("overview", {
      kind: "desktop",
      invoke: async () => {
        calls += 1;
        throw new Error("command not available");
      },
    }),
  ).rejects.toThrow("Use the View menu");
  expect(calls).toBe(1);
});

test("Direct and Activity are the workspace shell's own routes, and their payload is unchanged", async () => {
  expect(desktopViewHref("overview", { view: "direct", session: "a/1 b" })).toBe("/direct/a%2F1%20b");
  expect(desktopViewHref("overview", { view: "activity", note: "card:x&y" })).toBe("/activity?sel=card%3Ax%26y");
  expect(shellRoute({ view: "direct" })).toBe("/direct");
  expect(validMainDestination({ view: "activity", agent: "a1" })).toBe(false);
  const calls: unknown[] = [];
  const destination = { view: "direct" as const, session: "a1" };
  await openDesktopView("overview", { kind: "desktop", invoke: async (...args) => { calls.push(args); } }, destination);
  expect(calls).toEqual([["desk_open_main", { destination }]]);
});

test("'actualizar' opens the update dialog: exactly that view, and never a native route", () => {
  expect(updateDestination({ view: "update" })).toBe(true);
  for (const value of [null, undefined, "update", [], {}, { view: "Actualizar" }, { view: "update", note: "x" }, { view: "update", session: "s" }, { view: "direct" }]) {
    expect(updateDestination(value)).toBe(false);
  }
  // It is not a MainDestination: no native command or route ever carries it.
  expect(validMainDestination({ view: "update" })).toBe(false);
});
