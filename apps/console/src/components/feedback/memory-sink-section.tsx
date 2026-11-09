"use client";

/**
 * Off by default, because a ticked rule leaves harbard for a program the
 * operator chose; while off, the Retro view shows no save box and serve calls
 * nothing. Stored by serve in the desk.
 */

import * as React from "react";
import { fetchFeedback, postRule } from "@/lib/api";
import { checkLine, type DeskCheck } from "@/lib/checks";
import { createT } from "@/i18n";

const t = createT("feedback");

export type MemorySinkState = { configured: boolean; allowed: boolean };

export function MemorySinkSectionView({
  state,
  onChange,
  error,
  confirm,
}: {
  state: MemorySinkState;
  onChange: (on: boolean) => void;
  error?: string | null;
  /** Switching on asks again here before serve does it. */
  confirm?: { onConfirm: () => void; onCancel: () => void } | null;
}) {
  return (
    <fieldset className="hb-prefs-section">
      <legend>{t("memorySinkLegend")}</legend>
      <label className="hb-mode">
        <input type="checkbox" checked={state.allowed} disabled={!state.configured} data-testid="memory-sink-allow" onChange={(e) => onChange(e.target.checked)} />
        {t("memorySinkAllow")}
      </label>
      <p className="hb-prefs-hint">
        {state.configured
          ? t("memorySinkHint")
          : t("memorySinkMissing")}
      </p>
      {confirm ? (
        <div className="hb-prefs-confirm" role="alertdialog" aria-label={t("memorySinkConfirmLabel")}>
          <p>{t("memorySinkConfirm")}</p>
          <button type="button" className="hb-btn hb-btn-primary" data-testid="memory-sink-confirm" onClick={confirm.onConfirm}>
            {t("memorySinkAllowButton")}
          </button>
          <button type="button" className="hb-btn" onClick={confirm.onCancel}>
            {t("cancel")}
          </button>
        </div>
      ) : null}
      {error ? <p className="hb-error">{error}</p> : null}
    </fieldset>
  );
}

export function MemorySinkSection() {
  const [state, setState] = React.useState<MemorySinkState | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [code, setCode] = React.useState<string | null>(null);
  const [checks, setChecks] = React.useState<DeskCheck[]>([]);
  React.useEffect(() => {
    const c = new AbortController();
    fetchFeedback(c.signal)
      .then((v) => {
        setState(v.memory_sink ?? { configured: false, allowed: false });
        setChecks(v.checks ?? []);
      })
      .catch(() => {});
    return () => c.abort();
  }, []);
  if (!state) return null;
  const failed = () => {
    setState(state);
    setError(t("memorySinkSaveFailed"));
  };
  return (
    <>
    <MemorySinkSectionView
      state={state}
      error={error}
      confirm={
        code
          ? {
              onConfirm: () => {
                const c = code;
                setCode(null);
                postRule("memory-sink-allow", { on: true, confirm: c })
                  .then((out) => setState({ ...state, allowed: out.ok === true }))
                  .catch(failed);
              },
              onCancel: () => {
                setCode(null);
                setState({ ...state, allowed: false });
              },
            }
          : null
      }
      onChange={(on) => {
        setError(null);
        if (!on) {
          setState({ ...state, allowed: false });
          postRule("memory-sink-allow", { on: false }).catch(failed);
          return;
        }
        // On asks first: serve answers a one-time code and changes nothing.
        // The box shows the choice while the question is open.
        setState({ ...state, allowed: true });
        postRule("memory-sink-allow", { on: true })
          .then((out) => {
            const asked = (out as { confirm?: { id: string } }).confirm;
            if (asked) setCode(asked.id);
            else setState({ ...state, allowed: out.ok === true });
          })
          .catch(failed);
      }}
    />
    {checks.length > 0 ? (
      <fieldset className="hb-prefs-section" data-recent-checks="">
        <legend>{t("checksLegend")}</legend>
        <ul className="hb-checks">
          {checks.map((c) => (
            <li key={c.id}>{checkLine(c)}</li>
          ))}
        </ul>
      </fieldset>
    ) : null}
    </>
  );
}
