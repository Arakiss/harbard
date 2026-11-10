"use client";

import type { Route } from "next";
import * as React from "react";
import Link from "next/link";
import { createT } from "@/i18n";
import { openWorkspaceMenu } from "@/lib/workspace-menu-bus";
import { currentName, setCurrentWorkspace, workingElsewhere, type WorkspaceTile } from "@/lib/current-workspace";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { AlertTriangle, ChevronDown, Hash, Info, MessageCircle, MessageSquare, PackageCheck, Plus, Users, X } from "lucide-react";
import {
  agentLabel,
  channelComposer,
  channelStream,
  composerBlock,
  composerConfirm,
  composerNotice,
  dmHint,
  noteReader,
  fileName,
  hhmm,
  messageById,
  filterAgents,
  shortPath,
  sidebarAgents,
  sidebarChannels,
  temporaryAgents,
  thread,
  threads,
  type WsAgent,
  type WsChannel,
  type WorkspaceView,
} from "@/lib/workspace";
import { AgentHover } from "./agent-card";
import { ChannelMembers } from "./channel-members";
import { HiddenGroup } from "./hidden-group";
import { DeliveriesView } from "./deliveries";
import { deliveriesHint, pendingCount } from "@/lib/deliveries";
import { channelHeadline, channelPlaces } from "@/lib/legible";
import { channelMembers } from "@/lib/members";
import { aboutWork, workItems } from "@/lib/work-items";
import { WorkPanel } from "./work-panel";
import { hiddenAgents, hideAction } from "@/lib/hidden";
import { MoreTools } from "./more-tools";
import { AgentAvatar, AgentName, AgentStatus, Composer, Loading, MessageItem, RowMenu, Stream, useEscRead, type RowAction } from "./parts";
import { channelHref, defaultChannelId, findChannel, isRoom, sidebarRooms } from "@/lib/rooms";
import { ArchivedStrip, ChannelMenu, CreateRoomDialog, RoomView, useChannelActions } from "./rooms";
import { useAgentLive, useView, useWorkspace } from "./workspace-context";

const t = createT("home");
const tr = createT("rooms");

function ChannelRow({ c, active, actions }: { c: WsChannel; active: string | null; actions?: RowAction[] }) {
  const { post } = useWorkspace();
  return (
    <RowMenu label={`#${c.id}`} onRead={() => void post("read-channel", { channel: c.id }).catch(() => {})} actions={actions}>
      <Link href={channelHref(c.id) as Route} className="hb-row" aria-current={active === c.id ? "page" : undefined} title={isRoom(c) ? (c.purpose ?? undefined) : shortPath(c.repo)} data-unread={c.unread ? "true" : undefined} data-channel-row={c.id}>
        {/* A room is a conversation, a project channel a repository: they look apart before they are read. */}
        <span className="hb-row-hash" aria-hidden>
          {isRoom(c) ? <MessageCircle className="size-4" strokeWidth={2} /> : "#"}
        </span>
        <span className="truncate">{c.id}</span>
        {c.warnings.length > 0 ? <AlertTriangle className="hb-row-warn size-4" aria-label={t("twoAgentsSameCheckout")} /> : null}
        {c.unread ? <span className="hb-row-unread" aria-label={t("unreadCount", { count: c.unread })}>{c.unread}</span> : null}
      </Link>
    </RowMenu>
  );
}

/**
 * An agent: its robot, its name with where it works, and what it is doing.
 * One with nothing to do takes a single line, so the sidebar's height goes
 * to the agents that are working.
 */
function AgentRow({ a }: { a: WsAgent }) {
  const { post } = useWorkspace();
  const hide = hideAction(a);
  const idle = useAgentLive(a).activity?.state === "normal";
  return (
    <AgentHover agent={a} block>
      <RowMenu label={a.name} actions={[{ label: hide.label, run: () => void post(hide.action, { agent: a.key }).catch(() => {}) }]}>
        <Link href={`/direct/${encodeURIComponent(a.key)}`} className="hb-row hb-agent-row" title={a.description ?? a.name} data-agent={a.key} data-idle={idle ? "" : undefined}>
          <AgentAvatar name={a.name} size={idle ? 24 : 32} agent={a} />
          <span className="hb-agent-text">
            <AgentName agent={a} className="hb-name" />
            <AgentStatus agent={a} />
          </span>
        </Link>
      </RowMenu>
    </AgentHover>
  );
}

