/**
 * "Hide": agents the operator hid leave the sidebar's Agents and Direct,
 * except for a folded "Hidden (N)" group. Serve keeps the set on the desk
 * (`hidden` on each agent); nothing here decides what needs them: a hidden
 * agent's BLOCKED or `@operator` still raises its card, with an "hidden" note.
 */

import { createT } from "@/i18n";
import { filterAgents, messageById, type WorkspaceAction, type WorkspaceView, type WsAgent, type WsCard } from "./workspace";

const t = createT("shell");

function isHidden(a: Pick<WsAgent, "hidden">): boolean {
  return a.hidden === true;
}

/** Direct: agents with a conversation, newest message first, none hidden. */
export function directAgents(view: WorkspaceView, query: string): WsAgent[] {
  const withMessages = view.agents.filter((a) => !a.temporary && !isHidden(a) && (a.last_message || a.kind === "teammate" || a.kind === "session"));
  const at = (a: WsAgent) => (a.last_message ? (messageById(view, a.last_message)?.ts_ms ?? 0) : 0);
  return filterAgents(withMessages, query).sort((a, b) => at(b) - at(a) || a.key.localeCompare(b.key));
}

/** "Hidden (N)": every hidden agent, by name. */
export function hiddenAgents(view: Pick<WorkspaceView, "agents">): WsAgent[] {
  return view.agents.filter(isHidden).sort((a, b) => a.name.localeCompare(b.name, "es") || a.key.localeCompare(b.key));
}

/** The ⋯ menu's item for an agent. */
export function hideAction(a: Pick<WsAgent, "hidden">): { label: string; action: Extract<WorkspaceAction, "hide-agent" | "show-agent"> } {
  return isHidden(a) ? { label: t("show"), action: "show-agent" } : { label: t("hide"), action: "hide-agent" };
}

/** A card about a hidden agent: it still shows, with a small "hidden". */
export function cardHidden(view: Pick<WorkspaceView, "agents">, card: Pick<WsCard, "agents">): boolean {
  return card.agents.some((k) => view.agents.some((a) => a.key === k && isHidden(a)));
}
