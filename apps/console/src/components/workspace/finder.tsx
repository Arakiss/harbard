"use client";

/**
 * ⌘K: one finder for agents, channels and messages, over what the view
 * holds. Arrows move, Enter opens, Escape closes. Nothing leaves the page.
 */

import type { Route } from "next";
import * as React from "react";
import { useRouter } from "next/navigation";
import { Hash, MessageSquare, Search, UserRound } from "lucide-react";
import { createT } from "@/i18n";
import { hhmm, search, type WorkspaceView } from "@/lib/workspace";

const t = createT("shell");

export interface FinderHit {
  kind: "agent" | "channel" | "message";
  id: string;
  title: string;
  sub: string;
  href: string;
}

/** The hits for a query, agents first, then channels, then messages. */
export function finderHits(view: WorkspaceView, query: string): FinderHit[] {
  const r = search(view, query);
  const agents: FinderHit[] = r.agents.slice(0, 6).map((a) => ({
    kind: "agent",
    id: `agent:${a.key}`,
    title: a.name,
    sub: a.handle ? `@${a.handle}` : a.label,
    href: `/direct/${encodeURIComponent(a.key)}`,
  }));
  const channels: FinderHit[] = r.channels.slice(0, 6).map((c) => ({
    kind: "channel",
    id: `channel:${c.id}`,
    title: `#${c.id}`,
    sub: c.quiet ? t("inMoreChannels") : c.members.length === 1 ? t("oneAgent") : t("agentsCount", { count: c.members.length }),
    href: `/c/${encodeURIComponent(c.id)}`,
  }));
  const messages: FinderHit[] = r.messages.slice(0, 8).map((m) => {
    const agent = view.agents.find((a) => a.key === m.agent);
    const where = m.channels[0];
    const href = agent && !where ? `/direct/${encodeURIComponent(agent.key)}` : where ? `/c/${encodeURIComponent(where)}?thread=${encodeURIComponent(m.reply_to ?? m.id)}` : "/";
    return {
      kind: "message",
      id: `message:${m.id}`,
      title: m.text,
      sub: `${agent?.name ?? m.author} · ${hhmm(m.ts_ms)}${where ? ` · #${where}` : ""}`,
      href,
    };
  });
  return [...agents, ...channels, ...messages];
}

const ICON = { agent: UserRound, channel: Hash, message: MessageSquare } as const;

/** The finder's content; `query` seeds it (tests, links). */
export function FinderDialog({ view, onClose, query: initial = "", onGo }: { view: WorkspaceView; onClose: () => void; query?: string; onGo: (href: string) => void }) {
  const [query, setQuery] = React.useState(initial);
  const [at, setAt] = React.useState(0);
  const hits = React.useMemo(() => finderHits(view, query), [view, query]);
  const input = React.useRef<HTMLInputElement>(null);
  React.useEffect(() => input.current?.focus(), []);
  const go = (h: FinderHit | undefined) => {
    if (!h) return;
    onClose();
    onGo(h.href);
  };
  return (
    <div className="hb-finder-scrim" onMouseDown={onClose}>
      <div className="hb-finder" role="dialog" aria-modal="true" aria-label={t("searchHarbard")} onMouseDown={(e) => e.stopPropagation()}>
        <label className="hb-finder-input">
          <Search className="size-5" aria-hidden />
          <input
            ref={input}
            value={query}
            placeholder={t("searchPlaceholder")}
            aria-label={t("searchPlaceholder")}
            role="combobox"
            aria-expanded={hits.length > 0}
            aria-controls="hb-finder-list"
            aria-activedescendant={hits[at] ? `hb-hit-${at}` : undefined}
            onChange={(e) => {
              setQuery(e.target.value);
              setAt(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setAt((i) => Math.min(i + 1, hits.length - 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setAt((i) => Math.max(i - 1, 0));
              } else if (e.key === "Enter") {
                e.preventDefault();
                go(hits[at]);
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                onClose();
              }
            }}
          />
          <kbd className="hb-kbd">{t("escapeKey")}</kbd>
        </label>
        <ul id="hb-finder-list" role="listbox" className="hb-finder-list" aria-label={t("results")}>
          {query.trim() && hits.length === 0 ? <li className="hb-finder-empty">{t("nothingMatches", { query: query.trim() })}</li> : null}
          {hits.map((h, i) => {
            const Icon = ICON[h.kind];
            return (
              <li
                key={h.id}
                id={`hb-hit-${i}`}
                role="option"
                aria-selected={i === at}
                className="hb-finder-hit"
                data-kind={h.kind}
                onMouseEnter={() => setAt(i)}
                onClick={() => go(h)}
              >
                <Icon className="size-4 shrink-0" aria-hidden />
                <span className="min-w-0">
                  <span className="hb-finder-title">{h.title}</span>
                  <span className="hb-finder-sub">{h.sub}</span>
                </span>
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}

/** ⌘K (Ctrl+K) anywhere in the shell opens the finder. */
export function Finder({ view }: { view: WorkspaceView | null }) {
  const [open, setOpen] = React.useState(false);
  const router = useRouter();
  React.useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    const openIt = () => setOpen(true);
    window.addEventListener("keydown", on);
    window.addEventListener("hb-open-finder", openIt);
    return () => {
      window.removeEventListener("keydown", on);
      window.removeEventListener("hb-open-finder", openIt);
    };
  }, []);
  if (!open || !view) return null;
  return <FinderDialog view={view} onClose={() => setOpen(false)} onGo={(href) => router.push(href as Route)} />;
}
