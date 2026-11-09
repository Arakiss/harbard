import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { nameOrigin, type WorkspaceView } from "@/lib/workspace";
import { AgentName, NameVotes } from "./parts";
import { WorkspaceProvider } from "./workspace-context";

// Workshop names: synthetic agents only.
const VIEW = { now_ms: 0, agents: [], channels: [], messages: [], cards: [], unread: { directs: 0, activity: 0 } } as unknown as WorkspaceView;
const inShell = (el: ReturnType<typeof h>) => renderToStaticMarkup(h(WorkspaceProvider, { initial: VIEW }, el));

describe("workshop names", () => {
  test("the name, then its agreeing role and team", () => {
    const out = inShell(h(AgentName, { agent: { key: "a1", name: "Nora", role_line: "revisora · protocolo", name_source: "auto" }, className: "hb-name", editable: true }));
    expect(out).toContain('<span class="hb-name" title="Double-click to rename">Nora</span>');
    expect(out).toContain('<span class="hb-role-line">revisora · protocolo</span>');
  });

  test("only a workshop name can be renamed or voted; the coordinator cannot", () => {
    const lead = { key: "session:x", name: "Claude · lead", role_line: null, name_source: null };
    expect(inShell(h(AgentName, { agent: lead, editable: true }))).not.toContain("Double-click");
    expect(inShell(h(NameVotes, { agent: lead }))).toBe("");
    const votes = inShell(h(NameVotes, { agent: { key: "a1", name: "Nora", name_source: "auto" } }));
    expect(votes).toContain("👍");
    expect(votes).toContain("👎");
    expect(votes).toContain('aria-label="Do you like the name Nora?"');
  });

  test("Profile says where the name came from, with the session's reason", () => {
    expect(nameOrigin({ name_source: "picked", name_reason: "I guide the team" })).toBe("The session picked it: “I guide the team”");
    expect(nameOrigin({ name_source: "renamed", name_reason: null })).toBe("You gave it this name");
    expect(nameOrigin({ name_source: "auto", name_reason: null })).toBe("From the workshop, suggested by harbard");
    expect(nameOrigin({ name_source: "launched", name_reason: null })).toBe("Its lead launched it with this name");
    expect(nameOrigin({ name_source: null, name_reason: null })).toBeNull();
  });
});
