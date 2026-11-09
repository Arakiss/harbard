import "server-only";
import { cookies } from "next/headers";
import { type Feature, type Translator, translatorFor } from "./index";
import { LOCALE_COOKIE, type Locale, parseLocale } from "./locale";

export async function requestLocale(): Promise<Locale> {
  return parseLocale((await cookies()).get(LOCALE_COOKIE)?.value);
}

/** For Server Components and `generateMetadata`, whose module graph is not the one `LocaleSync` sets. */
export async function serverT<F extends Feature>(feature: F): Promise<Translator<F>> {
  return translatorFor(await requestLocale(), feature);
}
