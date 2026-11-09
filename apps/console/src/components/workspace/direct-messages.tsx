"use client";

import * as React from "react";
import Link from "next/link";
import type { Route } from "next";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { Archive, History, Radio, Search, SquarePen, UserRound, X } from "lucide-react";
import { ApiUnavailable, fetchHistoryConversation } from "@/lib/api";
import { localRole } from "@/lib/serve-text";
import { readOnlyNotice, directMode, endingText, spanText, type HistoryConversation } from "@/lib/history";
import {
  agentRoleLine,
  deliveryReason,
  presenceText,
  composerBlock,
  composerConfirm,
  composerNotice,
  conversation,
  agentRelation,
  dmHint,
  noteReader,
  nameOrigin,
  dotTone,
  hhmm,
  messageById,
  operatorName,
  presenceLine,
  shortPath,
  type WsAgent,
  type WorkspaceView,
} from "@/lib/workspace";
import { AgentHover } from "./agent-card";
import { HiddenGroup } from "./hidden-group";
import { directAgents, hiddenAgents, hideAction } from "@/lib/hidden";
import { LiveView } from "./live-pane";
import { AgentAvatar, AgentName, AgentStatus, Composer, FileCard, Loading, NameVotes, RowMenu, Stream, useEscRead } from "./parts";
import { useView, useWorkspace } from "./workspace-context";
import { createT } from "@/i18n";

const t = createT("direct");

