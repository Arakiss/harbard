import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToString } from "react-dom/server";
import type { CalmItem } from "@/lib/calm";
import { CalmCardView } from "./calm-screen";

function text(html: string): string {
  return html.replace(/<!--.*?-->/g, "").replace(/<[^>]+>/g, " ").replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&amp;/g, "&").replace(/\s+/g, " ").trim();
}

const blocked: CalmItem = {
  id: "card:b",
  kind: "blocked",
  who: "Ethan",
  agent: null,
  text: "Should I use the new API or the old one?",
  reason: "Bloqueado",
  tone: "red",
  title: "Should I use the new API or the old one?",
  detail: "",
  own: false,
  quote: null,
};

const base = { agent: null, onReply: () => {}, onLater: () => {} };

describe("the calm card (AC2)", () => {
  test("Reply is a button that stays on the calm screen, not a link to /activity", () => {
    const html = renderToString(h(CalmCardView, { ...base, top: blocked, replying: false, reply: null }));
    expect(html).not.toContain('href="/activity');
    expect(html).toMatch(/<button[^>]*data-testid="calm-open"[^>]*aria-expanded="false"/);
    expect(html).not.toContain('data-testid="calm-reply"');
  });

  test("once pressed, the reply box opens inside the calm card", () => {
    const box = h("textarea", { placeholder: "Responder a Ethan" });
    const html = renderToString(h(CalmCardView, { ...base, top: blocked, replying: true, reply: box }));
    expect(html).toMatch(/data-testid="calm-open"[^>]*aria-expanded="true"/);
    const reply = html.slice(html.indexOf('data-testid="calm-reply"'));
    expect(reply).toContain('placeholder="Responder a Ethan"');
    expect(html.indexOf('data-testid="calm-reply"')).toBeLessThan(html.indexOf("hb-calm-line"));
  });
});

describe("'No reply' in calm (AC5)", () => {
  test("headlines that nobody answered and quotes their message as theirs", () => {
    const own: CalmItem = {
      ...blocked,
      id: "card:u",
      kind: "unanswered",
      reason: "No reply",
      tone: "neutral",
      title: "Nobody answered your message",
      detail: "You wrote to Ethan at 20:41.",
      own: true,
      quote: "How is the export going?",
    };
    const html = renderToString(h(CalmCardView, { ...base, top: own, replying: false, reply: null }));
    const t = text(html);
    expect(html).toMatch(/<p class="hb-calm-question">Nobody answered your message<\/p>/);
    expect(html).toMatch(/<blockquote[^>]*>[\s\S]*How is the export going\?[\s\S]*<\/blockquote>/);
    expect(t).toContain("Your message");
    expect(t).toContain("You wrote to Ethan at 20:41.");
  });
});
