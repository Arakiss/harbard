/**
 * The Retro view's model (`GET /api/v1/retro`, harbard_core::retro): the
 * numbers as the operator reads them, and the sparkline geometry.
 *
 * Every tile is one number with a sentence-case label; a tile with a daily
 * series carries a 7-point sparkline (dataviz: de-emphasis line, the last
 * day in the accent, a table view beside it).
 */

import { type TranslationKey, createT, intlTag } from "@/i18n";
import { type ServeParams, serveText } from "./serve-text";

const t = createT("retro");

export type FindingClass = { class: string; count: number; branches: string[]; repeats: boolean; days: string[] };
export type DayPoint = { day: string; findings: number; not_understood: number; stalls: number; unanswered: number };

export type Lesson = { key: string; title: string; evidence: string; evidence_code?: string; evidence_params?: ServeParams; rule: string; count: number; dates: string[] };
export type ActiveRule = { id: string; key: string; title: string; text: string; approved_ms: number; edited: boolean; memory_sink: boolean };
export type RetiredRule = { id: string; key: string; text: string; retired_ms: number };

export type Retro = {
  v: number;
  now_ms: number;
  since_ms: number;
  scope: "week" | "team";
  lead: string | null;
  agents: number;
  delivery: {
    brief_to_install_min: { agent: string; minutes: number }[];
    brief_to_install_p50_min: number | null;
    review_rounds: Record<string, number>;
    findings: FindingClass[];
  };
  messages: { not_understood: number; jargon_chips: number; what_does_it_mean: number };
  waiting: {
    unanswered: { note: string; at_ms: number; open_min: number }[];
    answered: number;
    answer_p50_min: number | null;
    idle_min: number;
    stragglers: string[];
  };
  harness: {
    stalls: { agent: string; tool: string; pattern: string; at_ms: number; minutes: number }[];
    stall_min: number;
    two_writers: number;
    repeats: number;
  };
  names: { names_down: number; renamed?: number; generated: string[]; english_descriptions: string[] };
  feedback: Record<string, Record<string, number>>;
  daily: DayPoint[];
  lessons: Lesson[];
};

export type RetroBody = {
  v: number;
  retro: Retro;
  candidates: Lesson[];
  rules: ActiveRule[];
  retired: RetiredRule[];
  finished_teams: string[];
  memory_sink?: { configured: boolean; allowed?: boolean };
  /** When serve computed it: the week is cached for an hour. */
  computed_ms?: number;
  /** "Was it you?": open, and answered in the last days. */
  checks?: import("./checks").DeskCheck[];
};

export type MemorySinkOutcome = {
  saved: boolean;
  code: string | null;
  note: string | null;
  params?: Record<string, string> | null;
};

/**
 * By code so the operator reads it in their language; a code from a newer
 * serve than this console falls back to the English that serve sent.
 */
export function memorySinkNote(outcome: MemorySinkOutcome): string | null {
  const params = outcome.params ?? {};
  switch (outcome.code) {
    case "memory_sink_disabled":
      return t("memorySinkNoteDisabled");
    case "memory_sink_not_configured":
      return t("memorySinkNoteNotConfigured");
    case "memory_sink_locked":
      return t("memorySinkNoteLocked");
    case "memory_sink_timed_out":
      return t("memorySinkNoteTimedOut");
    case "memory_sink_refused":
      return t("memorySinkNoteRefused", { reason: params.reason ?? "" });
    case "memory_sink_failed":
      return t("memorySinkNoteFailed", { detail: params.detail ?? "" });
    case "memory_sink_bad_answer":
      return t("memorySinkNoteBadAnswer");
    default:
      return outcome.note;
  }
}

/**
 * `GET /api/v1/retro/count`: Activity's weekly card. Serve answers from the
 * retro it already holds; `candidates` is null until the first pass lands.
 */
export type RetroCount = {
  v: number;
  candidates: number | null;
  computed_ms: number | null;
  computing: boolean;
};

/** While the first pass has not landed, the card asks again this soon. */
export const RETRO_COUNT_WAIT_MS = 5_000;
/** Once it has, this often: the week moves by the hour. */
export const RETRO_COUNT_READY_MS = 300_000;

export function retroCountDelay(count: RetroCount | null): number {
  return count && count.candidates !== null ? RETRO_COUNT_READY_MS : RETRO_COUNT_WAIT_MS;
}

