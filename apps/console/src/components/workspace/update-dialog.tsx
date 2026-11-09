"use client";

/**
 * "Update harbard": the installed version beside the newer one, what it
 * brings (the changelog's notes, then every commit grouped the way a
 * changelog groups them) and one button. Once a run starts, the same dialog
 * shows its five steps live, its log, and how it ended.
 */

import * as React from "react";
import { ArrowRight, Check, ChevronRight, Circle, Loader2, Minus, X } from "lucide-react";
import { createT } from "@/i18n";
import {
  broughtBy,
  channels,
  stepDetail,
  stepLabel,
  stepStateText,
  sourceProblemText,
  changesCount,
  errorText,
  groupChanges,
  pageIsStale,
  runSteps,
  sentenceCase,
  shortSha,
  statusChip,
  statusSentence,
  stepElapsed,
  targetName,
  targetWhen,
  type StepState,
  type UpdateRun,
  type UpdateView,
} from "@/lib/update";
import { MarkdownView } from "./attachments";
import { useUpdate, type UpdateCtx } from "./update-state";

const t = createT("update");

const PAGE_COMMIT = process.env.NEXT_PUBLIC_HARBARD_COMMIT ?? "";

function useNow(active: boolean): number {
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [active]);
  return now;
}

/** Progress, rather than the changelog, while a run goes on or this page followed one. */
function showsRun(ctx: UpdateCtx): boolean {
  const run = ctx.view?.run;
  if (ctx.installing || ctx.restarting) return true;
  if (!run) return false;
  if (run.state === "running") return true;
  if (run.id === ctx.followed) return true;
  return run.state === "done" && pageIsStale(run, PAGE_COMMIT);
}

