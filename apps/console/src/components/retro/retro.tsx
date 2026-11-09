"use client";

/**
 * Retro: the week's (or one finished team's) numbers, the rules the retro
 * suggests, and the rules in force. The operator approves a rule as written, edits
 * it first, or discards it; an active rule can be retired. Every decision is
 * a `rule` event on the bus; an active rule reaches every session in the
 * protocol card and the SessionStart brief.
 *
 * Rule texts and evidence render as plain text only.
 */

import type { Route } from "next";
import * as React from "react";
import Link from "next/link";
import { BookCheck, Lightbulb } from "lucide-react";
import { createT, intlTag, richText } from "@/i18n";
import { RULES_CHANGED, fetchRetro, fetchRetroCount, postRule, type RuleAction } from "@/lib/api";
import { invisibleRefusal } from "@/lib/invisible";
import { feedbackLabel, feedbackValueLabel, lessonEvidence, lessonTitle, memorySinkNote, retroCountDelay, sparkPath, tiles, type ActiveRule, type Lesson, type MemorySinkOutcome, type RetroBody, type Tile } from "@/lib/retro";
import { checkLine, type DeskCheck } from "@/lib/checks";

import { useFeedback } from "@/components/feedback/feedback";
import { operatorTimeZone } from "@/lib/time-zone";

const t = createT("retro");

export type RetroScope = { kind: "week" } | { kind: "team"; lead: string };
type Act = (action: RuleAction, body: Record<string, unknown>) => Promise<unknown>;

/** "Was it you?": the open checks and those answered in the last days. */
function RecentChecks({ checks }: { checks?: DeskCheck[] }) {
  if (!checks || checks.length === 0) return null;
  return (
    <section aria-label={t("recentAria")} className="hb-retro-sec" data-recent-checks="">
      <h2 className="hb-retro-h">{t("recentTitle")}</h2>
      <p className="hb-retro-sub">{t("recentHint")}</p>
      <ul className="hb-checks">
        {checks.map((c) => (
          <li key={c.id}>{checkLine(c)}</li>
        ))}
      </ul>
    </section>
  );
}

const SPARK_W = 112;
const SPARK_H = 28;

function Spark({ id, tile, days }: { id: string; tile: Tile; days: string[] }) {
  const values = tile.series ?? [];
  if (values.length === 0) return null;
  const d = sparkPath(values, SPARK_W, SPARK_H);
  const last = d.split(" L").pop()?.replace("M", "").split(" ").map(Number) ?? [0, 0];
  const step = values.length > 1 ? (SPARK_W - 2) / (values.length - 1) : SPARK_W;
  return (
    <svg className="hb-spark" data-spark={id} width={SPARK_W} height={SPARK_H} viewBox={`0 0 ${SPARK_W} ${SPARK_H}`} role="img" aria-label={t("sparkAria", { label: tile.label, values: values.join(", ") })}>
      <path d={d} className="hb-spark-line" />
      <circle cx={last[0]} cy={last[1]} r={4} className="hb-spark-now" />
      {values.map((v, i) => (
        <rect key={i} x={Math.max(0, 1 + i * step - step / 2)} y={0} width={step} height={SPARK_H} className="hb-spark-hit">
          <title>{`${days[i] ?? ""}: ${v}`}</title>
        </rect>
      ))}
    </svg>
  );
}