export type Tile = {
  id: string;
  label: string;
  value: string;
  detail?: string;
  /** One value per day of the window, oldest first. */
  series?: number[];
};

const CLASS_COUNT: Record<string, [TranslationKey<"retro">, TranslationKey<"retro">]> = {
  secret_leak: ["classSecretLeakOne", "classSecretLeakMany"],
  file_race: ["classFileRaceOne", "classFileRaceMany"],
  host_check: ["classHostCheckOne", "classHostCheckMany"],
  wrong_state: ["classWrongStateOne", "classWrongStateMany"],
  ui: ["classUiOne", "classUiMany"],
  other: ["classOtherOne", "classOtherMany"],
};

function classCount(findingClass: string, count: number): string {
  const [one, many] = CLASS_COUNT[findingClass] ?? ["classFindingOne", "classFindingMany"];
  return t(count === 1 ? one : many, { count });
}

function minutes(m: number | null): string {
  if (m === null) return t("noData");
  if (m < 120) return `${m} min`;
  return `${(Math.round(m / 6) / 10).toLocaleString(intlTag())} h`;
}

export function tiles(r: Retro): Tile[] {
  const findings = r.delivery.findings.reduce((s, f) => s + f.count, 0);
  const rounds = Object.values(r.delivery.review_rounds).reduce((s, v) => s + v, 0);
  const day = (k: keyof Omit<DayPoint, "day">) => r.daily.map((d) => d[k]);
  const branches = Object.keys(r.delivery.review_rounds).length;
  const asked = r.messages.what_does_it_mean;
  const described = r.names.english_descriptions.length;
  return [
    {
      id: "findings",
      label: t("tileFindings"),
      value: String(findings),
      detail: r.delivery.findings
        .filter((f) => f.count > 0)
        .map((f) => classCount(f.class, f.count))
        .join(", ") || undefined,
      series: day("findings"),
    },
    {
      id: "repeated",
      label: t("tileRepeated"),
      value: String(r.delivery.findings.filter((f) => f.repeats).length),
    },
    { id: "rounds", label: t("tileRounds"), value: String(rounds), detail: branches ? (branches === 1 ? t("branchesOne", { count: branches }) : t("branchesMany", { count: branches })) : undefined },
    { id: "install", label: t("tileInstall"), value: minutes(r.delivery.brief_to_install_p50_min), detail: t("tileMedian") },
    { id: "unclear", label: t("tileUnclear"), value: String(r.messages.not_understood), detail: asked ? (asked === 1 ? t("whatDoesItMeanOne", { count: asked }) : t("whatDoesItMeanMany", { count: asked })) : undefined, series: day("not_understood") },
    { id: "unanswered", label: t("tileUnanswered"), value: String(r.waiting.unanswered.length), detail: r.waiting.answer_p50_min !== null ? t("tileUnansweredDetail", { time: minutes(r.waiting.answer_p50_min) }) : undefined, series: day("unanswered") },
    { id: "idle", label: t("tileIdle"), value: String(r.waiting.idle_min), detail: r.waiting.stragglers.length ? t("tileIdleDetail", { count: r.waiting.stragglers.length }) : undefined },
    { id: "stalls", label: t("tileStalls"), value: String(r.harness.stalls.length), detail: r.harness.stall_min ? t("tileStallsDetail", { minutes: r.harness.stall_min }) : undefined, series: day("stalls") },
    { id: "two_writers", label: t("tileTwoWriters"), value: String(r.harness.two_writers) },
    { id: "names", label: t("tileNames"), value: String(r.names.names_down), detail: described ? (described === 1 ? t("englishDescriptionsOne", { count: described }) : t("englishDescriptionsMany", { count: described })) : undefined },
  ];
}

/** The sparkline's path in a `w`×`h` box with a 1px inset; zero sits on the bottom. */
export function sparkPath(values: number[], w: number, h: number): string {
  if (values.length === 0) return "";
  const max = Math.max(...values, 0);
  const x = (i: number) => (values.length === 1 ? w / 2 : 1 + (i * (w - 2)) / (values.length - 1));
  const y = (v: number) => (max === 0 ? h - 1 : h - 1 - (v / max) * (h - 2));
  return values.map((v, i) => `${i === 0 ? "M" : "L"}${Math.round(x(i))} ${Math.round(y(v))}`).join(" ");
}

