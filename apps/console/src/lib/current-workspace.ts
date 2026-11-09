/**
 * The workspace this console shows: one organisation of the hub. Kept
 * in a cookie, not in localStorage, so the first HTML (rendered
 * on the server) is already the right workspace's. Every read of the
 * workspace view names it (`?ws=`) and every desk write carries it
 * (`workspace` in the body): serve resolves a write's targets only inside
 * the workspace it names.
 */

export const WORKSPACE_COOKIE = "hb-ws";
/** Fired on `window` when the operator switches workspace. */
export const WORKSPACE_CHANGE_EVENT = "hb:workspace-change";

/** One tile of the rail, as serve sends it on the full view. */
export interface WorkspaceTile {
  id: string;
  name: string;
  /** One or two characters. */
  tile: string;
  default: boolean;
  current: boolean;
  /** Unread messages in its channels and Direct. */
  unread: number;
  /** Open items in its "Needs you". */
  needs: number;
  /** Agents running in it now. */
  working: number;
}

const ID = /^[a-z0-9][a-z0-9-]{0,31}$/;

/** True for a workspace id serve could have sent. */
export function validWorkspaceId(id: string | null | undefined): id is string {
  return typeof id === "string" && ID.test(id);
}

/** The id in a `Cookie` header (or `document.cookie`), when there is a valid one. */
export function workspaceFromCookie(cookie: string | null | undefined): string | null {
  if (!cookie) return null;
  for (const part of cookie.split(";")) {
    const [k, ...v] = part.trim().split("=");
    if (k === WORKSPACE_COOKIE) {
      const id = decodeURIComponent(v.join("="));
      return validWorkspaceId(id) ? id : null;
    }
  }
  return null;
}

/** The workspace this browser chose, or null to let serve pick the default. */
export function currentWorkspace(): string | null {
  if (typeof document === "undefined") return null;
  return workspaceFromCookie(document.cookie);
}

/** What the strip reads: every workspace, in one view. */
export const ALL_WORKSPACES = "all";

/**
 * The workspace a read names. The strip is the operator's one glance at the
 * whole hub and has no switcher, so it never follows the main window's choice;
 * a write always names a real one ({@link currentWorkspace}).
 */
export function readScope(path = typeof location === "undefined" ? "" : location.pathname): string | null {
  return path === "/pulse" || path.startsWith("/pulse/") ? ALL_WORKSPACES : currentWorkspace();
}

/** Switch: remember the choice and tell the shell to read the new workspace. */
export function setCurrentWorkspace(id: string): void {
  if (!validWorkspaceId(id) || typeof document === "undefined") return;
  document.cookie = `${WORKSPACE_COOKIE}=${encodeURIComponent(id)}; path=/; max-age=31536000; samesite=lax`;
  window.dispatchEvent(new CustomEvent<string>(WORKSPACE_CHANGE_EVENT, { detail: id }));
}

/** `path` with `ws=<id>` added to its query, or unchanged without one. */
export function withWorkspace(path: string, ws: string | null): string {
  if (!validWorkspaceId(ws)) return path;
  return `${path}${path.includes("?") ? "&" : "?"}ws=${encodeURIComponent(ws)}`;
}

/** A desk write's body with the workspace it was written from. */
export function bodyWithWorkspace<T extends Record<string, unknown>>(body: T, ws: string | null): T & { workspace?: string } {
  return validWorkspaceId(ws) ? { ...body, workspace: ws } : body;
}

/** The rail's order: where you are first, then the others as the hub lists them. */
export function railOrder(tiles: WorkspaceTile[]): WorkspaceTile[] {
  return [...tiles.filter((t) => t.current), ...tiles.filter((t) => !t.current)];
}

/** True when the operator typed the tile's text; by default it is only the name's first letter. */
export function customTile(t: Pick<WorkspaceTile, "name" | "tile">): boolean {
  const first = [...t.name].find((c) => /[\p{L}\p{N}]/u.test(c))?.toUpperCase();
  return t.tile !== (first ?? "·");
}

/** The other workspace with the most agents running now, or null when none has any. */
export function workingElsewhere(tiles: WorkspaceTile[] | undefined): WorkspaceTile | null {
  let best: WorkspaceTile | null = null;
  for (const t of tiles ?? []) {
    if (!t.current && t.working > 0 && (!best || t.working > best.working)) best = t;
  }
  return best;
}

/** What a tile's corner says: the open "Needs you" count, a dot for unread, or nothing. */
export function tileBadge(tile: Pick<WorkspaceTile, "needs" | "unread" | "current">): { kind: "count"; text: string } | { kind: "dot" } | null {
  if (tile.needs > 0) return { kind: "count", text: tile.needs > 99 ? "99+" : String(tile.needs) };
  if (!tile.current && tile.unread > 0) return { kind: "dot" };
  return null;
}

/** The name the sidebar's head shows: the current workspace's, else harbard. */
export function currentName(tiles: WorkspaceTile[] | undefined): string {
  return tiles?.find((t) => t.current)?.name ?? "harbard";
}

/** More than one workspace: the rail's tiles switch and only the name opens the menu. */
export function isMulti(tiles: WorkspaceTile[] | undefined): boolean {
  return (tiles?.length ?? 0) > 1;
}
