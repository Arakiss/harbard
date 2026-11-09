"use client";

import type { Route } from "next";
import * as React from "react";
import Link from "next/link";
import { CheckCircle2, Clock3, Inbox, List, RefreshCw, X } from "lucide-react";
import { createT } from "@/i18n";
import { awayModel, awaySpan, awayTitle } from "@/lib/away";
import { states } from "@/lib/profile";
import { AgentAvatar, AgentStatus, Avatar } from "./parts";
import { refusalText, useProfile, type Away } from "./profile-state";
import { useModal } from "./use-modal";
import { useOptionalView } from "./workspace-context";

const t = createT("home");

/** The title per state: Away is "While you were away"; the others say which state held it. */
function titleOf(state: string, since: number, until: number): string {
  if (state === "away") return awayTitle(since, until);
  const label = states().find((s) => s.id === state)?.label.toLowerCase() ?? state;
  const head = state === "done" ? t("sinceEndOfDay") : t("whileYouWere", { state: label });
  const span = awaySpan(since, until);
  return span ? `${head} · ${span}` : head;
}

/**
 * "While you were away": what waits for them, by urgency, what
 * finished, and who is still at work; then back to Available or on to
 * Activity. Opens first on the way back from Away, and for any digest a
 * state cut.
 */
export function AwayCard() {
  const profile = useProfile();
  if (!profile?.away) return null;
  return <AwayDialog key={`${profile.away.digest ?? "pending"}:${profile.away.since}`} profile={profile} away={profile.away} />;
}

function AwayDialog({ profile, away }: { profile: NonNullable<ReturnType<typeof useProfile>>; away: Away }) {
  const view = useOptionalView();
  const ref = React.useRef<HTMLDivElement>(null);
  const [later, setLater] = React.useState<Set<string>>(new Set());
  const [error, setError] = React.useState<string | null>(null);
  const close = React.useCallback(() => profile.closeAway(), [profile]);
  useModal(ref, close);
  const model = awayModel({ view, held: away.items, since: away.since, now: away.until });
  const waiting = model.waiting.filter((w) => !later.has(w.id));
  const agentOf = (key: string | null) => (key ? (view?.agents.find((a) => a.key === key) ?? null) : null);
  const hrefOf = (id: string, agent: string | null): Route =>
    (id.startsWith("card:") ? `/activity?${new URLSearchParams({ sel: id })}` : agent ? `/direct/${encodeURIComponent(agent)}` : "/activity") as Route;

  return (
    <div className="hb-scrim" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div ref={ref} className="hb-modal hb-away" role="dialog" aria-modal="true" aria-labelledby="hb-away-title" data-testid="away-card">
        <header className="hb-away-head">
          <span className="hb-away-badge" aria-hidden>
            <Clock3 className="size-5" />
          </span>
          <h2 id="hb-away-title">{titleOf(away.state, away.since, away.until)}</h2>
          <button type="button" className="hb-close" aria-label={t("close")} onClick={close}>
            <X className="size-5" aria-hidden />
          </button>
        </header>
        <div className="hb-modal-body hb-away-body">
          <section className="hb-away-section" aria-labelledby="hb-away-waiting">
            <h3 id="hb-away-waiting">
              <Inbox className="size-5" aria-hidden /> {t("waitingForYou", { count: waiting.length })}
            </h3>
            {waiting.length === 0 ? <p className="hb-away-empty">{t("nothingWaiting")}</p> : null}
            {waiting.map((w) => {
              const a = agentOf(w.agent);
              return (
                <div key={w.id} className="hb-away-wait" data-kind={w.kind}>
                  {a ? <AgentAvatar name={a.name} agent={a} size={48} dot={false} /> : <Avatar name={w.who} size={48} />}
                  <div className="hb-away-wait-text">
                    <div className="hb-away-who">
                      <strong>{w.who}</strong>
                      <span className="hb-chip" data-tone={w.tone}>
                        {w.reason}
                      </span>
                    </div>
                    {w.text ? <p>{w.text}</p> : null}
                    <div className="hb-away-actions">
                      <Link href={hrefOf(w.id, w.agent)} className="hb-btn hb-btn-primary" onClick={close}>
                        {t("open")}
                      </Link>
                      <button type="button" className="hb-btn" onClick={() => setLater(new Set([...later, w.id]))}>
                        {t("later")}
                      </button>
                    </div>
                  </div>
                </div>
              );
            })}
          </section>

          <section className="hb-away-section" aria-labelledby="hb-away-done">
            <h3 id="hb-away-done">
              <CheckCircle2 className="size-5 hb-away-ok" aria-hidden /> {t("finishedCount", { count: model.done.length })}
            </h3>
            {model.done.length === 0 ? <p className="hb-away-empty">{t("nothingFinished")}</p> : null}
            <ul className="hb-away-done-list">
              {model.done.map((d) => (
                <li key={d.id}>
                  <CheckCircle2 className="size-4 hb-away-ok" aria-hidden />
                  <span className="hb-away-pill">
                    {d.text}
                    {d.who ? <span className="hb-away-pill-who"> · {d.who}</span> : null}
                  </span>
                </li>
              ))}
            </ul>
          </section>

          <section className="hb-away-section" aria-labelledby="hb-away-running">
            <h3 id="hb-away-running">
              <RefreshCw className="size-5" aria-hidden /> {t("stillRunning", { count: model.running.length })}
            </h3>
            {model.running.length === 0 ? <p className="hb-away-empty">{t("noAgentWorking")}</p> : null}
            {model.running.map((a) => (
              <div key={a.key} className="hb-away-run">
                <AgentAvatar name={a.name} agent={a} size={32} dot={false} />
                <strong>{a.name}</strong>
                <span className="hb-away-run-desc">{a.description ?? ""}</span>
                <AgentStatus agent={a} className="hb-away-run-status" />
              </div>
            ))}
          </section>
          {error ? (
            <p className="hb-error" role="alert">
              {error}
            </p>
          ) : null}
        </div>
        <footer className="hb-away-foot">
          {away.stillAway ? (
            <button
              type="button"
              className="hb-btn hb-btn-primary hb-away-primary"
              data-autofocus
              onClick={() => {
                setError(null);
                profile.update({ state: { id: "available" } }).then(close, (e) => setError(refusalText(e)));
              }}
            >
              <span className="hb-presence-dot" data-tone="green" aria-hidden /> {t("setAvailable")}
            </button>
          ) : (
            <button type="button" className="hb-btn hb-btn-primary hb-away-primary" data-autofocus onClick={close}>
              {t("done")}
            </button>
          )}
          <Link href={"/activity" as Route} className="hb-btn hb-away-secondary" onClick={close}>
            <List className="size-5" aria-hidden /> {t("seeAllInActivity")}
          </Link>
        </footer>
      </div>
    </div>
  );
}