function LessonCard({ lesson, act, memorySink }: { lesson: Lesson; act: Act; memorySink: boolean }) {
  const [editing, setEditing] = React.useState(false);
  const [draft, setDraft] = React.useState(lesson.rule);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [tick, setTick] = React.useState(false);
  // The operator's own words take a fresh confirmation: serve answers a one-time
  // code and the exact text it will keep, shown back here.
  const [confirm, setConfirm] = React.useState<{ id: string; text: string; body: Record<string, unknown> } | null>(null);
  const run = (action: RuleAction, body: Record<string, unknown>) => {
    setBusy(true);
    setError(null);
    act(action, body)
      .then((out) => {
        const asked = (out as { confirm?: { id: string; shown?: { text?: string } } } | null)?.confirm;
        setConfirm(asked ? { id: asked.id, text: asked.shown?.text ?? "", body } : null);
      })
      .catch((e: unknown) =>
        setError(
          (e instanceof Error && invisibleRefusal(e.message)) ||
            (e instanceof Error && e.message.includes("does not propose") ? t("errorRuleGone") : null) ||
            t("errorSaveFailed"),
        ),
      )
      .finally(() => setBusy(false));
  };
  const edited = editing && draft.trim() !== lesson.rule;
  const title = lessonTitle(lesson);
  return (
    <article className="hb-lesson" data-lesson={lesson.key}>
      <header className="hb-lesson-head">
        <Lightbulb className="size-4" aria-hidden />
        <h3>{title}</h3>
        <span className="hb-lesson-dates">{lesson.dates.join(", ")}</span>
      </header>
      <p className="hb-lesson-evidence">{lessonEvidence(lesson)}</p>
      {editing ? (
        <textarea
          className="hb-lesson-edit"
          aria-label={t("ruleTextAria", { title })}
          value={draft}
          maxLength={240}
          rows={3}
          onChange={(e) => setDraft(e.target.value)}
        />
      ) : (
        <blockquote className="hb-lesson-rule">{lesson.rule}</blockquote>
      )}
      <div className="hb-lesson-actions">
        <button
          type="button"
          className="hb-btn hb-btn-primary"
          disabled={busy || (editing && !draft.trim())}
          onClick={() =>
            run("rule-approve", { key: lesson.key, title: lesson.title, text: editing ? draft : lesson.rule, edited, evidence: lesson.evidence, memory_sink: memorySink && tick })
          }
        >
          {t("approve")}
        </button>
        {editing ? null : (
          <button type="button" className="hb-btn" disabled={busy} onClick={() => setEditing(true)}>
            {t("edit")}
          </button>
        )}
        <button type="button" className="hb-btn" disabled={busy} onClick={() => run("rule-discard", { key: lesson.key })}>
          {t("discard")}
        </button>
        {memorySink ? (
          <label className="hb-lesson-tick">
            <input type="checkbox" checked={tick} onChange={(e) => setTick(e.target.checked)} />
            {t("saveMemorySink")}
          </label>
        ) : null}
        {error ? <span className="hb-error">{error}</span> : null}
      </div>
      {confirm ? (
        <div className="hb-lesson-confirm" role="alertdialog" aria-label={t("confirmAria")}>
          <p>{richText(t("confirmBody"), { text: <q>{confirm.text}</q> })}</p>
          <button type="button" className="hb-btn hb-btn-primary" disabled={busy} onClick={() => run("rule-approve", { ...confirm.body, confirm: confirm.id })}>
            {t("confirm")}
          </button>
          <button type="button" className="hb-btn" disabled={busy} onClick={() => setConfirm(null)}>
            {t("cancel")}
          </button>
        </div>
      ) : null}
    </article>
  );
}

function RuleRow({ rule, act }: { rule: ActiveRule; act: Act }) {
  const [busy, setBusy] = React.useState(false);
  return (
    <li className="hb-rule" data-rule={rule.key}>
      <div className="min-w-0">
        <div className="hb-rule-title">{lessonTitle(rule)}</div>
        <div className="hb-rule-text">{rule.text}</div>
        <div className="hb-rule-meta">
          {t("ruleApproved", { date: new Date(rule.approved_ms).toLocaleDateString(intlTag(), { timeZone: operatorTimeZone() }) })}
          {rule.edited ? ` · ${t("ruleEdited")}` : ""}
          {rule.memory_sink ? ` · ${t("ruleSavedMemorySink")}` : ""}
        </div>
      </div>
      <button
        type="button"
        className="hb-btn"
        disabled={busy}
        onClick={() => {
          setBusy(true);
          act("rule-retire", { id: rule.id }).finally(() => setBusy(false));
        }}
      >
        {t("retire")}
      </button>
    </li>
  );
}

