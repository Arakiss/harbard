import { describe, expect, test } from "bun:test";
import { EMBLEM_PALETTES, emblemOf, palettesFor, seedOf } from "./workspace-emblem";

describe("a workspace's emblem", () => {
  test("is the same every time for the same id", () => {
    expect(emblemOf("acme-42-platform")).toEqual(emblemOf("acme-42-platform"));
    expect(seedOf("personal")).toBe(seedOf("personal"));
  });

  test("differs between workspaces", () => {
    const drawing = (id: string) => JSON.stringify(emblemOf(id, 0).shapes);
    expect(drawing("personal")).not.toBe(drawing("acme-42-platform"));
  });

  test("is four big shapes and nothing finer", () => {
    expect(emblemOf("personal").shapes.length).toBe(4);
  });

  test("uses both colours, so the shapes always stand apart", () => {
    for (const id of ["personal", "acme-42-platform", "acme", "x", "lab"]) {
      expect(new Set(emblemOf(id).shapes.map((s) => s.color)).size).toBe(2);
    }
  });

  test("changing palette repaints the same drawing", () => {
    const paths = (p: number) => emblemOf("personal", p).shapes.map((s) => s.d);
    expect(paths(0)).toEqual(paths(3));
  });
});

describe("palettes across a hub", () => {
  test("two workspaces never share one while there are palettes to go round", () => {
    const ids = EMBLEM_PALETTES.map((_, i) => `workspace-${i}`);
    expect(new Set(palettesFor(ids).values()).size).toBe(EMBLEM_PALETTES.length);
  });

  test("the operator's two real workspaces come out different", () => {
    const p = palettesFor(["personal", "acme-42-platform"]);
    expect(p.get("personal")).not.toBe(p.get("acme-42-platform"));
  });

  test("more workspaces than palettes still get one each", () => {
    const ids = Array.from({ length: EMBLEM_PALETTES.length + 3 }, (_, i) => `w${i}`);
    const p = palettesFor(ids);
    expect(p.size).toBe(ids.length);
    for (const at of p.values()) expect(EMBLEM_PALETTES[at]).toBeDefined();
  });

  test("a workspace added later leaves the earlier ones alone", () => {
    const before = palettesFor(["personal", "work"]);
    const after = palettesFor(["personal", "work", "new"]);
    expect(after.get("personal")).toBe(before.get("personal"));
    expect(after.get("work")).toBe(before.get("work"));
  });
});
