import { describe, expect, test } from "bun:test";
import { isInternal, machineText, machineMark, machineMarks, readableText, runs, techBody } from "./legible";
import type { WorkspaceView, WsAgent, WsMessage } from "./workspace";

const NOW = 1_794_086_000_000;

function message(id: string, author: string, extra: Partial<WsMessage> = {}): WsMessage {
  return {
    id,
    agent: author,
    from: "agent",
    author,
    ts_ms: NOW,
    text: `text ${id}`,
    body: null,
    label: null,
    version: null,
    flags: [],
    jargon: [],
    repeat_of: null,
    chars: 10,
    files: [],
    verified: null,
    channels: ["acme"],
    reply_to: null,
    replies: [],
    structured: null,
    undelivered: false,
    attachments: [],
    delivery: null,
    ...extra,
  };
}

/** The #acme screenshot: three loud chips on every message. */
const loud = { flags: ["unlabeled", "too_long", "jargon"], jargon: ["sha", "word"] } as Partial<WsMessage>;

describe("written for machines", () => {
  test("three chips become one quiet marker whose tooltip lists the reasons", () => {
    const mark = machineMark(message("a", "cap146", loud));
    expect(machineText()).toBe("written for machines");
    expect(mark).not.toBeNull();
    expect(mark!.reasons).toEqual([
      "has no label",
      "is over 1,200 characters",
      "has a commit code",
      "uses a technical word (rebase, merge, gate…)",
    ]);
    expect(mark!.hint).toBe(
      "This message is written for other machines, not for you: has no label, is over 1,200 characters, has a commit code and uses a technical word (rebase, merge, gate…). The original is under “Technical details”.",
    );
  });

  test("a clean message gets no marker, and an old serve without reasons still reads", () => {
    expect(machineMark(message("a", "x"))).toBeNull();
    expect(machineMark(message("a", "x", { flags: ["jargon"], jargon: undefined }))!.reasons).toEqual(["isn't in plain language"]);
    expect(machineMark(message("a", "x", { flags: ["repeat"] }))!.reasons).toEqual(["repeats an earlier message"]);
  });

  test("a run of one author shows the marker once, on its first flagged message", () => {
    const ms = [
      message("1", "cap146", loud),
      message("2", "cap146", loud),
      message("3", "cap146"),
      message("4", "cap146", loud),
      message("5", "cap143", loud),
      message("6", "lead", { from: "lead", author: "Claude · lead" }),
      message("7", "cap143", loud),
    ];
    const marks = machineMarks(ms);
    expect([...marks.keys()]).toEqual(["1", "5", "7"]);
    expect(marks.get("1")!.hint).toEndWith("Also in the next 2 messages from this author.");
    expect(marks.get("5")!.hint).not.toContain("Also in");
  });

  test("an operator note is never marked", () => {
    expect(machineMarks([message("n", "dana", { from: "operator", flags: ["too_long"] })]).size).toBe(0);
  });
});

test("a repeat drawn under its first copy does not count in the run", () => {
  const ms = [message("1", "a", loud), message("2", "a", { ...loud, repeat_of: "1", flags: ["unlabeled", "too_long", "jargon", "repeat"] })];
  expect(machineMarks(ms).get("1")!.more).toBe(0);
});

describe("the readable line", () => {
  const parts = [{ kind: "text" as const, text: "CAP-146 terminado y subido" }];
  test("the original line goes under the technical details, above the body", () => {
    const m = message("a", "cap146", { text: "CAP-146 is done: head acb3816f, pushed.", body: "Gates green.", readable: parts });
    expect(techBody(m)).toBe("CAP-146 is done: head acb3816f, pushed.\n\nGates green.");
    expect(techBody({ ...m, body: null })).toBe("CAP-146 is done: head acb3816f, pushed.");
  });

  test("without a readable line the details are the body, as before", () => {
    expect(techBody(message("a", "x", { body: "detail" }))).toBe("detail");
    expect(techBody(message("a", "x"))).toBeNull();
  });

  test("the readable line as plain text, chips at their short form", () => {
    expect(
      readableText([
        { kind: "text", text: "Rebased onto robots " },
        { kind: "sha", text: "fcf628c", full: "fcf628c0ab" },
      ]),
    ).toBe("Rebased onto robots fcf628c");
  });
});

