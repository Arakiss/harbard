import { describe, expect, test } from "bun:test";
import {
  ENDING_KEY,
  directMode,
  endingText,
  historyByDay,
  historyQuery,
  lasted,
  mergePage,
  readOnlyNotice,
  spanText,
  type HistoryItem,
} from "./history";
import { dayLabel, type WsAgent } from "./workspace";

// 2026-11-07T10:00:00Z. Synthetic data only.
const NOW = Date.UTC(2026, 10, 7, 10, 0);
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

function agent(key: string, extra: Partial<WsAgent> = {}): WsAgent {
  return {
    key,
    name: `Robot ${key}`,
    label: `general-purpose · ${key}`,
    lost: false,
    kind: "teammate",
    lead_session: "lead-1",
    lead_quiet_ms: null,
    delivery: "ready",
    state: "done",
    state_reason: "end_turn",
    presence: "finished",
    dormant: null,
    last_action_ms: NOW - HOUR,
    first_ms: NOW - 2 * HOUR,
    brief: null,
    description: null,
    checkout: null,
    branch: null,
    build_dir: null,
    channels: ["harbard"],
    deliveries: 0,
    verified_deliveries: 0,
    reports: [],
    unread: 0,
    last_message: null,
    can_stop: false,
    activity: { state: "finished", since_ms: NOW - HOUR, detail: null, source_ms: NOW - HOUR },
    reactions: [],
    ...extra,
  };
}

function item(key: string, end: number, extra: Partial<HistoryItem> = {}): HistoryItem {
  return {
    agent: agent(key),
    channel: "harbard",
    repo: "/work/harbard",
    start_ms: end - 90 * MIN,
    end_ms: end,
    duration_ms: 90 * MIN,
    last_line: "Ready",
    ending: "finished",
    read_only: true,
    ...extra,
  };
}

describe("History, grouped by day", () => {
  test("today, yesterday, then the date, newest first, in the operator's calendar", () => {
    const items = [
      item("a", NOW - HOUR),
      item("b", NOW - 9 * HOUR), // 01:00, still today
      item("c", NOW - 12 * HOUR), // 22:00 yesterday
      item("d", NOW - DAY - 2 * HOUR),
      item("e", NOW - 3 * DAY),
    ];
    const groups = historyByDay(items, NOW);
    expect(groups.map((g) => g.day)).toEqual([NOW - HOUR, NOW - 12 * HOUR, NOW - 3 * DAY].map((end) => dayLabel(end, NOW)));
    expect(groups.map((g) => g.items.map((i) => i.agent.key))).toEqual([["a", "b"], ["c", "d"], ["e"]]);
  });

  test("an item's day is the day it ended, not the day it started", () => {
    const overnight = item("n", NOW - 9 * HOUR, { start_ms: NOW - 14 * HOUR });
    expect(historyByDay([overnight], NOW)[0]!.day).toBe(dayLabel(NOW, NOW));
  });

  test("nothing to group is no group", () => {
    expect(historyByDay([], NOW)).toEqual([]);
  });
});

describe("each item's line", () => {
  test("how it ended, in the shell's words, and In progress while open", () => {
    expect(endingText(item("a", NOW, { ending: "finished" }))).toBe("Finished");
    expect(endingText(item("a", NOW, { ending: "closed" }))).toBe("Closed");
    expect(endingText(item("a", NOW, { ending: "verified" }))).toBe("Verified by harbard");
    expect(endingText(item("a", NOW, { ending: "no_closure" }))).toBe("Never closed");
    expect(endingText(item("a", NOW, { ending: null }))).toBe("In progress");
    expect(Object.keys(ENDING_KEY).sort()).toEqual(["closed", "finished", "no_closure", "verified"]);
  });

  test("start and end: the times on one day, the dates across days, and how long", () => {
    expect(spanText(item("a", NOW - HOUR), NOW)).toBe("07:30 – 09:00 · 1 h 30 min");
    const across = item("b", NOW - 9 * HOUR, { start_ms: NOW - 14 * HOUR, duration_ms: 5 * HOUR });
    expect(spanText(across, NOW)).toBe("6 Nov 20:00 – 7 Nov 01:00 · 5 h");
  });

  test("a duration is said the way a person says it", () => {
    expect(lasted(20_000)).toBe("under 1 min");
    expect(lasted(45 * MIN)).toBe("45 min");
    expect(lasted(2 * HOUR + 5 * MIN)).toBe("2 h 5 min");
    expect(lasted(3 * DAY + 4 * HOUR + 10 * MIN)).toBe("3 d 4 h");
  });
});

describe("pages", () => {
  test("the query says only what was chosen", () => {
    expect(historyQuery({})).toBe("");
    expect(historyQuery({ limit: 50 })).toBe("?limit=50");
    expect(historyQuery({ before: "1:a", channel: "harbard", q: "  brújula ", limit: 50 })).toBe(
      "?limit=50&before=1%3Aa&channel=harbard&q=br%C3%BAjula",
    );
    expect(historyQuery({ channel: "", q: "   " })).toBe("");
  });

  test("a next page is appended and an item seen twice is kept once", () => {
    const first = [item("a", NOW - HOUR), item("b", NOW - 2 * HOUR)];
    const second = [item("b", NOW - 2 * HOUR), item("c", NOW - 3 * HOUR)];
    expect(mergePage(first, second).map((i) => i.agent.key)).toEqual(["a", "b", "c"]);
    expect(mergePage([], second).map((i) => i.agent.key)).toEqual(["b", "c"]);
  });
});

describe("opening an item in Direct messages", () => {
  test("an agent the shell still has opens live, with its composer", () => {
    expect(directMode(agent("a"), undefined)).toBe("live");
  });

  test("the shell's own copy decides, so a row never says read-only and then opens a composer", () => {
    // serve marks it read-only (quiet for two days), but its lead still relays: the shell has it live.
    expect(directMode(agent("a"), item("a", NOW - 2 * DAY, { read_only: true }))).toBe("live");
  });

  test("an ended session is read-only even while the shell lists it", () => {
    expect(directMode(agent("a", { delivery: "ended" }), undefined)).toBe("read-only");
  });

  test("an agent only the archive has is read-only once it arrives", () => {
    expect(directMode(null, undefined)).toBe("loading");
    expect(directMode(null, item("a", NOW - 3 * DAY))).toBe("read-only");
    expect(directMode(null, null)).toBe("missing");
  });

  test("the read-only notice says why there is no composer", () => {
    expect(readOnlyNotice()).toContain("Read-only");
    expect(readOnlyNotice()).toContain("ended");
  });
});
