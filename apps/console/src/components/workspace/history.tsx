"use client";

import type { Route } from "next";
import * as React from "react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { History, Search } from "lucide-react";
import { fetchHistoryAgents } from "@/lib/api";
import {
  PAGE_SIZE,
  directMode,
  endingText,
  historyByDay,
  mergePage,
  spanText,
  type HistoryChannel,
  type HistoryItem,
} from "@/lib/history";
import { localRole } from "@/lib/serve-text";
import { agentRoleLine, shortPath, type WsAgent } from "@/lib/workspace";
import { AgentAvatar } from "./parts";
import { DeletedRooms } from "./rooms";
import { useView } from "./workspace-context";
import { createT } from "@/i18n";

const t = createT("history");

type State = {
  items: HistoryItem[];
  next: string | null;
  total: number;
  channels: HistoryChannel[];
  now: number;
  days: number;
  loading: boolean;
  error: string | null;
};

const EMPTY: State = { items: [], next: null, total: 0, channels: [], now: 0, days: 31, loading: true, error: null };

/** The role line under a name: "builder · harbard". */
function roleLine(item: HistoryItem): string | null {
  const a = item.agent;
  if (a.role_line) return agentRoleLine(a);
  const parts = [a.role && localRole(a.role), a.team].filter(Boolean);
  return parts.length ? parts.join(" · ") : null;
}