const FEEDBACK_KIND: Record<string, TranslationKey<"retro">> = {
  message: "feedbackMessage",
  delivery: "feedbackDelivery",
  needs_you: "feedbackNeedsYou",
  robot_state: "feedbackRobotState",
  dock: "feedbackDock",
};

/** A feedback kind by its id; an id this build does not know shows as it came. */
export function feedbackLabel(kind: string): string {
  const key = FEEDBACK_KIND[kind];
  return key ? t(key) : kind;
}

const FEEDBACK_VALUE: Record<string, TranslationKey<"retro">> = {
  up: "valueUp",
  down: "valueDown",
  unclear: "valueUnclear",
  yes: "valueYes",
  no: "valueNo",
  missing: "valueMissing",
  not_needed: "valueNotNeeded",
  wrong: "valueWrong",
  annoying: "valueAnnoying",
};

export function feedbackValueLabel(value: string): string {
  const key = FEEDBACK_VALUE[value];
  return key ? t(key) : value;
}

/** serve titles each lesson in English, so the title is looked up by the lesson's key. */
const LESSON_TITLE: Record<string, TranslationKey<"retro">> = {
  shared_security_primitive: "lessonSharedSecurity",
  review_repeat: "lessonReviewRepeat",
  permission_stalls: "lessonPermissionStalls",
  rm_variable: "lessonRmVariable",
  shell_wrapper: "lessonShellWrapper",
  unanswered_operator: "lessonUnanswered",
  plain_messages: "lessonPlainMessages",
  descriptions_in_spanish: "lessonDescriptions",
  readable_names: "lessonReadableNames",
  background_wait: "lessonBackgroundWait",
  needs_you_noise: "lessonNeedsYouNoise",
  delivery_quality: "lessonDeliveryQuality",
};

export function lessonTitle(lesson: Pick<Lesson, "key" | "title">): string {
  const key = LESSON_TITLE[lesson.key];
  return key ? t(key) : lesson.title;
}

/** The noun each counted param of a lesson's evidence is worded with. */
const EVIDENCE_NOUN: Record<string, [TranslationKey<"retro">, TranslationKey<"retro">]> = {
  n_stall: ["nounStallOne", "nounStallMany"],
  n_note: ["nounNoteOne", "nounNoteMany"],
  n_unclear: ["nounMessageOne", "nounMessageMany"],
  n_asked: ["nounNoteOne", "nounNoteMany"],
  n_jargon: ["nounMessageOne", "nounMessageMany"],
  n_down: ["nounNameOne", "nounNameMany"],
  n_renamed: ["nounRenamedOne", "nounRenamedMany"],
  n_generated: ["nounNameOne", "nounNameMany"],
  n_alert: ["nounAlertOne", "nounAlertMany"],
  n_delivery: ["nounDeliveryOne", "nounDeliveryMany"],
};

/** The findings of a repeated-class lesson, one clause per class. */
function evidenceItems(params: ServeParams, count: number): string {
  const items: string[] = [];
  for (let i = 0; i < count; i++) {
    const classes = classCount(String(params[`class_${i}`] ?? ""), Number(params[`count_${i}`] ?? 0));
    const total = Number(params[`branches_${i}`] ?? 0);
    const branches = t(total === 1 ? "branchesOne" : "branchesMany", { count: total });
    const names = params[`names_${i}`];
    items.push(names === undefined ? t("evidenceItem", { classes, branches }) : t("evidenceItemNamed", { classes, branches, names }));
  }
  return items.join("; ");
}

function evidenceVars(params: ServeParams): ServeParams {
  const vars: ServeParams = { ...params };
  for (const [name, value] of Object.entries(params)) {
    const noun = EVIDENCE_NOUN[name];
    if (noun && typeof value === "number") vars[name.slice(2)] = t(value === 1 ? noun[0] : noun[1], { count: value });
  }
  if (typeof params.items === "number") vars.items = evidenceItems(params, params.items);
  return vars;
}

/** A lesson's evidence: serve's counts, in this language's words. */
export function lessonEvidence(lesson: Pick<Lesson, "evidence" | "evidence_code" | "evidence_params">): string {
  return serveText(lesson.evidence_code, lesson.evidence_params ? evidenceVars(lesson.evidence_params) : undefined, lesson.evidence);
}
