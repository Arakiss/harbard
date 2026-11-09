/**
 * The single door to `harbard serve`.
 *
 * Every call the console makes goes through this module. The invariants are
 * structural here, not conventional (acceptance row B6):
 *   1. `get()` hardcodes GET; writes are `postNote`, `postEval`, and
 *      `postChannelAction` (operator-stamped by serve).
 *   2. `cursors/` is never addressed — the API exposes no such path and this
 *      client constructs none. Cursors belong to agents.
 *   3. The console never claims an identity. `postNote` sends text and an
 *      optional addressee; `serve` stamps who wrote it. Impersonating an agent
 *      is impossible from here by construction, not by discipline.
 *
 * Swapping the mock for the real serve is a base-URL change only: both bind the
 * same default `127.0.0.1:7419` and speak the same contract.
 */

import { createT } from "@/i18n";
import { historyQuery, type HistoryConversation, type HistoryPage } from "./history";
import { invisibleRefusal } from "./invisible";
import type { DigestItem, Digests, ProfilePatch, ProfileView, StateId } from "./profile";
import type { ThemeRefusal, ThemeState, ThemeUnchanged } from "./theme";
import type { ActivityView, WorkspaceAction, WorkspaceView } from "./workspace";
import { bodyWithWorkspace, currentWorkspace, readScope, withWorkspace } from "./current-workspace";
import type { LiveDelta } from "./live-agent";
import type { UpdateChannel, UpdateView } from "./update";
import { consoleCommit } from "./deliveries";
import type { FeedbackRecord, FeedbackSample, FeedbackView, TargetKind } from "./feedback";
import type { RetroBody, RetroCount } from "./retro";
import type {
  ApiError,
  MetaResponse,
  SetupInstallResponse,
  SetupStatus,
} from "./types";

const t = createT("errors");

/**
 * Browser traffic stays same-origin. The Next.js route handler forwards it to
 * `harbard serve` and adds any hub credential on the server side, so a bearer
 * token can never be compiled into the client bundle.
 */
const API_BASE = "/api/harbard";
export const API_V1 = API_BASE;

function authHeaders(extra?: HeadersInit): HeadersInit {
  const headers: Record<string, string> = {
    accept: "application/json",
  };
  if (extra) {
    Object.assign(headers, extra);
  }
  return headers;
}

export class ApiUnavailable extends Error {
  constructor(
    message: string,
    readonly cause?: unknown,
    /**
     * HTTP status when there was a response at all. Absent for a network
     * failure or an abort. Callers need this to tell "this serve does not have
     * the route" (404 — stand down) from "something broke just now" (5xx, no
     * connection — keep trying), a distinction the message string cannot carry
     * safely.
     */
    readonly status?: number,
  ) {
    super(message);
    this.name = "ApiUnavailable";
  }
}

async function get<T>(path: string, signal?: AbortSignal): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}${path}`, {
      method: "GET",
      headers: authHeaders(),
      cache: "no-store",
      signal,
    });
  } catch (cause) {
    throw new ApiUnavailable(t("getFailedToReach", { path }), cause);
  }
  if (!res.ok) {
    throw new ApiUnavailable(t("getReturned", { path, status: res.status }), undefined, res.status);
  }
  return (await res.json()) as T;
}

/**
 * `GET /api/v1/workspace` — the chat-shaped shell: channels, agents, messages
 * and the cards that need the operator. Pull-only, like `/team`.
 */
export function fetchWorkspace(signal?: AbortSignal): Promise<WorkspaceView> {
  return get<WorkspaceView>(workspacePath(), signal);
}

/**
 * `/workspace`, saying which commit this console runs ("Installed" needs it
 * in both builds) and which workspace it shows.
 */
function workspacePath(commit = consoleCommit(), ws = readScope()): string {
  return withWorkspace(commit ? `/workspace?console=${commit}` : "/workspace", ws);
}

/**
 * `GET /api/v1/live/<agent>`: one agent's steps since the cursor `after`,
 * and its prose unless `prose` is false. Local and private; see
 * `lib/live-agent.ts`.
 */
export function fetchLive(key: string, after: number | null, prose: boolean, signal?: AbortSignal, only?: "progress"): Promise<LiveDelta> {
  const q = new URLSearchParams();
  if (after !== null) q.set("after", String(after));
  if (!prose) q.set("prose", "0");
  if (only) q.set("only", only);
  const qs = q.toString();
  return get<LiveDelta>(`/live/${encodeURIComponent(key)}${qs ? `?${qs}` : ""}`, signal);
}

/** `GET /api/v1/workspace?only=activity`: faces, reactions and badges, without messages. */
export function fetchActivity(signal?: AbortSignal): Promise<ActivityView> {
  return get<ActivityView>(withWorkspace("/workspace?only=activity", readScope()), signal);
}

/**
 * `GET /api/v1/theme` — the stored theme and mode, every theme's tokens and
 * every refused file. With `since`, serve answers `{unchanged: true}` until
 * something changed, so the shell can poll it cheaply.
 */
export function fetchTheme(since?: string, signal?: AbortSignal): Promise<ThemeState | ThemeUnchanged> {
  const q = since ? `?since=${encodeURIComponent(since)}` : "";
  return get<ThemeState | ThemeUnchanged>(`/theme${q}`, signal);
}

/** A refused theme write, with serve's typed reason when there is one. */
export class ThemeRefused extends Error {
  constructor(
    message: string,
    readonly refusal?: ThemeRefusal,
  ) {
    super(message);
    this.name = "ThemeRefused";
  }
}

export type ThemeAction = "theme-set" | "theme-mode" | "theme-preview" | "theme-save" | "theme-import" | "theme-open-folder" | "theme-dock";

/** `POST /api/v1/workspace/theme-*`: choose, save, import, preview, open the folder. */
export async function postTheme<T = ThemeState>(action: ThemeAction, body: Record<string, unknown>): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/workspace/${action}`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: JSON.stringify(body),
    });
  } catch (cause) {
    throw new ThemeRefused(t("harbardNotResponding", { cause: String(cause) }));
  }
  const json = (await res.json().catch(() => null)) as { message?: string; refusal?: ThemeRefusal } | null;
  if (!res.ok) throw new ThemeRefused(json?.refusal?.message ?? json?.message ?? t("serveAnswered", { status: res.status }), json?.refusal);
  return json as T;
}

