import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const DEFAULT_HARBARD_API = "http://127.0.0.1:7419";

function configuredBase(): URL {
  const raw = (process.env.HARBARD_API ?? DEFAULT_HARBARD_API).trim();
  const base = new URL(raw);
  if (base.protocol !== "http:" && base.protocol !== "https:") {
    throw new Error("HARBARD_API must use http or https");
  }
  if (base.username || base.password) {
    throw new Error("HARBARD_API must not contain credentials");
  }
  base.search = "";
  base.hash = "";
  base.pathname = `${base.pathname.replace(/\/$/, "")}/api/v1/`;
  return base;
}

export function harbardUpstreamUrl(path: string, search = ""): URL {
  const cleanPath = path.replace(/^\/+/, "");
  if (cleanPath.split("/").some((segment) => segment === "..")) {
    throw new Error("Harbard API path must not traverse directories");
  }
  const url = new URL(cleanPath, configuredBase());
  const params = new URLSearchParams(search.startsWith("?") ? search.slice(1) : search);
  params.delete("token");
  url.search = params.toString();
  return url;
}

/**
 * The hub token on disk would go to an upstream that is not this Mac's own
 * serve. Refused rather than sent, or rather than sent without a token: the
 * proxy answers it as a 502 and says what to set.
 */
export class UpstreamTokenRefused extends Error {
  constructor(upstream: string) {
    super(
      `HARBARD_API points at ${upstream}, which is not the local serve (127.0.0.1:${servePort()}): ` +
        "the hub token on disk is sent only there. Set HARBARD_HUB_TOKEN for that upstream.",
    );
    this.name = "UpstreamTokenRefused";
  }
}

/** serve's port: 7419, or `HARBARD_API_PORT` as the LaunchAgents install it. */
function servePort(): string {
  return process.env.HARBARD_API_PORT?.trim() || "7419";
}

const LOOPBACK_NAMES: ReadonlySet<string> = new Set(["127.0.0.1", "localhost", "[::1]"]);

/**
 * Whether `base` is this Mac's own serve: plain http on a loopback name and
 * serve's port. Only that upstream may receive the token read from disk.
 */
export function localServe(base: URL): boolean {
  const port = base.port || (base.protocol === "https:" ? "443" : "80");
  return base.protocol === "http:" && LOOPBACK_NAMES.has(base.hostname) && port === servePort();
}

/**
 * The headers for a server-side call to serve. `HARBARD_HUB_TOKEN`, when set,
 * goes to whatever `HARBARD_API` names: whoever set both chose that. The
 * token on disk goes only to the local serve (`localServe`); any other
 * upstream without `HARBARD_HUB_TOKEN` throws `UpstreamTokenRefused`. A
 * throwaway console pointed at a mock once read the operator's real token from
 * disk and sent it to the mock.
 */
export function harbardAuthHeaders(initial?: HeadersInit): Headers {
  const headers = new Headers(initial);
  const token = configuredToken();
  if (token) headers.set("authorization", `Bearer ${token}`);
  return headers;
}

function configuredToken(): string | undefined {
  const fromEnvironment = process.env.HARBARD_HUB_TOKEN?.trim();
  if (fromEnvironment) return fromEnvironment;
  const base = configuredBase();
  if (!localServe(base)) throw new UpstreamTokenRefused(base.origin);
  const root = process.env.HARBARD_HOME?.trim() || join(homedir(), ".harbard");
  try {
    return readFileSync(join(root, "hub.token"), "utf8").trim() || undefined;
  } catch {
    return undefined;
  }
}

/**
 * Roots this console may proxy to `serve`, by method.
 *
 * They live here, beside a test, rather than inside the route handler, because
 * an allowlist with no test is a list that silently falls behind its callers:
 * `history` was missing from the day the History view shipped, so that tab
 * answered "Could not load archive · returned 404" to every operator who ever
 * opened it, and nothing failed anywhere.
 */
export const GET_ROOTS: ReadonlySet<string> = new Set([
  "aliases",
  // Files `harbard attach` copied into the harbard home: serve reads them from
  // there and nowhere else, so this root never reaches a private folder.
  "attachments",
  "channels",
  "delivery",
  "dependencies",
  "devices",
  // What the operator's coordination states held (`harbard_core::hold`).
  "digests",
  "doctor",
  // Preferences › Pairing: the paired devices and each one's log.
  "pairing",
  "evals",
  "events",
  // The fleet projection and the one paginated timeline (SPEC-FLEET-VIEW).
  "feed",
  "fleet",
  // The operator's one-tap feedback: the standing answers and Undo's target.
  "feedback",
  "history",
  "leases",
  "learning",
  // One agent's steps and prose, for the hover card and "See it live".
  "live",
  "meta",
  "notes",
  // Both were missing for the same reason `history` was, and both are live in
  // serve: `/api/v1/search` backs the ⌘K palette, `/api/v1/operator` backs the
  // operator profile. Nothing failed at build time because nothing checked.
  "operator",
  // The operator's "Profile", status, Do not disturb and photo.
  "profile",
  // The session retro: metrics, lesson candidates and the active rules.
  "retro",
  "search",
  "sessions",
  "snapshot",
  "setup",
  "stream",
  // The team view (teammates and subagents from Claude Code transcripts).
  "team",
  // The console theme and mode, and every theme's tokens (the picker).
  "theme",
  // The chat-shaped shell over the team view, the bus and the sluice.
  "workspace",
  // "Update harbard": the installed version, the newer one, a run.
  "update",
  // The hub's workspaces and where each repository is placed.
  "workspaces",
]);

