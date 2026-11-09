"use client";

import * as React from "react";
import { deskGetPrefs, deskPrefsAvailable, deskSetMute } from "@/lib/desk-prefs";
import { createT } from "@/i18n";
import { dndChoices, type DndChoice } from "@/lib/profile";
import { DockAttentionSection } from "./dock-attention";
import { refusalText, useProfile } from "./profile-state";

const t = createT("preferences");

/** "System notices": the desktop app's own notifications (`desk.json`, the tray's mute). */
function SystemAlerts() {
  const [available, setAvailable] = React.useState(false);
  const [on, setOn] = React.useState<boolean | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    if (!deskPrefsAvailable()) return;
    setAvailable(true);
    void deskGetPrefs().then((p) => setOn(p ? !p.mute : null));
  }, []);
  return (
    <fieldset className="hb-prefs-section">
      <legend>{t("systemAlerts")}</legend>
      <label className="hb-mode">
        <input
          type="checkbox"
          checked={available ? (on ?? true) : false}
          disabled={!available || on === null}
          data-testid="system-alerts"
          onChange={(e) => {
            const next = e.target.checked;
            setError(null);
            setOn(next);
            void deskSetMute(!next).then((ok) => {
              if (!ok) {
                setOn(!next);
                setError(t("couldNotSaveDesktop"));
              }
            });
          }}
        />
        {t("showMacAlerts")}
      </label>
      <p className="hb-prefs-hint">{available ? t("systemAlertsHint") : t("systemAlertsDesktopOnly")}</p>
      {error ? <p className="hb-error">{error}</p> : null}
    </fieldset>
  );
}

/** The duration "Do not disturb" offers first, kept in the profile. */
function DndDefault() {
  const profile = useProfile();
  const [error, setError] = React.useState<string | null>(null);
  const current: DndChoice = profile?.view?.profile.dnd_default ?? "1h";
  return (
    <fieldset className="hb-prefs-section">
      <legend>{t("doNotDisturb")}</legend>
      <p className="hb-prefs-hint">{t("dndHint")}</p>
      <div className="hb-mode-row" role="radiogroup" aria-label={t("dndDefaultDuration")}>
        {dndChoices().map((c) => (
          <label key={c.id} className="hb-mode">
            <input
              type="radio"
              name="hb-dnd-default"
              value={c.id}
              checked={current === c.id}
              disabled={!profile?.view}
              onChange={() => {
                setError(null);
                profile?.update({ dnd_default: c.id }).catch((e) => setError(refusalText(e)));
              }}
            />
            {c.label}
          </label>
        ))}
      </div>
      <p className="hb-prefs-hint">{t("dndDefaultHint")}</p>
      {error ? <p className="hb-error">{error}</p> : null}
    </fieldset>
  );
}

/** "Every 60 min if they ask for it": the hourly digest in Focused and Swamped. */
function HourlyDigest() {
  const profile = useProfile();
  const [error, setError] = React.useState<string | null>(null);
  const on = profile?.view?.profile.digest_hourly ?? false;
  return (
    <fieldset className="hb-prefs-section">
      <legend>{t("heldDigest")}</legend>
      <label className="hb-mode">
        <input
          type="checkbox"
          checked={on}
          disabled={!profile?.view}
          data-testid="digest-hourly"
          onChange={(e) => {
            setError(null);
            profile?.update({ digest_hourly: e.target.checked }).catch((err) => setError(refusalText(err)));
          }}
        />
        {t("hourlyDigest")}
      </label>
      <p className="hb-prefs-hint">{t("hourlyDigestHint")}</p>
      {error ? <p className="hb-error">{error}</p> : null}
    </fieldset>
  );
}

/** "Suggest Away": the desktop's away suggestion, on unless turned off. */
function AwaySuggest() {
  const profile = useProfile();
  const [error, setError] = React.useState<string | null>(null);
  const on = profile?.view?.profile.away_suggest !== false;
  return (
    <fieldset className="hb-prefs-section">
      <legend>{t("awayLegend")}</legend>
      <label className="hb-mode">
        <input
          type="checkbox"
          checked={on}
          disabled={!profile?.view}
          data-testid="away-suggest"
          onChange={(e) => {
            setError(null);
            profile?.update({ away_suggest: e.target.checked }).catch((err) => setError(refusalText(err)));
          }}
        />
        {t("awaySuggest")}
      </label>
      <p className="hb-prefs-hint">{t("awaySuggestHint")}</p>
      {error ? <p className="hb-error">{error}</p> : null}
    </fieldset>
  );
}

/** Preferences › Notifications: the Dock, the system alerts, sound and Do not disturb. */
export function NotificationsSection() {
  return (
    <section className="hb-prefs-group" id="hb-prefs-notificaciones" aria-labelledby="hb-notif-title">
      <h3 id="hb-notif-title" className="hb-prefs-group-title">
        {t("notifications")}
      </h3>
      <DockAttentionSection />
      <SystemAlerts />
      <fieldset className="hb-prefs-section">
        <legend>{t("sound")}</legend>
        <p className="hb-prefs-hint">{t("soundHint")}</p>
      </fieldset>
      <DndDefault />
      <HourlyDigest />
      <AwaySuggest />
    </section>
  );
}
