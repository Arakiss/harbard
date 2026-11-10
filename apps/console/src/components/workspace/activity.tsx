"use client";

import { OperatorAvatar } from "./operator-avatar";
import type { Route } from "next";
import * as React from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { AlertTriangle, Circle, ListChecks, MessageSquare, MessageSquareWarning } from "lucide-react";
import {
  activitySelection,
  entryId,
  agentLabel,
  cardBody,
  cardReason,
  composerBlock,
  composerConfirm,
  composerNotice,
  dmHint,
  noteReader,
  ellipsizeWords,
  hhmm,
  messageById,
  nudgeShown,
  thread,
  threads,
  todoItems,
  needsItems,
  readAllNotice,
  readMarkOf,
  replyCount,
  sessionGone,
  sessionGoneNotice,
  type ActivityEntry,
  type WsAgent,
  type WsCard,
  type WsMessage,
  type WorkspaceView,
  withoutLabelPrefix,
} from "@/lib/workspace";
import { cardHidden } from "@/lib/hidden";
import { AgentAvatar, Avatar, Composer, ComposerActionsContext, LabelChip, Loading, MessageItem, useEscRead } from "./parts";
import { WorkspaceRefused, useView, useWorkspace } from "./workspace-context";
import { DockFeedback, FeedbackRow } from "@/components/feedback/feedback";
import { WeeklyRetroCard } from "@/components/retro/retro";
import { createT } from "@/i18n";

const t = createT("activity");

type Tab = "needs" | "todo" | "threads";
function tabs(): { id: Tab; label: string }[] {
  return [
    { id: "needs", label: t("tabNeedsYou") },
    { id: "todo", label: t("tabAll") },
    { id: "threads", label: t("tabThreads") },
  ];
}

/** A row of the list: a card, or (All, Threads) a message. */
type Entry = ActivityEntry;


function entries(view: WorkspaceView, tab: Tab): Entry[] {
  if (tab === "needs") return needsItems(view).map((card) => ({ kind: "card", card }));
  if (tab === "threads") return threads(view).map((message) => ({ kind: "message", message }));
  return todoItems(view);
}

function ReasonIcon({ card }: { card: WsCard }) {
  if (card.kind === "blocked") return <Circle className="size-5" strokeWidth={2} aria-hidden />;
  if (card.kind === "approval") return <ListChecks className="size-5" strokeWidth={2} aria-hidden />;
  if (card.kind === "unanswered") return <MessageSquareWarning className="size-5" strokeWidth={2} aria-hidden />;
  return <AlertTriangle className="size-5" strokeWidth={2} aria-hidden />;
}

