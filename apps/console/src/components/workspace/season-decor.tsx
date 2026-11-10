"use client";

import * as React from "react";

/**
 * The season's decorations, drawn while the root carries `data-hb-decor`:
 * snow falling over the frame (the top band and the rail, never the panel
 * where text is read), a string of lights along the panel's top edge, and
 * in the Lobby a small sleigh that crosses the header now and then.
 *
 * Few moving layers, each moved whole by a CSS transform or opacity that the
 * compositor runs without the page's main thread: the snow is three tiled
 * layers per band sliding down one tile and starting over, the lights two
 * rows that fade in turn, the sleigh one drawing. A first version with a
 * layer per flake and per bulb (76) took the Lobby from 10.7% to 45% of one
 * core in headless Chromium (measured 10 November 2026, 20 s idle).
 *
 * They move only while the window is visible, focused and in use: two
 * minutes without a pointer or a key, or the window behind another, and they
 * settle (`data-hb-decor-still`) until the next input: the lights stay lit
 * and still, the snow and the sleigh leave.
 * Even few layers kept the compositor drawing ten frames a second, about
 * 10% of a core for a console left open all day. "Reduce motion", the OS's
 * or the console's, leaves only the lights, standing still.
 */

/** Two minutes without a pointer or a key settle the decorations. */
export const DECOR_IDLE_MS = 2 * 60_000;

/** Whether the decorations move: the window seen, focused, and used lately. */
export function decorAwake(now: { visible: boolean; focused: boolean; sinceInputMs: number }): boolean {
  return now.visible && now.focused && now.sinceInputMs < DECOR_IDLE_MS;
}

const INPUTS = ["pointermove", "pointerdown", "keydown", "wheel", "touchstart"] as const;

/**
 * `decorAwake`, kept current. Input only stamps a time; one timer, armed
 * while awake, settles the decorations when the idle time runs out, so a
 * console nobody touches runs no timer and draws no frame.
 */
export function useDecorAwake(enabled: boolean): boolean {
  const [awake, setAwake] = React.useState(true);
  React.useEffect(() => {
    if (!enabled) return;
    let last = Date.now();
    let moving = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const check = () => {
      clearTimeout(timer);
      const since = Date.now() - last;
      moving = decorAwake({ visible: document.visibilityState === "visible", focused: document.hasFocus(), sinceInputMs: since });
      setAwake(moving);
      if (moving) timer = setTimeout(check, DECOR_IDLE_MS - since);
    };
    const onInput = () => {
      last = Date.now();
      if (!moving) check();
    };
    for (const name of INPUTS) window.addEventListener(name, onInput, { passive: true, capture: true });
    window.addEventListener("focus", check);
    window.addEventListener("blur", check);
    document.addEventListener("visibilitychange", check);
    check();
    return () => {
      clearTimeout(timer);
      for (const name of INPUTS) window.removeEventListener(name, onInput, { capture: true });
      window.removeEventListener("focus", check);
      window.removeEventListener("blur", check);
      document.removeEventListener("visibilitychange", check);
    };
  }, [enabled]);
  return awake;
}

/**
 * The Strip is a window nobody focuses, on a screen left on all day, so it
 * cannot wait for input like the console: snow falls there in a flurry, the
 * first 40 seconds of every five minutes by the wall clock, and settles in
 * between with the lights lit and still. Moving all the time would cost about
 * 10% of a core for the whole day (the console's figure above).
 */
export const FLURRY_EVERY_MS = 5 * 60_000;
export const FLURRY_MS = 40_000;

/** Whether the Strip's snow is falling at this moment of the wall clock. */
export function inFlurry(nowMs: number): boolean {
  return nowMs % FLURRY_EVERY_MS < FLURRY_MS;
}

