"use client";

import * as React from "react";
import { X } from "lucide-react";
import { createT } from "@/i18n";
import { workStatus, type WorkItem } from "@/lib/work-items";
import type { WorkspaceView } from "@/lib/workspace";
import { AgentAvatar } from "./parts";

const t = createT("workspace");

/** Rows shown before "See all". */
const SHOWN = 5;

/** What the channel is working on: one row per ticket; a row narrows the channel to it. */
export function WorkPanel({
  items,
  selected,
  onSelect,
}: {
  view: WorkspaceView;
  items: WorkItem[];
  selected: string | null;
  onSelect: (key: string | null) => void;
}) {
  const [all, setAll] = React.useState(false);
  const chosen = items.find((i) => i.key === selected) ?? null;
  if (items.length === 0) return null;
  const shown = chosen ? [chosen] : all ? items : items.slice(0, SHOWN);
  return (
    <section className="hb-work" aria-label={t("workInProgress")}>
      <div className="hb-work-head">
        {chosen ? t("workOnlyThis") : t("workInProgressCount", { n: items.length })}
        {chosen ? (
          <button type="button" className="hb-fold" onClick={() => onSelect(null)}>
            <X className="mr-1 inline size-3" aria-hidden />
            {t("workWholeChannel")}
          </button>
        ) : items.length > SHOWN ? (
          <button type="button" className="hb-fold" aria-expanded={all} onClick={() => setAll(!all)}>
            {all ? t("workSeeLess") : t("workSeeAll", { n: items.length })}
          </button>
        ) : null}
      </div>
      <ul className="hb-work-list">
        {shown.map((item) => {
          const status = workStatus(item.status);
          return (
            <li key={item.key}>
              <button
                type="button"
                className="hb-work-row"
                aria-pressed={item.key === selected}
                title={t("workRowTitle")}
                data-testid={`work-${item.key}`}
                onClick={() => onSelect(item.key === selected ? null : item.key)}
              >
                <span className="hb-work-faces" aria-hidden>
                  {item.agents.slice(0, 3).map((a) => (
                    <AgentAvatar key={a.key} name={a.name} agent={a} size={22} dot={false} />
                  ))}
                </span>
                <span className="hb-work-text">
                  <span className="hb-work-title">{item.title}</span>
                  {item.line ? <span className="hb-work-line">{item.line}</span> : null}
                </span>
                <span className="hb-work-status" data-tone={status.tone}>
                  {status.label}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
