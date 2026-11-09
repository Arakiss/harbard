/**
 * "Update harbard": what serve says about the installed version, the
 * newer one, and a run that installs it (docs/SPEC-UPDATES.md). This module
 * holds the wire types, the labels by id and the pure decisions the banner,
 * the menu, the dialog and Preferences share; it fetches nothing.
 */

import { type TranslationKey, createT, intlTag } from "@/i18n";
import { clock } from "./progress";
import { atDate, hace } from "./workspace";

const t = createT("update");

// ---------------------------------------------------------------------------
// Wire: GET /api/v1/update
// ---------------------------------------------------------------------------

export type UpdateChannel = "main" | "releases";
export type CheckStatus = "current" | "available" | "error";
export type SourceProblem = "no_checkout" | "dirty" | "not_on_main" | "diverged";
export type ChangeKind = "feat" | "fix" | "perf" | "refactor" | "docs" | "test" | "build" | "chore" | "other";
export type RunState = "running" | "done" | "failed";
export type StepState = "pending" | "running" | "done" | "failed" | "skipped";
export type StepId = "fetch" | "cli" | "hooks" | "serve" | "console" | "desktop";

export interface UpdateError {
  code: string;
  message: string;
}

export interface UpdateTarget {
  version: string;
  commit: string;
  tag: string | null;
  title: string | null;
  date_ms: number | null;
}

export interface UpdateChange {
  commit: string;
  kind: ChangeKind;
  scope: string | null;
  subject: string;
  date_ms: number | null;
}

export interface UpdateCheck {
  checked_ms: number;
  channel: UpdateChannel;
  status: CheckStatus;
  error: UpdateError | null;
  target: UpdateTarget | null;
  behind: number;
  notes: string | null;
  changes: UpdateChange[];
  changes_truncated: boolean;
}

export interface UpdateStep {
  id: StepId;
  state: StepState;
  started_ms: number | null;
  finished_ms: number | null;
  detail: string | null;
}

export interface UpdateRun {
  id: string;
  state: RunState;
  started_ms: number;
  finished_ms: number | null;
  from: string | null;
  to: string | null;
  error: UpdateError | null;
  steps: UpdateStep[];
  log_tail: string[];
  /** What this run brought, copied from the check when it started; absent from older serves. */
  notes?: string | null;
  changes?: UpdateChange[];
  changes_truncated?: boolean;
}

export interface UpdateView {
  installed: { version: string; commit: string | null };
  prefs: { channel: UpdateChannel; auto_check: boolean };
  source: { path: string | null; problem: SourceProblem | null };
  check: UpdateCheck | null;
  run: UpdateRun | null;
}

// ---------------------------------------------------------------------------
// Copy
// ---------------------------------------------------------------------------

export function channels(): { id: UpdateChannel; label: string; hint: string }[] {
  return [
    { id: "main", label: t("channelMainLabel"), hint: t("channelMainHint") },
    { id: "releases", label: t("channelReleasesLabel"), hint: t("channelReleasesHint") },
  ];
}

const STEP_LABEL: Record<StepId, TranslationKey<"update">> = {
  fetch: "stepFetch",
  cli: "stepCli",
  hooks: "stepHooks",
  serve: "stepServe",
  console: "stepConsole",
  desktop: "stepDesktop",
};

export function stepLabel(id: StepId): string {
  return t(STEP_LABEL[id]);
}

const STEP_ORDER: StepId[] = ["fetch", "cli", "hooks", "serve", "console", "desktop"];

const SOURCE_PROBLEM: Record<SourceProblem, TranslationKey<"update">> = {
  no_checkout: "problemNoCheckout",
  dirty: "problemDirty",
  not_on_main: "problemNotOnMain",
  diverged: "problemDiverged",
};

/** What a source problem means for the person, and what to do about it. */
export function sourceProblemText(problem: SourceProblem): string {
  return t(SOURCE_PROBLEM[problem]);
}

const ERROR_TEXT: Record<string, TranslationKey<"update">> = {
  fetch_failed: "errorFetchFailed",
  dirty: SOURCE_PROBLEM.dirty,
  not_on_main: SOURCE_PROBLEM.not_on_main,
  diverged: SOURCE_PROBLEM.diverged,
  no_checkout: SOURCE_PROBLEM.no_checkout,
  step_failed: "errorStepFailed",
  interrupted: "errorInterrupted",
  busy: "errorBusy",
  forbidden_origin: "errorLocalOnly",
  loopback_only: "errorLocalOnly",
  role_forbids: "errorRoleForbids",
};

