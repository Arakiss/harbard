import { Fragment, createElement, type ReactNode } from "react";
import { type Dictionaries, type Feature, en, es } from "./dictionaries";
import { DEFAULT_LOCALE, type Locale, intlTagFor, localeFromCookies } from "./locale";

export type { Feature } from "./dictionaries";
export type { Locale } from "./locale";

export type TranslationKey<F extends Feature> = keyof Dictionaries[F] & string;
export type TranslationVars = Record<string, string | number>;
export type Translator<F extends Feature> = (key: TranslationKey<F>, vars?: TranslationVars) => string;

const TABLES: Record<Locale, Record<Feature, Record<string, string>>> = { en, es };

/**
 * One language for the whole process, because most of the console's copy is
 * composed in plain functions (`lib/*.ts`) that React context cannot reach.
 *
 * In the browser the cookie is read here, before any module's first render,
 * and a change of language reloads the page. On the server `LocaleSync` sets it
 * at the top of each render; the console is one operator on loopback, so two
 * concurrent renders in different languages would need a browser and the
 * desktop app set differently and loading at the same instant, and the worst
 * outcome is a first paint that hydration corrects.
 */
let active: Locale = typeof document === "undefined" ? DEFAULT_LOCALE : localeFromCookies(document.cookie);

export function activeLocale(): Locale {
  return active;
}

export function setActiveLocale(locale: Locale): void {
  active = locale;
}

/** The BCP 47 tag for `Intl` and `toLocale*`. Call it at use, never at module scope. */
export function intlTag(): string {
  return intlTagFor(active);
}

function fill(template: string, vars?: TranslationVars): string {
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (hole, name: string) => {
    const value = vars[name];
    return value === undefined ? hole : String(value);
  });
}

function lookup(locale: Locale, feature: Feature, key: string): string {
  return TABLES[locale][feature][key] ?? TABLES[DEFAULT_LOCALE][feature][key] ?? key;
}

/**
 * The translator for one feature's dictionary. Safe to create at module scope
 * because it reads the language on every call; calling it at module scope is
 * not, since the value would be frozen in the language of the first import.
 */
export function createT<F extends Feature>(feature: F): Translator<F> {
  return (key, vars) => fill(lookup(active, feature, key), vars);
}

/** The same lookup with the language given, for Server Components (`i18n/server.ts`). */
export function translatorFor<F extends Feature>(locale: Locale, feature: F): Translator<F> {
  return (key, vars) => fill(lookup(locale, feature, key), vars);
}

/**
 * A translated sentence with React nodes in its holes, so each language keeps
 * its own word order: `richText(t("frameValue"), { value: <code>{frame}</code> })`.
 */
export function richText(template: string, nodes: Record<string, ReactNode>): ReactNode[] {
  return template.split(/(\{\w+\})/).map((piece, index) => {
    const name = /^\{(\w+)\}$/.exec(piece)?.[1];
    const content = name !== undefined && name in nodes ? nodes[name] : piece;
    return createElement(Fragment, { key: index }, content);
  });
}