/** A refused workspace write: the HTTP status and serve's reason code. */
export class WorkspaceRefused extends Error {
  constructor(
    message: string,
    readonly status?: number,
    readonly reason?: string,
  ) {
    super(message);
    this.name = "WorkspaceRefused";
  }
}

/**
 * `POST /api/v1/workspace/<action>` — the operator's own desk marks, replies
 * stored as operator notes to a lead session, and asking the OS to open a
 * file card. serve decides where a reply goes; the console only names the
 * agent or the channel.
 */
export async function postWorkspace(action: WorkspaceAction, body: Record<string, unknown>): Promise<unknown> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/workspace/${action}`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      // serve resolves every target inside the workspace the write names.
      body: JSON.stringify(bodyWithWorkspace(body, currentWorkspace())),
    });
  } catch (cause) {
    throw new WorkspaceRefused(t("serveUnreachable", { cause: String(cause) }));
  }
  if (!res.ok) {
    const detail = (await res.json().catch(() => null)) as { message?: string; reason?: string } | null;
    // Hidden characters in a reply: serve names them; say it in the operator's language.
    const hidden = detail?.message ? invisibleRefusal(detail.message) : null;
    throw new WorkspaceRefused(hidden ?? detail?.message ?? t("serveAnswered", { status: res.status }), res.status, detail?.reason);
  }
  return res.json();
}

/** `GET /api/v1/workspaces`: every workspace, its rule, and where each repository is placed. */
export interface WorkspacesList {
  single: boolean;
  default: string;
  workspaces: { id: string; name: string; tile: string; icon: string; owners: string[]; default: boolean }[];
  repos: { repo: string; workspace: string; moved: boolean; reason: string }[];
}

export function fetchWorkspaces(signal?: AbortSignal): Promise<WorkspacesList> {
  return get<WorkspacesList>("/workspaces", signal);
}

/** `POST /api/v1/workspaces/<action>`: move a repository, give it back to the rule, rename, re-tile. */
export async function postWorkspaces(
  action: "move" | "reset" | "rename" | "icon",
  body: Record<string, unknown>,
): Promise<unknown> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/workspaces/${action}`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: JSON.stringify(body),
    });
  } catch (cause) {
    throw new WorkspaceRefused(t("harbardNotResponding", { cause: String(cause) }));
  }
  const json = (await res.json().catch(() => null)) as { message?: string; error?: string } | null;
  if (!res.ok) throw new WorkspaceRefused(json?.message ?? t("serveAnswered", { status: res.status }), res.status, json?.error);
  return json;
}

/** `GET /api/v1/feedback`: the operator's standing taps and the one Undo takes back. */
export function fetchFeedback(signal?: AbortSignal): Promise<FeedbackView> {
  return get<FeedbackView>("/feedback", signal);
}

export type FeedbackTap = { target_kind: TargetKind; target_id: string; value: string; reason?: string; sample?: FeedbackSample };

