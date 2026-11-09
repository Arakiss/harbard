/**
 * The robot clock: one timer for every robot and status icon on the page.
 *
 * The prototype runs each robot's motion as CSS animations at the screen's
 * frame rate. Thirty robots doing that repaint thirty masked, blended,
 * blurred faces sixty times a second, even while a keyframe holds still (a
 * blink that waits five seconds is still a running animation). So here the
 * same CSS keyframes are paused and advanced by this clock at a few frames a
 * second, which subtle motion does not need more of, and each blink is a
 * one-shot the clock fires on the robot's own period. Reactions (hop, shake,
 * glance, spark, boot) and a blink itself run natively, at full rate, once.
 *
 * Nothing ticks while the tab is hidden. Under Reduce motion the CSS
 * removes every animation, so there is nothing to drive.
 */

/** Ticks per second. A blink advances on every tick (three or four frames);
 * the slow loops on every second one, all on the same ticks, so a robot is
 * repainted at most four times a second however many loops it has. */
const CLOCK_FPS = 8;
const EVERY: [RegExp, number][] = [[/^rb-blink$/, 1]];
const LOOP_EVERY = 2;
/** Run natively, once: never paused or driven. */
const NATIVE = new Set(["rb-hop", "rb-shake-body", "rb-glance-l", "rb-spark", "rb-drop", "rb-power", "rb-shake", "ic-stop"]);
const BLINK_MS = 380;

interface Driven {
  anim: Animation;
  base: number;
  from: number;
  every: number;
}

interface Bot {
  periodMs: number;
  next: number;
}

const bots = new Map<HTMLElement, Bot>();
/** Robots scrolled out of sight: they neither blink nor move. */
const unseen = new WeakSet<Element>();
let seen: IntersectionObserver | undefined;
let driven: Driven[] = [];
let timer: ReturnType<typeof setInterval> | undefined;
let observer: MutationObserver | undefined;
let dirty = false;
let tick = 0;

function everyOf(name: string): number {
  for (const [re, n] of EVERY) if (re.test(name)) return n;
  return LOOP_EVERY;
}

/** Take over every looping robot or icon animation that is not yet driven. */
function collect() {
  dirty = false;
  const now = performance.now();
  const kept = driven.filter((d) => d.anim.playState !== "idle" && (d.anim.effect as KeyframeEffect | null)?.target?.isConnected);
  const known = new Set(kept.map((d) => d.anim));
  for (const el of document.querySelectorAll<HTMLElement | SVGElement>(".hb-bot, .hb-ic")) {
    for (const anim of el.getAnimations({ subtree: true })) {
      const name = (anim as CSSAnimation).animationName;
      if (!name || known.has(anim) || NATIVE.has(name)) continue;
      const base = Number(anim.currentTime ?? 0);
      anim.pause();
      kept.push({ anim, base, from: now, every: everyOf(name) });
      known.add(anim);
    }
  }
  driven = kept;
}

function step() {
  if (document.visibilityState !== "visible") return;
  if (dirty) collect();
  tick++;
  const now = performance.now();
  let gone = false;
  for (const d of driven) {
    const target = (d.anim.effect as KeyframeEffect | null)?.target;
    const bot = target?.closest(".hb-bot");
    if (bot && unseen.has(bot)) continue;
    // Cancelled by the CSS (Reduce motion, a new state): setting its
    // time would bring it back to life.
    if (d.anim.playState === "idle" || d.anim.playState === "finished") {
      gone = true;
      continue;
    }
    if (tick % d.every === 0) d.anim.currentTime = d.base + (now - d.from);
  }
  if (gone) driven = driven.filter((d) => d.anim.playState !== "idle" && d.anim.playState !== "finished");
  for (const [el, b] of bots) {
    if (now < b.next || unseen.has(el)) continue;
    b.next = now + b.periodMs;
    el.classList.add("rb-blinking");
    setTimeout(() => el.classList.remove("rb-blinking"), BLINK_MS);
  }
}

function start() {
  if (timer || typeof window === "undefined") return;
  timer = setInterval(step, 1000 / CLOCK_FPS);
  observer = new MutationObserver(() => {
    dirty = true;
  });
  observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["class", "data-hb-motion"] });
  seen = new IntersectionObserver((entries) => {
    for (const e of entries) {
      if (e.isIntersecting) unseen.delete(e.target);
      else unseen.add(e.target);
    }
  });
  for (const el of bots.keys()) seen.observe(el);
  dirty = true;
}

function stop() {
  clearInterval(timer);
  timer = undefined;
  observer?.disconnect();
  observer = undefined;
  seen?.disconnect();
  seen = undefined;
  driven = [];
}

/**
 * A robot joins the clock: it blinks every `periodS` seconds, starting
 * `delayS` (≤ 0, from its name) into its cycle. Returns its leave function.
 */
export function joinClock(el: HTMLElement, periodS: number, delayS: number): () => void {
  const periodMs = periodS * 1000;
  // Where the prototype's 93% blink falls in this robot's shifted cycle.
  const first = (((periodMs * 0.93 + delayS * 1000) % periodMs) + periodMs) % periodMs;
  bots.set(el, { periodMs, next: performance.now() + first });
  start();
  seen?.observe(el);
  dirty = true;
  return () => {
    seen?.unobserve(el);
    bots.delete(el);
    if (bots.size === 0) stop();
  };
}

