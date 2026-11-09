import * as React from "react";
import type { Face } from "@/lib/activity";

/**
 * A robot's face: light drawn by code in a 100×64 screen box. It sits in the glass layer, clipped to the model's own screen
 * mask and added with `mix-blend-mode: screen`, so the body's painted
 * reflection stays on top. Every state maps to something harbard observes.
 *
 * Classes are scoped (`rb-*`) and styled in robot.css; the light's colour is
 * `--g`, set by the avatar from a token.
 *
 * The blink sits on its own inner group. Sharing one element with the look
 * (`ponder`, `scan`, `glance`) would let one animation replace the other and
 * drop the raised eyes of `web` and `waiting` while anything moved; nested,
 * the eyes look up, scan and blink together.
 */
export function RobotFace({ face, sleepy = false }: { face: Face; sleepy?: boolean }) {
  const id = `rbf${React.useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  const eyeH = sleepy ? 9 : 18;
  const eye = (x: number, extra?: React.SVGProps<SVGRectElement>) => (
    <rect className="rb-eye" x={x - 7} y={22} width={14} height={eyeH} rx={7} {...extra} />
  );
  const arcs = (up: boolean) =>
    [34, 66].map((x) => <path key={x} d={`M${x - 8} ${up ? 34 : 30}q8 ${up ? -10 : 8} 16 0`} className="rb-ln" />);
  let body: React.ReactNode;
  switch (face) {
    case "normal":
      body = (
        <>
          <g className="rb-blink">
            {eye(34)}
            {eye(66)}
          </g>
          <path d="M45 47q5 4 10 0" className="rb-ln rb-thin" />
        </>
      );
      break;
    case "thinking":
      body = (
        <>
          <g className="rb-ponder">
            <g className="rb-blink">
              {eye(34)}
              {eye(66)}
            </g>
          </g>
          <g className="rb-dots">
            <circle cx={40} cy={52} r={2.6} />
            <circle cx={50} cy={52} r={2.6} />
            <circle cx={60} cy={52} r={2.6} />
          </g>
        </>
      );
      break;
    case "running":
      body = (
        <>
          <g className="rb-blink">
            <rect className="rb-eye" x={27} y={30} width={14} height={11} rx={5.5} />
            <rect className="rb-eye" x={59} y={30} width={14} height={11} rx={5.5} />
          </g>
          <rect className="rb-caret" x={46} y={49} width={8} height={3} rx={1.5} />
        </>
      );
      break;
    case "reading":
      body = (
        <>
          <g className="rb-scan">
            <g className="rb-blink">
              {eye(34)}
              {eye(66)}
            </g>
          </g>
          <path d="M44 48h12" className="rb-ln rb-thin" />
        </>
      );
      break;
    case "web":
      body = (
        <>
          <g transform="translate(0 -3)">
            <g className="rb-blink">
              {eye(34)}
              {eye(66)}
            </g>
          </g>
          <circle className="rb-eye" cx={50} cy={50} r={2.2} />
        </>
      );
      break;
    case "delegating":
      body = (
        <>
          <g className="rb-glance">
            <g className="rb-blink">
              {eye(34)}
              {eye(66)}
            </g>
          </g>
          <g className="rb-orbit">
            <circle cx={50} cy={44} r={2.2} />
            <circle cx={50} cy={60} r={2.2} />
          </g>
        </>
      );
      break;
    case "waiting":
      body = (
        <>
          <g transform="translate(0 -4)">
            <g className="rb-blink">
              {eye(34)}
              {eye(66)}
            </g>
          </g>
          <path d="M44 49h12" className="rb-ln rb-thin" />
        </>
      );
      break;
    case "asleep":
      body = (
        <>
          {arcs(false)}
          <g className="rb-zz">
            <text x={76} y={20}>
              z
            </text>
            <text x={84} y={12}>
              z
            </text>
          </g>
        </>
      );
      break;
    case "blocked":
      body = (
        <>
          <g>
            <rect className="rb-eye" x={27} y={24} width={14} height={16} rx={7} transform="rotate(-10 34 32)" />
            <rect className="rb-eye" x={59} y={24} width={14} height={16} rx={7} transform="rotate(10 66 32)" />
          </g>
          <path d="M43 50q3.5-3 7 0t7 0" className="rb-ln rb-thin" />
          <text className="rb-q" x={80} y={22}>
            ?
          </text>
        </>
      );
      break;
    case "finished":
      body = (
        <>
          {arcs(true)}
          <path d="M42 45q8 8 16 0" className="rb-ln" />
        </>
      );
      break;
    case "verified":
      body = (
        <>
          {arcs(true)}
          <path className="rb-check" d="M41 47l6 6 12-12" />
        </>
      );
      break;
    case "disconnected":
      body = <circle cx={50} cy={32} r={2} className="rb-off" />;
      break;
  }
  return (
    <svg className={`rb-screen rb-s-${face}`} viewBox="0 0 100 64" aria-hidden focusable={false}>
      <defs>
        <filter id={id} x="-40%" y="-40%" width="180%" height="180%">
          <feGaussianBlur stdDeviation={2.4} result="b" />
          <feMerge>
            <feMergeNode in="b" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
      </defs>
      <g filter={`url(#${id})`}>{body}</g>
    </svg>
  );
}