/** A main session: named, with where it works; its subagents report to it. */
function SessionRow({ a }: { a: WsAgent }) {
  return <AgentRow a={{ ...a, role_line: a.channels[0] ?? null, role_line_code: null }} />;
}

/** The way into the Lobby, and how many are present now. */
function LobbyRow({ view, current }: { view: WorkspaceView; current: boolean }) {
  const here = (view.lobby ?? []).filter((e) => e.presence === "live");
  return (
    <Link href="/lobby" className="hb-row hb-lobby-link" aria-current={current ? "page" : undefined} aria-label={tr("lobbyEntryAria", { count: here.length })} data-lobby-link>
      <Users className="size-[18px]" strokeWidth={1.8} aria-hidden />
      {tr("lobbyEntry")}
      {here.length > 0 ? <span className="hb-row-count">{here.length}</span> : null}
    </Link>
  );
}

/** Below this many agents and sessions the list is read at a glance; a filter would only take room. */
const FILTER_FROM = 8;

function Sidebar({ view, active }: { view: WorkspaceView; active: string | null }) {
  const pathname = usePathname() ?? "/";
  const [moreChannels, setMoreChannels] = React.useState(false);
  const [temps, setTemps] = React.useState(false);
  const [filter, setFilter] = React.useState("");
  const { sessions, shown, rest, inactive } = sidebarAgents(view);
  const [showInactive, setShowInactive] = React.useState(false);
  const [all, setAll] = React.useState(false);
  // The filter searches every agent listed, inactive ones too.
  const listed = sessions.length + shown.length + rest.length + inactive.length;
  const agents = filterAgents(filter.trim() ? [...shown, ...rest, ...inactive] : all ? [...shown, ...rest] : shown, filter);
  const channels = sidebarChannels(view);
  const { rooms, archived } = sidebarRooms(view);
  const { itemsFor, dialog: channelDialog } = useChannelActions();
  const [creating, setCreating] = React.useState(false);
  const [showArchived, setShowArchived] = React.useState(false);
  const temporary = temporaryAgents(view);
  const pendingDeliveries = pendingCount(view);
  const hidden = hiddenAgents(view);
  const { post } = useWorkspace();
  // Folded on the desk; the click shows at once, serve's answer follows.
  const [agentsFolded, setAgentsFolded] = React.useState(view.agents_collapsed === true);
  React.useEffect(() => setAgentsFolded(view.agents_collapsed === true), [view.agents_collapsed]);
  return (
    <aside className="hb-sidebar" aria-label={t("home")}>
      <button
        type="button"
        className="hb-sidebar-head hb-ws-name"
        aria-haspopup="menu"
        aria-label={t("menuOf", { name: currentName(view.workspaces) })}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          openWorkspaceMenu({ left: r.left + 10, bottom: r.bottom - 8 });
        }}
      >
        <span className="hb-ws-name-label">
          {currentName(view.workspaces)} <ChevronDown className="size-5" strokeWidth={2.4} aria-hidden />
        </span>
      </button>
      <div className="hb-sidebar-scroll">
        <LobbyRow view={view} current={pathname.startsWith("/lobby")} />
        <Link href="/threads" className="hb-row" aria-current={pathname.startsWith("/threads") ? "page" : undefined}>
          <MessageSquare className="size-[18px]" strokeWidth={1.8} aria-hidden /> {t("threads")}
        </Link>
        <Link
          href="/deliveries"
          className="hb-row"
          aria-current={pathname.startsWith("/deliveries") ? "page" : undefined}
          title={deliveriesHint()}
          aria-label={t("deliveriesAria", { count: pendingDeliveries, hint: deliveriesHint().toLowerCase() })}
        >
          <PackageCheck className="size-[18px]" strokeWidth={1.8} aria-hidden /> {t("deliveries")}
          <span className="hb-row-count" aria-hidden>
            {pendingDeliveries}
          </span>
        </Link>

        <div className="hb-section">{t("channels")}</div>
        {channels.shown.length + channels.more.length === 0 ? (
          <p className="px-[10px] text-[14px] text-[var(--hb-sidebar-ink-2)]">{t("noCheckoutsYet")}</p>
        ) : null}
        {channels.shown.map((c) => (
          <ChannelRow key={c.id} c={c} active={active} actions={itemsFor(c)} />
        ))}
        {channels.more.length > 0 ? (
          <>
            <button type="button" className="hb-row text-[14px] text-[var(--hb-sidebar-ink-2)]" aria-expanded={moreChannels} onClick={() => setMoreChannels(!moreChannels)}>
              <ChevronDown className="size-4" style={{ transform: moreChannels ? undefined : "rotate(-90deg)" }} aria-hidden /> {t("moreChannels", { count: channels.more.length })}
            </button>
            {moreChannels ? channels.more.map((c) => <ChannelRow key={c.id} c={c} active={active} actions={itemsFor(c)} />) : null}
          </>
        ) : null}

        {sessions.length > 0 ? (
          <>
            <div className="hb-section" title={t("mainSessionsHint")}>
              {t("mainSessions")}
            </div>
            {sessions.map((a) => (
              <SessionRow key={a.key} a={a} />
            ))}
          </>
        ) : null}
        <button
          type="button"
          className="hb-section hb-section-toggle"
          aria-expanded={!agentsFolded}
          onClick={() => {
            const next = !agentsFolded;
            setAgentsFolded(next);
            void post("agents-collapsed", { collapsed: next }).catch(() => setAgentsFolded(!next));
          }}
        >
          {t("agents")}
          <ChevronDown className="hb-section-chevron size-4" style={{ transform: agentsFolded ? "rotate(-90deg)" : undefined }} aria-hidden />
        </button>
        {agentsFolded ? null : (
        <>
        {listed > FILTER_FROM || filter ? (
        <label className="hb-side-filter">
          <span className="sr-only">{t("filterAgents")}</span>
          <input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder={t("filterAgents")} aria-label={t("filterAgents")} />
        </label>
        ) : null}
        {agents.length === 0 ? (
          <p className="px-[10px] text-[14px] text-[var(--hb-sidebar-ink-2)]">{filter.trim() ? t("noAgentMatches", { query: filter.trim() }) : t("noAgentsRecently")}
            {filter.trim() ? null : <WorkElsewhere tiles={view.workspaces} prefix=" " />}
          </p>
        ) : null}
        {agents.map((a) => (
          <AgentRow key={a.key} a={a} />
        ))}
        {rest.length > 0 && !filter.trim() ? (
          <button type="button" className="hb-row text-[14px] text-[var(--hb-sidebar-ink-2)]" aria-expanded={all} onClick={() => setAll(!all)}>
            {all ? t("seeLess") : t("seeAll", { count: shown.length + rest.length })}
          </button>
        ) : null}

        {inactive.length > 0 && !filter.trim() ? (
          <>
            <button type="button" className="hb-row text-[14px] text-[var(--hb-sidebar-ink-2)]" aria-expanded={showInactive} onClick={() => setShowInactive(!showInactive)}>
              <ChevronDown className="size-4" style={{ transform: showInactive ? undefined : "rotate(-90deg)" }} aria-hidden /> {t("inactiveCount", { count: inactive.length })}
            </button>
            {showInactive ? inactive.map((a) => <AgentRow key={a.key} a={a} />) : null}
          </>
        ) : null}

        <HiddenGroup agents={hidden} />
        </>
        )}

        {temporary.length > 0 && !agentsFolded ? (
          <>
            <button type="button" className="hb-row text-[14px] text-[var(--hb-sidebar-ink-2)]" aria-expanded={temps} onClick={() => setTemps(!temps)}>
              {t("temporarySessions", { count: temporary.length })}
            </button>
            {temps ? temporary.map((a) => <AgentRow key={a.key} a={a} />) : null}
          </>
        ) : null}

        <div className="hb-section hb-section-with-action">
          <span>{tr("sectionRooms")}</span>
          <button type="button" className="hb-section-action" aria-label={tr("createRoom")} title={tr("createRoom")} onClick={() => setCreating(true)}>
            <Plus className="size-4" strokeWidth={2.4} aria-hidden />
          </button>
        </div>
        {rooms.map((c) => (
          <ChannelRow key={c.id} c={c} active={active} actions={itemsFor(c)} />
        ))}
        {archived.length > 0 ? (
          <>
            <button type="button" className="hb-row text-[14px] text-[var(--hb-sidebar-ink-2)]" aria-expanded={showArchived} onClick={() => setShowArchived(!showArchived)}>
              <ChevronDown className="size-4" style={{ transform: showArchived ? undefined : "rotate(-90deg)" }} aria-hidden /> {tr("archivedCount", { count: archived.length })}
            </button>
            {showArchived ? archived.map((c) => <ChannelRow key={c.id} c={c} active={active} actions={itemsFor(c)} />) : null}
          </>
        ) : null}
        {creating ? <CreateRoomDialog view={view} onClose={() => setCreating(false)} /> : null}
        {channelDialog}
      </div>
      <MoreTools />
    </aside>
  );
}