function AgentList({ view, active, query, setQuery }: { view: WorkspaceView; active: string | null; query: string; setQuery: (q: string) => void }) {
  const { post } = useWorkspace();
  const list = directAgents(view, query);
  const hidden = query.trim() ? [] : hiddenAgents(view);
  return (
    <section className="hb-list" aria-label={t("title")}>
      <div className="hb-list-head">
        <h1 className="hb-list-title">{t("title")}</h1>
        {view.unread.directs > 0 ? <span className="hb-unread">{t("unreadCount", { count: view.unread.directs })}</span> : <span className="ml-auto" />}
        <Link href="/direct" className="hb-icon-btn border-0" aria-label={t("newMessage")} title={t("newMessage")}>
          <SquarePen className="size-5" />
        </Link>
      </div>
      <label className="hb-search">
        <Search className="size-5" aria-hidden />
        <input placeholder={t("searchAgents")} value={query} onChange={(e) => setQuery(e.target.value)} aria-label={t("searchAgents")} />
      </label>
      <div className="hb-list-scroll">
        {list.length === 0 ? <div className="hb-empty">{t("noAgentsMatch")}</div> : null}
        {list.map((a) => {
          const last = a.last_message ? messageById(view, a.last_message) : undefined;
          return (
            <AgentHover key={a.key} agent={a} block>
            <RowMenu
              label={a.name}
              onRead={() => void post("read", { agent: a.key, ts_ms: Date.now() }).catch(() => {})}
              actions={[{ label: hideAction(a).label, run: () => void post(hideAction(a).action, { agent: a.key }).catch(() => {}) }]}
            >
            <Link href={`/direct/${encodeURIComponent(a.key)}`} className="hb-item" aria-current={active === a.key ? "true" : undefined}>
              <AgentAvatar name={a.name} size={48} agent={a} />
              <span className="hb-item-body">
                <span className="hb-item-top">
                  <AgentName agent={a} className="hb-item-name" />
                  {a.unread > 0 ? <span className="hb-count ml-auto">{a.unread}</span> : last ? <span className="hb-item-time">{hhmm(last.ts_ms)}</span> : null}
                </span>
                <AgentStatus agent={a} className="hb-item-status" />
                <span className="hb-item-preview">
                  {last ? (last.from === "agent" ? "" : `${last.from === "operator" ? operatorName(last.author) : last.author}: `) + last.text : (a.description ?? presenceText(a.presence))}
                </span>
              </span>
            </Link>
            </RowMenu>
            </AgentHover>
          );
        })}
        <HiddenGroup agents={hidden} className="hb-hidden-toggle" />
      </div>
    </section>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="hb-field">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

function Profile({ agent, agents, now, onClose }: { agent: WsAgent; agents: WsAgent[]; now: number; onClose: () => void }) {
  return (
    <aside className="hb-pane" aria-label={t("profile")}>
      <div className="hb-pane-head">
        <span className="hb-pane-title">{t("profile")}</span>
        <button type="button" className="hb-close" onClick={onClose} aria-label={t("closeProfile")}>
          <X className="size-5" />
        </button>
      </div>
      <dl className="hb-profile">
        <div className="flex items-center gap-3">
          <AgentAvatar name={agent.name} size={56} agent={agent} />
          <div>
            <div className="text-[19px] font-black">
              <AgentName agent={agent} editable /> <NameVotes agent={agent} />
            </div>
            <div className="text-[14px] text-[var(--hb-ink-3)]">{agentRelation(agent, agents)}</div>
          </div>
        </div>
        {nameOrigin(agent) ? <Field label={t("fieldName")}>{nameOrigin(agent)}{agent.own_name ? ` · ${t("callsItself", { name: agent.own_name })}` : ""}</Field> : null}
        <Field label={t("fieldIdentifier")}>
          <span className="hb-mono break-all">{agent.key}</span>
          <span className="hb-mono block text-[13px] text-[var(--hb-ink-3)]">{agent.label}</span>
        </Field>
        <Field label={agent.kind === "session" ? t("fieldSession") : t("fieldLeadSession")}>
          <span className="hb-mono break-all">{agent.lead_session}</span>
        </Field>
        {agent.handle ? (
          <Field label={t("fieldMention")}>
            <span className="hb-mono">@{agent.handle}</span>
            {agent.team ? ` · ${t("teamMention", { team: agent.team })}` : ""}
          </Field>
        ) : null}
        <Field label={t("fieldBrief")}>{agent.brief ?? agent.description ?? t("noData")}</Field>
        {agent.description && agent.brief ? <Field label={t("fieldTask")}>{agent.description}</Field> : null}
        <Field label={t("fieldCopy")}>{agent.checkout ? <span className="hb-mono">{shortPath(agent.checkout)}</span> : t("noCopy")}</Field>
        <Field label={t("fieldBranch")}>{agent.branch ? <span className="hb-mono">{agent.branch}</span> : t("noData")}</Field>
        {agent.build_dir ? (
          <Field label={t("fieldBuildDir")}>
            <span className="hb-mono">{agent.build_dir}</span>
          </Field>
        ) : null}
        <Field label={t("fieldNow")}>
          <AgentStatus agent={agent} />
        </Field>
        <Field label={t("fieldStatus")}>
          {presenceLine(agent, now)}
          {agent.first_ms ? `; ${t("startedAt", { time: hhmm(agent.first_ms) })}` : ""}
        </Field>
        <Field label={t("fieldDeliveries")}>
          {agent.deliveries === 0
            ? t("deliveriesNone")
            : t("deliveriesLine", {
                deliveries: agent.deliveries === 1 ? t("deliveryOne", { count: agent.deliveries }) : t("deliveryMany", { count: agent.deliveries }),
                verified: agent.verified_deliveries === 1 ? t("verifiedOne", { count: agent.verified_deliveries }) : t("verifiedMany", { count: agent.verified_deliveries }),
              })}
        </Field>
        <Field label={t("fieldReports")}>
          {agent.reports.length === 0 ? (
            t("reportsNone")
          ) : (
            <div className="hb-files flex-col">
              {agent.reports.map((r) => (
                <FileCard key={r} path={r} />
              ))}
            </div>
          )}
        </Field>
        <Field label={t("fieldMessages")}>
          {agent.delivery === "ready"
            ? dmHint(agent, noteReader(agents, agent.key))
            : agent.delivery === "stale"
              ? t("messagesSavedAsNote", { notice: composerNotice([agent]) ?? "" })
              : t("messagesSavedUnread", { reason: deliveryReason(agent.delivery) })}
        </Field>
        {/* No stop button: harbard has no way to stop a Claude Code teammate or subagent. */}
      </dl>
    </aside>
  );
}

/** Where a read-only conversation has its composer: why there is none, and the way back to History. */
function ReadOnlyBar({ line }: { line?: string }) {
  return (
    <div className="hb-composer">
      <div className="hb-readonly" role="note">
        <Archive className="size-5 flex-none" aria-hidden />
        <span>
          {readOnlyNotice()}
          {line ? <span className="hb-readonly-line">{line}</span> : null}
        </span>
        <Link href="/history" className="hb-readonly-link">
          <History className="size-4" aria-hidden /> {t("history")}
        </Link>
      </div>
    </div>
  );
}

/**
 * An agent the shell no longer has, from History: undefined while serve
 * answers, null when the month has no such agent either.
 */
function useArchived(key: string | null): { conv: HistoryConversation | null | undefined; error: string | null } {
  const [got, setGot] = React.useState<{ key: string | null; conv: HistoryConversation | null | undefined; error: string | null }>({ key: null, conv: undefined, error: null });
  React.useEffect(() => {
    if (!key) return;
    const controller = new AbortController();
    fetchHistoryConversation(key, controller.signal)
      .then((conv) => setGot({ key, conv, error: null }))
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === "AbortError") return;
        if (error instanceof ApiUnavailable && error.status === 404) setGot({ key, conv: null, error: null });
        else setGot({ key, conv: null, error: error instanceof Error ? error.message : String(error) });
      });
    return () => controller.abort();
  }, [key]);
  return got.key === key ? { conv: got.conv, error: got.error } : { conv: undefined, error: null };
}

