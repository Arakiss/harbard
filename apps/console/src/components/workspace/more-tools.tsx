"use client";

import * as React from "react";
import Link from "next/link";
import type { Route } from "next";
import { ChevronRight, History, LayoutGrid, MonitorPlay, Plug, type LucideIcon } from "lucide-react";
import { DesktopViewLink } from "@/components/desktop-view-link";
import { createT } from "@/i18n";
import { moreTools, type MoreToolId } from "@/lib/workspace";

const t = createT("shell");

/** One icon per kept tool. */
const ICONS: Record<MoreToolId, LucideIcon> = {
  strip: MonitorPlay,
  history: History,
  setup: Plug,
};

/**
 * "More tools", at the end of Home's sidebar: the tools that are not
 * a conversation, each with what it is for. A disclosure, so the list sits
 * in the sidebar's own flow and the keyboard reaches it in order.
 */
export function MoreTools() {
  const [open, setOpen] = React.useState(false);
  return (
    <div className="hb-more">
      <button type="button" className="hb-row hb-more-button" aria-expanded={open} aria-controls="hb-more-list" onClick={() => setOpen(!open)}>
        <span className="hb-more-icon" aria-hidden>
          <LayoutGrid className="size-4" strokeWidth={2} />
        </span>
        {t("moreTools")}
        <ChevronRight className="hb-more-chevron size-4" data-open={open || undefined} aria-hidden />
      </button>
      {open ? (
        <ul id="hb-more-list" className="hb-more-list" aria-label={t("moreTools")}>
          {moreTools().map((m) => {
            const Icon = ICONS[m.id];
            const body = (
              <>
                <Icon className="hb-more-item-icon size-[18px]" strokeWidth={1.8} aria-hidden />
                <span className="hb-more-item-text">
                  <span className="hb-more-item-title">{m.title}</span>
                  <span className="hb-more-item-desc">{m.description}</span>
                </span>
              </>
            );
            return (
              <li key={m.id}>
                {m.id === "strip" ? (
                  <DesktopViewLink target="strip" className="hb-more-item" data-tool={m.id}>
                    {body}
                  </DesktopViewLink>
                ) : (
                  <Link href={m.href as Route} className="hb-more-item" data-tool={m.id}>
                    {body}
                  </Link>
                )}
              </li>
            );
          })}
        </ul>
      ) : null}
    </div>
  );
}