/** The view, from plain props (rendered on its own in tests). */
export function RetroView({
  body,
  scope,
  onScope,
  act,
  notice,
}: {
  body: RetroBody;
  scope: RetroScope;
  onScope: (s: RetroScope) => void;
  act: Act;
  notice?: string | null;
}) {
  const r = body.retro;
  const tileList = tiles(r);
  const days = r.daily.map((d) => d.day);
  const feedbackRows = Object.entries(r.feedback);
  return (
    <div className="hb-retro">
      <div className="hb-tabs" role="tablist" aria-label={t("scopeAria")}>
        <button type="button" role="tab" className="hb-tab" aria-selected={scope.kind === "week"} onClick={() => onScope({ kind: "week" })}>
          {t("thisWeek")}
        </button>
        {body.finished_teams.map((lead) => (
          <button
            key={lead}
            type="button"
            role="tab"
            className="hb-tab"
            aria-selected={scope.kind === "team" && scope.lead === lead}
            onClick={() => onScope({ kind: "team", lead })}
          >
            {t("finishedTeam", { id: lead.slice(0, 8) })}
          </button>
        ))}
      </div>
      <p className="hb-retro-sub">
        {t("rangeLine", { agents: r.agents === 1 ? t("agentsOne", { count: r.agents }) : t("agentsMany", { count: r.agents }), from: fmtDay(r.since_ms), to: fmtDay(r.now_ms) })}
      </p>

      <section aria-label={t("figuresAria")} className="hb-tiles">
        {tileList.map((tile) => (
          <div key={tile.id} className="hb-tile" data-tile={tile.id}>
            <div className="hb-tile-label">{tile.label}</div>
            <div className="hb-tile-row">
              <span className="hb-tile-value">{tile.value}</span>
              <Spark id={tile.id} tile={tile} days={days} />
            </div>
            {tile.detail ? <div className="hb-tile-detail">{tile.detail}</div> : null}
          </div>
        ))}
      </section>
      <details className="hb-retro-table">
        <summary>{t("dataByDay")}</summary>
        <table>
          <thead>
            <tr>
              <th scope="col">{t("colDay")}</th>
              <th scope="col">{t("colFindings")}</th>
              <th scope="col">{t("colUnclear")}</th>
              <th scope="col">{t("colStalls")}</th>
              <th scope="col">{t("colUnanswered")}</th>
            </tr>
          </thead>
          <tbody>
            {r.daily.map((d) => (
              <tr key={d.day}>
                <th scope="row">{d.day}</th>
                <td>{d.findings}</td>
                <td>{d.not_understood}</td>
                <td>{d.stalls}</td>
                <td>{d.unanswered}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {feedbackRows.length ? (
          <table>
            <caption>{t("yourTaps")}</caption>
            <tbody>
              {feedbackRows.map(([kind, values]) => (
                <tr key={kind}>
                  <th scope="row">{feedbackLabel(kind)}</th>
                  <td>
                    {Object.entries(values)
                      .map(([v, c]) => `${feedbackValueLabel(v)}: ${c}`)
                      .join(" · ")}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : null}
      </details>

      <section aria-label={t("lessonsAria")} className="hb-retro-sec">
        <h2 className="hb-retro-h">{t("proposedTitle")}</h2>
        {body.candidates.length === 0 ? <p className="hb-retro-none">{t("noCandidates")}</p> : null}
        {notice ? (
          <p className="hb-retro-note" role="status" data-retro-notice="">
            {notice}
          </p>
        ) : null}
        {body.candidates.map((l) => (
          <LessonCard key={l.key} lesson={l} act={act} memorySink={body.memory_sink?.configured === true && body.memory_sink.allowed === true} />
        ))}
      </section>

      <RecentChecks checks={body.checks} />

      <section aria-label={t("activeTitle")} className="hb-retro-sec">
        <h2 className="hb-retro-h">
          <BookCheck className="size-4" aria-hidden /> {t("activeTitle")}
        </h2>
        <p className="hb-retro-sub">{t("activeHint")}</p>
        {body.rules.length === 0 ? <p className="hb-retro-none">{t("noRules")}</p> : null}
        <ul className="hb-rules">
          {body.rules.map((rule) => (
            <RuleRow key={rule.id} rule={rule} act={act} />
          ))}
        </ul>
      </section>
    </div>
  );
}

function fmtDay(ms: number): string {
  return new Date(ms).toLocaleDateString(intlTag(), { day: "numeric", month: "long", timeZone: operatorTimeZone() });
}

export function RetroScreen() {
  const [scope, setScope] = React.useState<RetroScope>({ kind: "week" });
  const [body, setBody] = React.useState<RetroBody | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const load = React.useCallback(
    (s: RetroScope, signal?: AbortSignal) =>
      fetchRetro(s.kind === "team" ? s.lead : null, signal)
        .then((b) => {
          setBody(b);
          setError(null);
        })
        .catch((e) => {
          if (!(e instanceof DOMException && e.name === "AbortError")) setError(t("notResponding"));
        }),
    [],
  );
  React.useEffect(() => {
    const c = new AbortController();
    void load(scope, c.signal);
    return () => c.abort();
  }, [scope, load]);
  const [notice, setNotice] = React.useState<string | null>(null);
  const act: Act = async (action, b) => {
    const body = action === "rule-approve" && scope.kind === "team" ? { ...b, lead: scope.lead } : b;
    const out = (await postRule(action, body)) as { confirm?: unknown; memory_sink?: MemorySinkOutcome | null };
    // Asked to confirm: nothing is approved yet.
    if (out.confirm) return out;
    if (action === "rule-approve") {
      setNotice(out.memory_sink ? (out.memory_sink.saved ? t("noticeApprovedMemorySink") : memorySinkNote(out.memory_sink)) : t("noticeApproved"));
    } else {
      setNotice(action === "rule-retire" ? t("noticeRetired") : t("noticeDiscarded"));
    }
    await load(scope);
    return out;
  };
  return (
    <main className="hb-main" aria-label={t("title")}>
      <header className="hb-head">
        <div className="hb-head-title">
          <Lightbulb className="size-5" aria-hidden /> {t("title")}
        </div>
        <span className="hb-topic">{t("topic")}</span>
      </header>
      {body ? (
        <RetroView body={body} scope={scope} onScope={setScope} act={act} notice={notice} />
      ) : (
        <div className="hb-empty">{error ?? t("computing")}</div>
      )}
    </main>
  );
}

/** Activity's weekly card: shown while the week has rules to decide. */
export function WeeklyRetroCardView({ candidates }: { candidates: number }) {
  if (candidates <= 0) return null;
  return (
    <Link href={"/retro" as Route} className="hb-retro-card" data-retro-card="">
      <Lightbulb className="size-4" aria-hidden />
      <span>
        <strong>{t("weeklyCardTitle")}</strong> · {candidates === 1 ? t("proposedRulesOne", { count: candidates }) : t("proposedRulesMany", { count: candidates })}
      </span>
    </Link>
  );
}

/**
 * Reads serve's cached count, never the week itself (AC1: the week took the
 * shell's reads with it for seconds). Asks again soon while serve's first
 * pass has not landed, every few minutes after, and at once after a rule
 * decision.
 */
export function WeeklyRetroCard() {
  const fb = useFeedback();
  const [count, setCount] = React.useState(0);
  React.useEffect(() => {
    const c = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = () => {
      clearTimeout(timer);
      fetchRetroCount(c.signal)
        .then((b) => {
          setCount(b.candidates ?? 0);
          timer = setTimeout(read, retroCountDelay(b));
        })
        .catch(() => {
          if (!c.signal.aborted) timer = setTimeout(read, retroCountDelay(null));
        });
    };
    read();
    window.addEventListener(RULES_CHANGED, read);
    return () => {
      c.abort();
      clearTimeout(timer);
      window.removeEventListener(RULES_CHANGED, read);
    };
  }, []);
  // A card that asks for a decision is a prompt: none in the quiet states.
  if (fb && !fb.allowed) return null;
  return <WeeklyRetroCardView candidates={count} />;
}
