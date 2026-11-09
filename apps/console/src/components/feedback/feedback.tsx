"use client";

/**
 * One-tap feedback controls (lib/feedback.ts). The provider holds the operator's
 * standing answers; each slot is one `<FeedbackRow>` mounted beside what it
 * asks about. A row shows on hover or focus only (its host decides: a
 * message's hover tools, or `.hb-fb-host`), never twice for one target, and
 * not at all in Focused, Swamped or Away.
 *
 * Reasons are the operator's own words and render as plain text only.
 */

import * as React from "react";
import { ThumbsDown, ThumbsUp } from "lucide-react";
import { fetchFeedback, postFeedback } from "@/lib/api";
import {
  BOUNCE_OFFER_MS,
  controls,
  applyTap,
  applyUndo,
  bounceTarget,
  feedbackAllowed,
  standingFor,
  type FeedbackRecord,
  type FeedbackSample,
  type FeedbackState,
  type TargetKind,
} from "@/lib/feedback";
import { createT, intlTag } from "@/i18n";
import { operatorTimeZone } from "@/lib/time-zone";

const t = createT("feedback");

type Ctx = {
  state: FeedbackState;
  allowed: boolean;
  tap: (kind: TargetKind, id: string, value: string, extra?: { reason?: string; sample?: FeedbackSample }) => Promise<void>;
  undo: (rec: FeedbackRecord) => Promise<void>;
};

const FeedbackContext = React.createContext<Ctx | null>(null);

/** How long the acknowledgement stays in sight after a tap. */
const ACK_MS = 6_000;

export function useFeedback(): Ctx | null {
  return React.useContext(FeedbackContext);
}

/**
 * `operatorState` is the coordination state id (the profile's `stateOf(...).id`);
 * `initial` skips the first read (tests, server render).
 */
export function FeedbackProvider({
  children,
  operatorState = null,
  initial,
}: {
  children: React.ReactNode;
  operatorState?: string | null;
  initial?: FeedbackState;
}) {
  const [state, setState] = React.useState<FeedbackState>(initial ?? { standing: [], last: null });
  React.useEffect(() => {
    if (initial) return;
    const controller = new AbortController();
    fetchFeedback(controller.signal)
      .then((v) => setState({ standing: v.standing, last: v.last }))
      .catch(() => {});
    return () => controller.abort();
    // Read once on mount; every tap updates the copy from serve's answer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const tap = React.useCallback<Ctx["tap"]>(async (kind, id, value, extra) => {
    const out = await postFeedback("feedback", { target_kind: kind, target_id: id, value, ...extra });
    setState((s) => applyTap(s, out.record));
  }, []);
  const undo = React.useCallback<Ctx["undo"]>(async (rec) => {
    await postFeedback("feedback-undo", { id: rec.id });
    setState((s) => applyUndo(s, rec));
  }, []);
  const value = React.useMemo(() => ({ state, allowed: feedbackAllowed(operatorState), tap, undo }), [state, operatorState, tap, undo]);
  return <FeedbackContext.Provider value={value}>{children}</FeedbackContext.Provider>;
}

/** The slot: one row for one target, or nothing. */
export function FeedbackRow({ kind, id, sample }: { kind: TargetKind; id: string; sample?: FeedbackSample }) {
  const ctx = useFeedback();
  if (!ctx || !ctx.allowed) return null;
  return <FeedbackRowView ctx={ctx} kind={kind} id={id} sample={sample} />;
}

