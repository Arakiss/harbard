/**
 * Preferences › Pairing: the devices paired to this hub, their role, the
 * revocation and each one's log of remote reads and writes
 * (`serve/equipos_route.rs`). Pure helpers live here so the section stays a
 * thin view over them, and the tests read them without a DOM.
 */

import { type TranslationKey, createT } from "@/i18n";
import { API_V1 } from "./api";
import { relative } from "./progress";

const t = createT("teams");

export type Role = "operator" | "workshop" | "display";

export type DeviceAction = {
  ts_ms: number;
  method: string;
  path: string;
  status: number;
  write: boolean;
};

export type Device = {
  id: string;
  name: string;
  host: string;
  os: string;
  /** This Mac: always Operator, never revoked. */
  hub: boolean;
  access: Role;
  created_ms: number;
  joined_ms: number;
  last_seen_ms: number;
  revoked_ms?: number | null;
  /** False for a card paired before per-device credentials, or a revoked one. */
  credential: boolean;
  recent: DeviceAction[];
  reads: number;
  writes: number;
  /** The harbard the device runs, as its card last said (docs/SPEC-UPDATES.md); absent from older cards. */
  version?: string | null;
  commit?: string | null;
};

/**
 * "0.3.0-alpha.9 · a955a72" and whether the device runs another build than
 * the hub: a different commit, or a different version when either commit
 * is unknown. Nothing is said when the card carries no version.
 */
export function deviceVersion(d: Pick<Device, "version" | "commit" | "hub">, hub: { version: string; commit: string | null } | null): { text: string; behind: boolean } | null {
  if (!d.version && !d.commit) return null;
  const text = [d.version, d.commit ? d.commit.slice(0, 7) : null].filter(Boolean).join(" · ");
  if (d.hub || !hub) return { text, behind: false };
  const behind = d.commit && hub.commit ? !(hub.commit.startsWith(d.commit) || d.commit.startsWith(hub.commit)) : !!d.version && d.version !== hub.version;
  return { text, behind };
}

/** An invite nobody claimed yet: by name and a short reference, never its token. */
export type Invite = {
  ref: string;
  name: string;
  access: Role;
  created_ms: number;
  expires_ms?: number | null;
};

export type PairingView = { devices: Device[]; invites?: Invite[]; now_ms: number };

/** How long a pending invite still works, to the hour or the minute. */
export function expiresLine(i: Invite, now: number): string {
  if (!i.expires_ms) return t("expiresNever");
  const m = Math.floor((i.expires_ms - now) / 60_000);
  if (m < 1) return t("expiresUnderMinute");
  if (m < 60) return t("expiresMinutes", { minutes: m });
  return t("expiresHours", { hours: Math.floor(m / 60) });
}

export function roles(): { id: Role; label: string; hint: string }[] {
  return [
    { id: "operator", label: t("roleOperator"), hint: t("roleOperatorHint") },
    { id: "workshop", label: t("roleWorkshop"), hint: t("roleWorkshopHint") },
    { id: "display", label: t("roleDisplay"), hint: t("roleDisplayHint") },
  ];
}

export function roleLabel(role: Role): string {
  return roles().find((r) => r.id === role)?.label ?? role;
}

/** The deliberate act: Operator only once the device's name is typed. */
export function confirmMatches(typed: string, name: string): boolean {
  return typed.trim().toLocaleLowerCase("es") === name.trim().toLocaleLowerCase("es") && name.trim() !== "";
}

/** Where a device stands: the hub itself, revoked, without credential, or last seen. */
export function statusLine(d: Device, now: number): string {
  if (d.hub) return t("statusThisMac");
  if (d.revoked_ms) return t("statusRevoked", { when: relative(d.revoked_ms, now) });
  if (!d.credential) return t("statusNoCredential");
  if (!d.last_seen_ms) return t("statusNotConnected");
  return t("statusSeen", { when: relative(d.last_seen_ms, now) });
}

