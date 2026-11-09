export const LOCALES = ["en", "es"] as const;
export type Locale = (typeof LOCALES)[number];

export const DEFAULT_LOCALE: Locale = "en";
export const LOCALE_COOKIE = "harbard-locale";

/** Each language by its own name: someone who cannot read the current one still finds theirs. */
export const LOCALE_NAMES: Record<Locale, string> = { en: "English", es: "Español" };

/**
 * Formatting tags, separate from the language choice because plain "en" would
 * switch the console's clocks to 12-hour and its dates to month-first.
 */
const INTL_TAGS: Record<Locale, string> = { en: "en-GB", es: "es-ES" };

function isLocale(value: string | null | undefined): value is Locale {
  return LOCALES.some((locale) => locale === value);
}

export function parseLocale(value: string | null | undefined): Locale {
  return isLocale(value) ? value : DEFAULT_LOCALE;
}

export function localeFromCookies(cookieHeader: string): Locale {
  for (const part of cookieHeader.split(";")) {
    const [name, value] = part.trim().split("=");
    if (name === LOCALE_COOKIE) return parseLocale(value);
  }
  return DEFAULT_LOCALE;
}

export function intlTagFor(locale: Locale): string {
  return INTL_TAGS[locale];
}
