"use client";

import * as React from "react";
import { ChevronDown, Clock3, Cloud, MessageSquareReply, Power } from "lucide-react";
import { createT } from "@/i18n";
import { calmModel, type CalmItem } from "@/lib/calm";
import { messageById, type WsAgent } from "@/lib/workspace";
import { CardActions } from "./activity";
import { AgentAvatar, Avatar } from "./parts";
import { useOptionalView, useOptionalWorkspace } from "./workspace-context";

const t = createT("home");

/**
 * Calm mode: Swamped's console. One card, the only thing that
 * needs them now, and one calm line; the sidebar keeps its headings and
 * nothing else. "Reply" opens the reply box inside the card: the shell
 * keeps painting this screen while Swamped lasts, so a link to Activity
 * would change the URL and nothing else (harbard-final-review AC2). "More
 * tarde" moves on to the next item; "Exit" leaves calm mode for this
 * Swamped.
 */
export function CalmScreen({ onExit }: { onExit: () => void }) {
  const view = useOptionalView();
  const now = useOptionalWorkspace()?.now ?? Date.now();
  const [later, setLater] = React.useState<Set<string>>(new Set());
  // The item whose reply box is open: another top closes it.
  const [replyingTo, setReplyingTo] = React.useState<string | null>(null);
  const model = calmModel(view, later, Date.now());
  const top = model.top;
  const agent = top?.agent ? (view?.agents.find((a) => a.key === top.agent) ?? null) : null;
  const card = top && view ? (view.cards.find((c) => `card:${c.id}` === top.id) ?? null) : null;
  const replying = top !== null && replyingTo === top.id;
  const reply =
    replying && view ? (
      <CardActions view={view} card={card} message={card?.message ? messageById(view, card.message) : undefined} agent={agent ?? undefined} now={now} />
    ) : null;
  return (
    <>
      <aside className="hb-sidebar hb-calm-sidebar" aria-label={t("home")}>
        <div className="hb-sidebar-head">
          harbard <ChevronDown className="size-5" strokeWidth={2.4} aria-hidden />
        </div>
        <div className="hb-sidebar-scroll">
          <div className="hb-section">{t("channels")}</div>
          <div className="hb-section hb-calm-gap">{t("agents")}</div>
        </div>
      </aside>
      <main className="hb-main hb-calm" aria-label={t("calmMode")} data-testid="calm">
        <header className="hb-calm-head">
          <span className="hb-calm-pill">
            <Cloud className="size-5" aria-hidden /> {t("calmPill")}
          </span>
          <button type="button" className="hb-btn hb-calm-exit" onClick={onExit} title={t("calmExitHint")}>
            <Power className="size-4" aria-hidden /> {t("calmExit")}
          </button>
        </header>
        <CalmCardView
          top={top}
          agent={agent}
          working={model.working}
          replying={replying}
          reply={reply}
          onReply={() => setReplyingTo(replying ? null : (top?.id ?? null))}
          onLater={() => top && setLater(new Set([...later, top.id]))}
        />
      </main>
    </>
  );
}

/** The calm card itself, without the workspace: what the tests render. */
export function CalmCardView({
  top,
  agent,
  working = 0,
  replying,
  reply,
  onReply,
  onLater,
}: {
  top: CalmItem | null;
  agent: WsAgent | null;
  working?: number;
  replying: boolean;
  /** The reply box, shown inside the card while `replying`. */
  reply: React.ReactNode;
  onReply: () => void;
  onLater: () => void;
}) {
  return (
    <section className="hb-calm-card" aria-labelledby="hb-calm-title">
      <h1 id="hb-calm-title">{top ? t("calmOnlyThing") : t("calmNothing")}</h1>
      {top ? (
        <>
          <div className="hb-calm-item">
            {agent ? <AgentAvatar name={agent.name} agent={agent} size={120} dot={false} /> : <Avatar name={top.who} size={120} />}
            <div className="hb-calm-text">
              <span className="hb-calm-chip">
                {top.who} · {top.reason.toLowerCase()}
              </span>
              <p className="hb-calm-question">{top.title}</p>
              {top.own && top.quote ? (
                <blockquote className="hb-calm-quote" aria-label={t("yourMessage")}>
                  <span className="hb-calm-quote-who">{t("yourMessage")}</span>
                  <span className="hb-calm-quote-text">«{top.quote}»</span>
                </blockquote>
              ) : null}
              {top.detail ? <p className="hb-calm-detail">{top.detail}</p> : null}
            </div>
          </div>
          <div className="hb-calm-actions">
            <button
              type="button"
              className="hb-btn hb-calm-yes"
              data-testid="calm-open"
              aria-expanded={replying ? "true" : "false"}
              aria-controls="hb-calm-reply"
              onClick={onReply}
            >
              <MessageSquareReply className="size-6" aria-hidden /> {t("reply")}
            </button>
            <button type="button" className="hb-btn hb-calm-later" data-testid="calm-later" onClick={onLater}>
              <Clock3 className="size-6" aria-hidden /> {t("calmLater")}
            </button>
          </div>
          {replying ? (
            <div id="hb-calm-reply" className="hb-calm-reply" data-testid="calm-reply">
              {reply}
            </div>
          ) : null}
        </>
      ) : (
        <p className="hb-calm-detail">{t("calmEmpty")}</p>
      )}
      <p className="hb-calm-line">
        {t("calmLine")} · <span className="hb-calm-working">{t("workingCount", { count: working })}</span>
      </p>
    </section>
  );
}
