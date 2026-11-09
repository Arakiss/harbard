"use client";

import type { Route } from "next";
import * as React from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Bookmark, Search } from "lucide-react";
import { atDate, fileName, hhmm, messageById, search } from "@/lib/workspace";
import { AgentAvatar, Avatar, FileCard, Loading, MessageItem } from "./parts";
import { useView, useWorkspace } from "./workspace-context";
import { createT } from "@/i18n";

const t = createT("later");

export function LaterScreen() {
  const { state, now, post } = useWorkspace();
  const view = useView();
  if (!view) {
    return (
      <div className="hb-main">
        <Loading state={state} />
      </div>
    );
  }
  return (
    <main className="hb-main" aria-label={t("title")}>
      <header className="hb-head">
        <div className="hb-head-title">
          <Bookmark className="size-5" aria-hidden /> {t("title")}
        </div>
        <span className="hb-topic">{t("topic")}</span>
      </header>
      <div className="hb-stream">
        {view.saved.length === 0 ? (
          <div className="hb-empty">
            <strong>{t("emptyTitle")}</strong>
            {t("emptyHint")}
          </div>
        ) : null}
        {view.saved.map((s) => {
          const live = messageById(view, s.id);
          const href = live?.channels[0]
            ? `/c/${encodeURIComponent(live.channels[0])}?thread=${encodeURIComponent(live.reply_to ?? live.id)}`
            : live
              ? `/direct/${encodeURIComponent(live.agent)}`
              : null;
          return (
            <div key={s.id} className="hb-msg">
              {s.kind === "message" && s.agent ? (
                <AgentAvatar name={view.agents.find((a) => a.key === live?.agent)?.name ?? s.agent} agent={view.agents.find((a) => a.key === live?.agent) ?? null} />
              ) : (
                <Avatar name={s.agent ?? (s.path ? fileName(s.path) : "harbard")} />
              )}
              <div className="hb-msg-body">
                <div className="hb-msg-meta">
                  <span className="hb-msg-name">{s.kind === "report" ? t("report") : (s.agent ?? t("message"))}</span>
                  {s.channel ? <span className="hb-msg-time">#{s.channel}</span> : null}
                  {s.ts_ms ? <span className="hb-msg-time">{hhmm(s.ts_ms)}</span> : null}
                  <span className="hb-msg-time">{t("savedWhen", { when: atDate(s.saved_ms, now) })}</span>
                </div>
                <div className="hb-msg-text">{s.text}</div>
                {s.path ? <FileCard path={s.path} /> : null}
                <div className="mt-2 flex gap-4 text-[13px]">
                  {href ? (
                    <Link href={href as Route} className="font-bold text-[var(--hb-link)] hover:underline">
                      {t("viewInContext")}
                    </Link>
                  ) : s.kind === "message" ? (
                    <span className="text-[var(--hb-ink-3)]">{t("outOfWindow")}</span>
                  ) : null}
                  <button type="button" className="font-bold text-[var(--hb-ink-3)] hover:underline" onClick={() => void post("unsave", { id: s.id }).catch(() => {})}>
                    {t("remove")}
                  </button>
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </main>
  );
}

export function SearchScreen() {
  const { state } = useWorkspace();
  const view = useView();
  const router = useRouter();
  const params = useSearchParams();
  const [q, setQ] = React.useState(() => params?.get("q") ?? "");
  React.useEffect(() => {
    const t = setTimeout(() => router.replace((q ? `/search?q=${encodeURIComponent(q)}` : "/search") as Route, { scroll: false }), 250);
    return () => clearTimeout(t);
  }, [q, router]);
  if (!view) {
    return (
      <div className="hb-main">
        <Loading state={state} />
      </div>
    );
  }
  const r = search(view, q);
  const total = r.agents.length + r.messages.length + r.reports.length;
  return (
    <main className="hb-main" aria-label={t("search")}>
      <header className="hb-head">
        <label className="hb-search m-0 flex-1">
          <Search className="size-5" aria-hidden />
          <input autoFocus placeholder={t("searchPlaceholder")} value={q} onChange={(e) => setQ(e.target.value)} aria-label={t("searchPlaceholder")} />
        </label>
      </header>
      <div className="hb-stream hb-results">
        {!q.trim() ? <div className="hb-empty">{t("searchHint")}</div> : null}
        {q.trim() && total === 0 ? <div className="hb-empty">{t("noMatches", { query: q.trim() })}</div> : null}
        {r.agents.length > 0 ? <h2>{t("agents")}</h2> : null}
        {r.agents.map((a) => (
          <Link key={a.key} href={`/direct/${encodeURIComponent(a.key)}`} className="hb-msg">
            <AgentAvatar name={a.name} agent={a} />
            <div className="hb-msg-body">
              <div className="hb-msg-name">{a.name}</div>
              <div className="text-[14px] text-[var(--hb-ink-3)]">{a.description ?? a.brief ?? ""}</div>
            </div>
          </Link>
        ))}
        {r.reports.length > 0 ? <h2>{t("reports")}</h2> : null}
        {r.reports.map((x) => (
          <div key={x.path} className="px-5 py-1">
            <FileCard path={x.path} />
          </div>
        ))}
        {r.messages.length > 0 ? <h2>{t("messages")}</h2> : null}
        {r.messages.slice(0, 100).map((m) => (
          <MessageItem key={m.id} view={view} message={m} showReplies={false} />
        ))}
      </div>
    </main>
  );
}
