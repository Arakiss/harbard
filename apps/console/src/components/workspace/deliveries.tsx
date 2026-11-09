"use client";

import type { Route } from "next";
import * as React from "react";
import Link from "next/link";
import { AlertTriangle, CheckCircle2, PackageCheck } from "lucide-react";
import { type TranslationKey, createT, richText } from "@/i18n";
import {
  consoleCommit,
  deliveriesLine,
  installedLine,
  queueLine,
  stateLabel,
  steps,
  timesLine,
  untrackedText,
  type WsShipment,
} from "@/lib/deliveries";
import { hhmm, type WorkspaceView } from "@/lib/workspace";
import { AgentAvatar, Avatar } from "./parts";

const t = createT("deliveries");

/** The steps a delivery went through, the current one marked. */
function StepTrack({ s }: { s: WsShipment }) {
  return (
    <ol className="hb-ship-steps" aria-label={t("stepsAria")}>
      {steps(s).map((step) => (
        <li key={step.id} data-reached={step.reached ? "true" : undefined} data-current={step.current ? "true" : undefined} aria-current={step.current ? "step" : undefined}>
          <span className="hb-ship-dot" aria-hidden />
          {step.label}
          <span className="sr-only">{` (${step.current ? t("stepCurrent") : step.reached ? t("stepReached") : t("stepSkipped")})`}</span>
        </li>
      ))}
    </ol>
  );
}

/** One delivery: who, the human line, where it is now and when each step happened. Opens the conversation. */
function ShipmentRow({ view, s }: { view: WorkspaceView; s: WsShipment }) {
  const agent = view.agents.find((a) => a.key === s.agent);
  const name = agent?.name ?? s.agent.slice(0, 8);
  const why = untrackedText(s.untracked);
  return (
    <li>
      <Link href={`/direct/${encodeURIComponent(s.agent)}` as Route} className="hb-ship" data-state={s.state} data-pending={s.pending ? "true" : undefined}>
        {agent ? <AgentAvatar name={name} size={40} agent={agent} /> : <Avatar name={name} size={40} />}
        <span className="hb-ship-body">
          <span className="hb-ship-top">
            <span className="hb-ship-name">{name}</span>
            <span className="hb-ship-chip" data-state={s.state}>
              {stateLabel(s.state)}
            </span>
            {s.channel ? <span className="hb-ship-channel">#{s.channel}</span> : null}
            {s.sha ? (
              <span className="hb-ship-sha hb-mono" title={s.landed_as ? t("landedAsTitle", { sha: s.landed_as.slice(0, 7) }) : undefined}>
                {s.sha.slice(0, 7)}
                {s.landed_as ? ` → ${s.landed_as.slice(0, 7)}` : ""}
              </span>
            ) : null}
          </span>
          <span className="hb-ship-text">{s.text}</span>
          <span className="hb-ship-times">{timesLine(s, view.now_ms)}</span>
          {why ? <span className="hb-ship-why">{why}</span> : <StepTrack s={s} />}
        </span>
      </Link>
    </li>
  );
}

const VERDICT: Record<string, TranslationKey<"deliveries">> = {
  landed: "verdictLanded",
  conflict: "verdictConflict",
  rejected: "verdictRejected",
  failed: "verdictFailed",
  pending: "verdictPending",
};

function verdictText(verdict: string): string {
  const key = VERDICT[verdict];
  return key ? t(key) : verdict;
}

/** The old delivery queue (`harbard deliver`, `integrate`), when it has anything. */
function IntegrationQueue({ view }: { view: WorkspaceView }) {
  if (view.deliveries.length === 0) return null;
  return (
    <section className="hb-ship-section" aria-labelledby="hb-queue-title">
      <h2 id="hb-queue-title" className="hb-ship-h">
        {t("queueTitle", { count: view.deliveries.length })}
      </h2>
      <p className="hb-ship-note">{queueLine()}</p>
      {view.deliveries.map((d) => (
        <div key={d.id} className="hb-msg">
          <Avatar name={d.channel ?? "harbard"} />
          <div className="hb-msg-body">
            <div className="hb-msg-meta">
              <span className="hb-msg-name">{d.channel ? `#${d.channel}` : t("repositoryNoChannel")}</span>
              <span className="hb-msg-time">{hhmm(d.created_ms)}</span>
            </div>
            <div className="hb-msg-text">
              {d.decided_ms
                ? richText(t("deliveryLineAt"), { sha: <span className="hb-mono">{d.sha.slice(0, 10)}</span>, branch: <span className="hb-mono">{d.branch}</span>, verdict: verdictText(d.verdict), time: hhmm(d.decided_ms) })
                : richText(t("deliveryLine"), { sha: <span className="hb-mono">{d.sha.slice(0, 10)}</span>, branch: <span className="hb-mono">{d.branch}</span>, verdict: verdictText(d.verdict) })}
            </div>
            {d.verdict === "landed" ? (
              <span className="hb-pill" title={t("verifiedTitle")}>
                <CheckCircle2 className="size-3.5" aria-hidden /> {t("verified")}
              </span>
            ) : null}
          </div>
        </div>
      ))}
    </section>
  );
}

/** What the agents finished, and whether the operator already has it installed. */
export function DeliveriesView({ view, commit = consoleCommit() }: { view: WorkspaceView; commit?: string }) {
  const all = view.shipments ?? [];
  const pending = all.filter((s) => s.pending);
  const rest = all.filter((s) => !s.pending);
  const installed = installedLine(view, commit);
  return (
    <main className="hb-main" aria-label={t("title")}>
      <header className="hb-head">
        <div className="hb-head-title">
          <PackageCheck className="size-5" aria-hidden /> {t("title")}
        </div>
        <span className="hb-topic">{deliveriesLine()}</span>
      </header>
      <div className="hb-stream">
        <p className="hb-ship-installed" data-mismatch={installed.mismatch ? "true" : undefined}>
          {installed.mismatch ? <AlertTriangle className="size-4 shrink-0" aria-hidden /> : <CheckCircle2 className="size-4 shrink-0" aria-hidden />}
          {installed.text}
        </p>
        {all.length === 0 ? (
          <div className="hb-empty">
            <strong>{t("emptyTitle")}</strong>
            {t("emptyHint")}
          </div>
        ) : null}
        {pending.length > 0 ? (
          <section className="hb-ship-section" aria-labelledby="hb-ship-pending">
            <h2 id="hb-ship-pending" className="hb-ship-h">
              {t("pendingTitle", { count: pending.length })}
            </h2>
            <ul className="hb-ship-list">
              {pending.map((s) => (
                <ShipmentRow key={s.id} view={view} s={s} />
              ))}
            </ul>
          </section>
        ) : null}
        {rest.length > 0 ? (
          <section className="hb-ship-section" aria-labelledby="hb-ship-rest">
            <h2 id="hb-ship-rest" className="hb-ship-h">
              {t("restTitle", { count: rest.length })}
            </h2>
            <ul className="hb-ship-list">
              {rest.map((s) => (
                <ShipmentRow key={s.id} view={view} s={s} />
              ))}
            </ul>
          </section>
        ) : null}
        <IntegrationQueue view={view} />
      </div>
    </main>
  );
}
