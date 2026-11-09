import * as React from "react";
import { MARK } from "./mark-geometry";
import type { Mark } from "./parse-mark";

/**
 * The living logo's extra marks, placed on the mark's own presence dot so a
 * new logo keeps them: the wave ring under the dot, the check drawn in it and
 * the spinning arc that replaces it while connecting (animaciones.html).
 * Their motion is mark.css, keyed on the svg's `data-logo` and
 * `data-logo-once`.
 */
function dotOf(mark: Mark) {
  const dot = mark.shapes.find((s) => s.tone === "presence" && s.kind === "circle");
  return dot && dot.kind === "circle" ? dot : null;
}

export function MarkUnderlay({ mark = MARK }: { mark?: Mark }) {
  const dot = dotOf(mark);
  return dot ? <circle className="hb-mk-ring" cx={dot.cx} cy={dot.cy} r={dot.r} /> : null;
}

export function MarkOverlay({ mark = MARK }: { mark?: Mark }) {
  const dot = dotOf(mark);
  if (!dot) return null;
  const s = dot.r / 7;
  return (
    <>
      <path className="hb-mk-chk" d={`M${dot.cx - 3.4 * s} ${dot.cy + 0.2 * s}l${2.4 * s} ${2.4 * s} ${4.6 * s}-${4.8 * s}`} />
      <circle className="hb-mk-arc" cx={dot.cx} cy={dot.cy} r={dot.r - s} />
    </>
  );
}