/** "Nobody here" is only half the truth while another workspace is busy: says where, and goes there. */
function WorkElsewhere({ tiles, prefix = "" }: { tiles: WorkspaceTile[] | undefined; prefix?: string }) {
  const busy = workingElsewhere(tiles);
  return busy ? <GoElsewhere busy={busy} prefix={prefix} /> : null;
}

function GoElsewhere({ busy, prefix }: { busy: WorkspaceTile; prefix: string }) {
  const router = useRouter();
  const pathname = usePathname();
  return (
    <>
      {prefix}
      <button
        type="button"
        className="hb-why-elsewhere"
        onClick={(e) => {
          // It can sit inside a <summary>: going there must not also unfold it.
          e.preventDefault();
          e.stopPropagation();
          setCurrentWorkspace(busy.id);
          if (pathname !== "/") router.push("/" as Route);
        }}
      >
        {t("workingIn", { count: busy.working, name: busy.name })}
      </button>
    </>
  );
}

/**
 * Why this channel is here, under its name: "3 sessions in the last 24 h",
 * which opens the list of those agents, each a link to its conversation.
 */
export function ChannelWhy({ view, channel }: { view: WorkspaceView; channel: WsChannel }) {
  const why = channelHeadline(view, channel);
  // From the count, not the wording: the sentence changes with the language.
  const nobody = why.now !== null && channelMembers(view, channel).now === 0;
  const now = why.now ? (
    <span className="hb-why-now" data-zero={nobody ? "true" : undefined}>
      {" · "}
      {why.now}
      {nobody ? <WorkElsewhere tiles={view.workspaces} prefix=" · " /> : null}
    </span>
  ) : null;
  if (why.agents.length === 0)
    return (
      <span className="hb-why">
        {why.sessions}
        {now}
      </span>
    );
  return (
    <details className="hb-why">
      <summary>
        {why.sessions}
        {now}
      </summary>
      <ul className="hb-why-list">
        {why.agents.map((a) => (
          <li key={a.key}>
            <Link href={`/direct/${encodeURIComponent(a.key)}` as Route}>{a.name}</Link>
          </li>
        ))}
      </ul>
    </details>
  );
}