export function UpdateDialog({ onClose }: { onClose: () => void }) {
  const ctx = useUpdate();
  const close = React.useRef<HTMLButtonElement>(null);
  React.useEffect(() => {
    close.current?.focus();
    const on = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, [onClose]);
  if (!ctx) return null;
  const progress = showsRun(ctx);
  return (
    <div className="hb-finder-scrim hb-update-scrim" onMouseDown={onClose}>
      <div className="hb-update" role="dialog" aria-modal="true" aria-labelledby="hb-update-title" onMouseDown={(e) => e.stopPropagation()}>
        <div className="hb-pane-head">
          <span id="hb-update-title" className="hb-pane-title">
            {t("dialogTitle")}
          </span>
          <button ref={close} type="button" className="hb-close" onClick={onClose} aria-label={t("close")}>
            <X className="size-5" />
          </button>
        </div>
        {progress ? <RunView ctx={ctx} onClose={onClose} /> : <OfferView ctx={ctx} onClose={onClose} />}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// What is new, and the button
// ---------------------------------------------------------------------------

function OfferView({ ctx, onClose }: { ctx: UpdateCtx; onClose: () => void }) {
  const now = useNow(false);
  const view = ctx.view;
  const check = view?.check ?? null;
  const available = check?.status === "available";
  const problem = view?.source.problem ?? null;
  // After an update, "See what's new" shows what that run brought.
  const brought = available ? null : broughtBy(view?.run ?? null);
  const notes = (brought ? brought.notes : check?.notes)?.trim() ?? "";
  const changes = brought ? brought.changes : available ? (check?.changes ?? []) : [];
  const truncated = brought ? brought.truncated : (check?.changes_truncated ?? false);
  const groups = groupChanges(changes);
  const channel = channels().find((c) => c.id === (check?.channel ?? view?.prefs.channel))?.label;
  const when = check ? targetWhen(check, now) : null;
  const chip = ctx.checking ? { tone: undefined, text: t("checking") } : statusChip(check);
  return (
    <>
      <div className="hb-update-body">
        <section className="hb-update-hero" aria-label={t("versionsAria")}>
          <span className="hb-chip" data-tone={chip.tone}>
            {chip.text}
          </span>
          {view ? (
            <div className="hb-update-versions">
              <div className="hb-update-version">
                <span className="hb-update-version-label">{t("versionYours")}</span>
                <strong>{view.installed.version}</strong>
                {view.installed.commit ? <span className="hb-mono">{shortSha(view.installed.commit)}</span> : null}
              </div>
              {available && check?.target ? (
                <>
                  <ArrowRight className="hb-update-arrow size-5" aria-hidden />
                  <div className="hb-update-version" data-new="true">
                    <span className="hb-update-version-label">{t("versionNew")}</span>
                    <strong>{targetName(check, view.installed.version)}</strong>
                    <span className="hb-mono">{shortSha(check.target.commit)}</span>
                  </div>
                </>
              ) : null}
            </div>
          ) : null}
          <p className="hb-update-meta">
            {ctx.checking
              ? t("checkingRepository")
              : available && check
              ? [changesCount(check.behind), when ? t("metaLatest", { when }) : null, channel ? t("metaChannel", { channel: channel.toLowerCase() }) : null].filter(Boolean).join(" · ")
              : statusSentence(view, now)}
          </p>
          {check?.target?.title ? <p className="hb-update-release-title">{check.target.title}</p> : null}
        </section>

        {problem ? (
          <p className="hb-update-warn" role="note">
            {sourceProblemText(problem)}
            {view?.source.path ? (
              <>
                {" "}
                <span className="hb-mono">{view.source.path}</span>
              </>
            ) : null}
          </p>
        ) : null}
        {ctx.error ? (
          <p className="hb-update-warn" data-tone="red" role="alert">
            {ctx.error}
          </p>
        ) : null}

        {notes ? (
          <section className="hb-update-section" aria-labelledby="hb-update-notes">
            <h3 id="hb-update-notes">{available ? t("notesTitle") : brought ? t("notesBrought") : t("notesCurrent")}</h3>
            <MarkdownView source={notes} />
          </section>
        ) : available ? (
          <p className="hb-prefs-hint">{t("noNotes")}</p>
        ) : null}

        {groups.length > 0 ? (
          <details className="hb-update-commits" open={!notes}>
            <summary>
              <ChevronRight className="hb-update-caret size-4" aria-hidden />
              {t("commitsTitle")} <span className="hb-update-count">{available ? (check?.behind ?? changes.length) : changes.length}</span>
            </summary>
            {groups.map((g) => (
              <section key={g.id} className="hb-update-group" aria-label={g.label}>
                <h4>{g.label}</h4>
                <ul>
                  {g.changes.map((c) => (
                    <li key={c.commit}>
                      <span className="hb-mono hb-update-sha">{shortSha(c.commit)}</span>
                      {c.scope ? <span className="hb-update-scope">{c.scope}</span> : null}
                      <span className="hb-update-subject">{sentenceCase(c.subject)}</span>
                    </li>
                  ))}
                </ul>
              </section>
            ))}
            {truncated ? <p className="hb-prefs-hint">{t("truncatedHint")}</p> : null}
          </details>
        ) : null}
      </div>
      <footer className="hb-update-foot">
        {available ? (
          <>
            <button
              type="button"
              className="hb-btn"
              onClick={() => {
                ctx.dismiss();
                onClose();
              }}
            >
              {t("later")}
            </button>
            <button type="button" className="hb-btn hb-btn-primary" data-testid="update-now" disabled={!!problem || ctx.installing} onClick={() => void ctx.install()}>
              {t("updateNow")}
            </button>
          </>
        ) : (
          <>
            <button type="button" className="hb-btn" disabled={ctx.checking || !view} onClick={() => void ctx.check()}>
              {ctx.checking ? (
                <>
                  <Loader2 className="hb-spin mr-2 inline size-4" aria-hidden />
                  {t("checking")}
                </>
              ) : (
                t("checkNow")
              )}
            </button>
            <button type="button" className="hb-btn hb-btn-primary" onClick={onClose}>
              {t("close")}
            </button>
          </>
        )}
      </footer>
    </>
  );
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

const STATE_ICON: Record<StepState, React.ComponentType<{ className?: string; "aria-hidden"?: boolean }>> = {
  pending: Circle,
  running: Loader2,
  done: Check,
  failed: X,
  skipped: Minus,
};

function runHeadline(ctx: UpdateCtx, run: UpdateRun | null): { title: string; hint: string } {
  if (ctx.restarting) return { title: t("restarting"), hint: t("restartingHint") };
  if (run?.state === "failed") return { title: t("failedTitle"), hint: errorText(run.error) };
  if (run?.state === "done") return { title: t("done"), hint: pageIsStale(run, PAGE_COMMIT) ? t("doneHint") : t("doneCurrent") };
  return { title: t("bannerRunning"), hint: t("runningHint") };
}

function RunView({ ctx, onClose }: { ctx: UpdateCtx; onClose: () => void }) {
  const run = ctx.view?.run ?? null;
  const live = ctx.restarting || ctx.installing || run?.state === "running";
  const now = useNow(live);
  const steps = runSteps(run);
  const head = runHeadline(ctx, run);
  const [log, setLog] = React.useState(false);
  const logRef = React.useRef<HTMLPreElement>(null);
  React.useEffect(() => {
    if (log && logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [log, run?.log_tail]);
  const failed = run?.state === "failed";
  const done = run?.state === "done" && !ctx.restarting;
  return (
    <>
      <div className="hb-update-body">
        <section className="hb-update-run-head" data-state={ctx.restarting ? "running" : (run?.state ?? "running")} aria-live="polite">
          <span className="hb-update-run-badge" aria-hidden>
            {done ? <Check className="size-5" /> : failed ? <X className="size-5" /> : <Loader2 className="hb-spin size-5" />}
          </span>
          <div>
            <h3>{head.title}</h3>
            <p>{head.hint}</p>
          </div>
        </section>
        <ol className="hb-update-steps" aria-label={t("stepsAria")}>
          {steps.map((s) => {
            const Icon = STATE_ICON[s.state];
            const elapsed = stepElapsed(s, now);
            const detail = stepDetail(s);
            return (
              <li key={s.id} data-state={s.state}>
                <span className="hb-update-step-icon">
                  <Icon className={`size-4${s.state === "running" ? " hb-spin" : ""}`} aria-hidden />
                </span>
                <span className="hb-update-step-text">
                  <span className="hb-update-step-label">{stepLabel(s.id)}</span>
                  {detail?.where === "below" ? <span className="hb-update-step-detail hb-mono">{detail.text}</span> : null}
                </span>
                <span className="hb-update-step-side">
                  <span className="sr-only">{stepStateText(s.state)}</span>
                  {detail?.where === "side" ? detail.text : elapsed}
                </span>
              </li>
            );
          })}
        </ol>
        {run && run.log_tail.length > 0 ? (
          <div className="hb-update-log-wrap">
            <button type="button" className="hb-update-log-toggle" aria-expanded={log} onClick={() => setLog(!log)}>
              <ChevronRight className="hb-update-caret size-4" aria-hidden data-open={log ? "true" : undefined} />
              {log ? t("hideLog") : t("showLog")}
            </button>
            {log ? (
              <pre ref={logRef} className="hb-mono hb-update-log">
                {run.log_tail.join("\n")}
              </pre>
            ) : null}
          </div>
        ) : null}
      </div>
      <footer className="hb-update-foot">
        {failed ? (
          <>
            <button type="button" className="hb-btn" onClick={onClose}>
              {t("close")}
            </button>
            <button type="button" className="hb-btn hb-btn-primary" onClick={() => void ctx.install()}>
              {t("retry")}
            </button>
          </>
        ) : done ? (
          <>
            <button type="button" className="hb-btn" onClick={onClose}>
              {t("close")}
            </button>
            {pageIsStale(run!, PAGE_COMMIT) ? (
              <button type="button" className="hb-btn hb-btn-primary" onClick={() => window.location.reload()}>
                {t("reload")}
              </button>
            ) : null}
          </>
        ) : (
          <button type="button" className="hb-btn" onClick={onClose}>
            {t("close")}
          </button>
        )}
      </footer>
    </>
  );
}

/** For tests and static renders: the offer and the run, outside the provider. */
export function UpdateDialogStatic({ view, ctx }: { view: UpdateView; ctx?: Partial<UpdateCtx> }) {
  const full: UpdateCtx = {
    view,
    unsupported: false,
    restarting: false,
    checking: false,
    installing: false,
    followed: null,
    error: null,
    banner: null,
    available: view.check?.status === "available",
    check: async () => {},
    install: async () => {},
    setPrefs: async () => {},
    dismiss: () => {},
    openDialog: () => {},
    ...ctx,
  };
  return showsRun(full) ? <RunView ctx={full} onClose={() => {}} /> : <OfferView ctx={full} onClose={() => {}} />;
}
