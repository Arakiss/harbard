"use client";

import type { Route } from "next";
import * as React from "react";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { ArrowLeft, Hash, MoreHorizontal, Trash2, X } from "lucide-react";
import { createT } from "@/i18n";
import { ApiUnavailable, RoomRefused, fetchDeletedRooms, fetchRoomHistory, postChannelAction } from "@/lib/api";
import { useModal } from "./use-modal";
import {
  FEEDBACK,
  FEEDBACK_KINDS,
  channelActionLabel,
  channelActions,
  feedbackKindLabel,
  feedbackTone,
  isRoom,
  mergeRoomMessages,
  normalizeRoomName,
  parseFeedback,
  refusalMessage,
  roomComposerNotice,
  roomNameMessage,
  roomNameProblem,
  roomPurpose,
  roomStream,
  type ChannelAction,
  type DeletedRoom,
  type FeedbackKind,
} from "@/lib/rooms";
import { atDate, agentName, shownName, type WorkspaceView, type WsChannel, type WsMessage } from "@/lib/workspace";
import { ChannelMembers } from "./channel-members";
import { AgentAvatar, Composer, MentionText, Stream, type RowAction } from "./parts";
import { useView, useWorkspace } from "./workspace-context";

const t = createT("rooms");

/**
 * Messages asked for at a time. The contract allows 200; a smaller page keeps
 * the first paint of a long room quick and the next one is a scroll away.
 */
const HISTORY_PAGE = 50;

interface RoomHistory {
  older: WsMessage[];
  next: string | null;
  loading: boolean;
  error: boolean;
  /** Serve has no such route, or this room left no log: nothing further to ask. */
  absent: boolean;
  load: (before: string | null) => void;
}

/**
 * The room's log on disk, a page at a time and newest page first. The workspace
 * view holds only recent messages, so this is what makes everything ever said
 * reachable. Mount it keyed by room: a new room starts from nothing.
 */
function useRoomHistory(room: string): RoomHistory {
  const [state, setState] = React.useState({ older: [] as WsMessage[], next: null as string | null, loading: true, error: false, absent: false });
  const busy = React.useRef<AbortController | null>(null);
  const load = React.useCallback(
    (before: string | null) => {
      busy.current?.abort();
      const controller = new AbortController();
      busy.current = controller;
      setState((s) => ({ ...s, loading: true, error: false }));
      fetchRoomHistory(room, before, HISTORY_PAGE, controller.signal)
        .then((page) => setState((s) => ({ older: mergeRoomMessages(page.messages, s.older), next: page.next_before, loading: false, error: false, absent: false })))
        .catch((error: unknown) => {
          if (error instanceof DOMException && error.name === "AbortError") return;
          const absent = error instanceof ApiUnavailable && error.status === 404;
          setState((s) => ({ ...s, loading: false, error: !absent, absent }));
        });
    },
    [room],
  );
  React.useEffect(() => {
    load(null);
    return () => busy.current?.abort();
  }, [load]);
  return { ...state, load };
}

/** Calls `onNear` while `target` is within `margin` of being scrolled into `scroller`. */
function useNearEdge(target: React.RefObject<HTMLElement | null>, scroller: React.RefObject<HTMLElement | null>, enabled: boolean, onNear: () => void, margin: string) {
  const near = React.useRef(onNear);
  near.current = onNear;
  React.useEffect(() => {
    const el = target.current;
    if (!el || !enabled) return;
    const io = new IntersectionObserver((entries) => entries.some((e) => e.isIntersecting) && near.current(), { root: scroller.current, rootMargin: margin });
    io.observe(el);
    return () => io.disconnect();
  }, [target, scroller, enabled, margin]);
}

