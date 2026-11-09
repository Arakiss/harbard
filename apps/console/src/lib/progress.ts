/**
 * One agent, step by step: the hover card's "Últimos pasos" and "Progreso",
 * as serve folds them in the team projection (`harbard_core::progress`).
 *
 * serve sends safe labels (a basename, a host, a program and its
 * subcommand), the counts of a test run and never its output, and whether a
 * command was a `git commit`, never its message.
 */

import { type TranslationKey, createT } from "@/i18n";
import type { ActivityState } from "./activity";

const t = createT("progress");

export interface TestCounts {
  passed: number;
  failed: number;
  skipped: number;
}

export interface TestRun extends TestCounts {
  /** No failure counted and the command did not fail. */
  ok: boolean;
  at_ms: number;
}

export type StepStatus = "running" | "ok" | "error" | "stopped";

export type StepState = Extract<ActivityState, "thinking" | "running" | "testing" | "editing" | "reading" | "web" | "delegating">;

export interface StepRow {
  /** The face family of the call; `thinking` for any other tool. */
  state: StepState;
  /** Claude Code's own tool name; null for an MCP or unknown tool. */
  tool: string | null;
  detail: string | null;
  started_ms: number;
  ended_ms: number | null;
  status: StepStatus;
  test: TestCounts | null;
  /** A `git commit` run (absent from an older serve). */
  commit?: boolean;
}

export interface Progress {
  tool_calls: number;
  commits: number;
  test: TestRun | null;
  /** Newest first, at most five. */
  steps: StepRow[];
}

/** Passed and failed, plus skipped when there are any. */
export function testSummary(run: TestCounts): string {
  const parts = [
    run.passed === 1 ? t("testsPassOne", { count: run.passed }) : t("testsPassMany", { count: run.passed }),
    run.failed === 1 ? t("testsFailOne", { count: run.failed }) : t("testsFailMany", { count: run.failed }),
  ];
  if (run.skipped > 0) parts.push(run.skipped === 1 ? t("testsSkippedOne", { count: run.skipped }) : t("testsSkippedMany", { count: run.skipped }));
  return parts.join(", ");
}

const OTHER_TOOL: Record<string, TranslationKey<"progress">> = {
  SendMessage: "toolSendMessage",
  TodoWrite: "toolTodoWrite",
  TaskCreate: "toolTaskCreate",
  TaskUpdate: "toolTaskUpdate",
  TaskList: "toolTaskList",
  Skill: "toolSkill",
  ToolSearch: "toolToolSearch",
  BashOutput: "toolBashOutput",
  TaskOutput: "toolTaskOutput",
  KillShell: "toolKillShell",
  TaskStop: "toolTaskStop",
  Monitor: "toolMonitor",
  AskUserQuestion: "toolAskUserQuestion",
  ExitPlanMode: "toolExitPlanMode",
  EnterWorktree: "toolEnterWorktree",
  LSP: "toolLsp",
};

/** What a step looks like: the icon family the card draws for it. */
export type StepKind = "editing" | "reading" | "searching" | "running" | "testing" | "commit" | "web" | "delegating" | "message" | "thinking";

export function stepKind(step: Pick<StepRow, "state" | "tool" | "commit">): StepKind {
  if (step.commit) return "commit";
  if (step.state === "reading" && (step.tool === "Grep" || step.tool === "Glob")) return "searching";
  if (step.state === "thinking") return step.tool === "SendMessage" ? "message" : "thinking";
  return step.state;
}

/**
 * A step in words, with the safe label when serve gave one. The time beside
 * it says when.
 */
export function stepText(step: Pick<StepRow, "state" | "tool" | "detail" | "commit">): string {
  const d = step.detail;
  switch (stepKind(step)) {
    case "commit":
      return t("stepCommit");
    case "running":
      return d ? t("stepRunningDetail", { detail: d }) : t("stepRunning");
    case "testing":
      return d ? t("stepTestsDetail", { detail: d }) : t("stepTests");
    case "editing":
      return d ? t("stepEditingDetail", { detail: d }) : t("stepEditing");
    case "searching":
      return d ? t("stepSearchingDetail", { detail: d }) : t("stepSearching");
    case "reading":
      return d ? t("stepReadingDetail", { detail: d }) : t("stepReading");
    case "web":
      if (step.tool === "WebSearch") return t("stepWebSearch");
      return d ? t("stepOpeningDetail", { detail: d }) : t("stepOpening");
    case "delegating":
      return t("stepDelegating");
    case "message":
      return t("toolSendMessage");
    default: {
      const key = step.tool ? OTHER_TOOL[step.tool] : undefined;
      return key ? t(key) : t("stepTool");
    }
  }
}

/** How long ago, down to seconds for what just happened. */
export function relative(ms: number, now: number): string {
  const s = Math.max(0, Math.floor((now - ms) / 1000));
  if (s < 5) return t("relativeNow");
  if (s < 60) return t("relativeSeconds", { count: s });
  const m = Math.floor(s / 60);
  if (m < 60) return t("relativeMinutes", { count: m });
  return t("relativeHours", { count: Math.floor(m / 60) });
}

/** "12 s", "1 min 12 s", "2 h 14 min": a running clock with its seconds. */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return s % 60 ? `${m} min ${s % 60} s` : `${m} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
}
