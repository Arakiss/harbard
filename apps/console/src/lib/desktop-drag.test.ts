import { expect, test } from "bun:test";
import { DRAG_ZONES, NO_DRAG, desktopDragHandler, startsDrag } from "./desktop-drag";

// A fake element: `inside` lists the selectors an ancestor would match.
function el(...inside: string[]) {
  return { closest: (selector: string) => (inside.some((s) => selector.split(",").includes(s)) ? {} : null) };
}

function press(target: unknown, detail = 1, button = 0) {
  let prevented = false;
  return {
    event: { button, detail, target, preventDefault: () => void (prevented = true) },
    prevented: () => prevented,
  };
}

function host(label?: string) {
  const calls: [string, Record<string, unknown> | undefined][] = [];
  const invoke = async (command: string, args?: Record<string, unknown>) => void calls.push([command, args]);
  return { calls, h: { __TAURI_INTERNALS__: { invoke, metadata: label ? { currentWindow: { label } } : undefined } } };
}

test("empty rail and headers drag; controls inside them do not", () => {
  expect(startsDrag(el(".hb-topbar"))).toBe(true);
  expect(startsDrag(el(".hb-rail"))).toBe(true);
  expect(startsDrag(el(".hb-head"))).toBe(true);
  expect(startsDrag(el(".hb-dm-head"))).toBe(true);
  expect(startsDrag(el(".hb-rail", "a"))).toBe(false);
  expect(startsDrag(el(".hb-head", "button"))).toBe(false);
  expect(startsDrag(el(".hb-rail", ".hb-pop"))).toBe(false);
  expect(startsDrag(el(".hb-rail", "[role='dialog']"))).toBe(false);
});

test("content outside the chrome never drags", () => {
  expect(startsDrag(el())).toBe(false);
  expect(startsDrag(null)).toBe(false);
  expect(startsDrag({} as never)).toBe(false);
});

test("the selector lists stay in sync with the shell's class names", () => {
  expect(DRAG_ZONES.split(",")).toContain(".hb-rail");
  expect(NO_DRAG.split(",")).toContain("a");
});

test("a single press on chrome starts dragging the current window", () => {
  const { calls, h } = host("main");
  const p = press(el(".hb-rail"));
  expect(desktopDragHandler(h)(p.event)).toBe("drag");
  expect(p.prevented()).toBe(true);
  expect(calls).toEqual([["plugin:window|start_dragging", { label: "main" }]]);
});

test("a double press zooms the window like a native title bar", () => {
  const { calls, h } = host("main");
  expect(desktopDragHandler(h)(press(el(".hb-head"), 2).event)).toBe("zoom");
  expect(calls).toEqual([["plugin:window|internal_toggle_maximize", { label: "main" }]]);
});

test("clicks on controls, other buttons and browser tabs are left alone", () => {
  const { calls, h } = host("main");
  const handler = desktopDragHandler(h);
  const onLink = press(el(".hb-rail", "a"));
  expect(handler(onLink.event)).toBe("ignored");
  expect(onLink.prevented()).toBe(false);
  expect(handler(press(el(".hb-rail"), 1, 2).event)).toBe("ignored");
  expect(desktopDragHandler({})(press(el(".hb-rail")).event)).toBe("ignored");
  expect(calls).toEqual([]);
});
