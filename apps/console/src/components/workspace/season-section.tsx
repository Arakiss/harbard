"use client";

import * as React from "react";
import { Snowflake } from "lucide-react";
import { PrefsLegend } from "./prefs-legend";
import { activeLocale, createT, type TranslationKey } from "@/i18n";
import type { SeasonChoice, SeasonState } from "@/lib/theme";
import { useTheme } from "./theme-context";

const t = createT("preferences");

/** A season's name by id; a season serve adds later falls back to its id. */
const SEASON_NAMES: Record<string, TranslationKey<"preferences">> = {
  christmas: "seasonChristmas",
};

export function seasonName(id: string): string {
  const key = SEASON_NAMES[id];
  return key ? t(key) : id;
}

/** `MM-DD` as "1 December" / "1 de diciembre" in the console's language. */
export function seasonDay(mmdd: string): string {
  const [month, day] = mmdd.split("-").map(Number);
  // A leap year, so 29 February would format too.
  const date = new Date(Date.UTC(2028, (month ?? 1) - 1, day ?? 1));
  return new Intl.DateTimeFormat(activeLocale(), { day: "numeric", month: "long", timeZone: "UTC" }).format(date);
}

function choices(): readonly { id: SeasonChoice; label: string }[] {
  return [
    { id: "auto", label: t("seasonAuto") },
    { id: "on", label: t("seasonOn") },
    { id: "off", label: t("seasonOff") },
  ];
}

/**
 * Preferences › the season: Automatic paints its theme between its dates by
 * the operator's local date, On and Off override the dates; the chosen
 * theme is never touched. Decorations are a separate switch.
 */
export function SeasonSection({ season }: { season: SeasonState }) {
  const theme = useTheme();
  const [error, setError] = React.useState<string | null>(null);
  const name = seasonName(season.id);
  return (
    <fieldset className="hb-prefs-section" data-testid="season-section">
      <PrefsLegend icon={Snowflake}>{t("seasonTitle", { name })}</PrefsLegend>
      <div className="hb-mode-row">
        {choices().map((c) => (
          <label key={c.id} className="hb-mode">
            <input
              type="radio"
              name="hb-season"
              value={c.id}
              checked={season.choice === c.id}
              onChange={() => {
                setError(null);
                theme?.setSeason(c.id).catch(() => setError(t("couldNotSave")));
              }}
            />
            {c.label}
          </label>
        ))}
      </div>
      <p className="hb-prefs-hint">{t("seasonHint", { name, from: seasonDay(season.from), to: seasonDay(season.to) })}</p>
      <label className="hb-mode">
        <input
          type="checkbox"
          checked={season.decorations}
          data-testid="season-decorations"
          onChange={(e) => {
            setError(null);
            theme?.setDecorations(e.target.checked).catch(() => setError(t("couldNotSave")));
          }}
        />
        {t("seasonDecorations")}
      </label>
      <p className="hb-prefs-hint">{t("seasonDecorationsHint")}</p>
      {error ? <p className="hb-error">{error}</p> : null}
    </fieldset>
  );
}
