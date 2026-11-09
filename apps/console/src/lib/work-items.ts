/**
 * A channel read by piece of work instead of by arrival: one row per ticket,
 * what it is, how it goes, and the last thing said about it that a person can
 * read. An agent carries its ticket in its role, its description, its brief
 * or its branch; the title is the ticket's own, in the brief between «».
 */
import { createT, type TranslationKey } from "@/i18n";
import { openCards, type WorkspaceView, type WsAgent, type WsMessage } from "./workspace";

const t = createT("workspace");

export type WorkStatus = "needs" | "blocked" | "working" | "paused" | "finished";

export interface WorkItem {
  /** The ticket's number, or `pr-<n>` for work known only by its PR. */
  key: string;
  title: string;
  status: WorkStatus;
  agents: WsAgent[];
  last_ms: number;
  /** The newest line written for a person; null when nobody wrote one. */
  line: string | null;
}

const WORK_STATUS: Record<WorkStatus, { label: TranslationKey<"workspace">; tone: "amber" | "green" | "grey" }> = {
  needs: { label: "workStatusNeeds", tone: "amber" },
  blocked: { label: "workStatusBlocked", tone: "amber" },
  working: { label: "workStatusWorking", tone: "green" },
  paused: { label: "workStatusPaused", tone: "grey" },
  finished: { label: "workStatusFinished", tone: "grey" },
};

export function workStatus(status: WorkStatus): { label: string; tone: "amber" | "green" | "grey" } {
  const entry = WORK_STATUS[status];
  return { label: t(entry.label), tone: entry.tone };
}

/** A finished piece of work stays this long, then leaves the list. */
export const WORK_RECENT_MS = 3 * 3_600_000;

const TICKET = /(?:AB#|lane-)(\d{5,6})\b/i;
const BRANCH_TICKET = /-(\d{5,6})$/;
const PR = /\b(?:qa-|PR ?#?)(\d{4})\b/i;

/** Which piece of work an agent is on, or null when it does not say. */
export function workKey(a: Pick<WsAgent, "role" | "description" | "brief" | "branch">): string | null {
  for (const field of [a.role, a.description, a.brief]) {
    const hit = field?.match(TICKET);
    if (hit) return hit[1]!;
  }
  const branch = a.branch?.match(BRANCH_TICKET);
  if (branch) return branch[1]!;
  for (const field of [a.role, a.description, a.brief]) {
    const hit = field?.match(PR);
    if (hit) return `pr-${hit[1]!}`;
  }
  return null;
}

function titleOf(agents: WsAgent[], key: string): string {
  for (const a of agents) {
    const quoted = a.brief?.match(/«([^»]+)»?/);
    if (quoted) return quoted[1]!.trim();
  }
  const described = agents.find((a) => a.description)?.description;
  const plain = described?.replace(/^(?:AB#\d+|QA)\s*[:·-]?\s*/i, "").trim();
  if (plain) return plain;
  return key.startsWith("pr-") ? `PR ${key.slice(3)}` : `AB#${key}`;
}

/** Whether a message is about this piece of work. */
export function aboutWork(m: WsMessage, item: Pick<WorkItem, "key" | "agents">): boolean {
  if (item.agents.some((a) => a.key === m.agent)) return true;
  if (item.key.startsWith("pr-")) return new RegExp(`\\b(?:PR ?#?|qa-)${item.key.slice(3)}\\b`, "i").test(m.text);
  return m.text.includes(`AB#${item.key}`);
}

/** Written for a person: it carries a label, or nothing flagged it. */
function readsAsHuman(m: WsMessage): boolean {
  return m.from !== "operator" && (m.label !== null || m.flags.length === 0);
}

const RANK: Record<WorkStatus, number> = { needs: 0, blocked: 1, working: 2, paused: 3, finished: 4 };

/** The channel's pieces of work: what needs them first, then the newest. */
export function workItems(
  view: Pick<WorkspaceView, "agents" | "cards">,
  channel: string,
  messages: WsMessage[],
  now: number,
): WorkItem[] {
  const groups = new Map<string, WsAgent[]>();
  for (const a of view.agents) {
    if (!a.channels.includes(channel)) continue;
    const key = workKey(a);
    if (key) groups.set(key, [...(groups.get(key) ?? []), a]);
  }
  const open = openCards(view as WorkspaceView);
  // A block is the agent's own stop: it may not need them at all.
  const asking = new Set(open.filter((c) => c.kind !== "blocked").flatMap((c) => c.agents));
  const blocked = new Set(open.filter((c) => c.kind === "blocked").flatMap((c) => c.agents));
  const items: WorkItem[] = [];
  for (const [key, agents] of groups) {
    const status: WorkStatus = agents.some((a) => asking.has(a.key))
      ? "needs"
      : agents.some((a) => blocked.has(a.key))
        ? "blocked"
        : agents.some((a) => a.state === "running")
          ? "working"
          : agents.some((a) => a.state === "idle")
            ? "paused"
            : "finished";
    const last_ms = Math.max(...agents.map((a) => a.last_action_ms ?? 0));
    if (status === "finished" && now - last_ms > WORK_RECENT_MS) continue;
    const item: WorkItem = { key, title: titleOf(agents, key), status, agents, last_ms, line: null };
    const newest = messages.filter((m) => aboutWork(m, item) && readsAsHuman(m)).at(-1);
    items.push({ ...item, line: newest?.text ?? null });
  }
  return items.sort((a, b) => RANK[a.status] - RANK[b.status] || b.last_ms - a.last_ms);
}
