"use client";

import type { Route } from "next";
import * as React from "react";
import { FeedbackRow } from "@/components/feedback/feedback";
import { createT, intlTag, richText } from "@/i18n";
import { messageTarget } from "@/lib/feedback";
import Link from "next/link";
import { OperatorAvatar } from "./operator-avatar";
import { Bookmark, BookmarkCheck, CheckCircle2, ChevronRight, Compass, FileText, MessageSquare, SendHorizontal, Plus } from "lucide-react";
import {
  labelChip,
  leadDisplayName,
  leadKeyOf,
  leadRoleLabel,
  robotName,
  applyMention,
  deliveryLine,
  noteReader,
  matchMentions,
  mentionAt,
  mentionOptions,
  mentionSegments,
  type MentionOption,
  byDay,
  dotTone,
  fileName,
  foldText,
  hhmm,
  monogram,
  openFile,
  operatorName,
  replyBody,
  replyError,
  repeatsOf,
  replyCount,
  shortPath,
  agentName,
  agentRoleLine,
  shownName,
  tone,
  type WsAgent,
  type WsMessage,
  type WorkspaceView,
} from "@/lib/workspace";
import { microState } from "@/lib/activity";
import { machineText, RUN_SHOWN, machineMark, machineMarks, readablePartText, readableText, runs, techBody, type MachineMark, type StreamItem } from "@/lib/legible";
import { Robot } from "@/components/robot/robot";
import { StatusLine } from "@/components/robot/status";
import { WorkspaceRefused, useAgentLive, useLive, useOptionalView, useWorkspace } from "./workspace-context";
import { AgentHover } from "./agent-card";
import { AnswerEcho, MessageAttachments, operatorAnswer } from "@/components/huginn/view";
import { operatorTimeZone } from "@/lib/time-zone";

const t = createT("messages");

/** The OS "Reduce motion", live. */
export function useOsReducedMotion(): boolean {
  const [reduce, setReduce] = React.useState(false);
  React.useEffect(() => {
    const q = window.matchMedia("(prefers-reduced-motion: reduce)");
    setReduce(q.matches);
    const on = (e: MediaQueryListEvent) => setReduce(e.matches);
    q.addEventListener("change", on);
    return () => q.removeEventListener("change", on);
  }, []);
  return reduce;
}

/**
 * An agent's avatar: its robot, with the live face and its reactions, and
 * the presence dot. Without an agent (a peer known only by name) the same
 * robot by name, resting.
 */
export function AgentAvatar({
  name,
  agent,
  size = 36,
  dot = true,
}: {
  name: string;
  agent?: WsAgent | null;
  size?: number;
  dot?: boolean;
}) {
  const live = useLive();
  const os = useOsReducedMotion();
  const { activity, reactions } = useAgentLive(agent);
  return (
    <Robot
      name={agent ? robotName(agent) : name}
      size={size}
      agentKey={agent?.key}
      activity={activity}
      thresholds={live?.thresholds ?? null}
      now={live?.serverNow || undefined}
      reactions={reactions}
      baseline={live?.baseline}
      reduce={os || live?.motion === "reduce"}
      dot={dot && agent ? dotTone(agent) : null}
    />
  );
}

/** The agent's status line: icon, words and a ticking timer. */
export function AgentStatus({ agent, showTime = true, className }: { agent: WsAgent; showTime?: boolean; className?: string }) {
  const live = useLive();
  const { activity } = useAgentLive(agent);
  if (!activity) return null;
  const micro = live?.thresholds ? microState(activity, live.serverNow, live.thresholds) : null;
  return (
    <StatusLine
      activity={activity}
      micro={micro}
      reports={agent.reports.length}
      serverNow={live?.serverNow ?? 0}
      offset={live?.offset ?? 0}
      showTime={showTime}
      className={className}
    />
  );
}

export function Avatar({
  name,
  size = 36,
  agent,
  colour,
  initials,
}: {
  name: string;
  size?: number;
  agent?: Pick<WsAgent, "presence" | "dormant"> | null;
  colour?: string;
  /** Overrides the derived monogram (the operator's, from config). */
  initials?: string;
}) {
  return (
    <span
      className="hb-av"
      style={{ width: size, height: size, background: colour ?? tone(name), fontSize: Math.round(size * 0.4), borderRadius: Math.max(6, Math.round(size * 0.22)) }}
      aria-hidden
    >
      {initials || monogram(name)}
      {agent ? <span className="hb-av-dot" data-tone={dotTone(agent)} /> : null}
    </span>
  );
}

/** One more entry of a row's ⋯ menu (and its right-click). */
export interface RowAction {
  label: string;
  run: () => void;
}

/**
 * A row's menu: right-click, or the "⋯" button that shows on hover and
 * focus. "Mark as read" marks the row's channel or conversation, when
 * `onRead` is given; then come `actions` (such as "Hide"). ↑ and ↓ move between the
 * items, Esc closes it and gives the focus back to ⋯.
 */
