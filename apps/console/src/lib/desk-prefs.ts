/** Desktop shell prefs in `~/.harbard/desk.json` (Rust owns the file). */

export type DeskPrefs = {
  mute: boolean;
};

type InvokeFn = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

function resolveInvoke(): InvokeFn | null {
  if (typeof window === "undefined") return null;
  const w = window as unknown as {
    __TAURI__?: { core?: { invoke?: InvokeFn } };
    __TAURI_INTERNALS__?: { invoke?: InvokeFn };
  };
  const globalInvoke = w.__TAURI__?.core?.invoke;
  if (typeof globalInvoke === "function") return globalInvoke;
  const internalsInvoke = w.__TAURI_INTERNALS__?.invoke;
  if (typeof internalsInvoke === "function") return internalsInvoke;
  return null;
}

export function deskPrefsAvailable(): boolean {
  return resolveInvoke() !== null;
}

export async function deskGetPrefs(): Promise<DeskPrefs | null> {
  const invoke = resolveInvoke();
  if (!invoke) return null;
  try {
    return (await invoke("desk_get_prefs")) as DeskPrefs;
  } catch {
    return null;
  }
}

export async function deskSetMute(mute: boolean): Promise<boolean> {
  const invoke = resolveInvoke();
  if (!invoke) return false;
  try {
    await invoke("desk_set_mute", { mute });
    return true;
  } catch {
    return false;
  }
}

/** Tells the native shell which language its menus and notifications use. */
export async function deskSetLocale(locale: string): Promise<boolean> {
  const invoke = resolveInvoke();
  if (!invoke) return false;
  try {
    await invoke("desk_set_locale", { locale });
    return true;
  } catch {
    return false;
  }
}

export type OsNotifications = { allowed: boolean; badge: boolean };

/** What macOS allows harbard to show; null in a browser or when unknown. */
export async function deskOsNotifications(): Promise<OsNotifications | null> {
  const invoke = resolveInvoke();
  if (!invoke) return null;
  try {
    return (await invoke("desk_os_notifications")) as OsNotifications | null;
  } catch {
    return null;
  }
}

/** Open harbard's page in System Settings. */
export async function deskOpenNotificationSettings(): Promise<void> {
  await resolveInvoke()?.("desk_open_notification_settings").catch(() => {});
}