export function CardItem({ view, card, now }: { view: WorkspaceView; card: WsCard; now: number }) {
  const message = card.message ? messageById(view, card.message) : undefined;
  const agent = view.agents.find((a) => a.key === card.agents[0]);
  return (
    <>
      <span className="hb-card-reason">
        <ReasonIcon card={card} />
        {cardReason(card)}
        {cardHidden(view, card) ? (
          <span className="hb-hidden-note" title={t("hiddenTitle")}>
            {t("hidden")}
          </span>
        ) : null}
        {card.resolved_ms ? (
          <span className="ml-auto text-[13px] font-normal text-[var(--hb-ink-3)]">{t("resolvedAt", { time: hhmm(card.resolved_ms) })}</span>
        ) : !card.needs ? (
          <span className="ml-auto text-[13px] font-normal text-[var(--hb-ink-3)]">{t("cardHistory")}</span>
        ) : card.kind === "blocked" ? (
          <span className="hb-count">1</span>
        ) : (
          <span className="hb-dot-badge" aria-hidden />
        )}
      </span>
      {card.kind === "unanswered" ? (
        <span className="flex gap-3">
          <OperatorAvatar name={view.operator} initials={view.operator_initials} size={48} presence={null} />
          <span className="hb-item-body">
            <span className="hb-item-top">
              <span className="hb-item-name">{cardBody(card, view, now).title}</span>
            </span>
            <span className="hb-item-preview">{cardBody(card, view, now).text}</span>
          </span>
        </span>
      ) : card.kind === "dormant" && agent ? (
        <span className="flex gap-3">
          <AgentAvatar name={agent.name} size={48} agent={agent} />
          <span className="hb-item-body">
            <span className="hb-item-top">
              <span className="hb-item-name">{agent.name}</span>
            </span>
            <span className="hb-item-preview">{cardBody(card, view, now).text}</span>
          </span>
        </span>
      ) : message && agent ? (
        <span className="flex gap-3">
          <AgentAvatar name={agent.name} size={48} agent={agent} />
          <span className="hb-item-body">
            <span className="hb-item-top">
              <span className="hb-item-name">{agent.name}</span>
              <LabelChip label={message.label} />
            </span>
            <span className="hb-item-preview">{withoutLabelPrefix(message.text, message.label)}</span>
            {card.kind === "blocked" && sessionGone(agent) ? <span className="hb-item-gone">{sessionGoneNotice()}.</span> : null}
          </span>
        </span>
      ) : (
        <span className="block pl-8">
          <span className="hb-card-title block">{cardBody(card, view, now).title}</span>
          <span className="hb-item-preview">{cardBody(card, view, now).text}</span>
        </span>
      )}
    </>
  );
}

function MessageEntry({ view, message }: { view: WorkspaceView; message: WsMessage }) {
  const agent = view.agents.find((a) => a.key === message.agent);
  const name = agent?.name ?? message.author;
  return (
    <span className="flex gap-3">
      {agent ? <AgentAvatar name={name} size={48} agent={agent} /> : <Avatar name={name} size={48} />}
      <span className="hb-item-body">
        <span className="hb-item-top">
          <span className="hb-item-name">{name}</span>
          <LabelChip label={message.label} />
          <span className="hb-item-time">{hhmm(message.ts_ms)}</span>
        </span>
        <span className="hb-item-preview">{withoutLabelPrefix(message.text, message.label)}</span>
        {message.replies.length > 0 ? (
          <span className="hb-replies">
            <MessageSquare className="size-4" aria-hidden /> {replyCount(message.replies.length)}
          </span>
        ) : null}
      </span>
    </span>
  );
}

/**
 * What the operator can do about one item: answer its agent in a reply box, resend
 * an unanswered note, or mark it resolved. Activity's detail and the calm
 * card (AC2) show the same box, so "Reply" answers in both.
 */