/** The conversation of an agent only History has: read-only, its tools hidden. */
function ArchivedDirect({ view, conv, now }: { view: WorkspaceView; conv: HistoryConversation; now: number }) {
  const a = conv.item.agent;
  // The stream looks names and avatars up in a view: the shell's, with this agent and its messages.
  const archived = React.useMemo<WorkspaceView>(
    () => ({ ...view, agents: [a, ...view.agents.filter((x) => x.key !== a.key)], messages: conv.messages, saved: [] }),
    [view, a, conv.messages],
  );
  const role = (a.role_line && agentRoleLine(a)) ?? ([a.role && localRole(a.role), a.team].filter(Boolean).join(" · ") || null);
  return (
    <main className="hb-detail" aria-label={t("conversationReadOnly", { name: a.name })} data-readonly="true">
      <header className="hb-dm-head">
        <AgentAvatar name={a.name} size={64} agent={a} dot={false} />
        <div className="min-w-0">
          <div className="hb-dm-name">{a.name}</div>
          <div className="hb-dm-presence">
            <span className="hb-presence-dot" data-tone="grey" aria-hidden />
            {[role, endingText(conv.item), spanText(conv.item, conv.now_ms)].filter(Boolean).join(" · ")}
          </div>
        </div>
      </header>
      <Stream
        view={archived}
        messages={conv.messages}
        now={now}
        className="hb-stream-dm"
        readOnly
        empty={<div className="hb-empty">{t("noMessagesThisMonth", { name: a.name })}</div>}
      />
      {conv.truncated ? <div className="hb-hist-foot">{t("onlyRecent")}</div> : null}
      <ReadOnlyBar line={conv.item.channel ? t("workedIn", { channel: conv.item.channel }) : undefined} />
    </main>
  );
}

