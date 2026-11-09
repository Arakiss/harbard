import { describe, expect, test } from "bun:test";
import {
  bodyWithWorkspace,
  customTile,
  currentName,
  isMulti,
  railOrder,
  readScope,
  tileBadge,
  validWorkspaceId,
  withWorkspace,
  workingElsewhere,
  workspaceFromCookie,
  type WorkspaceTile,
} from "./current-workspace";

const tile = (over: Partial<WorkspaceTile>): WorkspaceTile => ({
  id: "personal",
  name: "Personal",
  tile: "P",
  default: true,
  current: false,
  unread: 0,
  needs: 0,
  working: 0,
  ...over,
});

describe("the rail's order", () => {
  test("the workspace on screen comes first; the others keep the hub's order", () => {
    const ids = railOrder([tile({ id: "a" }), tile({ id: "b" }), tile({ id: "c", current: true })]).map((t) => t.id);
    expect(ids).toEqual(["c", "a", "b"]);
  });
});

describe("a tile's text", () => {
  test("is only shown over the emblem when the operator chose it", () => {
    expect(customTile({ name: "Trabajo", tile: "T" })).toBe(false);
    expect(customTile({ name: "ágora", tile: "Á" })).toBe(false);
    expect(customTile({ name: "Trabajo", tile: "🛠" })).toBe(true);
    expect(customTile({ name: "Trabajo", tile: "OR" })).toBe(true);
  });
});

describe("where the work is", () => {
  test("the busiest other workspace, never the one on screen", () => {
    const here = tile({ id: "personal", current: true, working: 9 });
    const quiet = tile({ id: "quiet" });
    const work = tile({ id: "work", working: 3 });
    const side = tile({ id: "side", working: 1 });
    expect(workingElsewhere([here, side, work, quiet])?.id).toBe("work");
    expect(workingElsewhere([here, quiet])).toBeNull();
    expect(workingElsewhere(undefined)).toBeNull();
  });
});

describe("what a read names", () => {
  test("the strip reads every workspace; the main window its own", () => {
    expect(readScope("/pulse")).toBe("all");
    expect(readScope("/pulse/")).toBe("all");
    expect(readScope("/")).toBeNull();
    expect(readScope("/pulsera")).toBeNull();
    expect(readScope("/c/northwind")).toBeNull();
  });
});

describe("the current workspace", () => {
  test("is read from its cookie and only when it is a real id", () => {
    expect(workspaceFromCookie("a=1; hb-ws=acme; b=2")).toBe("acme");
    expect(workspaceFromCookie("hb-ws=Acme Corp")).toBeNull();
    expect(workspaceFromCookie("hb-ws=..%2Fetc")).toBeNull();
    expect(workspaceFromCookie("")).toBeNull();
    expect(validWorkspaceId("acme-42-platform")).toBe(true);
    expect(validWorkspaceId("-x")).toBe(false);
  });

  test("names every read and every desk write", () => {
    expect(withWorkspace("/workspace", "acme")).toBe("/workspace?ws=acme");
    expect(withWorkspace("/workspace?console=abc", "acme")).toBe("/workspace?console=abc&ws=acme");
    expect(withWorkspace("/workspace", null)).toBe("/workspace");
    expect(bodyWithWorkspace({ text: "hello" }, "acme")).toEqual({ text: "hello", workspace: "acme" });
    expect(bodyWithWorkspace({ text: "hello" }, null)).toEqual({ text: "hello" });
  });
});

describe("the rail's tiles", () => {
  test("carry a count for Needs you and a dot for unread elsewhere", () => {
    expect(tileBadge(tile({ needs: 3 }))).toEqual({ kind: "count", text: "3" });
    expect(tileBadge(tile({ needs: 120 }))).toEqual({ kind: "count", text: "99+" });
    expect(tileBadge(tile({ unread: 4 }))).toEqual({ kind: "dot" });
    // The workspace in front of you shows its unread in the sidebar instead.
    expect(tileBadge(tile({ unread: 4, current: true }))).toBeNull();
    expect(tileBadge(tile({}))).toBeNull();
  });

  test("with one workspace nothing changes but the name", () => {
    expect(isMulti([tile({ current: true })])).toBe(false);
    expect(isMulti(undefined)).toBe(false);
    expect(currentName(undefined)).toBe("harbard");
    expect(currentName([tile({ current: true })])).toBe("Personal");
    expect(isMulti([tile({ current: true }), tile({ id: "acme", name: "Trabajo" })])).toBe(true);
  });
});