function Row({ item, now, live }: { item: HistoryItem; now: number; live: WsAgent | null }) {
  const a = item.agent;
  const role = roleLine(item);
  // The same rule Direct opens it by: the shell's live copy, when it has one.
  const readOnly = directMode(live, item) === "read-only";
  return (
    <li>
      <Link
        href={`/direct/${encodeURIComponent(a.key)}` as Route}
        className="hb-hist-item"
        data-ending={item.ending ?? "abierto"}
        aria-label={readOnly ? t("rowLabelReadOnly", { who: role ? `${a.name}, ${role}` : a.name, ending: endingText(item) }) : t("rowLabel", { who: role ? `${a.name}, ${role}` : a.name, ending: endingText(item) })}
        data-readonly={readOnly || undefined}
      >
        <AgentAvatar name={a.name} agent={a} size={40} dot={false} />
        <span className="hb-hist-body">
          <span className="hb-hist-top">
            <span className="hb-hist-name">{a.name}</span>
            {role ? <span className="hb-hist-role">{role}</span> : null}
            <span className="hb-hist-ending" data-ending={item.ending ?? "abierto"}>
              {endingText(item)}
            </span>
          </span>
          <span className="hb-hist-meta">
            {item.channel ? <span>#{item.channel}</span> : <span>{t("noChannel")}</span>}
            {item.repo ? <span className="hb-mono">{shortPath(item.repo)}</span> : null}
            <span>{spanText(item, now)}</span>
          </span>
          {item.last_line ? <span className="hb-hist-line">{item.last_line}</span> : null}
        </span>
      </Link>
    </li>
  );
}

/**
 * History, under "More tools": the month the shell does not keep.
 * One page at a time, the next one asked for as the end of the list comes
 * into view; the filter and the search are serve's, so they reach the
 * whole month and not only what is loaded.
 */
export function HistoryScreen() {
  const router = useRouter();
  const pathname = usePathname() ?? "/history";
  const params = useSearchParams();
  const [channel, setChannel] = React.useState(() => params?.get("channel") ?? "");
  const [q, setQ] = React.useState(() => params?.get("q") ?? "");
  const [query, setQuery] = React.useState(q);
  const [state, setState] = React.useState<State>(EMPTY);
  const view = useView();
  const live = React.useMemo(() => new Map((view?.agents ?? []).map((a) => [a.key, a])), [view]);
  const busy = React.useRef<AbortController | null>(null);
  const sentinel = React.useRef<HTMLDivElement>(null);

  // The search waits for a pause in typing; the URL keeps both choices.
  React.useEffect(() => {
    const t = setTimeout(() => setQuery(q), 250);
    return () => clearTimeout(t);
  }, [q]);
  React.useEffect(() => {
    const p = new URLSearchParams();
    if (channel) p.set("channel", channel);
    if (query.trim()) p.set("q", query.trim());
    const s = p.toString();
    router.replace((s ? `${pathname}?${s}` : pathname) as Route, { scroll: false });
  }, [channel, query, pathname, router]);

  const load = React.useCallback(
    async (before: string | null) => {
      busy.current?.abort();
      const controller = new AbortController();
      busy.current = controller;
      setState((s) => ({ ...s, loading: true, error: null, ...(before ? {} : { items: [], next: null }) }));
      try {
        const page = await fetchHistoryAgents({ before, channel, q: query, limit: PAGE_SIZE }, controller.signal);
        setState((s) => ({
          items: before ? mergePage(s.items, page.items) : page.items,
          next: page.next,
          total: page.total,
          channels: page.channels,
          now: page.now_ms,
          days: page.window_days,
          loading: false,
          error: null,
        }));
      } catch (error) {
        if (error instanceof DOMException && error.name === "AbortError") return;
        setState((s) => ({ ...s, loading: false, error: error instanceof Error ? error.message : String(error) }));
      }
    },
    [channel, query],
  );

  React.useEffect(() => {
    void load(null);
    return () => busy.current?.abort();
  }, [load]);

  // The next page when the end of the list is near.
  const { next, loading } = state;
  React.useEffect(() => {
    const el = sentinel.current;
    if (!el || !next || loading) return;
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) void load(next);
    }, { rootMargin: "600px 0px" });
    io.observe(el);
    return () => io.disconnect();
  }, [next, loading, load]);

  const now = state.now || Date.now();
  const groups = historyByDay(state.items, now);
  const filtered = Boolean(channel || query.trim());
  return (
    <main className="hb-main" aria-label={t("title")}>
      <header className="hb-head">
        <div className="hb-head-title">
          <History className="size-5" aria-hidden /> {t("title")}
        </div>
        <span className="hb-topic">{t("topic")}</span>
      </header>
      <div className="hb-hist-tools">
        <label className="hb-search hb-hist-search">
          <Search className="size-5" aria-hidden />
          <input
            placeholder={t("searchPlaceholder")}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            aria-label={t("searchLabel")}
          />
        </label>
        <select className="hb-hist-channel" value={channel} onChange={(e) => setChannel(e.target.value)} aria-label={t("channel")}>
          <option value="">{t("allChannels")}</option>
          {state.channels.map((c) => (
            <option key={c.id} value={c.id}>
              #{c.id} ({c.items})
            </option>
          ))}
          {channel && !state.channels.some((c) => c.id === channel) ? <option value={channel}>#{channel}</option> : null}
        </select>
        <span className="hb-hist-count" aria-live="polite">
          {state.loading && state.items.length === 0
            ? t("readingMonth")
            : state.total === 1
              ? t("countOne", { count: state.total, days: state.days })
              : t("countMany", { count: state.total, days: state.days })}
        </span>
      </div>
      <div className="hb-stream hb-hist-list">
        <DeletedRooms now={now} />
        {state.error ? (
          <div className="hb-empty" role="alert">
            <strong>{t("loadFailed")}</strong>
            {state.error}
            <button type="button" className="hb-hist-more" onClick={() => void load(null)}>
              {t("retry")}
            </button>
          </div>
        ) : null}
        {!state.loading && !state.error && state.items.length === 0 ? (
          <div className="hb-empty">
            <strong>{filtered ? t("noMatches") : t("emptyTitle")}</strong>
            {filtered ? t("noMatchesHint") : t("emptyHint")}
          </div>
        ) : null}
        {groups.map((g) => (
          <section key={g.day} aria-label={g.day} className="hb-hist-day">
            <div className="hb-day">
              <span>{g.day}</span>
            </div>
            <ul className="hb-hist-ul">
              {g.items.map((i) => (
                <Row key={i.agent.key} item={i} now={now} live={live.get(i.agent.key) ?? null} />
              ))}
            </ul>
          </section>
        ))}
        {state.next ? (
          <div ref={sentinel} className="hb-hist-foot">
            <button type="button" className="hb-hist-more" disabled={state.loading} onClick={() => void load(state.next)}>
              {state.loading ? t("loading") : t("loadMore")}
            </button>
          </div>
        ) : state.items.length > 0 ? (
          <div className="hb-hist-foot">{t("endOfMonth")}</div>
        ) : null}
      </div>
    </main>
  );
}