export function CardActions({
  view,
  card,
  message,
  agent,
  now,
}: {
  view: WorkspaceView;
  card: WsCard | null;
  message: WsMessage | undefined;
  agent: WsAgent | undefined;
  now: number;
}) {
  const { post } = useWorkspace();
  const [error, setError] = React.useState<string | null>(null);
  // "Was it you?": Keep closes the card after a fresh confirmation (serve
  // answers a one-time code first); Revoke undoes the change.
  const [keepCode, setKeepCode] = React.useState<string | null>(null);
  const check =
    card?.kind === "check" && !card.resolved_ms ? (
      keepCode ? (
        <>
          <span className="hb-item-preview">{t("keepConfirm")}</span>
          <button
            type="button"
            className="hb-btn hb-btn-primary"
            onClick={() => {
              const code = keepCode;
              setKeepCode(null);
              setError(null);
              post("resolve", { card: card.id, confirm: code }).catch(() => setError(t("saveFailed")));
            }}
          >
            {t("yesKeep")}
          </button>
          <button type="button" className="hb-btn" onClick={() => setKeepCode(null)}>
            {t("cancel")}
          </button>
        </>
      ) : (
      <>
        <button
          type="button"
          className="hb-btn"
          onClick={() => {
            setError(null);
            post("resolve", { card: card.id })
              .then((out) => {
                const asked = (out as { confirm?: { id: string } } | null)?.confirm;
                if (asked) setKeepCode(asked.id);
              })
              .catch(() => setError(t("saveFailed")));
          }}
        >
          {t("keep")}
        </button>
        <button
          type="button"
          className="hb-btn hb-btn-primary"
          onClick={() => {
            setError(null);
            post("check-revoke", { card: card.id }).catch(() => setError(t("revokeFailed")));
          }}
        >
          {t("revoke")}
        </button>
      </>
      )
    ) : null;
  const resolve = check ? (
    check
  ) : card ? (
    <button
      type="button"
      className="hb-btn"
      onClick={() => {
        setError(null);
        post(card.resolved_ms ? "reopen" : "resolve", { card: card.id }).catch(() => setError(t("saveFailed")));
      }}
    >
      {card.resolved_ms ? t("reopen") : t("markResolved")}
    </button>
  ) : null;
  const blocked = agent ? composerBlock([agent]) : t("noAgentToAnswer");
  // "Reenviado" is what the bus says (the note's `nudged_ms`), not a local
  // flag a reload would forget; the button is off while the request runs,
  // and serve refuses a second one inside the window anyway.
  const [sending, setSending] = React.useState(false);
  const nudgedMs = message?.delivery?.nudged_ms ?? null;
  const nudged = nudgeShown(nudgedMs, now);
  const nudge =
    card?.kind === "unanswered" && card.needs && !card.resolved_ms ? (
      <button
        type="button"
        className="hb-btn hb-btn-primary"
        disabled={nudged || sending}
        onClick={() => {
          if (sending) return;
          setSending(true);
          setError(null);
          post("nudge", { card: card.id })
            .catch((e: unknown) => {
              if (!(e instanceof WorkspaceRefused && e.status === 409)) setError(t("resendFailed"));
            })
            .finally(() => setSending(false));
        }}
      >
        {sending ? t("resending") : nudged && nudgedMs ? t("resentAt", { time: hhmm(nudgedMs) }) : t("resendToLead")}
      </button>
    ) : null;
  return (
    <>
      {nudge ? (
        <>
          <p className="text-[15px]">
            {t("nudgeExplain", { title: cardBody(card!, view, now).title })}
          </p>
          <div className="hb-actions">
            {nudge}
            {resolve}
          </div>
          {nudged ? <p className="mt-3 text-[14px] text-[var(--hb-ink-3)]">{t("resentNote")}</p> : null}
        </>
      ) : agent && !blocked ? (
        <ComposerActionsContext.Provider value={resolve}>
          <Composer
            target={{ agent: agent.key, replyTo: message?.id ?? null }}
            placeholder={t("replyTo", { name: ellipsizeWords(agent.name, 40) })}
            blocked={null}
            notice={composerNotice([agent])}
            confirm={composerConfirm([agent])}
            hint={dmHint(agent, noteReader(view.agents, agent.key))}
            variant="box"
          />
        </ComposerActionsContext.Provider>
      ) : (
        <>
          <div className="hb-composer-none" role="note">
            {blocked}.
          </div>
          {resolve ? <div className="hb-actions">{resolve}</div> : null}
        </>
      )}
      {error ? <p className="hb-error">{error}</p> : null}
      {card?.kind !== "blocked" && card ? (
        <p className="mt-4 text-[13px] text-[var(--hb-ink-3)]">{t("resolveHint")}</p>
      ) : null}
    </>
  );
}