/** A refusal or failure in plain words; an unknown code keeps serve's own message. */
export function errorText(error: UpdateError | null | undefined): string {
  if (!error) return "";
  const known = ERROR_TEXT[error.code];
  return known ? t(known) : error.message || t("errorFallback");
}

// ---------------------------------------------------------------------------
// Decisions
// ---------------------------------------------------------------------------

export function shortSha(commit: string | null | undefined): string {
  return (commit ?? "").slice(0, 7);
}

/** "1 change", "11 changes". */
export function changesCount(n: number): string {
  return n === 1 ? t("changesOne") : t("changesMany", { count: n });
}

/** The banner's line: the headline, and how many changes when known. */
export function bannerLine(check: UpdateCheck): string {
  return check.behind > 0 ? `${t("bannerTitle")} · ${changesCount(check.behind)}` : t("bannerTitle");
}

/** Preferences and About: where the installed version stands. */
export function statusSentence(view: UpdateView | null, now: number): string {
  if (!view) return t("statusNoServe");
  const check = view.check;
  if (!check) return t("statusNeverChecked");
  const when = hace(check.checked_ms, now);
  if (check.status === "error") return t("statusCheckFailed", { when, reason: errorText(check.error) });
  if (check.status === "available") return t("statusAvailableLine", { line: bannerLine(check), when });
  return t("statusUpToDate", { when });
}

export type ChipTone = "amber" | "red" | "green" | undefined;

/** The status chip's tone and word: new version, failed check, up to date, never checked. */
export function statusChip(check: UpdateCheck | null): { tone: ChipTone; text: string } {
  if (!check) return { tone: undefined, text: t("chipNotChecked") };
  if (check.status === "available") return { tone: "amber", text: t("statusAvailable") };
  if (check.status === "error") return { tone: "red", text: t("chipFailed") };
  return { tone: "green", text: t("statusCurrent") };
}

export type BannerMode = "available" | "running" | "done" | null;

/**
 * What the shell's banner shows. A run in progress, or one that finished
 * while this page runs an older build, always shows; a new version shows
 * until "Later" for that target.
 */
export function bannerMode(view: UpdateView | null, dismissed: string | null, pageCommit: string, restarting: boolean): BannerMode {
  if (restarting) return "running";
  if (!view) return null;
  if (view.run?.state === "running") return "running";
  if (view.run?.state === "done" && pageIsStale(view.run, pageCommit)) return "done";
  const check = view.check;
  if (check?.status !== "available" || !check.target) return null;
  return dismissed === check.target.commit ? null : "available";
}

/** The page was built before the run's target: it needs a reload to show it. */
export function pageIsStale(run: UpdateRun, pageCommit: string): boolean {
  if (run.state !== "done" || !run.to || !pageCommit) return false;
  const a = run.to.toLowerCase();
  const b = pageCommit.toLowerCase();
  return !(a.startsWith(b) || b.startsWith(a));
}

const DISMISS_KEY = "harbard.update.dismissed";
const ACK_KEY = "harbard.update.acked-run";

type KV = Pick<Storage, "getItem" | "setItem">;

export function readDismissed(store: KV | null): string | null {
  try {
    return store?.getItem(DISMISS_KEY) ?? null;
  } catch {
    return null;
  }
}

export function writeDismissed(store: KV | null, commit: string): void {
  try {
    store?.setItem(DISMISS_KEY, commit);
  } catch {
    // Private mode or a full store: the banner simply comes back.
  }
}

/** How recent a finished run must be for the toast after a reload. */
const TOAST_WINDOW_MS = 10 * 60_000;

/** A finished run this browser has not said "actualizado" for yet. */
export function toastFor(run: UpdateRun | null, now: number, acked: string | null): boolean {
  if (!run || run.state !== "done" || run.finished_ms === null) return false;
  if (now - run.finished_ms > TOAST_WINDOW_MS || now < run.finished_ms - 60_000) return false;
  return acked !== run.id;
}

export function readAck(store: KV | null): string | null {
  try {
    return store?.getItem(ACK_KEY) ?? null;
  } catch {
    return null;
  }
}

export function writeAck(store: KV | null, id: string): void {
  try {
    store?.setItem(ACK_KEY, id);
  } catch {
    // Nothing to keep: the toast may show once more.
  }
}