/** `POST /api/v1/workspace/feedback` and `feedback-undo`, through the desk's door. */
export async function postFeedback(
  action: "feedback" | "feedback-undo",
  body: FeedbackTap | { id?: string },
): Promise<{ ok: boolean; outcome?: "recorded" | "already"; record: FeedbackRecord }> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/workspace/${action}`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: JSON.stringify(bodyWithWorkspace(body, currentWorkspace())),
    });
  } catch (cause) {
    throw new WorkspaceRefused(t("serveUnreachable", { cause: String(cause) }));
  }
  const json = (await res.json().catch(() => null)) as { message?: string; error?: string; record?: FeedbackRecord; outcome?: "recorded" | "already" } | null;
  if (!res.ok || !json?.record) throw new WorkspaceRefused(json?.message ?? t("serveAnswered", { status: res.status }), res.status, json?.error);
  return { ok: true, outcome: json.outcome, record: json.record };
}

/** `GET /api/v1/profile`: the operator's profile, status and Do not disturb in force now. */
export function fetchProfile(signal?: AbortSignal): Promise<ProfileView> {
  return get<ProfileView>("/profile", signal);
}

/** The photo, served by serve; `rev` changes with every new photo. */
export function profilePhotoUrl(rev: number): string {
  return `${API_V1}/profile/photo?v=${rev}`;
}

/** A refused profile write: serve's code and the field it names. */
export class ProfileRefused extends Error {
  constructor(
    message: string,
    readonly code?: string,
    readonly field?: string,
    readonly status?: number,
  ) {
    super(message);
    this.name = "ProfileRefused";
  }
}

async function profileAnswer(res: Response): Promise<ProfileView> {
  const json = (await res.json().catch(() => null)) as (ProfileView & { error?: string; message?: string; field?: string }) | null;
  if (!res.ok || !json) throw new ProfileRefused(json?.message ?? t("serveAnswered", { status: res.status }), json?.error, json?.field, res.status);
  return json;
}

/** `POST /api/v1/profile`: a partial change, all or nothing. */
export async function postProfile(patch: ProfilePatch): Promise<ProfileView> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/profile`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: JSON.stringify(patch),
    });
  } catch (cause) {
    throw new ProfileRefused(t("harbardNotResponding", { cause: String(cause) }));
  }
  return profileAnswer(res);
}

/** `POST /api/v1/profile/photo`: the bytes of the image the operator picked. */
export async function postProfilePhoto(file: Blob): Promise<ProfileView> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/profile/photo`, {
      method: "POST",
      headers: authHeaders({ "content-type": file.type || "application/octet-stream" }),
      body: file,
    });
  } catch (cause) {
    throw new ProfileRefused(t("harbardNotResponding", { cause: String(cause) }));
  }
  return profileAnswer(res);
}

/** `GET /api/v1/digests`: what the coordination states held, newest last. */
export function fetchDigests(signal?: AbortSignal): Promise<Digests> {
  return get<Digests>("/digests", signal);
}

/** `GET /api/v1/digests/pending`: what the current state holds so far, uncut. */
export function fetchPendingDigest(signal?: AbortSignal): Promise<{ state: StateId; since_ms: number | null; items: DigestItem[] }> {
  return get("/digests/pending", signal);
}

/** `POST /api/v1/digests/read`: the operator has seen this digest. */
export async function postDigestRead(id: string): Promise<void> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/digests/read`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: JSON.stringify({ id }),
    });
  } catch (cause) {
    throw new ProfileRefused(t("harbardNotResponding", { cause: String(cause) }));
  }
  if (!res.ok) throw new ProfileRefused(t("serveAnswered", { status: res.status }), undefined, undefined, res.status);
}

/** `GET /api/v1/retro`: the week, or one finished team with `lead`. */
export function fetchRetro(lead?: string | null, signal?: AbortSignal): Promise<RetroBody> {
  const q = lead ? `?scope=team&lead=${encodeURIComponent(lead)}` : "";
  return get<RetroBody>(`/retro${q}`, signal);
}

/** `GET /api/v1/retro/count`: the week's undecided rules, from serve's cache. */
export function fetchRetroCount(signal?: AbortSignal): Promise<RetroCount> {
  return get<RetroCount>("/retro/count", signal);
}

/** Fired after a rule decision lands, so the weekly card reads its count again. */
export const RULES_CHANGED = "harbard:rules-changed";

export type RuleAction = "rule-approve" | "rule-discard" | "rule-retire" | "memory-sink-allow";

