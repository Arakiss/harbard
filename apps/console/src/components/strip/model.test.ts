import { describe, expect, test } from "bun:test";
import type { Activity, ActivityState } from "@/lib/activity";
import { closeDesktopStrip, desktopViewHref, openDesktopView, validMainDestination, type DesktopViewBridge } from "@/lib/desktop-view";
import { cardBody, type WsAgent, type WsCard, type WsMessage, type WorkspaceView } from "@/lib/workspace";
import {
  NEEDS_MAX,
  NEEDS_MORE,
  SHELF_MAX,
  SHELF_MORE,
  cardDestination,
  cardPill,
  cardView,
  STAGE_FIT_SCRIPT,
  clock,
  closesStrip,
  stripFit,
  dimmed,
  today,
  todayLine,
  nameSize,
  needs,
  robotDestination,
  sentence,
  shelf,
  shelfGroup,
  type LiveAgent,
} from "./model";

// 2026-11-07 13:52 UTC. Synthetic data only.
const NOW = Date.UTC(2026, 10, 7, 13, 52);
const S = 1000;
const MIN = 60 * S;
const HOUR = 60 * MIN;

function agent(key: string, extra: Partial<WsAgent> = {}, state: ActivityState = "thinking"): WsAgent {
  const last = extra.last_action_ms === undefined ? NOW - MIN : extra.last_action_ms;
  const activity: Activity = { state, since_ms: last ?? NOW, detail: null, source_ms: last ?? NOW };
  return {
    key,
    name: `agent-${key}`,
    label: `general-purpose · ${key}`,
    lost: false,
    kind: "teammate",
    lead_session: "lead-1",
    lead_quiet_ms: 0,
    delivery: "ready",
    state: "running",
    state_reason: "mid_turn",
    presence: "working",
    dormant: null,
    last_action_ms: last,
    first_ms: NOW - 3 * HOUR,
    brief: null,
    description: null,
    checkout: null,
    branch: null,
    build_dir: null,
    channels: [],
    deliveries: 0,
    verified_deliveries: 0,
    reports: [],
    unread: 0,
    last_message: null,
    can_stop: false,
    activity,
    reactions: [],
    ...extra,
  };
}

function message(id: string, extra: Partial<WsMessage> = {}): WsMessage {
  return {
    id,
    agent: "a",
    from: "agent",
    author: "agent-a",
    ts_ms: NOW - HOUR,
    text: `DONE · harbard · entrega ${id}`,
    label: "DONE",
    version: null,
    flags: [],
    repeat_of: null,
    chars: 20,
    files: [],
    verified: null,
    channels: [],
    reply_to: null,
    replies: [],
    structured: null,
    undelivered: false,
    ...extra,
  };
}

function card(id: string, extra: Partial<WsCard> = {}): WsCard {
  return { id, kind: "blocked", channel: "harbard", agents: ["a"], message: null, checkout: null, since_ms: NOW - 10 * MIN, resolved_ms: null, needs: true, ...extra };
}

function view(agents: WsAgent[], messages: WsMessage[] = [], cards: WsCard[] = []): WorkspaceView {
  return {
    v: 1,
    now_ms: NOW,
    operator: "Dana",
    operator_initials: "DR",
    thresholds: { amber_ms: 15 * MIN, red_ms: 45 * MIN, thinking_long_ms: 2 * MIN, waiting_sleepy_ms: 5 * MIN, running_long_ms: 10 * MIN },
    channels: [],
    agents,
    messages,
    cards,
    deliveries: [],
    saved: [],
    unread: { directs: 0, activity: 0 },
    team_root_found: true,
    format_changed: false,
    motion: null,
  };
}

