"use client";

import * as React from "react";
import { createT } from "@/i18n";
import { postTheme } from "@/lib/api";
import { useTheme } from "./theme-context";

const t = createT("preferences");

/** Preferences › "Notify in the Dock": stored by serve beside the theme. */
export function DockAttentionSection() {
  const theme = useTheme();
  const [error, setError] = React.useState<string | null>(null);
  const state = theme?.state ?? null;
  const on = state?.dock_attention ?? true;
  return (
    <fieldset className="hb-prefs-section">
      <legend>Dock</legend>
      <label className="hb-mode">
        <input
          type="checkbox"
          checked={on}
          disabled={!state}
          data-testid="dock-attention"
          onChange={(e) => {
            setError(null);
            const next = e.target.checked;
            const run = () => postTheme("theme-dock", { on: next });
            (theme ? theme.write(run) : run()).catch(() => setError(t("couldNotSaveNotResponding")));
          }}
        />
        {t("dockAttention")}
      </label>
      <p className="hb-prefs-hint">{t("dockAttentionHint")}</p>
      {error ? <p className="hb-error">{error}</p> : null}
    </fieldset>
  );
}
