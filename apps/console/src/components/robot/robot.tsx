"use client";

import * as React from "react";
import {
  REACTION_MS,
  microState,
  nextReaction,
  spendReactions,
  type Activity,
  type Face,
  type Reaction,
  type ReactionKind,
  type Thresholds,
} from "@/lib/activity";
import type { dotTone } from "@/lib/workspace";
import { joinClock } from "./clock";
import { RobotFace } from "./face";
import { ROBOT_DIR, assetSize, bodySrcSet, glowVar, maskUrl, robotFor, type RobotIdentity } from "./identity";
import { ROBOT_MODELS } from "./robots";
import { useSeasonVariant } from "./season-robots";

function withPick(id: RobotIdentity, pick?: { model?: string; glow?: RobotIdentity["glow"]; tile?: number }): RobotIdentity {
  if (!pick) return id;
  return {
    ...id,
    model: ROBOT_MODELS.find((m) => m.id === pick.model) ?? id.model,
    glow: pick.glow ?? id.glow,
    tile: pick.tile ?? id.tile,
  };
}

/** At this size and below, the tiny face: bolder eyes, no mouth or extras. */
const TINY_MAX = 40;
/** Below this a breath is under a device pixel at 2x, and is not drawn. */
const BREATHE_MIN = 56;

/** Every reaction that played on this page, by `key:kind:at` (shared by all instances). */
const PLAYED = new Map<string, number>();

/**
 * One agent's robot: its tile, its painted body and a live face clipped to
 * the model's screen. The model, light and tile come from the name; the face
 * from `activity` and its microstate; a reaction plays once when serve
 * reports it, in every view that shows this agent at that moment.
 */
export function Robot({
  name,
  size = 36,
  activity,
  thresholds,
  now,
  reactions,
  agentKey,
  baseline = Number.POSITIVE_INFINITY,
  reduce = false,
  dot,
  face: forcedFace,
  pick,
  className,
}: {
  name: string;
  size?: number;
  activity?: Pick<Activity, "state" | "since_ms"> | null;
  thresholds?: Thresholds | null;
  now?: number;
  reactions?: readonly Reaction[];
  agentKey?: string;
  /** Reactions older than this already happened before the page loaded. */
  baseline?: number;
  reduce?: boolean;
  /** The presence dot's tone, or none. */
  dot?: ReturnType<typeof dotTone> | null;
  /** Draw this face whatever the activity says (sheets, tests). */
  face?: Face;
  /** Another model, light or tile than the name's (the comparison sheet). */
  pick?: { model?: string; glow?: RobotIdentity["glow"]; tile?: number };
  className?: string;
}) {
  const id = withPick(robotFor(name), pick);
  // The season's variant of this model, when there is one: body, mask and screen.
  const variant = useSeasonVariant(id.model.id);
  const model = variant?.model ?? id.model;
  const dir = variant ? `${ROBOT_DIR}/${variant.dir}` : ROBOT_DIR;
  const micro = activity && thresholds && now !== undefined ? microState(activity, now, thresholds) : null;
  const face: Face = forcedFace ?? micro?.face ?? (activity?.state === "testing" || activity?.state === "editing" ? "running" : ((activity?.state as Face) ?? "normal"));
  const amber = face === "blocked" || face === "waiting";
  const tiny = size <= TINY_MAX;
  const reaction = useReaction(agentKey ?? name, reactions ?? [], baseline, reduce);
  const ref = React.useRef<HTMLSpanElement>(null);
  React.useEffect(() => {
    if (!ref.current) return;
    return joinClock(ref.current, id.blinkS, id.blinkDelayS);
  }, [id.blinkS, id.blinkDelayS]);
  const box = model.screen;
  const style = {
    width: size,
    height: size,
    "--tile": `var(--hb-robot-tile-${id.tile})`,
    "--mask": `url(${maskUrl(model.id, size, dir)})`,
    "--x": box.x,
    "--y": box.y,
    "--w": box.w,
    "--h": box.h,
    "--g": glowVar(amber ? "amber" : id.glow),
    "--bd": `${id.breatheDelayS.toFixed(1)}s`,
  } as React.CSSProperties;
  const classes = ["hb-bot", `s-${face}`, tiny ? "tiny" : "", size < BREATHE_MIN ? "still" : "", reaction ? `r-${reaction}` : "", className ?? ""].filter(Boolean).join(" ");
  return (
    <span ref={ref} className={classes} style={style} data-robot={model.id} data-season={variant?.dir} data-face={face} aria-hidden>
      <span className="hb-bot-tile">
        <span className="hb-bot-rig">
          {/* eslint-disable-next-line @next/next/no-img-element -- fixed, pre-sharpened sizes; no optimiser */}
          <img src={`${dir}/${model.id}-${assetSize(size)}.webp`} srcSet={bodySrcSet(model.id, dir)} sizes={`${size}px`} alt="" draggable={false} decoding="async" />
          <span className="hb-bot-glass">
            <span className="hb-bot-tint" />
            <RobotFace face={face} sleepy={micro?.sleepy ?? false} />
          </span>
        </span>
      </span>
      {dot ? <span className="hb-av-dot" data-tone={dot} /> : null}
    </span>
  );
}

/** The reaction class to wear now; cleared when its one run ends. */
function useReaction(key: string, reactions: readonly Reaction[], baseline: number, reduce: boolean): ReactionKind | null {
  const [playing, setPlaying] = React.useState<ReactionKind | null>(null);
  const timer = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const signature = reactions.map((r) => `${r.kind}:${r.at_ms}`).join(",");
  React.useEffect(() => {
    if (!Number.isFinite(baseline) || reactions.length === 0) return;
    if (reduce) {
      // Skipped, and spent: turning motion back on must not replay them.
      spendReactions(key, reactions, baseline, PLAYED, Date.now());
      return;
    }
    const next = nextReaction(key, reactions, baseline, PLAYED, Date.now());
    if (!next) return;
    clearTimeout(timer.current);
    // Off for a frame first, so the same reaction twice in a row restarts.
    setPlaying(null);
    const raf = requestAnimationFrame(() => setPlaying(next.kind));
    timer.current = setTimeout(() => setPlaying(null), REACTION_MS[next.kind] + 150);
    return () => cancelAnimationFrame(raf);
    // `signature` stands for `reactions`: a new array with the same events is no news.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, signature, baseline, reduce]);
  React.useEffect(() => () => clearTimeout(timer.current), []);
  return reduce ? null : playing;
}