/** "Load older", or where the room begins: the button is also the keyboard's way back. */
function OlderControl({ history, room, hasMessages, sentinel }: { history: RoomHistory; room: string; hasMessages: boolean; sentinel: React.RefObject<HTMLDivElement | null> }) {
  return (
    <div ref={sentinel} className="hb-room-older">
      {history.error ? (
        <span role="alert">
          {t("historyFailed")}{" "}
          <button type="button" className="hb-hist-more" onClick={() => history.load(history.next)}>
            {t("retry")}
          </button>
        </span>
      ) : history.next ? (
        <button type="button" className="hb-hist-more" disabled={history.loading} onClick={() => history.load(history.next)}>
          {history.loading ? t("loadingOlder") : t("loadOlder")}
        </button>
      ) : !history.loading && !history.absent && hasMessages ? (
        <span className="hb-room-start">{t("startOfRoom", { room })}</span>
      ) : null}
    </div>
  );
}

/**
 * A room's messages: the live view joined to the log on disk, older pages
 * arriving as the top comes near. Growing above the fold keeps what is on
 * screen where it was.
 */
export function RoomFeed({ view, room, now, readOnly = false, className, empty }: { view: WorkspaceView; room: string; now: number; readOnly?: boolean; className?: string; empty: string }) {
  const history = useRoomHistory(room);
  const live = React.useMemo(() => roomStream(view, room), [view, room]);
  const messages = React.useMemo(() => mergeRoomMessages(history.older, live), [history.older, live]);
  const scroller = React.useRef<HTMLDivElement>(null);
  const sentinel = React.useRef<HTMLDivElement>(null);
  const seen = React.useRef({ older: 0, height: 0 });
  React.useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    if (history.older.length !== seen.current.older && seen.current.height > 0) el.scrollTop += el.scrollHeight - seen.current.height;
    seen.current = { older: history.older.length, height: el.scrollHeight };
  });
  useNearEdge(sentinel, scroller, Boolean(history.next) && !history.loading && !history.error, () => history.load(history.next), "300px 0px 0px 0px");
  return (
    <Stream
      view={view}
      messages={messages}
      now={now}
      readOnly={readOnly}
      className={className}
      scroller={scroller}
      head={<OlderControl history={history} room={room} hasMessages={messages.length > 0} sentinel={sentinel} />}
      empty={history.loading ? null : <div className="hb-empty">{empty}</div>}
    />
  );
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

function reasonOf(error: unknown): string {
  if (error instanceof RoomRefused) return refusalMessage(error.code, error.message);
  return error instanceof Error ? error.message : String(error);
}