describe("the shelf", () => {
  test("working first, then waiting, sleeping and finished; newest first inside a group, then by key", () => {
    const v = view([
      agent("done", { presence: "finished", last_action_ms: NOW - 2 * MIN }, "finished"),
      agent("sleep", { dormant: "amber", last_action_ms: NOW - 20 * MIN }, "asleep"),
      agent("wait", { presence: "waiting", last_action_ms: NOW - 30 * S }, "waiting"),
      agent("w-old", { last_action_ms: NOW - 5 * MIN }),
      agent("w-b", { last_action_ms: NOW - MIN }),
      agent("w-a", { last_action_ms: NOW - MIN }),
      agent("blocked", { presence: "waiting", last_action_ms: NOW - 3 * MIN }, "blocked"),
    ]);
    expect(shelf(v, NOW).shown.map((a) => a.key)).toEqual(["w-a", "w-b", "w-old", "wait", "blocked", "sleep", "done"]);
  });

  test("only agents active in the last two hours stand on it; lost ones do not", () => {
    const v = view([
      agent("in", { last_action_ms: NOW - 2 * HOUR }),
      agent("out", { last_action_ms: NOW - 2 * HOUR - 1 }),
      agent("lost", { lost: true, last_action_ms: NOW - MIN }),
      agent("never", { last_action_ms: null }),
    ]);
    const s = shelf(v, NOW);
    expect(s.shown.map((a) => a.key)).toEqual(["in"]);
    expect(s.empty).toBe(false);
  });

  test("the live slice's fresher action and face count, never an older one", () => {
    const v = view([agent("a", { last_action_ms: NOW - 3 * HOUR }), agent("b", { last_action_ms: NOW - 10 * MIN })]);
    const live = new Map<string, LiveAgent>([
      ["a", { activity: { state: "thinking", since_ms: NOW - S, detail: null, source_ms: NOW - S }, last_action_ms: NOW - S }],
      ["b", { activity: { state: "finished", since_ms: NOW - 10 * MIN, detail: null, source_ms: NOW }, last_action_ms: NOW - 4 * HOUR }],
    ]);
    // a is active again and working; b keeps its own newer action but now reads as finished.
    expect(shelf(v, NOW, live).shown.map((a) => a.key)).toEqual(["a", "b"]);
    expect(shelfGroup(v.agents[1]!, live.get("b")!.activity)).toBe("finished");
  });

  test("at most eight robots; the rest are '+N' at the end", () => {
    const many = Array.from({ length: 11 }, (_, i) => agent(`k${String(i).padStart(2, "0")}`, { last_action_ms: NOW - (i + 1) * MIN }));
    const s = shelf(view(many), NOW);
    expect(SHELF_MAX).toBe(8);
    expect(s.shown).toHaveLength(8);
    expect(s.more).toBe(3);
    expect(s.shown.map((a) => a.key)).toEqual(["k00", "k01", "k02", "k03", "k04", "k05", "k06", "k07"]);
    expect(shelf(view(many.slice(0, 8)), NOW).more).toBe(0);
    expect(shelf(view(many.slice(0, 9)), NOW).more).toBe(1);
  });

  test("nobody active: the latest agents sleep on the shelf, and an empty fleet leaves it bare", () => {
    const v = view([
      agent("old", { presence: "finished", last_action_ms: NOW - 5 * HOUR }, "finished"),
      agent("older", { presence: "finished", last_action_ms: NOW - 9 * HOUR }, "finished"),
      agent("never", { last_action_ms: null }),
    ]);
    const s = shelf(v, NOW);
    expect(s.empty).toBe(true);
    expect(s.more).toBe(0);
    expect(s.shown.map((a) => a.key)).toEqual(["old", "older"]);
    expect(shelf(view([]), NOW)).toEqual({ shown: [], more: 0, empty: true });
  });

  test("a name's first estimate is as large as its slot allows, between 28 and 38 px", () => {
    expect(nameSize("Atlas", 206)).toBe(38);
    expect(nameSize("parser", 206)).toBe(38);
    expect(nameSize("harbard-themes", 206)).toBe(28);
    expect(nameSize("a-very-long-agent-name-indeed", 167)).toBe(28);
  });
});

