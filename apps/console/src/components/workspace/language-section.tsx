"use client";

import { Languages } from "lucide-react";
import { PrefsLegend } from "./prefs-legend";
import { createT, activeLocale } from "@/i18n";
import { LOCALES, LOCALE_NAMES } from "@/i18n/locale";
import { chooseLocale } from "@/i18n/locale-sync";

const t = createT("preferences");

export function LanguageSection() {
  const current = activeLocale();
  return (
    <fieldset className="hb-prefs-section" data-testid="language">
      <PrefsLegend icon={Languages}>{t("languageLegend")}</PrefsLegend>
      <p className="hb-prefs-hint">{t("languageHint")}</p>
      <div className="hb-mode-row">
        {LOCALES.map((locale) => (
          <label key={locale} className="hb-mode">
            <input
              type="radio"
              name="hb-language"
              value={locale}
              checked={current === locale}
              data-testid={`language-${locale}`}
              onChange={() => void chooseLocale(locale)}
            />
            {LOCALE_NAMES[locale]}
          </label>
        ))}
      </div>
    </fieldset>
  );
}