/** The shell of a small dialog in the app: keyboard kept inside, Esc closes. */
function RoomDialog({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  const ref = React.useRef<HTMLDivElement>(null);
  useModal(ref, onClose);
  return (
    <div className="hb-finder-scrim" onMouseDown={onClose}>
      <div ref={ref} className="hb-small-dialog" role="dialog" aria-modal="true" aria-label={title} onMouseDown={(e) => e.stopPropagation()}>
        <div className="hb-pane-head">
          <span className="hb-pane-title">{title}</span>
          <button type="button" className="hb-close" onClick={onClose} aria-label={t("dismiss")}>
            <X className="size-5" />
          </button>
        </div>
        <div className="hb-small-body">{children}</div>
      </div>
    </div>
  );
}

/** Delete, in the app: what goes, and in the same breath that the history does not. */
function DeleteRoomDialog({ room, onClose, onDeleted }: { room: string; onClose: () => void; onDeleted: () => void }) {
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const confirm = () => {
    setBusy(true);
    setError(null);
    postChannelAction({ action: "delete", name: room })
      .then(onDeleted)
      .catch((e: unknown) => {
        setBusy(false);
        setError(reasonOf(e));
      });
  };
  return (
    <RoomDialog title={t("deleteTitle", { room })} onClose={onClose}>
      <p className="hb-room-dialog-text">{t("deleteBody")}</p>
      <p className="hb-room-dialog-kept" data-testid="delete-kept">
        {t("deleteKept")}
      </p>
      {error ? (
        <p className="hb-error" role="alert">
          {error}
        </p>
      ) : null}
      <div className="hb-room-dialog-actions">
        <button type="button" className="hb-btn" onClick={onClose} data-autofocus>
          {t("cancel")}
        </button>
        <button type="button" className="hb-btn hb-btn-danger" onClick={confirm} disabled={busy}>
          <Trash2 className="size-4" aria-hidden /> {busy ? t("deleting") : t("deleteConfirm")}
        </button>
      </div>
    </RoomDialog>
  );
}

/**
 * Archive, unarchive and delete for one surface. Render `dialog` wherever the
 * hook is used. After a delete from inside the room, the operator lands on its
 * read-only history: the proof that it was kept.
 */
export function useChannelActions(): { itemsFor: (channel: WsChannel) => RowAction[]; run: (action: ChannelAction, channel: WsChannel) => void; dialog: React.ReactNode } {
  const { refresh } = useWorkspace();
  const router = useRouter();
  const pathname = usePathname();
  const [deleting, setDeleting] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const run = (action: ChannelAction, channel: WsChannel) => {
    setError(null);
    if (action === "delete") {
      setDeleting(channel.id);
      return;
    }
    postChannelAction({ action, name: channel.id })
      .then(refresh)
      .catch((e: unknown) => setError(t("actionFailed", { reason: reasonOf(e) })));
  };
  const deleted = (room: string) => {
    setDeleting(null);
    refresh();
    if (pathname === `/c/${encodeURIComponent(room)}`) router.replace(`/history/rooms/${encodeURIComponent(room)}` as Route);
  };
  return {
    run,
    itemsFor: (channel) => channelActions(channel).map((action) => ({ label: channelActionLabel(action), run: () => run(action, channel) })),
    dialog: (
      <>
        {deleting ? <DeleteRoomDialog room={deleting} onClose={() => setDeleting(null)} onDeleted={() => deleted(deleting)} /> : null}
        {error ? (
          <div className="hb-banner hb-room-toast" role="alert">
            {error}
            <button type="button" className="hb-hist-more" onClick={() => setError(null)}>
              {t("dismiss")}
            </button>
          </div>
        ) : null}
      </>
    ),
  };
}

/** The header's menu: only what this channel allows; a built-in room shows none. */
export function ChannelMenu({ channel }: { channel: WsChannel }) {
  const { itemsFor, dialog } = useChannelActions();
  const [open, setOpen] = React.useState(false);
  const wrap = React.useRef<HTMLDivElement>(null);
  const menu = React.useRef<HTMLDivElement>(null);
  const button = React.useRef<HTMLButtonElement>(null);
  const items = itemsFor(channel);
  React.useEffect(() => {
    if (!open) return;
    menu.current?.querySelector<HTMLButtonElement>("[role='menuitem']")?.focus();
    const away = (e: MouseEvent) => wrap.current && !wrap.current.contains(e.target as Node) && setOpen(false);
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);
  if (items.length === 0) return null;
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setOpen(false);
      button.current?.focus();
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const all = [...(menu.current?.querySelectorAll<HTMLButtonElement>("[role='menuitem']") ?? [])];
    const at = all.findIndex((el) => el === document.activeElement);
    all[(at + (e.key === "ArrowDown" ? 1 : -1) + all.length) % all.length]?.focus();
  };
  return (
    <div ref={wrap} className="hb-room-menu">
      <button ref={button} type="button" className="hb-tool" aria-label={t("channelMenu", { channel: channel.id })} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)}>
        <MoreHorizontal className="size-5" aria-hidden />
      </button>
      {open ? (
        <div ref={menu} className="hb-row-menu hb-room-menu-pop" role="menu" aria-label={channel.id} onKeyDown={onKeyDown}>
          {items.map((item) => (
            <button
              key={item.label}
              type="button"
              role="menuitem"
              className="hb-pop-item"
              onClick={() => {
                setOpen(false);
                item.run();
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
      ) : null}
      {dialog}
    </div>
  );
}

/** A name, an optional purpose; the same rules the bus applies, said before sending. */
export function CreateRoomDialog({ view, onClose }: { view: WorkspaceView; onClose: () => void }) {
  const { refresh } = useWorkspace();
  const router = useRouter();
  const [raw, setRaw] = React.useState("");
  const [purpose, setPurpose] = React.useState("");
  const [tried, setTried] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [refused, setRefused] = React.useState<string | null>(null);
  const name = normalizeRoomName(raw);
  const problem = roomNameProblem(
    name,
    view.channels.map((c) => c.id),
  );
  const shown = problem && (tried || problem !== "empty") ? roomNameMessage(problem) : null;
  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    setTried(true);
    if (problem || busy) return;
    setBusy(true);
    setRefused(null);
    postChannelAction({ action: "create", name, ...(purpose.trim() ? { purpose: purpose.trim() } : {}) })
      .then(() => {
        refresh();
        onClose();
        router.push(`/c/${encodeURIComponent(name)}` as Route);
      })
      .catch((error: unknown) => {
        setBusy(false);
        setRefused(reasonOf(error));
      });
  };
  return (
    <RoomDialog title={t("dialogCreateTitle")} onClose={onClose}>
      <form onSubmit={submit} noValidate>
        <label className="hb-room-field">
          <span className="hb-room-label">{t("fieldName")}</span>
          <span className="hb-room-name-input">
            <Hash className="size-4" aria-hidden />
            <input
              className="hb-input"
              value={raw}
              onChange={(e) => setRaw(e.target.value)}
              autoComplete="off"
              spellCheck={false}
              data-autofocus
              aria-invalid={shown ? true : undefined}
              aria-describedby="hb-room-name-note"
            />
          </span>
        </label>
        <p id="hb-room-name-note" className="hb-room-note" data-problem={shown ? "true" : undefined} role={shown ? "alert" : undefined}>
          {shown ?? (name && name !== raw.trim() ? t("willBeCreatedAs", { name }) : t("fieldNameHint"))}
        </p>
        <label className="hb-room-field">
          <span className="hb-room-label">{t("fieldPurpose")}</span>
          <input className="hb-input" value={purpose} onChange={(e) => setPurpose(e.target.value)} placeholder={t("fieldPurposePlaceholder")} maxLength={240} />
        </label>
        {refused ? (
          <p className="hb-error" role="alert">
            {refused}
          </p>
        ) : null}
        <div className="hb-room-dialog-actions">
          <button type="button" className="hb-btn" onClick={onClose}>
            {t("cancel")}
          </button>
          <button type="submit" className="hb-btn hb-btn-primary" disabled={busy}>
            {busy ? t("creating") : t("create")}
          </button>
        </div>
      </form>
    </RoomDialog>
  );
}

// ---------------------------------------------------------------------------
// The room
// ---------------------------------------------------------------------------

function RoomHeader({ view, channel }: { view: WorkspaceView; channel: WsChannel }) {
  const purpose = roomPurpose(channel);
  return (
    <header className="hb-head">
      <div className="hb-head-title">
        <Hash className="size-5" strokeWidth={2.6} aria-hidden />
        {channel.id}
      </div>
      {purpose ? (
        <span className="hb-topic" title={purpose}>
          {purpose}
        </span>
      ) : null}
      <div className="hb-head-right">
        {channel.members.length === 0 && !channel.built_in && !channel.archived ? <span className="hb-topic">{t("noMembers")}</span> : <ChannelMembers view={view} channel={channel} />}
        <ChannelMenu channel={channel} />
      </div>
    </header>
  );
}

/** Says that nothing is lost and how to bring it back, on a room or a project channel. */
export function ArchivedStrip({ channel }: { channel: WsChannel }) {
  const { run, dialog } = useChannelActions();
  return (
    <div className="hb-room-strip" role="status">
      <span>
        <strong>{t("archivedLead", { channel: channel.id })}</strong> {isRoom(channel) ? t("archivedRoomBody") : t("archivedProjectBody")}
      </span>
      <button type="button" className="hb-btn" onClick={() => run("unarchive", channel)}>
        {channelActionLabel("unarchive")}
      </button>
      {dialog}
    </div>
  );
}

/** One screen for every room except the Lobby, which has its own. */
export function RoomView({ view, channel, now }: { view: WorkspaceView; channel: WsChannel; now: number }) {
  return (
    <main className="hb-main" aria-label={t("writeIn", { room: channel.id })}>
      <RoomHeader view={view} channel={channel} />
      {channel.archived ? <ArchivedStrip channel={channel} /> : null}
      {channel.id === FEEDBACK ? (
        <FeedbackInbox view={view} now={now} />
      ) : (
        <>
          <RoomFeed view={view} room={channel.id} now={now} empty={t("emptyRoom", { room: channel.id })} />
          {channel.archived ? (
            <div className="hb-composer">
              <div className="hb-composer-none" role="note">
                {t("archivedReadOnly")}
              </div>
            </div>
          ) : (
            <Composer target={{ channel: channel.id }} placeholder={t("writeIn", { room: channel.id })} blocked={null} notice={roomComposerNotice(channel)} />
          )}
        </>
      )}
    </main>
  );
}

// ---------------------------------------------------------------------------
// #feedback
// ---------------------------------------------------------------------------

function InboxRow({ view, message, now }: { view: WorkspaceView; message: WsMessage; now: number }) {
  const note = parseFeedback(message.text);
  const agent = view.agents.find((a) => a.key === message.agent) ?? null;
  const who = agent ? agentName(agent) : shownName(message.author);
  return (
    <li className="hb-inbox-row" data-kind={note.kind ?? "note"} data-message={message.id}>
      <AgentAvatar name={who} agent={agent} size={32} dot={false} />
      <div className="hb-inbox-body">
        <div className="hb-inbox-meta">
          <span className="hb-chip" data-tone={feedbackTone(note.kind)}>
            {feedbackKindLabel(note.kind)}
          </span>
          <span className="hb-inbox-who">{who}</span>
          <span className="hb-msg-time">{atDate(message.ts_ms, now)}</span>
          {note.version ? <span className="hb-inbox-version hb-mono">{t("inboxVersion", { version: note.version })}</span> : null}
        </div>
        <div className="hb-msg-text">
          <MentionText text={note.text} mentions={message.mentions} />
        </div>
      </div>
    </li>
  );
}

/** `#feedback` reads as an inbox: newest first, each with the kind its author gave it. */
function FeedbackInbox({ view, now }: { view: WorkspaceView; now: number }) {
  const history = useRoomHistory(FEEDBACK);
  const live = React.useMemo(() => roomStream(view, FEEDBACK), [view]);
  const newestFirst = React.useMemo(() => mergeRoomMessages(history.older, live).reverse(), [history.older, live]);
  const [kind, setKind] = React.useState<FeedbackKind | null>(null);
  const scroller = React.useRef<HTMLDivElement>(null);
  const sentinel = React.useRef<HTMLDivElement>(null);
  useNearEdge(sentinel, scroller, Boolean(history.next) && !history.loading && !history.error, () => history.load(history.next), "0px 0px 300px 0px");
  const count = (k: FeedbackKind) => newestFirst.filter((m) => parseFeedback(m.text).kind === k).length;
  const shown = kind ? newestFirst.filter((m) => parseFeedback(m.text).kind === kind) : newestFirst;
  return (
    <>
      <div className="hb-inbox-filters" role="group" aria-label={t("inboxAria")}>
        <button type="button" className="hb-inbox-filter" aria-pressed={kind === null} onClick={() => setKind(null)}>
          {t("inboxAll")} <span>{newestFirst.length}</span>
        </button>
        {FEEDBACK_KINDS.map((k) => (
          <button key={k} type="button" className="hb-inbox-filter" aria-pressed={kind === k} data-tone={feedbackTone(k)} onClick={() => setKind(k)}>
            {feedbackKindLabel(k)} <span>{count(k)}</span>
          </button>
        ))}
      </div>
      <div className="hb-stream hb-inbox" ref={scroller}>
        {shown.length === 0 && !history.loading ? <div className="hb-empty">{kind ? t("inboxEmptyKind") : t("inboxEmpty")}</div> : null}
        <ul className="hb-inbox-list">
          {shown.map((m) => (
            <InboxRow key={m.id} view={view} message={m} now={now} />
          ))}
        </ul>
        <OlderControl history={history} room={FEEDBACK} hasMessages={newestFirst.length > 0} sentinel={sentinel} />
      </div>
      <p className="hb-inbox-foot">
        {t("inboxHowTo")}{" "}
        {/* i18n-exempt: a command, typed the same in every language. */}
        <span className="hb-mono">harbard feedback</span>. {t("inboxLocal")}
      </p>
    </>
  );
}

// ---------------------------------------------------------------------------
// Deleted rooms
// ---------------------------------------------------------------------------

function deletedMeta(room: DeletedRoom, now: number): string {
  const when = atDate(room.deleted_ms, now);
  return room.messages === 1 ? t("deletedRowMetaOne", { when }) : t("deletedRowMeta", { when, count: room.messages });
}

/** History's way to a deleted room. Absent while there are none or serve predates them. */
export function DeletedRooms({ now }: { now: number }) {
  const [rooms, setRooms] = React.useState<DeletedRoom[]>([]);
  React.useEffect(() => {
    const controller = new AbortController();
    fetchDeletedRooms(controller.signal)
      .then(setRooms)
      .catch(() => setRooms([]));
    return () => controller.abort();
  }, []);
  if (rooms.length === 0) return null;
  return (
    <section className="hb-hist-deleted" aria-labelledby="hb-deleted-title">
      <h2 id="hb-deleted-title">{t("deletedTitle")}</h2>
      <p>{t("deletedHint")}</p>
      <ul>
        {rooms.map((room) => (
          <li key={room.name}>
            <Link href={`/history/rooms/${encodeURIComponent(room.name)}` as Route} className="hb-hist-deleted-row">
              <span className="hb-hist-deleted-name">#{room.name}</span>
              <span className="hb-hist-deleted-meta">{deletedMeta(room, now)}</span>
            </Link>
          </li>
        ))}
      </ul>
    </section>
  );
}

/** A deleted room, read-only, on the same history as any other. */
export function DeletedRoomScreen({ room }: { room: string }) {
  const view = useView();
  const { now } = useWorkspace();
  const [known, setKnown] = React.useState<DeletedRoom | null | undefined>(undefined);
  React.useEffect(() => {
    const controller = new AbortController();
    fetchDeletedRooms(controller.signal)
      .then((all) => setKnown(all.find((r) => r.name === room) ?? null))
      .catch(() => setKnown(null));
    return () => controller.abort();
  }, [room]);
  return (
    <main className="hb-main" aria-label={t("deletedTitle")}>
      <header className="hb-head">
        <Link href="/history" className="hb-tool" aria-label={t("backToHistory")} title={t("backToHistory")}>
          <ArrowLeft className="size-5" aria-hidden />
        </Link>
        <div className="hb-head-title">
          <Hash className="size-5" strokeWidth={2.6} aria-hidden />
          {room}
        </div>
        <span className="hb-chip" data-tone="red">
          {t("deletedBadge")}
        </span>
      </header>
      {known ? (
        <div className="hb-room-strip" role="status">
          <span>{t("deletedStrip", { when: atDate(known.deleted_ms, now) })}</span>
        </div>
      ) : null}
      {known === null ? <div className="hb-empty">{t("deletedMissing", { room })}</div> : null}
      {known && view ? <RoomFeed key={room} view={view} room={room} now={now} readOnly empty={t("emptyRoom", { room })} /> : null}
    </main>
  );
}