describe("Needs you", () => {
  test("Activity's open cards in its order, at most three, then '+N more'", () => {
    const cards = [card("1"), card("2", { resolved_ms: NOW }), card("3", { needs: false }), card("4"), card("5"), card("6")];
    const n = needs(view([agent("a")], [], cards));
    expect(NEEDS_MAX).toBe(3);
    expect(n.shown.map((c) => c.id)).toEqual(["1", "4", "5"]);
    expect(n.more).toBe(1);
    expect(needs(view([agent("a")], [], cards.slice(0, 1))).more).toBe(0);
  });

  test("a blocked card shows the sentence of its message, without the protocol's label and name", () => {
    const blocked = message("b", { label: "BLOCKED", text: "BLOCKED · keychain-review · Can it ask for the password twice?\nMás detalle." });
    const v = view([agent("a", { name: "keychain-review" })], [blocked], [card("c", { message: "b" })]);
    expect(cardView(v.cards[0]!, v, NOW)).toEqual({
      who: "keychain-review",
      pill: { text: "Blocked", tone: "warn" },
      text: "Can it ask for the password twice?",
      agent: "a",
      action: "reply",
      hidden: false,
    });
    const writers = view([agent("a"), agent("b")], [], [card("w", { kind: "two_writers", agents: ["a", "b"], checkout: "/work/x" })]);
    expect(cardView(writers.cards[0]!, writers, NOW)).toMatchObject({ who: cardBody(writers.cards[0]!, writers, NOW).title, pill: { text: "Alert" }, action: "open" });
  });

  test("a hidden agent's card still shows, marked hidden", () => {
    const blocked = message("b", { label: "BLOCKED", text: "BLOCKED · keychain-review · Continue?" });
    const v = view([agent("a", { name: "keychain-review", hidden: true }), agent("z")], [blocked], [card("c", { message: "b" }), card("d", { agents: ["z"] })]);
    expect(needs(v).shown.map((c) => c.id)).toEqual(["c", "d"]);
    expect(cardView(v.cards[0]!, v, NOW).hidden).toBe(true);
    expect(cardView(v.cards[1]!, v, NOW).hidden).toBe(false);
  });

  test("pills say why the card is there", () => {
    expect(cardPill({ kind: "dormant", since_ms: Date.UTC(2026, 10, 7, 0, 12) })).toEqual({ text: "No activity since 00:12", tone: "quiet" });
    expect(cardPill({ kind: "approval", since_ms: NOW }).text).toBe("Approval");
    expect(cardPill({ kind: "two_writers", since_ms: NOW }).tone).toBe("alert");
  });

  test("sentence() keeps unlabelled text and strips only a protocol prefix", () => {
    expect(sentence("Hello, continue?")).toBe("Hello, continue?");
    expect(sentence("DONE v2 · harbard · Ready")).toBe("Ready");
    expect(sentence("FINDING: un hallazgo sin nombre")).toBe("un hallazgo sin nombre");
    expect(sentence("\n\nSTART · harbard-strip · Starting the strip")).toBe("Starting the strip");
  });
});

describe("Today", () => {
  const at = (h: number, m: number) => Date.UTC(2026, 10, 7, h, m);
  const evidence = { sha: "abc1234", branch: "main", decided_ms: NOW };

  test("counts today's deliveries once each, and verified only on harbard's evidence", () => {
    const messages = [
      message("v1", { ts_ms: at(14, 41), verified: evidence }),
      message("v2", { ts_ms: at(9, 14), verified: evidence }),
      // A claim in the text is not evidence.
      message("claim", { ts_ms: at(12, 8), text: "DONE · harbard · verificado y probado, todo verde" }),
      message("plain", { ts_ms: at(10, 36) }),
      message("repeat", { ts_ms: at(10, 40), repeat_of: "plain", verified: evidence }),
      message("yesterday", { ts_ms: at(0, 0) - MIN, verified: evidence }),
      message("not-done", { ts_ms: at(13, 0), label: "FINDING" }),
      message("from-lead", { ts_ms: at(13, 5), from: "lead" }),
      message("structured", { ts_ms: at(13, 6), structured: "idle_notification" }),
    ];
    const h = today(view([agent("a")], messages), NOW);
    expect(h.items.map((m) => m.id)).toEqual(["v1", "claim", "plain", "v2"]);
    expect([h.deliveries, h.verified]).toEqual([4, 2]);
    expect(todayLine(h)).toBe("Today: 4 deliveries · 2 verified");
  });

  test("the day ends at the operator's midnight: 00:30 on the 8th is tomorrow, 00:30 on the 7th is today", () => {
    const late = Date.UTC(2026, 10, 7, 23, 59);
    const h = today(view([], [message("tomorrow", { ts_ms: Date.UTC(2026, 10, 8, 0, 30) }), message("today", { ts_ms: Date.UTC(2026, 10, 7, 0, 30) })]), late);
    expect(h.items.map((m) => m.id)).toEqual(["today"]);
  });

  test("the summary's words, singular and none", () => {
    expect(todayLine({ deliveries: 6, verified: 4 })).toBe("Today: 6 deliveries · 4 verified");
    expect(todayLine({ deliveries: 1, verified: 1 })).toBe("Today: 1 delivery · 1 verified");
    expect(todayLine({ deliveries: 2, verified: 0 })).toBe("Today: 2 deliveries · 0 verified");
    expect(todayLine({ deliveries: 0, verified: 0 })).toBe("Today: no deliveries yet");
  });
});

