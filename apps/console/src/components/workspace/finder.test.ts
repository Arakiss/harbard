import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WorkspaceView } from "@/lib/workspace";
import { FinderDialog, finderHits } from "./finder";
import { PlusMenu, MentionText, RowMenu } from "./parts";
import { AboutHarbard, ShortcutsList, WorkspaceMenu, menuStep, workspaceStatus, type MenuItem } from "./workspace-menu";

const NOW = Date.UTC(2026, 10, 7, 17, 0);
const VIEW = {
  now_ms: NOW,
  agents: [{ key: "a1", name: "robots · builder", handle: "robots-builder", team: "robots", label: "x", description: null, brief: null }],
  channels: [{ id: "harbard", repo: "/w/harbard", members: ["a1"], quiet: false }],
  messages: [{ id: "m1", agent: "a1", from: "agent", author: "robots · builder", ts_ms: NOW, text: "The blue robot is ready", channels: ["harbard"], reply_to: null, structured: null, files: [] }],
} as unknown as WorkspaceView;

describe("⌘K finder (part 4.3)", () => {
  test("finds agents, channels and messages, each with where it goes", () => {
    expect(finderHits(VIEW, "robot").map((x) => [x.kind, x.href])).toEqual([
      ["agent", "/direct/a1"],
      ["message", "/c/harbard?thread=m1"],
    ]);
    expect(finderHits(VIEW, "harb")[0]).toMatchObject({ kind: "channel", title: "#harbard", href: "/c/harbard" });
    expect(finderHits(VIEW, "")).toEqual([]);
  });

  test("renders a combobox with options, and says so when nothing matches", () => {
    const out = renderToStaticMarkup(h(FinderDialog, { view: VIEW, query: "blue", onClose: () => {}, onGo: () => {} }));
    expect(out).toContain('role="dialog"');
    expect(out).toContain('role="combobox"');
    expect(out).toContain('role="option"');
    expect(out).toContain("The blue robot is ready");
    const none = renderToStaticMarkup(h(FinderDialog, { view: VIEW, query: "zzz", onClose: () => {}, onGo: () => {} }));
    expect(none).toContain("Nothing matches “zzz”.");
  });
});

describe("the logo's workspace menu (part 5.3)", () => {
  const items: MenuItem[] = ["Preferences", "Themes", "Keyboard shortcuts", "Mark all as read", "About harbard", "Open Strip"].map((label, i) => ({
    id: String(i),
    label,
    icon: () => null,
    run: () => {},
  }));

  test("a real menu with every item, its name and state", () => {
    const out = renderToStaticMarkup(h(WorkspaceMenu, { status: workspaceStatus("connected", 4), items, onClose: () => {} }));
    expect(out).toContain('role="menu"');
    expect(out.match(/role="menuitem"/g)?.length).toBe(6);
    for (const i of items) expect(out).toContain(i.label);
    expect(out).toContain("serve is responding · 4 active agents");
    expect(workspaceStatus("disconnected", 1)).toBe("serve isn't responding · 1 active agent");
  });

  test("arrows, Home and End move through it, wrapping", () => {
    expect(menuStep(0, 6, "ArrowDown")).toBe(1);
    expect(menuStep(5, 6, "ArrowDown")).toBe(0);
    expect(menuStep(0, 6, "ArrowUp")).toBe(5);
    expect(menuStep(3, 6, "Home")).toBe(0);
    expect(menuStep(0, 6, "End")).toBe(5);
  });

  test("shortcuts and about say what they are", () => {
    const keys = renderToStaticMarkup(h(ShortcutsList));
    for (const k of ["⌘K", "Esc", "⇧Esc", "@"]) expect(keys).toContain(k);
    const about = renderToStaticMarkup(h(AboutHarbard, { version: "0.3.0-alpha.9", commit: "abc1234", protocol: "harbard/protocol/v1" }));
    expect(about).toContain("0.3.0-alpha.9");
    expect(about).toContain("abc1234");
  });
});

describe("the composer's + menu, mentions and row menus (parts 4.1, 5.2, 7)", () => {
  test("every item is there only when it does something", () => {
    const full = renderToStaticMarkup(
      h(PlusMenu, { onAttach: () => {}, onMention: () => {}, agents: [{ key: "a1", name: "robots · reviewer" }], onReview: () => {}, onNewThread: () => {}, onClose: () => {} }),
    );
    for (const label of ["Attach file", "Mention", "Ask for a review from…", "New thread"]) expect(full).toContain(label);
    const bare = renderToStaticMarkup(h(PlusMenu, { onAttach: () => {}, onMention: () => {}, agents: [], onReview: () => {}, onClose: () => {} }));
    expect(bare).not.toContain("New thread");
    expect(bare).not.toContain("Ask for a review");
  });

  test("a recognised mention is highlighted in the message", () => {
    const out = renderToStaticMarkup(h(MentionText, { text: "@dana blue or green?", mentions: ["dana"] }));
    expect(out).toBe('<span class="hb-mention">@dana</span> blue or green?');
  });

  test("a row offers Mark as read from its menu", () => {
    const out = renderToStaticMarkup(h(RowMenu, { label: "#harbard", onRead: () => {} }, h("a", null, "#harbard")));
    expect(out).toContain('aria-label="More actions for #harbard"');
    expect(out).toContain('aria-haspopup="menu"');
  });
});
