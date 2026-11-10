/* Server-only: imported by the layouts, never by client components. */
import "server-only";
import type { ThemeState } from "./theme";
import { harbardAuthHeaders, harbardUpstreamUrl } from "./upstream";

/**
 * The stored theme in the first HTML, so the shell never paints the default theme
 * before a custom theme. Bounded like the workspace read: a slow or dead
 * serve degrades to `undefined`, the token block in workspace.css paints the light
 * default theme, and the client poll picks the theme up when serve answers.
 */
export async function fetchInitialTheme(): Promise<ThemeState | undefined> {
  try {
    const res = await fetch(harbardUpstreamUrl("theme"), {
      headers: harbardAuthHeaders({ accept: "application/json" }),
      cache: "no-store",
      signal: AbortSignal.timeout(1500),
    });
    if (!res.ok) return undefined;
    const body = (await res.json()) as ThemeState;
    return Array.isArray(body.themes) ? body : undefined;
  } catch {
    return undefined;
  }
}
