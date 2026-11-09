/**
 * The channel header's members: how many agents worked in a channel, how
 * many work there now, and the popover's order. "Now" is serve's own
 * presence (`working`) without its dormant mark: one quiet past the desk's
 * threshold, or lost, says "no activity" and does not count.
 */

import { createT } from "@/i18n";
import { localRole } from "./serve-text";
import { LEAD_ROLE, agentRoleLine, leadRoleLabel, members, type WorkspaceView, type WsAgent, type WsChannel } from "./workspace";

const t = createT("workspace");

export interface ChannelMembers {
  total: number;
  now: number;
  /** "43 agents have worked in #channel · 5 now": the tooltip and aria-label. */
  label: string;
  /** Working first, newest first; then the rest by last activity. */
  ordered: WsAgent[];
  /** The stacked avatars: the first three of `ordered`. */
  stack: WsAgent[];
}

export function isWorkingNow(a: Pick<WsAgent, "presence" | "lost" | "dormant">): boolean {
  return a.presence === "working" && !a.lost && !a.dormant;
}

/**
 * Teammates that belong to no channel of their own (one working in a checkout
 * no channel claims) count in their lead's: leaving them out said "nobody
 * working" over a running agent.
 */
function unplaced(view: WorkspaceView, channel: WsChannel): WsAgent[] {
  return view.agents.filter((a) => {
    if (a.channels?.length || !a.lead_session || channel.members.includes(a.key)) return false;
    const lead = view.agents.find((l) => l.key === `session:${a.lead_session}`);
    return lead?.channels?.includes(channel.id) ?? false;
  });
}

export function channelMembers(view: WorkspaceView, channel: WsChannel): ChannelMembers {
  const all = [...members(view, channel), ...unplaced(view, channel)];
  const byLast = (a: WsAgent, b: WsAgent) => (b.last_action_ms ?? 0) - (a.last_action_ms ?? 0) || a.name.localeCompare(b.name, "es");
  const working = all.filter(isWorkingNow).sort(byLast);
  const rest = all.filter((a) => !isWorkingNow(a)).sort(byLast);
  const ordered = [...working, ...rest];
  const total = ordered.length;
  const now = working.length;
  const who = total === 1 ? t("membersWorkedOne", { channel: channel.id }) : t("membersWorkedMany", { n: total, channel: channel.id });
  const label = `${who} · ${now === 0 ? t("membersNowNone") : t("membersNowCount", { n: now })}`;
  return { total, now, label, ordered, stack: ordered.slice(0, 3) };
}

/** "builder · harbard": its workshop line, or role and team, or its type. */
export function memberRole(a: Pick<WsAgent, "role_line" | "role_line_code" | "role_line_params" | "role" | "team" | "label">): string {
  if (a.role_line) return agentRoleLine(a);
  const role = a.role === LEAD_ROLE ? leadRoleLabel() : a.role && localRole(a.role);
  if (role && a.team) return `${role} · ${a.team}`;
  return role ?? a.label;
}