describe("internal updates", () => {
  const v = { agents: [] } as unknown as WorkspaceView;
  const raw = (id: string, who: string, extra: Partial<WsMessage> = {}) =>
    message(id, who, { flags: ["unlabeled", "jargon"], ...extra });

  test("reports with no line for the operator fold together, across authors", () => {
    const items = runs(v, [raw("1", "Naomi"), raw("2", "Celia"), raw("3", "Naomi")], { foldInternal: true });
    expect(items.map((i) => i.kind)).toEqual(["internal"]);
    expect((items[0] as { label: string }).label).toBe("3 internal updates · Naomi, Celia");
  });

  test("a message written for them splits them, and stays whole", () => {
    const told = message("2", "Celia", { label: "DONE", flags: [] });
    const items = runs(v, [raw("1", "Naomi"), told, raw("3", "Naomi")], { foldInternal: true });
    expect(items.map((i) => i.kind)).toEqual(["internal", "message", "internal"]);
    expect((items[0] as { label: string }).label).toBe("1 internal update · Naomi");
  });

  test("what names someone, answers or carries a file is not internal", () => {
    expect(isInternal(raw("1", "a", { mentions: ["dana"] }))).toBe(false);
    expect(isInternal(raw("1", "a", { replies: ["r"] }))).toBe(false);
    expect(isInternal(raw("1", "a", { from: "lead" }))).toBe(false);
    expect(isInternal(message("1", "a"))).toBe(false);
  });

  test("a session's note with jargon folds; a plain one stays", () => {
    expect(isInternal(message("bus:1", "a", { from: "lead", jargon: ["word"] }))).toBe(true);
    expect(isInternal(message("bus:2", "a", { from: "lead" }))).toBe(false);
  });

  test("a direct conversation keeps every message", () => {
    expect(runs(v, [raw("1", "a"), raw("2", "a")]).map((i) => i.kind)).toEqual(["message", "message"]);
  });
});

describe("runs", () => {
  const agentOf = (key: string, team: string | null) => ({ key, name: key, team }) as unknown as WsAgent;
  const AG = [agentOf("cap82", "acme"), agentOf("cap150", "acme"), agentOf("cap149", "acme"), agentOf("cap52", "acme"), agentOf("cap86", "acme"), agentOf("rev", "acme"), agentOf("x", "harbard")];
  const v = { agents: AG } as unknown as WorkspaceView;
  const done = (id: string, who: string, extra: Partial<WsMessage> = {}) => message(id, who, { shape: "delivery", ...extra });

  test("five deliveries of one team fold into one line, the newest kept whole", () => {
    const ms = [message("0", "rev"), done("1", "cap82"), done("2", "cap150"), done("3", "cap149"), done("4", "cap52"), done("5", "cap86"), message("6", "rev")];
    const items = runs(v, ms);
    expect(items.map((i) => i.kind)).toEqual(["message", "run", "message"]);
    const run = items[1] as Extract<(typeof items)[number], { kind: "run" }>;
    expect(run.label).toBe("5 deliveries from acme");
    expect(run.older.map((m) => m.id)).toEqual(["1", "2", "3", "4"]);
    expect(run.newest.id).toBe("5");
  });

  test("a run needs three; another team, another kind or a thread breaks it", () => {
    expect(runs(v, [done("1", "cap82"), done("2", "cap150")]).map((i) => i.kind)).toEqual(["message", "message"]);
    const mixed = [done("1", "cap82"), done("2", "cap150"), done("3", "x"), done("4", "cap149"), message("5", "cap52", { shape: "review" })];
    expect(runs(v, mixed).every((i) => i.kind === "message")).toBe(true);
    const threaded = [done("1", "cap82"), done("2", "cap150", { replies: ["r"] }), done("3", "cap149"), done("4", "cap52")];
    expect(runs(v, threaded).map((i) => i.kind)).toEqual(["message", "message", "message", "message"]);
  });

  test("three reviews name their team; without a team there is no team in the label; an operator note never joins", () => {
    const rv = (id: string) => message(id, "rev", { shape: "review" });
    expect((runs(v, [rv("1"), rv("2"), rv("3")])[0] as { label: string }).label).toBe("3 reviews from acme");
    const lone = { agents: [agentOf("a", null)] } as unknown as WorkspaceView;
    expect((runs(lone, [done("1", "a"), done("2", "a"), done("3", "a")])[0] as { label: string }).label).toBe("3 deliveries");
    const note = message("n", "cap82", { from: "operator", shape: "delivery" });
    expect(runs(v, [done("1", "cap82"), note, done("2", "cap150"), done("3", "cap149")]).map((i) => i.kind)).toEqual(["message", "message", "message", "message"]);
  });

  test("a repeat drawn under its first copy neither joins nor breaks a run", () => {
    const ms = [done("1", "cap82"), done("1b", "cap82", { repeat_of: "1" }), done("2", "cap150"), done("3", "cap149")];
    const items = runs(v, ms);
    expect(items.map((i) => i.kind)).toEqual(["run"]);
    expect((items[0] as { older: WsMessage[] }).older.map((m) => m.id)).toEqual(["1", "2"]);
  });
});
