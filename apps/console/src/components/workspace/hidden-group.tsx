"use client";

import * as React from "react";
import { ChevronDown } from "lucide-react";
import type { WsAgent } from "@/lib/workspace";
import { createT } from "@/i18n";
import { AgentAvatar } from "./parts";
import { useWorkspace } from "./workspace-context";

const t = createT("shell");

/**
 * "Hidden (N)", folded, at the end of a list of agents: each with
 * "Show", and "Show all".
 */
export function HiddenGroup({ agents, className = "hb-row text-[14px] text-[var(--hb-sidebar-ink-2)]" }: { agents: WsAgent[]; className?: string }) {
  const { post } = useWorkspace();
  const [open, setOpen] = React.useState(false);
  if (agents.length === 0) return null;
  return (
    <div className="hb-hidden-group">
      <button type="button" className={className} aria-expanded={open} onClick={() => setOpen(!open)}>
        <ChevronDown className="size-4" style={{ transform: open ? undefined : "rotate(-90deg)" }} aria-hidden /> {t("hiddenCount", { count: agents.length })}
      </button>
      {open ? (
        <ul className="hb-hidden-list" aria-label={t("hiddenAgents")}>
          {agents.map((a) => (
            <li key={a.key} className="hb-hidden-row">
              <AgentAvatar name={a.name} size={24} agent={a} dot={false} />
              <span className="truncate">{a.name}</span>
              <button type="button" className="hb-hidden-show" aria-label={t("showAgent", { name: a.name })} onClick={() => void post("show-agent", { agent: a.key }).catch(() => {})}>
                {t("show")}
              </button>
            </li>
          ))}
          <li>
            <button type="button" className="hb-hidden-show hb-hidden-all" onClick={() => void post("show-all-agents", {}).catch(() => {})}>
              {t("showAll")}
            </button>
          </li>
        </ul>
      ) : null}
    </div>
  );
}