export type GroupId = "news" | "fixes" | "improvements" | "other";

const GROUP_LABEL: Record<GroupId, TranslationKey<"update">> = {
  news: "groupNew",
  fixes: "groupFixes",
  improvements: "groupImprovements",
  other: "groupOther",
};

function groupOf(kind: ChangeKind): GroupId {
  if (kind === "feat") return "news";
  if (kind === "fix") return "fixes";
  if (kind === "perf" || kind === "refactor") return "improvements";
  return "other";
}

/** The commits by group, in a fixed order, empty groups left out; order inside kept. */
export function groupChanges(changes: UpdateChange[]): { id: GroupId; label: string; changes: UpdateChange[] }[] {
  const order: GroupId[] = ["news", "fixes", "improvements", "other"];
  const by = new Map<GroupId, UpdateChange[]>(order.map((g) => [g, []]));
  for (const c of changes) by.get(groupOf(c.kind))!.push(c);
  return order.filter((g) => by.get(g)!.length > 0).map((g) => ({ id: g, label: t(GROUP_LABEL[g]), changes: by.get(g)! }));
}

/** A subject starts with a capital in the list, the way a changelog reads. */
export function sentenceCase(subject: string): string {
  const s = subject.trim();
  return s ? s[0]!.toLocaleUpperCase(intlTag()) + s.slice(1) : s;
}

/** "0.3.0-alpha.10", or "main" when the channel is main and the number did not move (the commit says which). */
export function targetName(check: UpdateCheck, installedVersion: string): string {
  const target = check.target;
  if (!target) return installedVersion;
  if (target.tag) return target.tag.replace(/^v/, "");
  return target.version !== installedVersion ? target.version : "main";
}

/** When the target was published, or null when serve does not know. */
export function targetWhen(check: UpdateCheck, now: number): string | null {
  const ms = check.target?.date_ms;
  return ms ? atDate(ms, now) : null;
}

/** The steps to draw: serve's, in the run's order, or every step pending before the first answer. */
export function runSteps(run: UpdateRun | null): UpdateStep[] {
  if (run?.steps.length) return run.steps;
  return STEP_ORDER.map((id) => ({ id, state: "pending" as StepState, started_ms: null, finished_ms: null, detail: null }));
}

/** "1 min 12 s" for a finished step, the running clock for one in progress, nothing otherwise. */
export function stepElapsed(step: UpdateStep, now: number): string | null {
  if (step.started_ms === null) return null;
  if (step.state === "skipped" || step.state === "pending") return null;
  return clock((step.finished_ms ?? now) - step.started_ms);
}

/** Why a step was skipped, by serve's code. */
const SKIP_TEXT: Record<string, TranslationKey<"update">> = {
  not_installed: "skipNotInstalled",
  not_open: "skipNotOpen",
};

/**
 * A step's detail as the row shows it: a skipped step's code in words
 * (beside the step, in place of its time); a failed step's own message as a
 * small mono line under its label (the headline is the run's error code).
 * Anything else stays in "See the log".
 */
export function stepDetail(step: UpdateStep): { text: string; where: "side" | "below" } | null {
  if (step.state === "skipped") {
    const reason = step.detail ? SKIP_TEXT[step.detail] : undefined;
    return { text: reason ? t(reason) : stepStateText("skipped"), where: "side" };
  }
  if (step.state === "failed" && step.detail) return { text: step.detail, where: "below" };
  return null;
}

/** "See what's new" after an update: what the finished run brought, when serve kept it. */
export function broughtBy(run: UpdateRun | null): { notes: string | null; changes: UpdateChange[]; truncated: boolean } | null {
  if (!run || run.state !== "done") return null;
  const notes = run.notes?.trim() || null;
  const changes = run.changes ?? [];
  if (!notes && changes.length === 0) return null;
  return { notes, changes, truncated: run.changes_truncated ?? false };
}

const STEP_STATE_TEXT: Record<StepState, TranslationKey<"update">> = {
  pending: "stepPending",
  running: "stepRunning",
  done: "stepDone",
  failed: "stepFailed",
  skipped: "stepSkipped",
};

/** The state as a word, for screen readers and the skipped row. */
export function stepStateText(state: StepState): string {
  return t(STEP_STATE_TEXT[state]);
}

/** How often the view is read while a run goes on, and otherwise. */
export const RUN_POLL_MS = 1_500;
export const IDLE_POLL_MS = 10 * 60_000;