export function RowMenu({ label, onRead, actions = [], children }: { label: string; onRead?: () => void; actions?: RowAction[]; children: React.ReactNode }) {
  const [open, setOpen] = React.useState(false);
  const ref = React.useRef<HTMLDivElement>(null);
  const menu = React.useRef<HTMLDivElement>(null);
  const more = React.useRef<HTMLButtonElement>(null);
  const items: RowAction[] = [...(onRead ? [{ label: t("markRead"), run: onRead }] : []), ...actions];
  React.useEffect(() => {
    if (!open) return;
    menu.current?.querySelector<HTMLButtonElement>("[role='menuitem']")?.focus();
    const away = (e: MouseEvent) => ref.current && !ref.current.contains(e.target as Node) && setOpen(false);
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);
  if (items.length === 0) return <>{children}</>;
  return (
    <div
      ref={ref}
      className="hb-row-wrap"
      onContextMenu={(e) => {
        e.preventDefault();
        setOpen(true);
      }}
    >
      {children}
      <button ref={more} type="button" className="hb-row-more" aria-label={t("moreActions", { label })} aria-haspopup="menu" aria-expanded={open} onClick={() => setOpen(!open)}>
        ⋯
      </button>
      {open ? (
        <div
          ref={menu}
          className="hb-row-menu"
          role="menu"
          aria-label={label}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              e.stopPropagation();
              setOpen(false);
              more.current?.focus();
              return;
            }
            if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
            e.preventDefault();
            const all = [...(menu.current?.querySelectorAll<HTMLButtonElement>("[role='menuitem']") ?? [])];
            const at = all.indexOf(document.activeElement as HTMLButtonElement);
            const next = (at + (e.key === "ArrowDown" ? 1 : -1) + all.length) % all.length;
            all[next]?.focus();
          }}
        >
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
    </div>
  );
}

/**
 * Esc marks the open channel or conversation read, as in a chat app; a
 * dialog, a menu or a list that handles Esc itself stops it first.
 */
export function useEscRead(onRead: (() => void) | null) {
  React.useEffect(() => {
    if (!onRead) return;
    const on = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.shiftKey || e.defaultPrevented) return;
      if (document.querySelector(".hb-lightbox, .hb-finder-scrim, .hb-report-pane, [role='dialog']")) return;
      onRead();
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, [onRead]);
}

/**
 * An agent's name and, under a workshop name, its `<role> · <team>`.
 * With `editable`, a double click renames it (Enter keeps, Esc leaves it).
 */
export function AgentName({
  agent,
  className,
  editable = false,
}: {
  agent: Pick<WsAgent, "key" | "name" | "name_code" | "name_params" | "role_line" | "role_line_code" | "role_line_params" | "name_source">;
  className?: string;
  editable?: boolean;
}) {
  const { post } = useWorkspace();
  const [editing, setEditing] = React.useState(false);
  const [draft, setDraft] = React.useState(agent.name);
  const [error, setError] = React.useState<string | null>(null);
  const canRename = editable && Boolean(agent.name_source);
  const save = () => {
    const name = draft.trim();
    setEditing(false);
    if (!name || name === agent.name) return;
    post("rename-agent", { agent: agent.key, name })
      .then(() => setError(null))
      .catch((e: unknown) => setError(e instanceof WorkspaceRefused && e.status === 409 ? t("nameTaken") : t("nameSaveFailed")));
  };
  return (
    <span className="hb-agent-name">
      {editing ? (
        <input
          className="hb-rename"
          value={draft}
          autoFocus
          aria-label={t("newNameFor", { name: agent.name })}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={save}
          onKeyDown={(e) => {
            if (e.key === "Enter") save();
            if (e.key === "Escape") {
              e.stopPropagation();
              setEditing(false);
              setDraft(agent.name);
            }
          }}
        />
      ) : (
        <span
          className={className}
          title={canRename ? t("doubleClickRename") : undefined}
          onDoubleClick={
            canRename
              ? () => {
                  setDraft(agent.name);
                  setEditing(true);
                }
              : undefined
          }
        >
          {agentName(agent)}
        </span>
      )}
      {agent.role_line ? <span className="hb-role-line">{agentRoleLine(agent)}</span> : null}
      {error ? <span className="hb-error block">{error}</span> : null}
    </span>
  );
}

/** 👍 or 👎 on a workshop name; a 👎 gives the agent a new one, and the old one is never given again. */
export function NameVotes({ agent }: { agent: Pick<WsAgent, "key" | "name" | "name_source"> }) {
  const { post } = useWorkspace();
  const [done, setDone] = React.useState<"up" | "down" | null>(null);
  if (!agent.name_source) return null;
  const vote = (v: "up" | "down") => {
    setDone(v);
    void post("name-vote", { agent: agent.key, vote: v }).catch(() => setDone(null));
  };
  return (
    <span className="hb-name-votes" role="group" aria-label={t("likeName", { name: agent.name })}>
      <button type="button" aria-pressed={done === "up"} onClick={() => vote("up")} title={t("nameUp")}>
        👍
      </button>
      <button type="button" aria-pressed={done === "down"} onClick={() => vote("down")} title={t("nameDown")}>
        👎
      </button>
    </span>
  );
}