function FeedbackRowView({ ctx, kind, id, sample }: { ctx: Ctx; kind: TargetKind; id: string; sample?: FeedbackSample }) {
  const control = controls()[kind];
  const standing = standingFor(ctx.state, kind, id);
  const [asking, setAsking] = React.useState<string | null>(null);
  const [reason, setReason] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [fresh, setFresh] = React.useState(false);
  const input = React.useRef<HTMLInputElement>(null);
  const undoButton = React.useRef<HTMLButtonElement>(null);
  React.useEffect(() => {
    if (asking) input.current?.focus();
  }, [asking]);
  // The acknowledgement stays in sight a few seconds after the tap, with
  // the focus on Undo, then goes back to hover or focus only.
  React.useEffect(() => {
    if (!fresh) return;
    undoButton.current?.focus({ preventScroll: true });
    const t = setTimeout(() => setFresh(false), ACK_MS);
    return () => clearTimeout(t);
  }, [fresh]);

  const send = (value: string, why?: string) => {
    setBusy(true);
    setError(null);
    const extra: { reason?: string; sample?: FeedbackSample } = {};
    if (why && why.trim()) extra.reason = why.trim();
    if (sample) extra.sample = { state: sample.state, label: sample.label ?? null };
    ctx
      .tap(kind, id, value, extra)
      .then(() => {
        setAsking(null);
        setReason("");
        setFresh(true);
      })
      .catch(() => setError(t("saveFailed")))
      .finally(() => setBusy(false));
  };

  const common = { className: "hb-fb", "data-fb-kind": kind, "data-fb-sample": sample?.state } as const;

  if (standing) {
    const isLast = ctx.state.last?.id === standing.id;
    return (
      <div {...common} data-fb-given={standing.value} data-fb-fresh={fresh ? "" : undefined} role="status">
        <span className="hb-fb-ack">{control.ack[standing.value] ?? t("ackDefault")}</span>
        {isLast ? (
          <button
            ref={undoButton}
            type="button"
            className="hb-fb-undo"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              ctx
                .undo(standing)
                .catch(() => setError(t("undoFailed")))
                .finally(() => setBusy(false));
            }}
          >
            {t("undo")}
          </button>
        ) : null}
        {error ? <span className="hb-fb-error">{error}</span> : null}
      </div>
    );
  }

  if (asking) {
    const option = control.options.find((o) => o.value === asking);
    return (
      <form
        {...common}
        onSubmit={(e) => {
          e.preventDefault();
          send(asking, reason);
        }}
      >
        <label className="hb-fb-q" htmlFor={`fb-${kind}-${id}`}>
          {option?.label}:
        </label>
        <input
          ref={input}
          id={`fb-${kind}-${id}`}
          className="hb-fb-reason"
          type="text"
          maxLength={200}
          placeholder={t("reasonPlaceholder")}
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              setAsking(null);
              setReason("");
            }
          }}
        />
        <button type="submit" className="hb-fb-btn" disabled={busy}>
          {t("send")}
        </button>
        {error ? <span className="hb-fb-error">{error}</span> : null}
      </form>
    );
  }

  return (
    <div {...common} role="group" aria-label={control.question ?? t("yourFeedback")}>
      {control.question ? <span className="hb-fb-q">{control.question}</span> : null}
      {control.options.map((o) => (
        <button
          key={o.value}
          type="button"
          className="hb-fb-btn"
          data-icon={o.icon}
          aria-label={o.icon ? o.label : undefined}
          title={o.icon ? o.label : undefined}
          disabled={busy}
          onClick={() => (o.reason ? setAsking(o.value) : send(o.value))}
        >
          {o.icon === "up" ? <ThumbsUp className="size-4" aria-hidden /> : o.icon === "down" ? <ThumbsDown className="size-4" aria-hidden /> : o.label}
        </button>
      ))}
      {error ? <span className="hb-fb-error">{error}</span> : null}
    </div>
  );
}

// ---------------------------------------------------------------------------
// The last Dock bounce this console rang
// ---------------------------------------------------------------------------

export type Bounce = { at_ms: number; state: string };

let lastBounce: Bounce | null = null;
const listeners = new Set<() => void>();

/** Called where the Dock is told that what needs the operator rose. */
export function recordBounce(b: Bounce): void {
  lastBounce = b;
  for (const l of listeners) l();
}

function useLastBounce(): Bounce | null {
  return React.useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => lastBounce,
    () => null,
  );
}

function hhmm(ms: number): string {
  return new Date(ms).toLocaleTimeString(intlTag(), { hour: "2-digit", minute: "2-digit", timeZone: operatorTimeZone() });
}

/** "The Dock bounced at 14:02 · This bothered me", for half an hour. */
export function DockFeedbackView({ bounce, now }: { bounce: Bounce | null; now: number }) {
  if (!bounce || now - bounce.at_ms > BOUNCE_OFFER_MS) return null;
  return (
    <div className="hb-fb-host hb-fb-dock">
      <span className="hb-fb-dock-what">{t("dockBouncedAt", { time: hhmm(bounce.at_ms) })}</span>
      <FeedbackRow kind="dock" id={bounceTarget(bounce.at_ms)} sample={{ state: bounce.state }} />
    </div>
  );
}

export function DockFeedback({ now }: { now: number }) {
  return <DockFeedbackView bounce={useLastBounce()} now={now} />;
}
