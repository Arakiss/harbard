import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { COMPONENT_NAMES, composeAnswer, parseView, type ViewSpec } from "@harbard/huginn";
import valid from "../../../../../packages/huginn/fixtures/valid.json";
import type { WorkspaceView, WsAttachment, WsMessage } from "@/lib/workspace";
import { HuginnView, answersOf } from "./view";

const html = (source: string, files: WsAttachment[] = [], message: WsMessage | null = null) =>
  renderToStaticMarkup(h(HuginnView, { file: "v.huginn", source, message, files }));

const textView = JSON.stringify({ root: "a", elements: { a: { type: "Text", props: { text: "<img src=x onerror=alert(1)>" } } } });
const image = (name: string): WsAttachment => ({ id: `01K9Z8Y7X6W5V4T3S2R1Q0P9N8/${name}`, name, kind: "image", label: null, bytes: 10, path: `/h/${name}` });

function message(overrides: Partial<WsMessage>): WsMessage {
  return {
    id: "m1", agent: "a1", from: "agent", author: "Nora", ts_ms: 1000, text: "Mira", label: null, version: null, flags: [],
    repeat_of: null, chars: 4, files: [], verified: null, channels: [], reply_to: null, replies: [], structured: null, undelivered: false,
    ...overrides,
  };
}

const viewWith = (messages: WsMessage[]) => ({ messages }) as unknown as WorkspaceView;

describe("rendering a view", () => {
  test("every fixture renders with every component, and none falls back to an error", () => {
    const seen = new Set<string>();
    for (const fixture of valid) {
      for (const element of Object.values(fixture.view.elements)) seen.add((element as { type: string }).type);
      const out = html(JSON.stringify(fixture.view), [image("captura.png"), image("antes.png"), image("despues.png")]);
      expect(out).toContain('class="hb-huginn"');
      expect(out).not.toContain("Missing attachment");
    }
    expect(COMPONENT_NAMES.filter((name) => !seen.has(name))).toEqual([]);
  });

  test("markup in a string is shown as text, never parsed", () => {
    const out = html(textView);
    expect(out).toContain("&lt;img src=x onerror=alert(1)&gt;");
    expect(out).not.toContain("<img");
  });

  test("a raw script in Markdown stays inert", () => {
    const out = html(JSON.stringify({ root: "a", elements: { a: { type: "Markdown", props: { source: "<script>alert(1)</script>" } } } }));
    expect(out).not.toContain("<script>");
  });

  test("a view that breaks the catalog says so instead of rendering", () => {
    const out = html(JSON.stringify({ root: "a", elements: { a: { type: "Script", props: {} } } }));
    expect(out).toContain("the component catalog");
    expect(out).toContain("unknown_component");
    expect(out).not.toContain('class="hb-huginn"');
  });

  test("an image not attached to the message says which one is missing", () => {
    const out = html(JSON.stringify({ root: "a", elements: { a: { type: "Image", props: { file: "x.png", alt: "x" } } } }));
    expect(out).toContain("Missing attachment x.png");
  });

  test("a link opens outside the console without an opener", () => {
    const out = html(JSON.stringify({ root: "a", elements: { a: { type: "Link", props: { href: "https://example.com", label: "x" } } } }));
    expect(out).toContain('rel="noopener noreferrer nofollow"');
  });
});

const choice = JSON.stringify({
  root: "c",
  elements: { c: { type: "Choice", props: { id: "plan", question: "Continue?", options: [{ label: "Yes" }, { label: "No" }] } } },
});

describe("answering a view", () => {
  test("outside a message the options are disabled and say why", () => {
    const out = html(choice);
    expect(out).toContain("be answered from here");
    expect(out).toContain("disabled");
  });

  test("a view the operator sent cannot be answered by the operator", () => {
    expect(html(choice, [], message({ from: "operator" }))).toContain("be answered from here");
  });

  const answer = composeAnswer({ file: "v.huginn", id: "plan", lines: [{ field: "plan", value: "option 1" }] });
  const parsedChoice = parseView(choice);
  if (!parsedChoice.ok) throw new Error("the fixture must parse");
  const spec: ViewSpec = parsedChoice.spec;

  test("an operator reply to the message counts as the answer, addressed or not", () => {
    const parent = message({ replies: ["r1"] });
    const reply = message({ id: "r1", from: "operator", text: `Para Nora: ${answer}`, ts_ms: 2000 });
    expect(answersOf("v.huginn", spec, parent, viewWith([parent, reply])).get("plan")).toEqual({ lines: [{ label: "Continue?", value: "Yes" }], tsMs: 2000 });
  });

  test("the same text from an agent is not an answer", () => {
    const parent = message({ replies: ["r1"] });
    const forged = message({ id: "r1", from: "agent", text: answer });
    expect(answersOf("v.huginn", spec, parent, viewWith([parent, forged])).size).toBe(0);
  });

  test("an answer to another view in the message does not count", () => {
    const parent = message({ replies: ["r1"] });
    const reply = message({ id: "r1", from: "operator", text: answer });
    expect(answersOf("otra.huginn", spec, parent, viewWith([parent, reply])).size).toBe(0);
  });
});

describe("a view with state", () => {
  const dynamic = (elements: object, state: object) => html(JSON.stringify({ root: "s", state, elements }));

  test("a repeat draws one child per item and reads each item", () => {
    const out = dynamic(
      { s: { type: "Stack", props: {}, children: ["t"], repeat: { statePath: "/rows", key: "name" } }, t: { type: "Text", props: { text: { $item: "name" } } } },
      { rows: [{ name: "api" }, { name: "web" }] },
    );
    expect(out).toContain("api");
    expect(out).toContain("web");
  });

  test("a condition hides what it does not match", () => {
    const out = dynamic(
      { s: { type: "Stack", props: {}, children: ["a", "b"] }, a: { type: "Text", props: { text: "visible" }, visible: { $state: "/x", eq: 1 } }, b: { type: "Text", props: { text: "hidden" }, visible: { $state: "/x", eq: 2 } } },
      { x: 1 },
    );
    expect(out).toContain("visible");
    expect(out).not.toContain("hidden");
  });

  test("a javascript link read from state never reaches an href", () => {
    const out = dynamic({ s: { type: "Link", props: { href: { $state: "/u" }, label: "x" } } }, { u: "javascript:alert(1)" });
    expect(out).not.toContain("javascript:");
    expect(out).toContain("be shown");
  });

  test("a value missing from state says so instead of crashing", () => {
    expect(dynamic({ s: { type: "Metric", props: { label: "x", value: { $state: "/nada" } } } }, {})).toContain("be shown");
  });
});

describe("the view stylesheet", () => {
  const css = readFileSync(join(import.meta.dir, "huginn.css"), "utf8");

  test("holds no colour literal, so a theme can recolour all of it", () => {
    expect(css.match(/#[0-9a-fA-F]{3,8}\b|\brgba?\(/g)).toBeNull();
  });

  // The CSS compiler rewrites light-dark() against the document's fixed dark class,
  // which painted light themes with the dark palette.
  test("never picks a colour by light-dark()", () => {
    expect(css.includes("light-dark(")).toBe(false);
  });
});
