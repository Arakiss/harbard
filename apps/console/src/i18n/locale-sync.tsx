"use client";

import * as React from "react";
import { deskSetLocale } from "@/lib/desk-prefs";
import { setActiveLocale } from "./index";
import { LOCALE_COOKIE, type Locale } from "./locale";

/** A year: the choice should outlast any update, and the console is the only reader. */
const COOKIE_MAX_AGE_SECONDS = 60 * 60 * 24 * 365;

/**
 * Sets the server's language before any child renders, and keeps the native
 * shell's menus in the language the page is using.
 */
export function LocaleSync({ locale, children }: { locale: Locale; children: React.ReactNode }) {
  if (typeof window === "undefined") setActiveLocale(locale);
  React.useEffect(() => {
    void deskSetLocale(locale);
  }, [locale]);
  return children;
}

/**
 * Reloads instead of re-rendering: memoised labels and module state in the
 * running page were all computed in the old language.
 */
export async function chooseLocale(locale: Locale): Promise<void> {
  document.cookie = `${LOCALE_COOKIE}=${locale}; path=/; max-age=${COOKIE_MAX_AGE_SECONDS}; samesite=lax`;
  await deskSetLocale(locale);
  window.location.reload();
}