describe("taps reach the main window through the desktop bridge", () => {
  function fakeBridge(): { bridge: DesktopViewBridge; calls: unknown[][] } {
    const calls: unknown[][] = [];
    return { calls, bridge: { kind: "desktop", invoke: async (...args) => void calls.push(args) } };
  }

  test("a robot opens its Direct messages conversation", async () => {
    const { bridge, calls } = fakeBridge();
    const destination = robotDestination(agent("4f2a-b"));
    expect(await openDesktopView("overview", bridge, destination)).toBe("opened");
    expect(calls).toEqual([["desk_open_main", { destination: { view: "direct", session: "4f2a-b" } }]]);
    expect(desktopViewHref("overview", destination)).toBe("/direct/4f2a-b");
  });

  test("Reply opens that card in Activity", async () => {
    const { bridge, calls } = fakeBridge();
    const destination = cardDestination(card("blocked:a:01JX"));
    await openDesktopView("overview", bridge, destination);
    expect(calls).toEqual([["desk_open_main", { destination: { view: "activity", note: "card:blocked:a:01JX" } }]]);
    expect(desktopViewHref("overview", destination)).toBe("/activity?sel=card%3Ablocked%3Aa%3A01JX");
  });

  test("'+N more' and the shelf's '+N' open Activity and Direct messages", async () => {
    const { bridge, calls } = fakeBridge();
    await openDesktopView("overview", bridge, NEEDS_MORE);
    await openDesktopView("overview", bridge, SHELF_MORE);
    expect(calls).toEqual([
      ["desk_open_main", { destination: { view: "activity" } }],
      ["desk_open_main", { destination: { view: "direct" } }],
    ]);
    expect(desktopViewHref("overview", NEEDS_MORE)).toBe("/activity");
    expect(desktopViewHref("overview", SHELF_MORE)).toBe("/direct");
  });

  test("a hostile agent key stays data: rejected with a control character, escaped in the route", async () => {
    const { bridge, calls } = fakeBridge();
    await expect(openDesktopView("overview", bridge, robotDestination({ key: "a\nb" }))).rejects.toThrow("Invalid main");
    expect(calls).toEqual([]);
    expect(validMainDestination({ view: "direct", session: "x", extra: 1 })).toBe(false);
    expect(desktopViewHref("overview", robotDestination({ key: "../../api?x=1" }))).toBe("/direct/..%2F..%2Fapi%3Fx%3D1");
  });
});

describe("clock and night", () => {
  test("the clock follows the language, on the operator's clock", () => {
    expect(clock(NOW)).toEqual({ time: "13:52", date: "Saturday 7 Nov" });
    expect(clock(Date.UTC(2026, 6, 1, 6, 5))).toEqual({ time: "06:05", date: "Wednesday 1 Jul" });
  });

  test("the strip dims from 22:00 to 7:00, only when the OS is dark and the preference is on", () => {
    const at = (h: number, m = 0) => Date.UTC(2026, 10, 7, h, m);
    expect(dimmed(at(21, 59), true, true)).toBe(false);
    expect(dimmed(at(22, 0), true, true)).toBe(true);
    expect(dimmed(at(3, 0), true, true)).toBe(true);
    expect(dimmed(at(6, 59), true, true)).toBe(true);
    expect(dimmed(at(7, 0), true, true)).toBe(false);
    expect(dimmed(at(23, 0), false, true)).toBe(false);
    expect(dimmed(at(23, 0), true, false)).toBe(false);
  });
});

