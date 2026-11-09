/**
 * Every date and time the console shows is on the operator's own clock, and
 * this is where that clock is read. The server renders the first paint and
 * the browser takes over on the same machine (the console listens on
 * loopback), so both answer the same and a server-rendered time hydrates
 * unchanged.
 */
const FALLBACK_TIME_ZONE = "UTC";

export function operatorTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || FALLBACK_TIME_ZONE;
  } catch {
    return FALLBACK_TIME_ZONE;
  }
}
