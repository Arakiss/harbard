"use client";

import type { Route } from "next";
import * as React from "react";
import Link from "next/link";
import { createT } from "@/i18n";
import { channelMembers, isWorkingNow, memberRole } from "@/lib/members";
import { agentName, presenceLine, type WorkspaceView, type WsAgent, type WsChannel } from "@/lib/workspace";
import { AgentAvatar } from "./parts";

const t = createT("workspace");

/** One row of the popover: robot, name, role · team and state, a link to its conversation. */
function MemberRow({ a, now, first }: { a: WsAgent; now: number; first?: React.Ref<HTMLAnchorElement> }) {
  const working = isWorkingNow(a);
  return (
    <li>
      <Link ref={first} href={`/direct/${encodeURIComponent(a.key)}` as Route} className="hb-member-row" data-working={working ? "true" : undefined}>
        <AgentAvatar name={a.name} size={28} agent={a} />
        <span className="hb-member-text">
          <span className="hb-member-name">{agentName(a)}</span>
          <span className="hb-member-role">{memberRole(a)}</span>
        </span>
        <span className="hb-member-state">{presenceLine(a, now)}</span>
      </Link>
    </li>
  );
}

/** The popover's body: who works now, then everyone else by last activity. */
export function MembersList({ view, channel, first }: { view: WorkspaceView; channel: WsChannel; first?: React.Ref<HTMLAnchorElement> }) {
  const m = channelMembers(view, channel);
  const working = m.ordered.slice(0, m.now);
  const rest = m.ordered.slice(m.now);
  return (
    <>
      <div className="hb-pop-title">{m.label}</div>
      {working.length > 0 ? (
        <>
          <div className="hb-member-group">{t("membersWorkingNow", { n: working.length })}</div>
          <ul className="hb-member-list">
            {working.map((a, i) => (
              <MemberRow key={a.key} a={a} now={view.now_ms} first={i === 0 ? first : undefined} />
            ))}
          </ul>
        </>
      ) : null}
      {rest.length > 0 ? (
        <>
          <div className="hb-member-group">{t("membersEarlier", { n: rest.length })}</div>
          <ul className="hb-member-list">
            {rest.map((a, i) => (
              <MemberRow key={a.key} a={a} now={view.now_ms} first={working.length === 0 && i === 0 ? first : undefined} />
            ))}
          </ul>
        </>
      ) : null}
    </>
  );
}

/**
 * The channel header's members button: three robots and the total, on one
 * line (who works now is in the header line). It says what the numbers are
 * in its tooltip and aria-label, and opens the members popover.
 */
export function ChannelMembers({ view, channel }: { view: WorkspaceView; channel: WsChannel }) {
  const m = channelMembers(view, channel);
  const [open, setOpen] = React.useState(false);
  const wrap = React.useRef<HTMLDivElement>(null);
  const button = React.useRef<HTMLButtonElement>(null);
  const first = React.useRef<HTMLAnchorElement>(null);
  const close = React.useCallback((focus: boolean) => {
    setOpen(false);
    if (focus) button.current?.focus();
  }, []);
  React.useEffect(() => {
    if (!open) return;
    first.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      // Esc here closes the popover; it does not mark the channel read.
      e.preventDefault();
      e.stopImmediatePropagation();
      close(true);
    };
    const onDown = (e: MouseEvent) => wrap.current && !wrap.current.contains(e.target as Node) && close(false);
    document.addEventListener("keydown", onKey, true);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      document.removeEventListener("mousedown", onDown);
    };
  }, [open, close]);
  if (m.total === 0) return null;
  return (
    <div ref={wrap} className="hb-members-wrap">
      <button
        ref={button}
        type="button"
        className="hb-members"
        title={m.label}
        aria-label={m.label}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <span className="hb-members-stack" aria-hidden>
          {m.stack.map((p) => (
            <AgentAvatar key={p.key} name={p.name} size={24} agent={p} dot={false} />
          ))}
        </span>
        {/* Who works now is in the header line; the button keeps the total. */}
        <span className="hb-members-total" aria-hidden>
          {m.total}
        </span>
      </button>
      {open ? (
        <div className="hb-members-pop" role="dialog" aria-label={t("membersOf", { channel: channel.id })} onClick={(e) => (e.target as HTMLElement).closest("a") && close(false)}>
          <MembersList view={view} channel={channel} first={first} />
        </div>
      ) : null}
    </div>
  );
}