/** A message's text with the `@` mentions it holds highlighted. */
export function MentionText({ text, mentions }: { text: string; mentions?: string[] }) {
  return (
    <>
      {mentionSegments(text, mentions).map((seg, i) =>
        seg.mention ? (
          <span key={i} className="hb-mention">
            {seg.text}
          </span>
        ) : (
          <React.Fragment key={i}>{seg.text}</React.Fragment>
        ),
      )}
    </>
  );
}

/** The lead session: Claude, the coordinator, with a look of its own. */
function LeadAvatar({ size = 36 }: { size?: number }) {
  return (
    <span
      className="hb-av hb-av-lead"
      style={{ width: size, height: size, borderRadius: Math.max(6, Math.round(size * 0.22)) }}
      aria-hidden
      data-lead="true"
    >
      <Compass style={{ width: Math.round(size * 0.58), height: Math.round(size * 0.58) }} strokeWidth={2} />
    </span>
  );
}

/** One quiet grey marker instead of the protocol chips; the reasons in its tooltip. */
function MachineMarker({ mark }: { mark: MachineMark | null }) {
  if (!mark) return null;
  return (
    <span className="hb-machine" title={mark.hint} aria-label={`${machineText()}: ${mark.reasons.join(", ")}`} data-machine={mark.reasons.length}>
      {machineText()}
    </span>
  );
}

/**
 * The plain line shown for a line written for machines: text, with
 * commit codes, branches and paths as small grey chips (the whole value in
 * the tooltip). The original is under "Technical details".
 */
function ReadableLine({ parts }: { parts: NonNullable<WsMessage["readable"]> }) {
  return (
    <div className="hb-msg-text" data-readable="true">
      {parts.map((p, i) =>
        p.kind === "text" ? (
          <React.Fragment key={i}>{readablePartText(p)}</React.Fragment>
        ) : (
          <span key={i} className="hb-tok hb-mono" data-kind={p.kind} title={p.full ?? p.text}>
            {p.text}
          </span>
        ),
      )}
    </div>
  );
}

/** "Technical details": the body of a v2 message, with the file cards. */
export function TechDetails({ body, files, open }: { body: string; files: React.ReactNode; open?: boolean }) {
  return (
    <details className="hb-tech" open={open}>
      <summary>{t("techDetails")}</summary>
      <div className="hb-tech-body hb-mono">{body}</div>
      {files}
    </details>
  );
}

export function LabelChip({ label }: { label: WsMessage["label"] }) {
  if (!label) return null;
  const chip = labelChip(label);
  return (
    <span className="hb-chip" data-tone={chip.tone}>
      {chip.text}
    </span>
  );
}

/** A local report. Opening it asks serve to hand the path to the OS. */
export function FileCard({ path }: { path: string }) {
  const { post } = useWorkspace();
  const [error, setError] = React.useState<string | null>(null);
  return (
    <span>
      <button
        type="button"
        className="hb-file"
        title={t("openFile", { path })}
        onClick={() => {
          setError(null);
          openFile(path, post).catch((e: unknown) =>
            setError(e instanceof WorkspaceRefused && e.status === 404 ? t("fileMissing") : t("fileOpenFailed")),
          );
        }}
      >
        <FileText className="size-6 shrink-0" strokeWidth={1.6} aria-hidden />
        <span className="min-w-0">
          <span className="block truncate">{fileName(path)}</span>
          <span className="hb-file-sub truncate">{shortPath(path)}</span>
        </span>
      </button>
      {error ? <span className="hb-error block">{error}</span> : null}
    </span>
  );
}

function useSaved(view: WorkspaceView, id: string): boolean {
  return view.saved.some((s) => s.id === id);
}