export function DirectsScreen({ agent }: { agent: string | null }) {
  const { state, now, post } = useWorkspace();
  const view = useView();
  const [query, setQuery] = React.useState("");
  const params = useSearchParams();
  const [profile, setProfile] = React.useState(() => params?.get("profile") === "1");
  const router = useRouter();
  const pathname = usePathname() ?? "/direct";
  // "See it live": the agent's live view in place of the conversation..
  const direct = params?.get("direct") === "1";
  const setDirect = (on: boolean) => router.replace((on ? `${pathname}?direct=1` : pathname) as Route, { scroll: false });
  const selected = view?.agents.find((a) => a.key === agent) ?? null;
  // Not in the shell's day: History has it, read-only.
  const archived = useArchived(view && agent && !selected ? agent : null);
  const mode = directMode(selected, archived.conv === undefined ? undefined : (archived.conv?.item ?? null));
  const messages = React.useMemo(() => (view && selected ? conversation(view, selected.key) : []), [view, selected]);
  const lastTs = messages.filter((m) => m.from === "agent").at(-1)?.ts_ms ?? 0;
  const unread = selected?.unread ?? 0;
  React.useEffect(() => {
    if (selected && unread > 0 && lastTs > 0) void post("read", { agent: selected.key, ts_ms: lastTs }).catch(() => {});
  }, [selected, unread, lastTs, post]);
  const selectedKey = selected?.key ?? null;
  const markRead = React.useCallback(() => {
    if (selectedKey) void post("read", { agent: selectedKey, ts_ms: Date.now() }).catch(() => {});
  }, [selectedKey, post]);
  useEscRead(selectedKey ? markRead : null);
  if (!view) {
    return (
      <div className="hb-main">
        <Loading state={state} />
      </div>
    );
  }
  return (
    <>
      <AgentList view={view} active={agent} query={query} setQuery={setQuery} />
      {selected && direct ? (
        <LiveView key={selected.key} agent={selected} onOpenChat={() => setDirect(false)} />
      ) : selected ? (
        <main className="hb-detail" aria-label={t("conversationWith", { name: selected.name })}>
          <header className="hb-dm-head">
            <AgentAvatar name={selected.name} size={64} agent={selected} dot={false} />
            <div className="min-w-0">
              <div className="hb-dm-name">
                <AgentName agent={selected} editable />
              </div>
              <div className="hb-dm-presence">{agentRelation(selected, view.agents)}</div>
              <div className="hb-dm-presence">
                <span className="hb-presence-dot" data-tone={dotTone(selected)} aria-hidden />
                {selected.lost ? presenceLine(selected, now) : <AgentStatus agent={selected} />}
              </div>
            </div>
            <div className="hb-head-right">
              <button type="button" className="hb-icon-btn" onClick={() => setDirect(true)} aria-label={t("watchLive")} title={t("watchLive")}>
                <Radio className="size-5" />
              </button>
              <button type="button" className="hb-icon-btn" aria-pressed={profile} onClick={() => setProfile(!profile)} aria-label={t("profile")} title={t("profile")}>
                <UserRound className="size-5" />
              </button>
            </div>
          </header>
          <Stream
            view={view}
            messages={messages}
            now={now}
            className="hb-stream-dm"
            readOnly={mode === "read-only"}
            empty={<div className="hb-empty">{t("noMessagesYet", { name: selected.name })}</div>}
          />
          {mode === "read-only" ? (
            <ReadOnlyBar />
          ) : (
            <Composer
              target={{ agent: selected.key }}
              placeholder={t("writeTo", { name: selected.name })}
              blocked={composerBlock([selected])}
              notice={composerNotice([selected])}
              confirm={composerConfirm([selected])}
              hint={dmHint(selected, noteReader(view.agents, selected.key))}
            />
          )}
        </main>
      ) : agent && archived.conv ? (
        <ArchivedDirect view={view} conv={archived.conv} now={now} />
      ) : agent && mode === "loading" ? (
        <main className="hb-detail">
          <div className="hb-empty">{t("searchingHistory")}</div>
        </main>
      ) : agent ? (
        <main className="hb-detail">
          <div className="hb-empty">
            <strong>{t("notInLastMonth")}</strong>
            {archived.error ?? t("agentNotFound")}
          </div>
        </main>
      ) : (
        <main className="hb-detail">
          <div className="hb-empty">
            <strong>{t("chooseAgent")}</strong>
            {t("chooseAgentHint")}
          </div>
        </main>
      )}
      {selected && profile ? <Profile agents={view.agents} agent={selected} now={now} onClose={() => setProfile(false)} /> : null}
    </>
  );
}
