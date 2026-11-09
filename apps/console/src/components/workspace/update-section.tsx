"use client";

/**
 * Preferences › Updates: which changes count as a new version, the
 * background check, when it last looked, and the checkout it updates from.
 */

import * as React from "react";
import { Loader2 } from "lucide-react";
import { createT } from "@/i18n";
import { channels, sourceProblemText, statusSentence } from "@/lib/update";
import { useUpdate } from "./update-state";

const t = createT("update");

export function UpdateSection() {
  const ctx = useUpdate();
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, []);
  // Refresh the clock right after a check lands.
  const checked = ctx?.view?.check?.checked_ms;
  React.useEffect(() => setNow(Date.now()), [checked]);
  if (!ctx || ctx.unsupported) return null;
  const view = ctx.view;
  const disabled = !view || ctx.checking;
  return (
    <section className="hb-prefs-group" id="hb-prefs-updates" aria-labelledby="hb-update-prefs-title">
      <h3 id="hb-update-prefs-title" className="hb-prefs-group-title">
        {t("sectionTitle")}
      </h3>
      <fieldset className="hb-prefs-section">
        <legend>{t("statusLegend")}</legend>
        <p className="hb-prefs-hint" role="status" aria-live="polite">
          {statusSentence(view, now)}
        </p>
        <div className="hb-custom-row">
          <button type="button" className="hb-btn" disabled={disabled} onClick={() => void ctx.check()} data-testid="update-check-now">
            {ctx.checking ? (
              <>
                <Loader2 className="hb-spin mr-2 inline size-4" aria-hidden />
                {t("checking")}
              </>
            ) : (
              t("checkNow")
            )}
          </button>
          {ctx.available ? (
            <button type="button" className="hb-btn hb-btn-primary" onClick={ctx.openDialog}>
              {t("seeNotes")}
            </button>
          ) : null}
        </div>
        {ctx.error ? <p className="hb-error">{ctx.error}</p> : null}
      </fieldset>

      <fieldset className="hb-prefs-section">
        <legend>{t("channelLegend")}</legend>
        <div className="hb-update-channels" role="radiogroup" aria-label={t("channelAria")}>
          {channels().map((c) => (
            <label key={c.id} className="hb-update-channel">
              <input
                type="radio"
                name="hb-update-channel"
                value={c.id}
                checked={view?.prefs.channel === c.id}
                disabled={disabled}
                onChange={() => void ctx.setPrefs({ channel: c.id }).catch(() => {})}
              />
              <span>
                <strong>{c.label}</strong>
                <span className="hb-prefs-hint">{c.hint}</span>
              </span>
            </label>
          ))}
        </div>
      </fieldset>

      <fieldset className="hb-prefs-section">
        <legend>{t("backgroundLegend")}</legend>
        <label className="hb-mode">
          <input
            type="checkbox"
            checked={view?.prefs.auto_check ?? true}
            disabled={!view}
            data-testid="update-auto-check"
            onChange={(e) => void ctx.setPrefs({ auto_check: e.target.checked }).catch(() => {})}
          />
          {t("autoCheckLabel")}
        </label>
        <p className="hb-prefs-hint">{t("autoCheckHint")}</p>
      </fieldset>

      <fieldset className="hb-prefs-section">
        <legend>{t("repoLegend")}</legend>
        <p className="hb-prefs-hint">{t("repoHint")}</p>
        {view?.source.path ? <p className="hb-mono hb-update-path">{view.source.path}</p> : null}
        {view?.source.problem ? (
          <p className="hb-update-warn" role="note">
            {sourceProblemText(view.source.problem)}
          </p>
        ) : view?.source.path ? (
          <p className="hb-prefs-hint">{t("repoReady")}</p>
        ) : null}
      </fieldset>
    </section>
  );
}