export function MessageItem({
  view,
  message,
  onOpenThread,
  selected,
  showReplies = true,
  whole = false,
  readOnly = false,
  machine,
}: {
  view: WorkspaceView;
  message: WsMessage;
  onOpenThread?: (id: string) => void;
  selected?: boolean;
  showReplies?: boolean;
  /** "written for machines", as the stream decided for its run; alone when absent. */
  machine?: MachineMark | null;
  /** The thread pane: the text whole and the technical details open, never folded. */
  whole?: boolean;
  /** An archived conversation (History): nothing to save, rate or answer. */
  readOnly?: boolean;
}) {
  const { post } = useWorkspace();
  const agent = view.agents.find((a) => a.key === message.agent) ?? null;
  const isOperator = message.from === "operator";
  const isLead = message.from === "lead";
  const leadKey = isLead ? leadKeyOf(view.agents, message.agent) : null;
  const leadAgent = leadKey ? (view.agents.find((a) => a.key === leadKey) ?? null) : null;
  const name = message.from === "agent" && agent ? agentName(agent) : shownName(message.from === "agent" ? message.author : isOperator ? operatorName(message.author) : isLead ? leadDisplayName(message.author) : message.author);
  const saved = useSaved(view, message.id);
  const copies = repeatsOf(view, message.id);
  // A repeat is collapsed under its first copy, so it is not drawn twice.
  if (message.repeat_of && view.messages.some((m) => m.id === message.repeat_of)) return null;
  const feedbackKind = messageTarget(message);
  const viewAnswer = operatorAnswer(message);
  const details = techBody(message);
  const profileHref = agent && message.from === "agent" ? `/direct/${encodeURIComponent(agent.key)}` : undefined;
  const files =
    message.files.length > 0 ? (
      <div className="hb-files">
        {message.files.map((f) => (
          <FileCard key={f} path={f} />
        ))}
      </div>
    ) : null;
  return (
    <div className="hb-msg" data-selected={selected ? "true" : undefined} data-message={message.id}>
      {message.from === "agent" ? (
        <AgentHover agent={agent}>
          <AgentAvatar name={name} agent={agent} />
        </AgentHover>
      ) : message.from === "peer" || message.from === "session" ? (
        <AgentHover agent={view.agents.find((a) => a.key === message.agent || a.name === message.author) ?? null}>
          <AgentAvatar name={name} agent={view.agents.find((a) => a.key === message.agent || a.name === message.author) ?? null} />
        </AgentHover>
      ) : isLead && leadKey ? (
        <AgentHover agent={leadAgent}>
          <AgentAvatar name={leadKey} agent={leadAgent} />
        </AgentHover>
      ) : isLead ? (
        <LeadAvatar />
      ) : isOperator ? (
        <OperatorAvatar name={name} initials={view.operator_initials} size={36} presence={null} />
      ) : (
        <Avatar name={name} colour="var(--hb-av-lead)" />
      )}
      <div className="hb-msg-body">
        <div className="hb-msg-meta">
          {profileHref ? (
            <AgentHover agent={agent}>
              <Link href={profileHref as Route} className="hb-msg-name">
                {name}
              </Link>
            </AgentHover>
          ) : (
            <span className="hb-msg-name">{name}</span>
          )}
          {isLead ? <span className="hb-chip hb-lead-tag">{leadRoleLabel()}</span> : null}
          <LabelChip label={message.label} />
          <span className="hb-msg-time" title={new Date(message.ts_ms).toLocaleString(intlTag(), { timeZone: operatorTimeZone() })}>
            {hhmm(message.ts_ms)}
          </span>
          <MachineMarker mark={machine === undefined ? machineMark(message) : machine} />
        </div>
        {viewAnswer ? (
          <AnswerEcho answer={viewAnswer} message={message} />
        ) : message.readable && message.readable.length > 0 ? (
          <ReadableLine parts={message.readable} />
        ) : message.text ? (
          <MessageText text={message.text} mentions={message.mentions} whole={whole} />
        ) : (
          <div className="hb-msg-text">
            <em className="text-[13px] text-[var(--hb-ink-4)]">{t("noText")}</em>
          </div>
        )}
        {message.delivery ? (
          <div className="hb-delivery" data-state={message.delivery.state}>
            {deliveryLine(message.delivery, noteReader(view.agents, message.agent))}
          </div>
        ) : null}
        <MessageAttachments message={message} />
        {details ? <TechDetails body={details} files={files} open={whole || undefined} /> : files}
        {message.verified ? (
          <span
            className="hb-pill"
            title={t("verifiedTitle", { sha: message.verified.sha, branch: message.verified.branch, time: hhmm(message.verified.decided_ms) })}
          >
            <CheckCircle2 className="size-3.5" aria-hidden /> {t("verified")}
          </span>
        ) : null}
        {copies.length > 0 ? (
          <div className="hb-repeat">
            {copies.length === 1
              ? t("repeatOnce", { times: copies.map((c) => hhmm(c.ts_ms)).join(", ") })
              : t("repeatMany", { n: copies.length, times: copies.map((c) => hhmm(c.ts_ms)).join(", ") })}
          </div>
        ) : null}
        {showReplies && message.replies.length > 0 && onOpenThread ? (
          <button type="button" className="hb-replies" onClick={() => onOpenThread(message.id)}>
            <MessageSquare className="size-[18px]" strokeWidth={1.8} aria-hidden />
            {replyCount(message.replies.length)}
          </button>
        ) : null}
      </div>
      {readOnly ? null : (
      <div className="hb-msg-tools">
        {feedbackKind ? <FeedbackRow kind={feedbackKind} id={message.id} /> : null}
        {onOpenThread && !message.reply_to ? (
          <button type="button" className="hb-tool" onClick={() => onOpenThread(message.id)} title={t("replyInThread")}>
            <MessageSquare className="size-4" aria-hidden />
          </button>
        ) : null}
        <button
          type="button"
          className="hb-tool"
          title={saved ? t("unsave") : t("save")}
          onClick={() => void post(saved ? "unsave" : "save", { id: message.id }).catch(() => {})}
        >
          {saved ? <BookmarkCheck className="size-4" aria-hidden /> : <Bookmark className="size-4" aria-hidden />}
        </button>
      </div>
      )}
    </div>
  );
}