/** `inFlurry`, kept current by one timer armed for the next change. */
export function useFlurry(enabled: boolean): boolean {
  const [on, setOn] = React.useState(false);
  React.useEffect(() => {
    if (!enabled) {
      setOn(false);
      return;
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    const check = () => {
      const now = Date.now();
      const flurry = inFlurry(now);
      setOn(flurry);
      const phase = now % FLURRY_EVERY_MS;
      // A few milliseconds past the edge, so the next check lands on the other side of it.
      timer = setTimeout(check, (flurry ? FLURRY_MS : FLURRY_EVERY_MS) - phase + 20);
    };
    check();
    return () => clearTimeout(timer);
  }, [enabled]);
  return on;
}

const BULBS = 36;
/** The Strip's stage is 2560 px wide: the same spacing takes twice the bulbs. */
const STRIP_BULBS = 64;
const SNOW_LAYERS = [1, 2, 3] as const;

function Snow({ where }: { where: "rail" | "top" | "strip" }) {
  return (
    <div className="hb-festive-snow" data-where={where}>
      {SNOW_LAYERS.map((layer) => (
        <span key={layer} className="hb-festive-snow-layer" data-layer={layer} />
      ))}
    </div>
  );
}

/** One row of the string: the bulbs of its set lit, the others left as gaps. */
function Lights({ set, count = BULBS, where }: { set: 0 | 1; count?: number; where?: "strip" }) {
  return (
    <div className="hb-festive-lights" data-set={set} data-where={where}>
      {Array.from({ length: count }, (_, i) => (
        <span key={i} className="hb-festive-bulb" data-tone={(i % 4) + 1} data-off={i % 2 === set ? undefined : ""} />
      ))}
    </div>
  );
}

/** Snow over the frame and lights along the panel's top edge. */
export function SeasonDecor() {
  return (
    <div className="hb-festive" aria-hidden data-testid="season-decor">
      <Snow where="top" />
      <Snow where="rail" />
      <div className="hb-festive-wire" />
      <Lights set={0} />
      <Lights set={1} />
    </div>
  );
}

/** The Strip's: snow over its band and a string of lights along the top of its panels. */
export function StripDecor() {
  return (
    <div className="hb-festive" aria-hidden data-testid="season-decor">
      <Snow where="strip" />
      <div className="hb-festive-wire" data-where="strip" />
      <Lights set={0} count={STRIP_BULBS} where="strip" />
      <Lights set={1} count={STRIP_BULBS} where="strip" />
    </div>
  );
}

/**
 * The Lobby's visitor: a sleigh and two reindeer that cross the sky above
 * the title once every 50 seconds, a few seconds after the Lobby opens.
 */
export function SeasonSleigh() {
  return (
    <div className="hb-festive-sky" aria-hidden data-testid="season-sleigh">
      <svg className="hb-festive-sleigh" viewBox="0 0 72 20" width="86" height="24">
        {/* Two reindeer, the first with a red nose, harnessed to the sleigh. */}
        <g className="hb-festive-deer">
          <path d="M44 12h8l2-3 2 3h3l1-4 1.4.6-.8 4.4-2 3h-1l.6-2.6h-6.6l-1.4 2.6h-1l.8-2.6-2.6-.4z" />
          <path d="M30 12h8l2-3 2 3h3l1-4 1.4.6-.8 4.4-2 3h-1l.6-2.6h-6.6l-1.4 2.6h-1l.8-2.6-2.6-.4z" />
        </g>
        <path d="M57 8l-1-3m2 3 1-3.4m-2.4 1.2-1.6-.8m3.8.6 1.8-.8" className="hb-festive-antler" />
        <path d="M43 8l-1-3m2 3 1-3.4m-2.4 1.2-1.6-.8m3.8.6 1.8-.8" className="hb-festive-antler" />
        <circle cx="62.6" cy="9.2" r="1.3" className="hb-festive-nose" />
        <path d="M31 13.5 22 11" className="hb-festive-rein" />
        {/* The sleigh: a curled runner and a body with a sack of parcels. */}
        <path d="M4 8h14l4 3-2 4H6z" className="hb-festive-body" />
        <path d="M2 17h19c2 0 3-1 3.6-2.4" className="hb-festive-runner" />
        <circle cx="10" cy="6" r="3.4" className="hb-festive-sack" />
      </svg>
    </div>
  );
}