/** The copies and branches of a channel, behind an "ℹ": never in the header line. */
function ChannelPlaces({ channel }: { channel: WsChannel }) {
  const places = channelPlaces(channel);
  if (places.copies.length === 0) return null;
  return (
    <details className="hb-info">
      <summary aria-label={t("copiesAndBranchesOf", { channel: channel.id })} title={t("copiesAndBranches")}>
        <Info className="size-4" aria-hidden />
      </summary>
      <div className="hb-info-pop">
        <div className="hb-pop-title">{places.title}</div>
        <ul>
          {places.copies.map((c) => (
            <li key={c.full}>
              <span className="hb-mono" title={c.full}>
                {c.path}
              </span>
              {c.branch ? <span className="hb-info-branch hb-mono">{c.branch}</span> : null}
            </li>
          ))}
        </ul>
      </div>
    </details>
  );
}

export function ChannelHeader({ view, channel, menu }: { view: WorkspaceView; channel: WsChannel; menu?: React.ReactNode }) {
  return (
    <header className="hb-head">
      <div className="hb-head-title">
        <Hash className="size-5" strokeWidth={2.6} aria-hidden />
        {channel.id}
        <ChevronDown className="size-4" strokeWidth={2.6} aria-hidden />
      </div>
      <ChannelWhy view={view} channel={channel} />
      <ChannelPlaces channel={channel} />
      <div className="hb-head-right">
        <ChannelMembers view={view} channel={channel} />
        {menu}
      </div>
    </header>
  );
}