/** `POST /api/v1/workspace/rule-*`: the operator's decision on a lesson or a rule. */
export async function postRule(action: RuleAction, body: Record<string, unknown>): Promise<Record<string, unknown>> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/workspace/${action}`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: JSON.stringify(body),
    });
  } catch (cause) {
    throw new WorkspaceRefused(t("serveUnreachable", { cause: String(cause) }));
  }
  const json = (await res.json().catch(() => null)) as Record<string, unknown> | null;
  if (!res.ok || !json) throw new WorkspaceRefused(String(json?.message ?? t("serveAnswered", { status: res.status })), res.status, String(json?.error ?? ""));
  if (typeof window !== "undefined") window.dispatchEvent(new Event(RULES_CHANGED));
  return json;
}

/** History: one page of the month's sessions and agents, newest first. */
export function fetchHistoryAgents(
  opts: { before?: string | null; channel?: string | null; q?: string | null; limit?: number } = {},
  signal?: AbortSignal,
): Promise<HistoryPage> {
  return get<HistoryPage>(withWorkspace(`/history/agents${historyQuery(opts)}`, currentWorkspace()), signal);
}

/** History: one agent's conversation, masked; 404 when the month has no such agent. */
export function fetchHistoryConversation(key: string, signal?: AbortSignal): Promise<HistoryConversation> {
  return get<HistoryConversation>(withWorkspace(`/history/agents/${encodeURIComponent(key)}`, currentWorkspace()), signal);
}

export function fetchSetup(signal?: AbortSignal): Promise<SetupStatus> {
  return get<SetupStatus>("/setup", signal);
}

/** Wire selected harnesses via local serve (loopback-only). */
export async function postSetupInstall(
  targets: string[],
): Promise<SetupInstallResponse> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/setup/install`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: JSON.stringify({ targets }),
    });
  } catch (cause) {
    throw new ApiUnavailable(t("busUnreachable"), cause);
  }
  // 207 = partial success (some targets failed).
  if (!res.ok && res.status !== 207) {
    const detail = await res
      .json()
      .then((j: ApiError) => j.error)
      .catch(() => null);
    throw new ApiUnavailable(detail ?? t("setupInstallRefused", { status: res.status }));
  }
  return (await res.json()) as SetupInstallResponse;
}

/** Persist that first-run Setup finished (wired and/or skipped). */
export async function postSetupComplete(body: {
  skipped?: string[];
  wired?: string[];
}): Promise<{ ok: boolean; setup: SetupStatus }> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/setup/complete`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: JSON.stringify(body),
    });
  } catch (cause) {
    throw new ApiUnavailable(t("busUnreachable"), cause);
  }
  if (!res.ok) {
    const detail = await res
      .json()
      .then((j: ApiError) => j.error)
      .catch(() => null);
    throw new ApiUnavailable(detail ?? t("setupCompleteRefused", { status: res.status }));
  }
  return (await res.json()) as { ok: boolean; setup: SetupStatus };
}

/** `GET /api/v1/update`: the installed version, the newer one, and a run (docs/SPEC-UPDATES.md). */
export function fetchUpdate(signal?: AbortSignal): Promise<UpdateView> {
  return get<UpdateView>("/update", signal);
}

/** A refused update write: serve's code (`busy`, `dirty`…) and its message. */
export class UpdateRefused extends Error {
  constructor(
    message: string,
    readonly code?: string,
    readonly status?: number,
  ) {
    super(message);
    this.name = "UpdateRefused";
  }
}

async function updateAnswer(res: Response): Promise<UpdateView> {
  // A refusal is serve's ApiError: `{ "error": "<code>", "message": "…" }`.
  const json = (await res.json().catch(() => null)) as (UpdateView & { error?: unknown; message?: unknown }) | null;
  if (!res.ok || !json || !("installed" in json)) {
    const code = typeof json?.error === "string" ? json.error : undefined;
    const message = typeof json?.message === "string" ? json.message : t("serveAnswered", { status: res.status });
    throw new UpdateRefused(message, code, res.status);
  }
  return json;
}

/** `POST /api/v1/update/check`: look for a newer version now (up to about 30 s). */
export async function postUpdateCheck(): Promise<UpdateView> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/update/check`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: "{}",
    });
  } catch (cause) {
    throw new UpdateRefused(t("harbardNotResponding", { cause: String(cause) }));
  }
  return updateAnswer(res);
}

/** `POST /api/v1/update/install`: start a run; `busy` while one goes on. */
export async function postUpdateInstall(): Promise<UpdateView> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/update/install`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: "{}",
    });
  } catch (cause) {
    throw new UpdateRefused(t("harbardNotResponding", { cause: String(cause) }));
  }
  return updateAnswer(res);
}

/** `POST /api/v1/update/prefs`: the channel and the background check. */
export async function postUpdatePrefs(prefs: { channel?: UpdateChannel; auto_check?: boolean }): Promise<UpdateView> {
  let res: Response;
  try {
    res = await fetch(`${API_V1}/update/prefs`, {
      method: "POST",
      headers: authHeaders({ "content-type": "application/json" }),
      body: JSON.stringify(prefs),
    });
  } catch (cause) {
    throw new UpdateRefused(t("harbardNotResponding", { cause: String(cause) }));
  }
  return updateAnswer(res);
}

export function fetchMeta(signal?: AbortSignal): Promise<MetaResponse> {
  return get<MetaResponse>("/meta", signal);
}

