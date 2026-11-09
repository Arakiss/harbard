import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { FOLD_CHARS, foldText, type WorkspaceView, type WsMessage } from "@/lib/workspace";
import { MessageItem, MessageText } from "./parts";
import { WorkspaceProvider } from "./workspace-context";

// A 600-character note from the lead, ending in a mark the tests look for.
const LONG = (() => {
  let s = "";
  while (s.length < 590) s += "I will review it calmly and ";
  return `${s}DONE-DEL-TEXTO`;
})();

function message(text: string, body: string | null = null): WsMessage {
  return {
    id: "bus:01LONG",
    agent: "",
    from: "lead",
    author: "Claude · lead",
    ts_ms: 0,
    text,
    body,
    label: null,
    version: null,
    flags: [],
    jargon: [],
    repeat_of: null,
    chars: text.length,
    files: [],
    verified: null,
    channels: ["acme-config"],
    reply_to: null,
    replies: [],
    structured: null,
    undelivered: false,
    attachments: [],
    delivery: null,
    mentions: [],
  } as unknown as WsMessage;
}

const viewWith = (m: WsMessage) =>
  ({ now_ms: 0, agents: [], channels: [], messages: [m], cards: [], saved: [], unread: { directs: 0, activity: 0 } }) as unknown as WorkspaceView;
const render = (m: WsMessage, whole: boolean) => {
  const view = viewWith(m);
  return renderToStaticMarkup(h(WorkspaceProvider, { initial: view }, h(MessageItem, { view, message: m, whole })));
};
/** The visible text, without tags and with entities back. */
const visible = (html: string) => html.replace(/<[^>]+>/g, "").replace(/&#x27;/g, "'").replace(/&quot;/g, '"').replace(/&amp;/g, "&");

describe("a long note is never cut silently", () => {
  test("the fixture is the case the operator saw: 600 characters", () => {
    expect(LONG.length).toBeGreaterThanOrEqual(600);
  });

  test("in the thread pane, all 600 characters show, and nothing folds", () => {
    const out = render(message(LONG), true);
    expect(visible(out)).toContain(LONG);
    expect(out).not.toContain("Show more");
    expect(out).not.toContain("…");
  });

  test("in the thread pane, a technical body shows open", () => {
    const out = render(message("Short END", "technical detail"), true);
    expect(out).toContain('<details class="hb-tech" open="">');
  });

  test("in the channel, it folds behind an explicit 'See more', at a word boundary", () => {
    const out = render(message(LONG), false);
    expect(out).toContain("Show more");
    expect(out).toContain('aria-expanded="false"');
    expect(out).toContain('data-folded="true"');
    expect(visible(out)).not.toContain("DONE-DEL-TEXTO");
    const head = foldText(LONG)!;
    expect(head.length).toBeLessThanOrEqual(FOLD_CHARS);
    expect(LONG.startsWith(head)).toBe(true);
    // The cut falls between words: the next character is a space.
    expect(LONG[head.length]).toBe(" ");
  });

  test("a short note shows whole in the channel, with no 'See more'", () => {
    const out = render(message("Lo reviso ahora."), false);
    expect(visible(out)).toContain("Lo reviso ahora.");
    expect(out).not.toContain("Show more");
  });

  test("many short lines fold too, after the eighth", () => {
    const text = Array.from({ length: 12 }, (_, i) => `line ${i + 1}`).join("\n");
    expect(foldText(text)).toBe(Array.from({ length: 8 }, (_, i) => `line ${i + 1}`).join("\n"));
    expect(foldText("one\ntwo")).toBeNull();
    const out = renderToStaticMarkup(h(MessageText, { text }));
    expect(out).toContain("Show more");
  });

  test("the thread pane and Activity's detail render messages whole", async () => {
    const home = await Bun.file(new URL("./home.tsx", import.meta.url)).text();
    const pane = home.slice(home.indexOf("function ThreadPane"), home.indexOf("function Shell"));
    expect(pane.match(/<MessageItem\b[^>]*\/>/g)?.every((tag) => / whole\b/.test(tag))).toBe(true);
    const activity = await Bun.file(new URL("./activity.tsx", import.meta.url)).text();
    const detail = activity.slice(activity.indexOf('className="hb-detail"'));
    const tags = detail.match(/<MessageItem\b[^>]*\/>/g) ?? [];
    expect(tags.length).toBeGreaterThanOrEqual(2);
    expect(tags.slice(0, 2).every((tag) => / whole\b/.test(tag))).toBe(true);
  });
});