export const POST_ROOTS: ReadonlySet<string> = new Set([
  // "Attach file": the bytes the operator picked, stored by serve in the
  // attachments folder (loopback, their origin only).
  "attachments",
  "channels",
  // `POST /digests/read`: a digest seen; loopback-only in serve.
  "digests",
  // `POST /pairing/<id>/role|revoke`: Preferences › Pairing, desk door in serve.
  "pairing",
  "evals",
  "notes",
  // `POST /operator` (the profile) and `POST /operator/reads` (the operator's
  // own read cursors, §4) share this root. Never `cursors/`, which is the
  // agents' and stays unreachable from here.
  "operator",
  // `POST /profile` (a partial change) and `POST /profile/photo` (the bytes
  // the operator picked), both loopback-only desk writes in serve.
  "profile",
  "setup",
  // `POST /update/check|install|prefs`: loopback-only, Operator, in serve.
  "update",
  // The operator's desk marks, replies and file opens (`/workspace/<action>`).
  "workspace",
  // Moving a repository, renaming a workspace or its tile (`/workspaces/<action>`),
  // a loopback-only desk write in serve.
  "workspaces",
]);

/** First path segment — what the allowlist actually matches on. */
export function upstreamRoot(path: string): string {
  return path.replace(/^\/+/, "").split(/[/?]/)[0] ?? "";
}

/**
 * Whether the proxy accepts a write. The Host must be the console's own
 * loopback name with a port (`127.0.0.1:<port>` or `localhost:<port>`), so a
 * DNS-rebinding page, whose Host and Origin agree on its own name, cannot
 * write; and the browser's Origin, when sent, must match that Host (CSRF).
 * When the console's `PORT` is known, the port must be it.
 */
/**
 * Whether the request's Host is the console's own loopback name with a port.
 * Every proxied request must pass it, reads too: a DNS-rebinding
 * page on the console port has its own name as Host, and the proxy would
 * otherwise hand it the workspace and its attachments with the hub token.
 */
export function hostAllowed(host: string | null): boolean {
  const match = /^(127\.0\.0\.1|localhost|\[::1\]):(\d{1,5})$/.exec(host ?? "");
  if (match) {
    const port = process.env.PORT?.trim();
    return !port || match[2] === port;
  }
  return namedHostAllowed(host);
}

/**
 * The named dev host (portless, `harbard.localhost`), only when opted in:
 * `HARBARD_ALLOWED_HOSTS=harbard.localhost,foo.local`. Never on by default.
 * The name must match exactly; a port, if any, is free.
 */
export function namedHostAllowed(host: string | null): boolean {
  const list = (process.env.HARBARD_ALLOWED_HOSTS ?? "")
    .split(",")
    .map((h) => h.trim().toLowerCase())
    .filter(Boolean);
  if (list.length === 0 || !host) return false;
  const name = host.toLowerCase().replace(/:\d{1,5}$/, "");
  return list.includes(name);
}

export function writeAllowed(req: { host: string | null; origin: string | null; protocol: string }): boolean {
  if (!hostAllowed(req.host)) return false;
  if (!req.origin) return true;
  try {
    const parsed = new URL(req.origin);
    return parsed.host === req.host && parsed.protocol === req.protocol;
  } catch {
    return false;
  }
}

/**
 * GET roots whose answer is private enough to need the same Host rule as a
 * write: `live` carries an agent's own prose. A DNS-rebinding page that
 * learnt an agent key from another GET still cannot read it, because its
 * Host is its own name, not a loopback one.
 * `history` joined it with History, which reads a month of that prose.
 * The other GETs keep their present rule; widening it is a separate change.
 */
const PRIVATE_GET_ROOTS: ReadonlySet<string> = new Set([
  "live",
  // History: a month of agents' last lines and whole conversations.
  "history",
  // The update run's log and the checkout's path on this Mac.
  "update",
]);

/** May this GET or HEAD be forwarded? Every read needs the console's own Host; private roots also a matching Origin. */
export function readAllowed(root: string, req: { host: string | null; origin: string | null; protocol: string }): boolean {
  if (!hostAllowed(req.host)) return false;
  return !PRIVATE_GET_ROOTS.has(root) || writeAllowed(req);
}
