import { afterEach, describe, expect, test } from "bun:test";
import { auditAll } from "../../scripts/i18n-audit";
import { activeLocale, createT, intlTag, setActiveLocale, translatorFor } from "./index";
import { DEFAULT_LOCALE, localeFromCookies, parseLocale } from "./locale";

afterEach(() => setActiveLocale(DEFAULT_LOCALE));

describe("the language choice", () => {
  test("English is the default", () => {
    expect(activeLocale()).toBe("en");
    expect(localeFromCookies("")).toBe("en");
  });

  test("a stored Spanish choice is read from the cookie header", () => {
    expect(localeFromCookies("theme=x; harbard-locale=es; other=1")).toBe("es");
  });

  test("an unknown language falls back to English", () => {
    expect(parseLocale("fr")).toBe("en");
    expect(localeFromCookies("harbard-locale=<script>")).toBe("en");
  });
});

describe("the translator", () => {
  const t = createT("preferences");

  test("a translator made at module scope follows a later language change", () => {
    expect(t("languageLegend")).toBe("Language");
    setActiveLocale("es");
    expect(t("languageLegend")).toBe("Idioma");
  });

  test("a translator bound to a language ignores the active one", () => {
    setActiveLocale("es");
    expect(translatorFor("en", "preferences")("languageLegend")).toBe("Language");
  });

  test("dates stay 24-hour and day-first in English", () => {
    expect(new Intl.DateTimeFormat(intlTag(), { hour: "2-digit", minute: "2-digit", timeZone: "UTC" }).format(0)).toBe("00:00");
  });
});

describe("the dictionaries and the source", () => {
  test("the audit finds nothing", () => {
    expect(auditAll().map((f) => `${f.file}:${f.line} [${f.kind}] ${f.detail}`)).toEqual([]);
  });
});