function Detail({ view, entry, now }: { view: WorkspaceView; entry: Entry; now: number }) {
  const message = entry.kind === "message" ? entry.message : entry.card.message ? messageById(view, entry.card.message) : undefined;
  const channel = entry.kind === "card" ? entry.card.channel : (message?.channels[0] ?? null);
  const agent = view.agents.find((a) => a.key === (message?.agent ?? (entry.kind === "card" ? entry.card.agents[0] : "")));
  const card = entry.kind === "card" ? entry.card : null;
  return (
    <main className="hb-detail" aria-label={t("detail")}>
      <div className="hb-detail-head">
        {channel ? <># {channel}</> : <>{t("detailFallback")}</>}
        {channel ? <Link href={`/c/${encodeURIComponent(channel)}${message ? `?thread=${encodeURIComponent(message.reply_to ?? message.id)}` : ""}`}>{t("goToChannel")}</Link> : null}
      </div>
      <div className="hb-detail-body">
        {message ? (
          <div className="-mx-5">
            <MessageItem view={view} message={message} showReplies={false} whole />
            {/* What was said since, live: their replies with their delivery
                state, and the answers the lead posted with a ref. */}
            {thread(view, message.id).length > 0 ? (
              <div className="hb-detail-thread" aria-label={t("replies")}>
                {thread(view, message.id).map((r) => (
                  <MessageItem key={r.id} view={view} message={r} showReplies={false} whole />
                ))}
              </div>
            ) : null}
          </div>
        ) : card ? (
          <div className="flex gap-3">
            <AlertTriangle className="mt-1 size-6 shrink-0 text-[var(--hb-warn-icon)]" aria-hidden />
            <div>
              <div className="hb-card-title">{cardBody(card, view, now).title}</div>
              <p className="mt-1">{cardBody(card, view, now).text}</p>
              {card.checkout ? <p className="hb-mono mt-2 text-[var(--hb-ink-3)]">{card.checkout}</p> : null}
              <p className="mt-2 text-[14px] text-[var(--hb-ink-3)]">
                {card.agents.map((k) => agentLabel(view, k)).join(", ")}
              </p>
              <div className="hb-fb-host mt-3">
                <FeedbackRow kind="needs_you" id={card.id} />
              </div>
            </div>
          </div>
        ) : null}
        {message && card ? (
          <div className="hb-fb-host mt-3">
            <FeedbackRow kind="needs_you" id={card.id} />
          </div>
        ) : null}
        <div className="mt-6">
          <CardActions view={view} card={card} message={message} agent={agent} now={now} />
        </div>
      </div>
    </main>
  );
}

/** "Resolve all" leaves a one-step undo for a few seconds. */
const UNDO_MS = 8_000;

export function ActivityScreen() {
  const { state, now, post } = useWorkspace();
  const [undo, setUndo] = React.useState<string[] | null>(null);
  // An item answered from its own detail leaves "Needs you"; the detail
  // stays on it, so the operator sees their message, its state and the answer.
  const shown = React.useRef<Entry | null>(null);
  const shownAt = React.useRef(0);
  React.useEffect(() => {
    if (!undo) return;
    const t = setTimeout(() => setUndo(null), UNDO_MS);
    return () => clearTimeout(t);
  }, [undo]);
  const view = useView();
  const router = useRouter();
  const params = useSearchParams();
  // Esc marks the open item read, as in Direct (AC7).
  const mark = React.useRef<{ agent: string; ts_ms: number } | null>(null);
  const markRead = React.useCallback(() => {
    if (mark.current) void post("read", mark.current).catch(() => {});
  }, [post]);
  useEscRead(markRead);
  const [readNotice, setReadNotice] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!readNotice) return;
    const t = setTimeout(() => setReadNotice(null), UNDO_MS);
    return () => clearTimeout(t);
  }, [readNotice]);
  const tab = (tabs().find((entry) => entry.id === params?.get("tab"))?.id ?? "needs") as Tab;
  const picked = params?.get("sel") ?? null;
  if (!view) {
    return (
      <div className="hb-main">
        <Loading state={state} />
      </div>
    );
  }
  const list = entries(view, tab);
  const selected = activitySelection(list, picked, shown.current, shownAt.current);
  shown.current = selected;
  shownAt.current = selected ? Math.max(0, list.findIndex((e) => entryId(e) === entryId(selected))) : 0;
  mark.current = selected ? readMarkOf(view, selected) : null;
  const go = (t: Tab, sel?: string) => {
    const q = new URLSearchParams();
    if (t !== "needs") q.set("tab", t);
    if (sel) q.set("sel", sel);
    const s = q.toString();
    router.replace(`/activity${s ? `?${s}` : ""}` as Route, { scroll: false });
  };
  return (
    <>
      <section className="hb-list" aria-label={t("title")}>
        <div className="hb-list-head">
          <h1 className="hb-list-title">{t("title")}</h1>
          {view.unread.activity > 0 ? <span className="hb-unread">{t("needsYouCount", { count: view.unread.activity })}</span> : null}
        </div>
        <div className="hb-list-actions">
          <span className="hb-read-all">
            <button
              type="button"
              className="hb-link-btn"
              aria-describedby="hb-read-all-note"
              onClick={() => {
                const before = view.unread;
                post("read-all", {})
                  .then(() => {
                    setUndo(null);
                    setReadNotice(readAllNotice(before));
                  })
                  .catch(() => setReadNotice(t("saveFailed")));
              }}
            >
              {t("markAllRead")}
            </button>
            <span id="hb-read-all-note" className="hb-list-note">
              {t("doesNotResolve")}
            </span>
          </span>
          {tab === "needs" && list.length > 0 ? (
            <button
              type="button"
              className="hb-link-btn"
              onClick={() => {
                const ids = list.flatMap((e) => (e.kind === "card" ? [e.card.id] : []));
                post("resolve-all", { cards: ids })
                  .then(() => setUndo(ids))
                  .catch(() => {});
              }}
            >
              {t("resolveAll")}
            </button>
          ) : null}
        </div>
        <WeeklyRetroCard />
        <DockFeedback now={now} />
        <div className="hb-tabs" role="tablist">
          {tabs().map((entry) => (
            <button key={entry.id} type="button" role="tab" className="hb-tab" aria-selected={tab === entry.id} onClick={() => go(entry.id)}>
              {entry.label}
              {entry.id === "todo" && (view.unread.todo ?? 0) > 0 ? <span className="hb-count ml-1">{view.unread.todo}</span> : null}
            </button>
          ))}
        </div>
        <div className="hb-list-scroll" style={{ borderTop: 0 }}>
          {list.length === 0 ? (
            <div className="hb-empty">
              <strong>{tab === "needs" ? t("emptyNeeds") : tab === "threads" ? t("emptyThreads") : t("emptyAll")}</strong>
              {tab === "needs" ? t("emptyNeedsHint") : null}
            </div>
          ) : null}
          {list.map((e) => {
            const id = entryId(e);
            const active = selected ? entryId(selected) === id : false;
            return (
              <button key={id} type="button" className="hb-item hb-item-block" aria-current={active ? "true" : undefined} onClick={() => go(tab, id)}>
                {e.kind === "card" ? <CardItem view={view} card={e.card} now={now} /> : <MessageEntry view={view} message={e.message} />}
              </button>
            );
          })}
        </div>
      </section>
      {readNotice && !undo ? (
        <div className="hb-toast" role="status" data-testid="read-all-notice">
          {readNotice}
        </div>
      ) : null}
      {undo ? (
        <div className="hb-toast" role="status">
          {undo.length === 1 ? t("resolvedOne") : t("resolvedMany", { count: undo.length })}
          <button
            type="button"
            onClick={() => {
              const ids = undo;
              setUndo(null);
              void post("reopen-all", { cards: ids }).catch(() => {});
            }}
          >
            {t("undo")}
          </button>
        </div>
      ) : null}
      {selected ? (
        <Detail key={entryId(selected)} view={view} entry={selected} now={now} />
      ) : (
        <main className="hb-detail">
          <div className="hb-empty">{t("pickAlert")}</div>
        </main>
      )}
    </>
  );
}
