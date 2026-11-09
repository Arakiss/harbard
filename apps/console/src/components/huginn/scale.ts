/** The arithmetic behind the charts, free of React so it can be tested alone. */

import { createT, intlTag } from "@/i18n";

const t = createT("huginn");

export interface Domain {
  readonly min: number;
  readonly max: number;
  readonly ticks: readonly number[];
}

const present = (value: number | null): value is number => value !== null && Number.isFinite(value);

/** Round tick marks that cover the values, always including zero for bars. */
export function niceDomain(values: readonly (number | null)[], options: { includeZero: boolean; count?: number }): Domain {
  const known = values.filter(present);
  const count = options.count ?? 4;
  let low = known.length > 0 ? Math.min(...known) : 0;
  let high = known.length > 0 ? Math.max(...known) : 1;
  if (options.includeZero) {
    low = Math.min(0, low);
    high = Math.max(0, high);
  }
  if (low === high) {
    low -= Math.abs(low) * 0.1 || 1;
    high += Math.abs(high) * 0.1 || 1;
  }
  const rough = (high - low) / count;
  const magnitude = 10 ** Math.floor(Math.log10(rough));
  const residual = rough / magnitude;
  const step = (residual >= 5 ? 10 : residual >= 2 ? 5 : residual >= 1.5 ? 2 : 1) * magnitude;
  const first = Math.floor(low / step) * step;
  const last = Math.ceil(high / step) * step;
  const ticks: number[] = [];
  // Stepping by an index keeps 0.1 + 0.2 style error out of the labels.
  for (let i = 0; first + i * step <= last + step / 2; i += 1) ticks.push(Number((first + i * step).toPrecision(12)));
  return { min: first, max: last, ticks };
}

export interface StackSegment {
  readonly start: number;
  readonly end: number;
}

/** Positive values pile up from zero and negative ones down from it. */
export function stack(values: readonly (number | null)[]): StackSegment[] {
  let up = 0;
  let down = 0;
  return values.map((value) => {
    const amount = value ?? 0;
    if (amount >= 0) {
      const start = up;
      up += amount;
      return { start, end: up };
    }
    const start = down;
    down += amount;
    return { start, end: down };
  });
}

/** Labels that fit, evenly spread, always keeping the first. */
export function thinnedLabels(count: number, roomPx: number, labelPx: number): Set<number> {
  const every = Math.max(1, Math.ceil(count / Math.max(1, Math.floor(roomPx / labelPx))));
  const kept = new Set<number>();
  for (let i = 0; i < count; i += every) kept.add(i);
  return kept;
}

/** An arc of a ring between two angles in radians, clockwise from the top. */
export function ringSegment(cx: number, cy: number, outer: number, inner: number, from: number, to: number): string {
  // A full circle is a zero-length arc in SVG, so one whole segment stops just short.
  const end = to - from >= Math.PI * 2 ? from + Math.PI * 2 - 0.0001 : to;
  const point = (radius: number, angle: number) => `${cx + radius * Math.sin(angle)} ${cy - radius * Math.cos(angle)}`;
  const large = end - from > Math.PI ? 1 : 0;
  return `M ${point(outer, from)} A ${outer} ${outer} 0 ${large} 1 ${point(outer, end)} L ${point(inner, end)} A ${inner} ${inner} 0 ${large} 0 ${point(inner, from)} Z`;
}

export function formatNumber(value: number | null, unit?: string): string {
  if (value === null) return t("noData");
  const number = new Intl.NumberFormat(intlTag(), { maximumFractionDigits: 2 }).format(value);
  return unit ? `${number} ${unit}` : number;
}