/**
 * A message's text. In a channel a long one folds after its first lines,
 * with an explicit "See more" that unfolds it in place, never a silent "…";
 * `whole` (the thread pane) always shows all of it.
 */
export function MessageText({ text, mentions, whole = false }: { text: string; mentions?: string[]; whole?: boolean }) {
  const [open, setOpen] = React.useState(false);
  const head = whole ? null : foldText(text);
  if (head === null) {
    return (
      <div className="hb-msg-text">
        <MentionText text={text} mentions={mentions} />
      </div>
    );
  }
  return (
    <div className="hb-msg-text" data-folded={open ? undefined : "true"}>
      <MentionText text={open ? text : `${head}…`} mentions={mentions} />{" "}
      <button type="button" className="hb-fold" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        {open ? t("seeLess") : t("seeMore")}
      </button>
    </div>
  );
}

/**
 * "5 deliveries from acme": the older messages of a run, one line each
 * (who, when, what), the last few shown and "See all" for the rest. A line
 * opens its message whole, in place; the newest is drawn whole after it.
 */
function RunGroup({
  view,
  run,
  selected,
  expand,
}: {
  view: WorkspaceView;
  run: Extract<StreamItem, { kind: "run" }>;
  selected?: string | null;
  expand: (m: WsMessage) => React.ReactNode;
}) {
  const [all, setAll] = React.useState(false);
  const [open, setOpen] = React.useState<Set<string>>(() => new Set());
  const hidden = all ? 0 : Math.max(0, run.older.length - RUN_SHOWN);
  const shown = run.older.slice(hidden);
  return (
    <div className="hb-run" data-run={run.id}>
      <div className="hb-run-head">
        {run.label}
        {hidden > 0 || all ? (
          <button type="button" className="hb-fold" aria-expanded={all} onClick={() => setAll(!all)}>
            {all ? t("seeLess") : t("seeAllCount", { n: run.older.length + 1 })}
          </button>
        ) : null}
      </div>
      <ul className="hb-run-list">
        {shown.map((m) => {
          if (open.has(m.id) || selected === m.id) return <li key={m.id}>{expand(m)}</li>;
          const agent = view.agents.find((a) => a.key === m.agent) ?? null;
          const name = agent ? agentName(agent) : shownName(m.author);
          const line = m.readable && m.readable.length > 0 ? readableText(m.readable) : m.text;
          return (
            <li key={m.id}>
              <button
                type="button"
                className="hb-run-row"
                data-message={m.id}
                title={t("seeWhole")}
                onClick={() => setOpen((s) => new Set(s).add(m.id))}
              >
                <AgentAvatar name={name} agent={agent} size={20} dot={false} />
                <span className="hb-run-name">{name}</span>
                <span className="hb-msg-time">{hhmm(m.ts_ms)}</span>
                <span className="hb-run-text">{line}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/** Messages under "Today" / "Yesterday" separators. */
/** Agents' work chatter, folded: nothing in it was written for the operator. */
function InternalGroup({
  view,
  group,
  selected,
  expand,
}: {
  view: WorkspaceView;
  group: Extract<StreamItem, { kind: "internal" }>;
  selected?: string | null;
  expand: (m: WsMessage) => React.ReactNode;
}) {
  const [open, setOpen] = React.useState(false);
  const shown = open || group.messages.some((m) => m.id === selected);
  const faces = [...new Set(group.messages.map((m) => m.agent))]
    .slice(0, 4)
    .map((key) => view.agents.find((a) => a.key === key) ?? null);
  return (
    <div className="hb-internal" data-run={group.id}>
      <button type="button" className="hb-internal-head" aria-expanded={shown} onClick={() => setOpen(!shown)}>
        <ChevronRight className="hb-internal-chevron size-3.5" aria-hidden />
        <span className="hb-internal-faces" aria-hidden>
          {faces.map((agent, i) => (
            <AgentAvatar key={agent?.key ?? i} name={agent?.name ?? ""} agent={agent} size={20} dot={false} />
          ))}
        </span>
        <span>{group.label}</span>
      </button>
      {shown ? group.messages.map(expand) : null}
    </div>
  );
}

export function Stream({
  view,
  messages,
  now,
  onOpenThread,
  selected,
  className,
  empty,
  readOnly = false,
  foldInternal = false,
  scroller,
  head,
}: {
  view: WorkspaceView;
  messages: WsMessage[];
  now: number;
  onOpenThread?: (id: string) => void;
  selected?: string | null;
  className?: string;
  empty?: React.ReactNode;
  /** The scrolling element, for a caller that loads older messages above. */
  scroller?: React.RefObject<HTMLDivElement | null>;
  /** Above the first message: "load older", or the start of the room. */
  head?: React.ReactNode;
  /** An archived conversation: messages without their tools. */
  readOnly?: boolean;
  /** A channel: agents' reports to their leads fold away. */
  foldInternal?: boolean;
}) {
  const own = React.useRef<HTMLDivElement>(null);
  const ref = scroller ?? own;
  const last = messages[messages.length - 1]?.id;
  const marks = React.useMemo(() => machineMarks(messages), [messages]);
  React.useEffect(() => {
    const el = ref.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [last, ref]);
  return (
    <div className={`hb-stream ${className ?? ""}`} ref={ref}>
      {head}
      {messages.length === 0 ? empty : null}
      {byDay(messages, now).map((group) => (
        <section key={group.day} aria-label={group.day}>
          <div className="hb-day">
            <span>{group.day}</span>
          </div>
          {runs(view, group.items, { foldInternal }).map((item) => {
            const one = (m: WsMessage) => (
              <MessageItem
                key={m.id}
                view={view}
                message={m}
                onOpenThread={onOpenThread}
                selected={selected === m.id}
                readOnly={readOnly}
                machine={marks.get(m.id) ?? null}
              />
            );
            if (item.kind === "message") return one(item.message);
            if (item.kind === "internal") {
              return <InternalGroup key={item.id} view={view} group={item} selected={selected} expand={one} />;
            }
            return (
              <React.Fragment key={item.id}>
                <RunGroup view={view} run={item} selected={selected} expand={one} />
                {one(item.newest)}
              </React.Fragment>
            );
          })}
        </section>
      ))}
    </div>
  );
}

/** What "Attach file" uploaded, cited in the message when it is sent. */
interface Uploaded {
  path: string;
  name: string;
}

/** Send the picked file's bytes to serve, which stores them like `harbard attach`. */
async function uploadAttachment(file: Blob & { name: string }): Promise<Uploaded> {
  const r = await fetch(`/api/harbard/attachments?name=${encodeURIComponent(file.name)}`, {
    method: "POST",
    headers: { "content-type": file.type || "application/octet-stream" },
    body: file,
  });
  const body = (await r.json().catch(() => ({}))) as { path?: string; name?: string; message?: string; error?: string };
  if (!r.ok || !body.path) throw new Error(body.message ?? body.error ?? `HTTP ${r.status}`);
  return { path: body.path, name: body.name ?? file.name };
}

/** The "+" menu: every item does something, or it is not there. */
export function PlusMenu({
  onAttach,
  onMention,
  agents,
  onReview,
  onNewThread,
  onClose,
}: {
  onAttach: () => void;
  onMention: () => void;
  agents: Pick<WsAgent, "key" | "name">[];
  onReview: (agent: string) => void;
  onNewThread?: () => void;
  onClose: () => void;
}) {
  const [picking, setPicking] = React.useState(false);
  const first = React.useRef<HTMLButtonElement>(null);
  React.useEffect(() => first.current?.focus(), [picking]);
  const item = (label: string, run: () => void, ref?: React.Ref<HTMLButtonElement>) => (
    <button
      ref={ref}
      type="button"
      role="menuitem"
      className="hb-pop-item"
      onClick={() => {
        run();
      }}
    >
      {label}
    </button>
  );
  return (
    <div
      className="hb-plus-menu"
      role="menu"
      aria-label={picking ? t("askReviewFrom") : t("add")}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onClose();
        }
      }}
    >
      {picking ? (
        <>
          <div className="hb-pop-title">{t("askReviewFrom")}</div>
          {agents.length === 0 ? <p className="hb-pop-empty">{t("noReviewers")}</p> : null}
          {agents.map((a, i) =>
            item(
              a.name,
              () => {
                onClose();
                onReview(a.key);
              },
              i === 0 ? first : undefined,
            ),
          )}
        </>
      ) : (
        <>
          {item(
            t("attachFile"),
            () => {
              onClose();
              onAttach();
            },
            first,
          )}
          {item(t("mention"), () => {
            onClose();
            onMention();
          })}
          {agents.length > 0 ? item(t("askReviewFrom"), () => setPicking(true)) : null}
          {onNewThread
            ? item(t("newThread"), () => {
                onClose();
                onNewThread();
              })
            : null}
        </>
      )}
    </div>
  );
}

/** The `@` suggestions under the composer. */
function MentionList({ options, at, onPick }: { options: MentionOption[]; at: number; onPick: (o: MentionOption) => void }) {
  if (options.length === 0) return null;
  return (
    <ul className="hb-mention-list" role="listbox" aria-label={t("mention")}>
      {options.map((o, i) => (
        <li
          key={o.kind + o.handle}
          role="option"
          aria-selected={i === at}
          data-kind={o.kind}
          onMouseDown={(e) => {
            e.preventDefault();
            onPick(o);
          }}
        >
          <span className="hb-mention-label">{o.label}</span>
          <span className="hb-mention-hint">{o.hint}</span>
        </li>
      ))}
    </ul>
  );
}

/**
 * Where the operator writes. A reply is stored on the bus: to an agent as an ask
 * its lead relays (or its own session reads), in a channel as a note per
 * reader. `@` mentions wake whoever they name; "+" attaches a file, inserts
 * an `@`, asks an agent for a review or starts a new thread.
 */
export function Composer({
  target,
  placeholder,
  blocked,
  hint,
  notice,
  confirm = null,
  variant = "line",
  onSent,
  onNewThread,
}: {
  target: { agent: string; replyTo?: string | null } | { channel: string };
  placeholder: string;
  blocked: string | null;
  hint?: string;
  /** A stale lead: shown above the composer; the note is stored and waits. */
  notice?: string | null;
  /** Nobody live reads it: asked before sending ("send anyway?"). */
  confirm?: string | null;
  variant?: "line" | "box";
  onSent?: () => void;
  /** In a thread: "New thread" leaves it for a new top-level message. */
  onNewThread?: (draft: string) => void;
}) {
  const { post } = useWorkspace();
  const view = useOptionalView();
  const [text, setText] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [asking, setAsking] = React.useState(false);
  const [plus, setPlus] = React.useState(false);
  const [uploads, setUploads] = React.useState<Uploaded[]>([]);
  const [caret, setCaret] = React.useState(0);
  const [pick, setPick] = React.useState(0);
  const [mentionOff, setMentionOff] = React.useState(false);
  const area = React.useRef<HTMLTextAreaElement>(null);
  const file = React.useRef<HTMLInputElement>(null);
  const channel = "channel" in target ? target.channel : null;
  const options = React.useMemo(() => (view ? mentionOptions(view, channel) : []), [view, channel]);
  const typing = mentionAt(text, caret);
  const shown = typing && !mentionOff ? matchMentions(options, typing.query) : [];
  if (blocked) {
    return (
      <div className={variant === "line" ? "hb-composer" : undefined}>
        <div className="hb-composer-none" role="note">
          {blocked}.
        </div>
      </div>
    );
  }
  const full = () => [text.trim(), ...uploads.map((u) => u.path)].filter(Boolean).join("\n");
  const send = async (anyway = false) => {
    const body = full();
    if (!body || busy) return;
    if (confirm && !anyway) {
      setAsking(true);
      return;
    }
    setAsking(false);
    setBusy(true);
    setError(null);
    try {
      await post("reply", replyBody(target, body, anyway));
      setText("");
      setUploads([]);
      onSent?.();
    } catch (e) {
      setError(e instanceof WorkspaceRefused ? replyError(e.status, e.reason) : replyError(undefined, undefined));
    } finally {
      setBusy(false);
    }
  };
  const choose = (o: MentionOption) => {
    if (!typing) return;
    const next = applyMention(text, typing.start, caret, o.handle);
    setText(next.text);
    setCaret(next.caret);
    requestAnimationFrame(() => area.current?.setSelectionRange(next.caret, next.caret));
  };
  const onKey = (e: React.KeyboardEvent<HTMLTextAreaElement>, submit: (e: React.KeyboardEvent) => boolean) => {
    if (shown.length > 0) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setPick((i) => (e.key === "ArrowDown" ? (i + 1) % shown.length : (i - 1 + shown.length) % shown.length));
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        choose(shown[Math.min(pick, shown.length - 1)]!);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setMentionOff(true);
        return;
      }
    }
    if (submit(e)) {
      e.preventDefault();
      void send();
    }
  };
  const track = (e: React.SyntheticEvent<HTMLTextAreaElement>) => setCaret(e.currentTarget.selectionStart ?? 0);
  const change = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    setText(e.target.value);
    setCaret(e.target.selectionStart ?? e.target.value.length);
    setMentionOff(false);
    setPick(0);
  };
  const insertAt = () => {
    const at = area.current?.selectionStart ?? text.length;
    const lead = at > 0 && !/\s$/.test(text.slice(0, at)) ? " @" : "@";
    const next = text.slice(0, at) + lead + text.slice(at);
    setText(next);
    setCaret(at + lead.length);
    setMentionOff(false);
    requestAnimationFrame(() => {
      area.current?.focus();
      area.current?.setSelectionRange(at + lead.length, at + lead.length);
    });
  };
  const review = (agent: string) => {
    const ask = text.trim() ? t("reviewRequest", { text: text.trim() }) : t("reviewRequestLatest");
    post("reply", replyBody({ agent }, [ask, ...uploads.map((u) => u.path)].join("\n")))
      .then(() => {
        setText("");
        setUploads([]);
        setError(null);
      })
      .catch((e: unknown) => setError(e instanceof WorkspaceRefused ? replyError(e.status, e.reason) : replyError(undefined, undefined)));
  };
  const picked = (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = [...(e.target.files ?? [])];
    e.target.value = "";
    for (const f of files) {
      uploadAttachment(f)
        .then((u) => setUploads((list) => [...list, u]))
        .catch((err: unknown) => setError(t("attachFailed", { name: f.name, error: err instanceof Error ? err.message : String(err) })));
    }
  };
  const reviewers = (view?.agents ?? []).filter((a) => !a.temporary && a.presence !== "finished").slice(0, 12);
  const wait = asking && confirm ? (
    <ConfirmSend question={confirm} onSend={() => void send(true)} onCancel={() => setAsking(false)} />
  ) : notice ? (
    <p className="hb-composer-wait" role="note">
      {notice.charAt(0).toUpperCase() + notice.slice(1)}.
    </p>
  ) : null;
  const chips =
    uploads.length > 0 ? (
      <div className="hb-uploads">
        {uploads.map((u) => (
          <span key={u.path} className="hb-upload">
            <FileText className="size-4" aria-hidden />
            {u.name}
            <button type="button" aria-label={t("remove", { name: u.name })} onClick={() => setUploads((l) => l.filter((x) => x.path !== u.path))}>
              ×
            </button>
          </span>
        ))}
      </div>
    ) : null;
  const picker = <input ref={file} type="file" className="hidden" multiple onChange={picked} aria-hidden tabIndex={-1} />;
  const mentions = <MentionList options={shown} at={Math.min(pick, Math.max(0, shown.length - 1))} onPick={choose} />;
  if (variant === "box") {
    return (
      <div className="hb-composer-wrap">
        {wait}
        {mentions}
        <textarea
          ref={area}
          className="hb-reply-box"
          placeholder={placeholder}
          value={text}
          onChange={change}
          onSelect={track}
          onKeyDown={(e) => onKey(e, (k) => k.key === "Enter" && (k.metaKey || k.ctrlKey))}
          aria-label={placeholder}
        />
        {chips}
        {error ? <p className="hb-error">{error}</p> : hint ? <p className="hb-composer-hint">{hint}</p> : null}
        <ComposerActions send={() => void send()} busy={busy} ready={full().length > 0} />
      </div>
    );
  }
  return (
    <div className="hb-composer hb-composer-wrap">
      {wait}
      {mentions}
      {chips}
      <div className="hb-composer-box">
        <span className="relative">
          <button type="button" className="hb-composer-plus" aria-label={t("add")} aria-haspopup="menu" aria-expanded={plus} onClick={() => setPlus(!plus)}>
            <Plus className="size-4" />
          </button>
          {plus ? (
            <PlusMenu
              onAttach={() => file.current?.click()}
              onMention={insertAt}
              agents={reviewers}
              onReview={review}
              onNewThread={onNewThread ? () => onNewThread(text) : undefined}
              onClose={() => setPlus(false)}
            />
          ) : null}
          {picker}
        </span>
        <textarea
          ref={area}
          rows={1}
          placeholder={placeholder}
          aria-label={placeholder}
          value={text}
          onChange={change}
          onSelect={track}
          onKeyDown={(e) => onKey(e, (k) => k.key === "Enter" && !k.shiftKey)}
        />
        <button type="button" className="hb-send" data-ready={full() ? "true" : undefined} onClick={() => void send()} disabled={busy} aria-label={t("send")}>
          <SendHorizontal className="size-5" aria-hidden />
        </button>
      </div>
      {error ? <p className="hb-error">{error}</p> : hint ? <p className="hb-composer-hint">{hint}</p> : null}
    </div>
  );
}

/** "Nobody will read it until a session opens; send anyway?", before sending. */
function ConfirmSend({ question, onSend, onCancel }: { question: string; onSend: () => void; onCancel: () => void }) {
  return (
    <div className="hb-confirm" role="alertdialog" aria-label={t("confirmSend")}>
      <span>{question}</span>
      <span className="hb-confirm-actions">
        <button type="button" className="hb-btn hb-btn-primary" onClick={onSend}>
          {t("sendAnyway")}
        </button>
        <button type="button" className="hb-btn" onClick={onCancel}>
          {t("cancel")}
        </button>
      </span>
    </div>
  );
}

/** Filled in by the Activity detail, which adds "Mark resolved" beside it. */
export const ComposerActionsContext = React.createContext<React.ReactNode>(null);

function ComposerActions({ send, busy, ready }: { send: () => void; busy: boolean; ready: boolean }) {
  const extra = React.useContext(ComposerActionsContext);
  return (
    <div className="hb-actions">
      <button type="button" className="hb-btn hb-btn-primary" onClick={send} disabled={busy || !ready}>
        {t("reply")}
      </button>
      {extra}
    </div>
  );
}

export function Loading({ state }: { state: ReturnType<typeof useWorkspace>["state"] }) {
  if (state.kind === "failed") {
    return (
      <div className="hb-empty" role="alert">
        <strong>{t("serveDown")}</strong>
        {richText(t("loadFailed"), {
          error: state.error,
          // i18n-exempt: a command, typed the same in every language.
          command: <span className="hb-mono">harbard serve</span>,
        })}
      </div>
    );
  }
  return (
    <div className="hb-empty" role="status">
      {t("loading")}
    </div>
  );
}