const PLACES: readonly [RegExp, TranslationKey<"teams">][] = [
  [/^\/api\/v1\/events$/, "placeEvents"],
  [/^\/api\/v1\/devices$/, "placeCard"],
  [/^\/api\/v1\/summary$/, "placeSummary"],
  [/^\/api\/v1\/profile\/brief$/, "placeContext"],
  [/^\/api\/v1\/profile(\/photo)?$/, "placeProfile"],
  [/^\/api\/v1\/attachments\//, "placeAttachment"],
  [/^\/api\/v1\/live\//, "placeLive"],
  [/^\/api\/v1\/workspace(\/|$)/, "placeDesktop"],
  [/^\/api\/v1\/(meta|health|protocol)$/, "placeHubState"],
];

function placeOf(path: string): string {
  const key = PLACES.find(([re]) => re.test(path))?.[1];
  return key ? t(key) : path;
}

/** One log line as words, refusals and errors marked. */
export function describeAction(a: DeviceAction): string {
  const place = placeOf(a.path);
  const refused = a.status === 401 || a.status === 403;
  const what =
    a.write && a.path === "/api/v1/events" && !refused
      ? t("actionPublishedEvents")
      : a.write
        ? refused
          ? t("actionTriedWrite", { place })
          : t("actionWrote", { place })
        : refused
          ? t("actionTriedRead", { place })
          : t("actionRead", { place });
  if (refused) return t("actionDenied", { what });
  if (a.status >= 400) return t("actionError", { what, status: a.status });
  return what;
}

async function send(path: string, body: Record<string, unknown>): Promise<Record<string, unknown>> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}${path}`, {
      method: "POST",
      headers: { accept: "application/json", "content-type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch {
    throw new Error(t("notResponding"));
  }
  const json = (await res.json().catch(() => null)) as Record<string, unknown> | null;
  if (!res.ok) throw new Error(refusalText(String(json?.error ?? ""), res.status));
  return json ?? {};
}

/** serve's refusal codes, in the operator's words. */
export function refusalText(code: string, status: number): string {
  switch (code) {
    case "confirm_required":
      return t("refusalConfirmRequired");
    case "revoked":
      return t("refusalRevoked");
    case "hub_itself":
      return t("refusalHubItself");
    case "forbidden_origin":
      return t("refusalForbiddenOrigin");
    case "unknown_invite":
      return t("refusalUnknownInvite");
    default:
      return t("refusalDefault", { status });
  }
}

export async function fetchPairing(signal?: AbortSignal): Promise<PairingView> {
  const res = await fetch(`${API_V1}/pairing`, { headers: { accept: "application/json" }, cache: "no-store", signal });
  if (!res.ok) throw new Error(`GET /pairing ${res.status}`);
  return (await res.json()) as PairingView;
}

export async function fetchDeviceLog(id: string, signal?: AbortSignal): Promise<DeviceAction[]> {
  const res = await fetch(`${API_V1}/pairing/${encodeURIComponent(id)}/log?limit=100`, {
    headers: { accept: "application/json" },
    cache: "no-store",
    signal,
  });
  if (!res.ok) throw new Error(`GET /pairing/${id}/log ${res.status}`);
  return ((await res.json()) as { actions: DeviceAction[] }).actions;
}

export function postRole(id: string, role: Role, confirm?: string): Promise<Record<string, unknown>> {
  return send(`/pairing/${encodeURIComponent(id)}/role`, confirm === undefined ? { role } : { role, confirm });
}

export function postRevoke(id: string): Promise<Record<string, unknown>> {
  return send(`/pairing/${encodeURIComponent(id)}/revoke`, {});
}

/** Cancel a pending invite: its pair line stops working at once. */
export function postRevokeInvite(ref: string): Promise<Record<string, unknown>> {
  return send(`/pairing/invites/${encodeURIComponent(ref)}/revoke`, {});
}