describe("the stage fits any window, whole and centred", () => {
  test("1280×600: half size, letterboxed above and below", () => {
    expect(stripFit(1280, 600)).toEqual({ k: 0.5, width: 1280, height: 360, left: 0, top: 120 });
  });
  test("1512×945 (a MacBook at default scaling): the width decides", () => {
    const f = stripFit(1512, 945);
    expect(f.k).toBeCloseTo(0.590625, 9);
    expect(f.width).toBeCloseTo(1512, 9);
    expect(f.height).toBeCloseTo(425.25, 9);
    expect(f.left).toBe(0);
    expect(f.top).toBeCloseTo(259.875, 9);
  });
  test("2560×720, the stage's own grid: exactly 1, no margin", () => {
    expect(stripFit(2560, 720)).toEqual({ k: 1, width: 2560, height: 720, left: 0, top: 0 });
  });
  test("a window wider than 32:9 is letterboxed left and right, never cropped", () => {
    const f = stripFit(3000, 600);
    expect(f.height).toBeCloseTo(600, 9);
    expect(f.width).toBeLessThanOrEqual(3000);
    expect(f.left).toBeCloseTo((3000 - 2560 * (600 / 720)) / 2, 9);
  });
  test("every size keeps the stage inside the window at one uniform scale", () => {
    for (const [w, h] of [[640, 180], [800, 800], [1280, 600], [1512, 945], [2133, 600], [2560, 720], [3840, 1080], [5120, 300]]) {
      const f = stripFit(w, h);
      expect(f.width).toBeLessThanOrEqual(w + 1e-9);
      expect(f.height).toBeLessThanOrEqual(h + 1e-9);
      expect(f.width / f.height).toBeCloseTo(2560 / 720, 9);
      // It touches two opposite edges: the largest scale that fits.
      expect(Math.min(w - f.width, h - f.height)).toBeCloseTo(0, 9);
    }
  });
  test("a window with no size yet keeps the stage at 1", () => {
    expect(stripFit(0, 0).k).toBe(1);
    expect(stripFit(Number.NaN, 600).k).toBe(1);
  });
  test("the pre-paint inline script sets the same scale as stripFit", () => {
    for (const [w, h] of [[1280, 600], [1512, 945], [2560, 720], [0, 0]]) {
      const props = new Map<string, string>();
      const doc = { documentElement: { style: { setProperty: (k: string, v: string) => props.set(k, v) } } };
      new Function("innerWidth", "innerHeight", "document", "addEventListener", STAGE_FIT_SCRIPT)(w, h, doc, () => {});
      expect(Number(props.get("--cs-k"))).toBe(stripFit(w, h).k);
    }
  });
});

describe("Esc and ⌘W hide the Strip window", () => {
  const key = (k: string, mods: Partial<Record<"metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "defaultPrevented", boolean>> = {}) =>
    ({ key: k, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...mods });
  test("Esc, ⌘W and Ctrl+W close; anything else does not", () => {
    expect(closesStrip(key("Escape"))).toBe(true);
    expect(closesStrip(key("w", { metaKey: true }))).toBe(true);
    expect(closesStrip(key("W", { metaKey: true }))).toBe(true);
    expect(closesStrip(key("w", { ctrlKey: true }))).toBe(true);
    expect(closesStrip(key("w"))).toBe(false);
    expect(closesStrip(key("Escape", { shiftKey: true }))).toBe(false);
    expect(closesStrip(key("w", { metaKey: true, shiftKey: true }))).toBe(false);
    expect(closesStrip(key("Escape", { defaultPrevented: true }))).toBe(false);
  });
  test("in the desktop app they invoke only desk_close_strip; in a browser, nothing", async () => {
    const calls: string[] = [];
    const desktop: DesktopViewBridge = { kind: "desktop", invoke: async (command) => { calls.push(command); } };
    expect(await closeDesktopStrip(desktop)).toBe("closed");
    expect(calls).toEqual(["desk_close_strip"]);
    expect(await closeDesktopStrip({ kind: "browser" })).toBe("browser");
    expect(await closeDesktopStrip({ kind: "desktop", invoke: null })).toBe("browser");
    expect(calls).toEqual(["desk_close_strip"]);
  });
});

test("the layouts' inline scripts carry the CSP nonce, or the browser blocks them", async () => {
  const pulse = await Bun.file(new URL("../../app/(strip)/pulse/layout.tsx", import.meta.url)).text();
  expect(pulse).toContain('h.get("x-nonce")');
  expect(pulse).toMatch(/<script nonce=\{nonce\} dangerouslySetInnerHTML=\{\{ __html: FIT \}\} \/>/);
  const root = await Bun.file(new URL("../../app/layout.tsx", import.meta.url)).text();
  expect(root).toContain('get("x-nonce")');
  // Every inline <script> in either layout names the nonce.
  for (const source of [pulse, root]) {
    const scripts = source.match(/<script\b[^>]*/g) ?? [];
    expect(scripts.length).toBeGreaterThan(0);
    for (const tag of scripts) expect(tag).toContain("nonce={nonce}");
  }
});
