import { describe, expect, test } from "bun:test";
import invalid from "../fixtures/invalid.json";
import valid from "../fixtures/valid.json";
import { COMPONENT_NAMES, LIMITS } from "./catalog";
import { parseView, resolvedPropsProblem, type IssueCode } from "./parse";

const codes = (source: string): IssueCode[] => {
  const parsed = parseView(source);
  return parsed.ok ? [] : parsed.issues.map((issue) => issue.code);
};

describe("a view that respects the catalog", () => {
  for (const { name, view } of valid) {
    test(`is accepted: ${name}`, () => {
      expect(codes(JSON.stringify(view))).toEqual([]);
    });
  }

  test("the fixtures exercise every component in the catalog", () => {
    const used = new Set(valid.flatMap(({ view }) => Object.values(view.elements).map((element) => (element as { type: string }).type)));
    expect(COMPONENT_NAMES.filter((name) => !used.has(name))).toEqual([]);
  });
});

describe("a view that breaks a rule", () => {
  for (const { name, expect: code, view } of invalid) {
    test(`is refused as ${code}: ${name}`, () => {
      expect(codes(JSON.stringify(view))).toContain(code as IssueCode);
    });
  }

  test("text that is not JSON is refused, never thrown", () => {
    expect(codes("{ not json")).toEqual(["invalid_json"]);
  });

  test("a view over the byte limit is refused before it is parsed", () => {
    expect(codes(" ".repeat(LIMITS.bytes + 1))).toEqual(["too_large"]);
  });

  test("a view over the element limit is refused", () => {
    const elements = Object.fromEntries(Array.from({ length: LIMITS.elements + 1 }, (_, i) => [`e${i}`, { type: "Separator", props: {} }]));
    expect(codes(JSON.stringify({ root: "e0", elements }))).toEqual(["too_many_elements"]);
  });

  test("a child called __proto__ does not reach the prototype", () => {
    const source = '{"root":"a","elements":{"a":{"type":"Stack","props":{},"children":["__proto__"]}}}';
    expect(codes(source)).toEqual(["missing_child"]);
  });
});

describe("props after the renderer resolved them", () => {
  test("a link read from state passes the same schema as a literal", () => {
    expect(resolvedPropsProblem("Link", { href: "https://example.com", label: "x" })).toBeNull();
    expect(resolvedPropsProblem("Link", { href: "javascript:alert(1)", label: "x" })).not.toBeNull();
  });

  test("a value that never arrived is a problem, not a crash", () => {
    expect(resolvedPropsProblem("Metric", { label: "x" })).not.toBeNull();
  });

  test("a table read from state still needs one cell per column", () => {
    expect(resolvedPropsProblem("Table", { columns: [{ label: "a" }, { label: "b" }], rows: [[1]] })).not.toBeNull();
  });
});
