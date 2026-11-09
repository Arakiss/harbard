import { createT } from "@/i18n";

const t = createT("desktop");

/** Fixed view destinations; native commands only show existing windows. */
export type DesktopView = "overview" | "strip";
export type MainDestination = {
  /** The workspace shell's Direct (`session` is the agent key) and
   * Activity (`note` is the item, `card:<id>`). The previous console's
   * views were removed with it. */
  view: "direct" | "activity";
  note?: string;
  session?: string;
};
type Invoke = (command: string, args?: { destination: MainDestination }) => Promise<unknown>;
type Host = {
  __TAURI__?: { core?: { invoke?: Invoke } };
  __TAURI_INTERNALS__?: { invoke?: Invoke };
  navigator?: { userAgent: string };
};

export type DesktopViewBridge =
  { kind: "browser" } | { kind: "desktop"; invoke: Invoke | null };

const destinations = {
  overview: { href: "/?shell=desktop", command: "desk_open_main" },
  strip: { href: "/pulse?shell=desktop", command: "desk_open_strip" },
} as const;

const MAIN_VIEWS = ["direct", "activity"];
/** Destinations the workspace shell opens, by route: all of them now. */
export const SHELL_VIEWS = ["direct", "activity"] as const;

export function validMainDestination(value: unknown): value is MainDestination {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  if (typeof record.view !== "string" || !MAIN_VIEWS.includes(record.view)) return false;
  if (Object.keys(record).some((key) => !["view", "note", "session"].includes(key))) return false;
  return ["note", "session"].every((key) => record[key] === undefined ||
    (typeof record[key] === "string" && record[key].length > 0 && record[key].length <= 512 && !/[\u0000-\u001f\u007f]/.test(record[key])));
}

/**
 * "Check for updates…" in the tray and the native "A new version of
 * harbard is available" notification: `{ view: "update" }`, exactly, with
 * no other key. It opens the update dialog, never a route, so it stays out
 * of `MainDestination` and the native commands that carry one.
 */
export function updateDestination(value: unknown): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const keys = Object.keys(value);
  return keys.length === 1 && keys[0] === "view" && (value as Record<string, unknown>).view === "update";
}

/** The window event the update provider listens to (update-state.tsx). */
export const OPEN_UPDATE_EVENT = "harbard:open-update";

/**
 * The workspace shell's own route for Direct and Activity: one agent's
 * conversation, or one item of Activity selected.
 */
export function shellRoute(destination: MainDestination): string {
  if (destination.view === "direct") {
    return destination.session ? `/direct/${encodeURIComponent(destination.session)}` : "/direct";
  }
  return destination.note ? `/activity?${new URLSearchParams({ sel: destination.note })}` : "/activity";
}

export function desktopViewHref(target: DesktopView, destination?: MainDestination): string {
  if (!destination || target !== "overview") return destinations[target].href;
  if (!validMainDestination(destination)) throw new Error("Invalid main view destination.");
  return shellRoute(destination);
}

export function desktopViewBridge(
  host: Host | undefined = typeof window === "undefined" ? undefined : window,
): DesktopViewBridge {
  if (!host) return { kind: "browser" };
  const invoke =
    host.__TAURI__?.core?.invoke ?? host.__TAURI_INTERNALS__?.invoke;
  if (typeof invoke === "function") return { kind: "desktop", invoke };
  // The query flag also appears in ordinary browser links, so it is not IPC proof.
  const native =
    "__TAURI__" in host ||
    "__TAURI_INTERNALS__" in host ||
    /Tauri/i.test(host.navigator?.userAgent ?? "");
  return native ? { kind: "desktop", invoke: null } : { kind: "browser" };
}

export async function openDesktopView(
  target: DesktopView,
  bridge: DesktopViewBridge = desktopViewBridge(),
  destination?: MainDestination,
): Promise<"opened" | "browser"> {
  if (destination && !validMainDestination(destination)) throw new Error("Invalid main view destination.");
  if (bridge.kind === "browser") return "browser";
  if (!bridge.invoke) {
    throw new Error(t("windowControlsUnavailable"));
  }
  try {
    await bridge.invoke(destinations[target].command,
      target === "overview" && destination ? { destination } : undefined);
    return "opened";
  } catch {
    // A failed invocation can follow a partially completed show operation.
    // Never navigate or create another window as an implicit retry.
    throw new Error(t("couldNotSwitchViewsHelp"));
  }
}

/**
 * Hide the Strip window (Esc or ⌘W on the strip). Nothing to do in a
 * browser tab, or when the native bridge is missing; the window's own close
 * button and the View menu still work there.
 */
export async function closeDesktopStrip(
  bridge: DesktopViewBridge = desktopViewBridge(),
): Promise<"closed" | "browser"> {
  if (bridge.kind === "browser" || !bridge.invoke) return "browser";
  await bridge.invoke("desk_close_strip");
  return "closed";
}