function ThreadPane({ view, id, channel, onClose }: { view: WorkspaceView; id: string; channel?: string; onClose: () => void }) {
  const newThread = (draft: string) => {
    onClose();
    // The channel's own composer takes the draft: a new top-level message.
    requestAnimationFrame(() => {
      const main = document.querySelector<HTMLTextAreaElement>(".hb-main .hb-composer textarea");
      if (!main) return;
      const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
      set?.call(main, draft);
      main.dispatchEvent(new Event("input", { bubbles: true }));
      main.focus();
    });
  };
  const parent = messageById(view, id);
  if (!parent) return null;
  const replies = thread(view, id);
  const agent = view.agents.find((a) => a.key === parent.agent);
  return (
    <aside className="hb-pane" aria-label={t("thread")}>
      <div className="hb-pane-head">
        <span className="hb-pane-title">{t("thread")}</span>
        {(channel ?? parent.channels[0]) ? <span className="hb-pane-sub"># {channel ?? parent.channels[0]}</span> : null}
        <button type="button" className="hb-close" onClick={onClose} aria-label={t("closeThread")}>
          <X className="size-5" />
        </button>
      </div>
      <div className="hb-stream">
        <MessageItem view={view} message={parent} showReplies={false} whole />
        <div className="hb-divider">{replies.length === 1 ? t("oneReply") : t("replies", { count: replies.length })}</div>
        {replies.map((r) => (
          <MessageItem key={r.id} view={view} message={r} showReplies={false} whole />
        ))}
      </div>
      {agent ? (
        <Composer
          target={{ agent: agent.key, replyTo: parent.id }}
          placeholder={t("replyPlaceholder")}
          blocked={composerBlock([agent])}
          notice={composerNotice([agent])}
          confirm={composerConfirm([agent])}
          hint={dmHint(agent, noteReader(view.agents, agent.key))}
          onNewThread={channel ? newThread : undefined}
        />
      ) : null}
    </aside>
  );
}

export function HomeShell({ active, children }: { active: string | null; children: (view: WorkspaceView) => React.ReactNode }) {
  const { state } = useWorkspace();
  const view = useView();
  if (!view) {
    return (
      <div className="hb-main">
        <Loading state={state} />
      </div>
    );
  }
  return (
    <>
      <Sidebar view={view} active={active} />
      {children(view)}
    </>
  );
}

export function ChannelScreen({ channel }: { channel: string | null }) {
  const view = useView();
  const router = useRouter();
  const params = useSearchParams();
  const pathname = usePathname() ?? "/";
  const { now, post } = useWorkspace();
  const id = channel ?? (view ? defaultChannelId(view) : null);
  const markRead = React.useRef<(() => void) | null>(null);
  const seenUnread = React.useRef<string>("");
  const onEsc = React.useCallback(() => markRead.current?.(), []);
  useEscRead(onEsc);
  const thread = params?.get("thread") ?? null;
  const work = params?.get("t") ?? null;
  const go = (q: { thread?: string | null; t?: string | null }) => {
    const next = new URLSearchParams();
    if (q.t) next.set("t", q.t);
    if (q.thread) next.set("thread", q.thread);
    const query = next.toString();
    router.replace((query ? `${pathname}?${query}` : pathname) as Route, { scroll: false });
  };
  const openThread = (m: string) => go({ thread: m, t: work });
  const closeThread = () => go({ t: work });
  return (
    <HomeShell active={id}>
      {(v) => {
        const ch = id ? findChannel(v, id) : undefined;
        if (!ch) {
          return (
            <main className="hb-main">
              <div className="hb-empty">
                <strong>{id ? t("noSuchChannel", { id }) : t("noChannelsYet")}</strong>
                {t("channelAppearsWhen")}
                {!v.team_root_found ? ` ${t("claudeProjectsMissing")}` : null}
              </div>
            </main>
          );
        }
        markRead.current = () => void post("read-channel", { channel: ch.id }).catch(() => {});
        if (ch.unread && seenUnread.current !== `${ch.id}:${ch.unread}`) {
          // Open is read, as in a chat app; Esc does it again on demand.
          seenUnread.current = `${ch.id}:${ch.unread}`;
          queueMicrotask(() => markRead.current?.());
        }
        if (isRoom(ch)) return <RoomView key={ch.id} view={v} channel={ch} now={now} />;
        const whole = channelStream(v, ch.id);
        const items = workItems(v, ch.id, whole, now);
        const chosen = items.find((i) => i.key === work) ?? null;
        const stream = chosen ? whole.filter((m) => aboutWork(m, chosen)) : whole;
        const composer = channelComposer(ch);
        return (
          <>
            <main className="hb-main" aria-label={t("channelAria", { channel: ch.id })}>
              <ChannelHeader view={v} channel={ch} menu={<ChannelMenu channel={ch} />} />
              {ch.archived ? <ArchivedStrip channel={ch} /> : null}
              {ch.warnings.map((w) => (
                <div key={w.checkout} className="hb-strip" role="alert">
                  <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden />
                  <span>
                    <strong>{w.agents.length === 2 ? t("twoAgentsSameCheckoutLead") : t("agentsSameCheckoutLead", { count: w.agents.length })}</strong>{" "}
                    {t("agentsWritingSince", { agents: w.agents.map((k) => agentLabel(v, k)).join(", "), file: fileName(w.checkout), time: hhmm(w.since_ms) })}
                  </span>
                </div>
              ))}
              <WorkPanel view={v} items={items} selected={chosen?.key ?? null} onSelect={(t) => go({ t, thread })} />
              <Stream
                view={v}
                messages={stream}
                now={now}
                onOpenThread={openThread}
                selected={thread}
                foldInternal={!chosen}
                empty={<div className="hb-empty">{t("nobodyWroteYet", { channel: ch.id })}</div>}
              />
              <Composer
                target={{ channel: ch.id }}
                placeholder={t("writeInChannel", { channel: ch.id })}
                blocked={composer.blocked}
                notice={composer.notice}
                confirm={composer.confirm}
              />
            </main>
            {thread && messageById(v, thread) ? <ThreadPane view={v} id={thread} channel={ch.id} onClose={closeThread} /> : null}
          </>
        );
      }}
    </HomeShell>
  );
}

export function ThreadsScreen() {
  const router = useRouter();
  const params = useSearchParams();
  const thread = params?.get("thread") ?? null;
  return (
    <HomeShell active={null}>
      {(v) => {
        const list = threads(v);
        return (
          <>
            <main className="hb-main" aria-label={t("threads")}>
              <header className="hb-head">
                <div className="hb-head-title">
                  <MessageSquare className="size-5" aria-hidden /> {t("threads")}
                </div>
              </header>
              <div className="hb-stream">
                {list.length === 0 ? <div className="hb-empty">{t("noThreadsYet")}</div> : null}
                {list.map((m) => (
                  <MessageItem key={m.id} view={v} message={m} selected={thread === m.id} onOpenThread={(id) => router.replace(`/threads?thread=${encodeURIComponent(id)}`)} />
                ))}
              </div>
            </main>
            {thread && messageById(v, thread) ? <ThreadPane view={v} id={thread} onClose={() => router.replace("/threads")} /> : null}
          </>
        );
      }}
    </HomeShell>
  );
}

export function DeliveriesScreen() {
  return <HomeShell active={null}>{(v) => <DeliveriesView view={v} />}</HomeShell>;
}
