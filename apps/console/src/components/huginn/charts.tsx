"use client";

/**
 * Charts drawn as plain SVG: thin marks, a recessive grid, one axis, a legend
 * for more than one series, a tooltip on every mark and the numbers one click
 * away. Series colours are identity only, so all text stays in ink.
 */

import * as React from "react";
import type { PropsOf } from "@harbard/huginn";
import { createT } from "@/i18n";
import { formatNumber, niceDomain, ringSegment, stack, thinnedLabels, type Domain } from "./scale";

const t = createT("huginn");

const HEIGHT = 240;
const MARGIN = { top: 12, right: 12, bottom: 28, left: 48 } as const;
const series = (index: number) => `var(--hb-series-${index + 1})`;

/** Width of the element, so text in the SVG stays its real size at any pane width. */
function useWidth(initial: number): [React.RefObject<HTMLDivElement | null>, number] {
  const ref = React.useRef<HTMLDivElement>(null);
  const [width, setWidth] = React.useState(initial);
  React.useEffect(() => {
    const node = ref.current;
    if (!node) return;
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setWidth(Math.max(200, entry.contentRect.width));
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, []);
  return [ref, width];
}

interface Tip {
  readonly x: number;
  readonly y: number;
  readonly title: string;
  readonly lines: readonly { readonly name: string; readonly value: string; readonly colour?: string }[];
}

function Tooltip({ tip, width }: { tip: Tip | null; width: number }) {
  if (!tip) return null;
  const flip = tip.x > width - 160;
  return (
    <div className="hb-huginn-tip" role="status" style={{ left: tip.x, top: tip.y, transform: flip ? "translate(calc(-100% - 12px), 0)" : "translate(12px, 0)" }}>
      <strong>{tip.title}</strong>
      {tip.lines.map((line) => (
        <span key={line.name}>
          {line.colour ? <i style={{ background: line.colour }} aria-hidden /> : null}
          {line.name}: <b>{line.value}</b>
        </span>
      ))}
    </div>
  );
}

function Legend({ names }: { names: readonly string[] }) {
  return (
    <ul className="hb-huginn-legend">
      {names.map((name, index) => (
        <li key={name}>
          <i style={{ background: series(index) }} aria-hidden />
          {name}
        </li>
      ))}
    </ul>
  );
}

function ChartFrame({
  title,
  legend,
  head,
  rows,
  children,
}: {
  title: string;
  legend?: readonly string[];
  head: readonly string[];
  rows: readonly (readonly string[])[];
  children: React.ReactNode;
}) {
  return (
    <figure className="hb-huginn-chart">
      <figcaption>{title}</figcaption>
      {legend && legend.length > 1 ? <Legend names={legend} /> : null}
      {children}
      <details className="hb-huginn-data">
        <summary>{t("showData")}</summary>
        <div className="hb-huginn-table">
          <table>
            <thead>
              <tr>
                {head.map((cell, index) => (
                  <th key={`${index}-${cell}`}>{cell}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, r) => (
                <tr key={r}>
                  {row.map((cell, c) => (
                    <td key={c}>{cell}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </details>
    </figure>
  );
}

function Axes({ domain, width, labels, x, unit }: { domain: Domain; width: number; labels: readonly string[]; x: (index: number) => number; unit?: string }) {
  const y = yScale(domain);
  const kept = thinnedLabels(labels.length, width - MARGIN.left - MARGIN.right, 64);
  return (
    <g className="hb-huginn-axes">
      {domain.ticks.map((tick) => (
        <g key={tick}>
          <line x1={MARGIN.left} x2={width - MARGIN.right} y1={y(tick)} y2={y(tick)} data-zero={tick === 0 || undefined} />
          <text x={MARGIN.left - 6} y={y(tick) + 4} textAnchor="end">
            {formatNumber(tick)}
          </text>
        </g>
      ))}
      {labels.map((label, index) =>
        kept.has(index) ? (
          <text key={`${index}-${label}`} x={x(index)} y={HEIGHT - 8} textAnchor="middle">
            {label}
          </text>
        ) : null,
      )}
      {unit ? (
        <text x={MARGIN.left} y={9} className="hb-huginn-unit">
          {unit}
        </text>
      ) : null}
    </g>
  );
}

const yScale = (domain: Domain) => (value: number) =>
  MARGIN.top + (1 - (value - domain.min) / (domain.max - domain.min || 1)) * (HEIGHT - MARGIN.top - MARGIN.bottom);

function dataTable(labels: readonly string[], all: PropsOf<"BarChart">["series"], unit?: string) {
  return {
    head: ["", ...all.map((s) => s.name)],
    rows: labels.map((label, index) => [label, ...all.map((s) => formatNumber(s.values[index] ?? null, unit))]),
  };
}

export function BarChart({ title, labels, series: all, unit, stacked }: PropsOf<"BarChart">) {
  const [ref, width] = useWidth(640);
  const [tip, setTip] = React.useState<Tip | null>(null);
  const stacks = labels.map((_, index) => stack(all.map((s) => s.values[index] ?? null)));
  const extent = stacked ? stacks.flat().flatMap((segment) => [segment.start, segment.end]) : all.flatMap((s) => s.values);
  const domain = niceDomain(extent, { includeZero: true });
  const y = yScale(domain);
  const band = (width - MARGIN.left - MARGIN.right) / labels.length;
  const centre = (index: number) => MARGIN.left + band * (index + 0.5);
  const groupWidth = Math.min(band * 0.72, 72);
  const barWidth = stacked ? groupWidth : Math.max(2, (groupWidth - (all.length - 1) * 2) / all.length);
  const table = dataTable(labels, all, unit);
  return (
    <ChartFrame title={title} legend={all.map((s) => s.name)} head={table.head} rows={table.rows}>
      <div ref={ref} className="hb-huginn-plot" onMouseLeave={() => setTip(null)}>
        <svg width={width} height={HEIGHT} role="img" aria-label={title}>
          <Axes domain={domain} width={width} labels={labels} x={centre} unit={unit} />
          {labels.map((label, index) => (
            <g key={`${index}-${label}`}>
              {all.map((s, seriesIndex) => {
                const value = s.values[index] ?? null;
                if (value === null) return null;
                const segment = stacks[index]?.[seriesIndex];
                const from = stacked && segment ? segment.start : 0;
                const to = stacked && segment ? segment.end : value;
                const top = Math.min(y(from), y(to));
                const left = stacked ? centre(index) - barWidth / 2 : centre(index) - groupWidth / 2 + seriesIndex * (barWidth + 2);
                // 2px of surface between stacked segments.
                const height = Math.max(1, Math.abs(y(from) - y(to)) - (stacked ? 2 : 0));
                return <rect key={s.name} x={left} y={top} width={barWidth} height={height} rx={Math.min(3, barWidth / 2)} fill={series(seriesIndex)} />;
              })}
              <rect
                x={centre(index) - band / 2}
                y={MARGIN.top}
                width={band}
                height={HEIGHT - MARGIN.top - MARGIN.bottom}
                fill="transparent"
                onMouseEnter={() =>
                  setTip({
                    x: centre(index),
                    y: MARGIN.top,
                    title: label,
                    lines: all.map((s, seriesIndex) => ({ name: s.name, value: formatNumber(s.values[index] ?? null, unit), colour: series(seriesIndex) })),
                  })
                }
              />
            </g>
          ))}
        </svg>
        <Tooltip tip={tip} width={width} />
      </div>
    </ChartFrame>
  );
}

/** The segments of one series between its gaps. */
function runs(points: readonly (readonly [number, number] | null)[]): (readonly [number, number])[][] {
  const out: (readonly [number, number])[][] = [];
  let current: (readonly [number, number])[] = [];
  for (const point of points) {
    if (point) current.push(point);
    else if (current.length > 0) {
      out.push(current);
      current = [];
    }
  }
  if (current.length > 0) out.push(current);
  return out;
}

const path = (points: readonly (readonly [number, number])[]) => points.map(([x, y], index) => `${index === 0 ? "M" : "L"} ${x} ${y}`).join(" ");

export function LineChart({ title, labels, series: all, unit, area }: PropsOf<"LineChart">) {
  const [ref, width] = useWidth(640);
  const [hover, setHover] = React.useState<number | null>(null);
  const domain = niceDomain(all.flatMap((s) => s.values), { includeZero: Boolean(area) });
  const y = yScale(domain);
  const inner = width - MARGIN.left - MARGIN.right;
  const x = (index: number) => MARGIN.left + (labels.length > 1 ? (inner * index) / (labels.length - 1) : inner / 2);
  const table = dataTable(labels, all, unit);
  const baseline = y(Math.max(domain.min, 0));
  const onMove = (event: React.MouseEvent<SVGSVGElement>) => {
    const left = event.currentTarget.getBoundingClientRect().left;
    const ratio = (event.clientX - left - MARGIN.left) / inner;
    setHover(Math.max(0, Math.min(labels.length - 1, Math.round(ratio * (labels.length - 1)))));
  };
  const tip: Tip | null =
    hover === null
      ? null
      : {
          x: x(hover),
          y: MARGIN.top,
          title: labels[hover] ?? "",
          lines: all.map((s, index) => ({ name: s.name, value: formatNumber(s.values[hover] ?? null, unit), colour: series(index) })),
        };
  return (
    <ChartFrame title={title} legend={all.map((s) => s.name)} head={table.head} rows={table.rows}>
      <div ref={ref} className="hb-huginn-plot">
        <svg width={width} height={HEIGHT} role="img" aria-label={title} onMouseMove={onMove} onMouseLeave={() => setHover(null)}>
          <Axes domain={domain} width={width} labels={labels} x={x} unit={unit} />
          {all.map((s, seriesIndex) => {
            const points = s.values.map((value, index) => (value === null ? null : ([x(index), y(value)] as const)));
            const parts = runs(points);
            return (
              <g key={s.name}>
                {area && all.length === 1
                  ? parts.map((part, index) => (
                      <path key={index} d={`${path(part)} L ${part[part.length - 1]![0]} ${baseline} L ${part[0]![0]} ${baseline} Z`} fill={series(seriesIndex)} opacity={0.14} />
                    ))
                  : null}
                {parts.map((part, index) =>
                  part.length === 1 ? (
                    <circle key={index} cx={part[0]![0]} cy={part[0]![1]} r={3} fill={series(seriesIndex)} />
                  ) : (
                    <path key={index} d={path(part)} fill="none" stroke={series(seriesIndex)} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
                  ),
                )}
                {hover !== null && s.values[hover] != null ? <circle className="hb-huginn-dot" cx={x(hover)} cy={y(s.values[hover]!)} r={4.5} fill={series(seriesIndex)} /> : null}
              </g>
            );
          })}
          {hover !== null ? <line className="hb-huginn-cross" x1={x(hover)} x2={x(hover)} y1={MARGIN.top} y2={HEIGHT - MARGIN.bottom} /> : null}
        </svg>
        <Tooltip tip={tip} width={width} />
      </div>
    </ChartFrame>
  );
}

export function Sparkline({ label, values, unit }: PropsOf<"Sparkline">) {
  const width = 120;
  const height = 28;
  const min = Math.min(...values);
  const span = Math.max(...values) - min || 1;
  const points = values.map((value, index) => [2 + (index * (width - 4)) / Math.max(1, values.length - 1), height - 3 - ((value - min) / span) * (height - 6)] as const);
  const last = values[values.length - 1] ?? null;
  return (
    <div className="hb-huginn-spark">
      <span>{label}</span>
      <svg width={width} height={height} role="img" aria-label={`${label}: ${formatNumber(last, unit)}`}>
        <path d={path(points)} fill="none" stroke={series(0)} strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" />
      </svg>
      <b>{formatNumber(last, unit)}</b>
    </div>
  );
}

export function Donut({ title, segments, unit }: PropsOf<"Donut">) {
  const [hover, setHover] = React.useState<number | null>(null);
  const size = 180;
  const total = segments.reduce((sum, segment) => sum + segment.value, 0);
  const percent = (value: number) => t("percent", { value: total > 0 ? Math.round((value / total) * 100) : 0 });
  const gap = segments.filter((segment) => segment.value > 0).length > 1 ? 0.02 : 0;
  const arcs = segments.reduce<{ from: number; drawn: { from: number; to: number }[] }>(
    (state, segment) => {
      const sweep = total > 0 ? (segment.value / total) * Math.PI * 2 : 0;
      return { from: state.from + sweep, drawn: [...state.drawn, { from: state.from, to: state.from + sweep }] };
    },
    { from: 0, drawn: [] },
  ).drawn;
  const focus = hover === null ? null : segments[hover];
  return (
    <ChartFrame title={title} head={["", unit ?? t("value"), "%"]} rows={segments.map((s) => [s.label, formatNumber(s.value), percent(s.value)])}>
      <div className="hb-huginn-donut">
        <div className="hb-huginn-ring">
          <svg width={size} height={size} role="img" aria-label={title} onMouseLeave={() => setHover(null)}>
            {segments.map((segment, index) => {
              const arc = arcs[index];
              if (!arc || arc.to - arc.from <= gap) return null;
              return (
                <path
                  key={segment.label}
                  d={ringSegment(size / 2, size / 2, 86, 58, arc.from + gap / 2, arc.to - gap / 2)}
                  fill={series(index)}
                  opacity={hover === null || hover === index ? 1 : 0.45}
                  onMouseEnter={() => setHover(index)}
                />
              );
            })}
          </svg>
          <div className="hb-huginn-ring-centre">
            <b>{formatNumber(focus ? focus.value : total, unit)}</b>
            <span>{focus ? focus.label : t("total")}</span>
          </div>
        </div>
        <ul className="hb-huginn-legend" data-vertical="true">
          {segments.map((segment, index) => (
            <li key={segment.label}>
              <i style={{ background: series(index) }} aria-hidden />
              {segment.label} <b>{percent(segment.value)}</b>
            </li>
          ))}
        </ul>
      </div>
    </ChartFrame>
  );
}

export function Heatmap({ title, rows, columns, values, unit }: PropsOf<"Heatmap">) {
  const known = values.flat().filter((value): value is number => value !== null);
  const min = known.length > 0 ? Math.min(...known) : 0;
  const max = known.length > 0 ? Math.max(...known) : 1;
  const strength = (value: number) => Math.round(12 + ((value - min) / (max - min || 1)) * 88);
  return (
    <ChartFrame title={title} head={["", ...columns]} rows={rows.map((row, r) => [row, ...columns.map((_, c) => formatNumber(values[r]?.[c] ?? null, unit))])}>
      <div className="hb-huginn-heat" style={{ gridTemplateColumns: `minmax(48px, max-content) repeat(${columns.length}, minmax(24px, 1fr))` }}>
        <span />
        {columns.map((column, index) => (
          <span key={`${index}-${column}`} className="hb-huginn-heat-head">
            {column}
          </span>
        ))}
        {rows.map((row, r) => (
          <React.Fragment key={`${r}-${row}`}>
            <span className="hb-huginn-heat-row">{row}</span>
            {columns.map((column, c) => {
              const value = values[r]?.[c] ?? null;
              return (
                <span
                  key={`${c}-${column}`}
                  className="hb-huginn-heat-cell"
                  data-empty={value === null || undefined}
                  title={`${row} · ${column}: ${formatNumber(value, unit)}`}
                  style={value === null ? undefined : { background: `color-mix(in oklab, var(--hb-series-1) ${strength(value)}%, var(--hb-surface))` }}
                />
              );
            })}
          </React.Fragment>
        ))}
      </div>
      <div className="hb-huginn-heat-scale">
        {formatNumber(min, unit)} <i aria-hidden /> {formatNumber(max, unit)}
      </div>
    </ChartFrame>
  );
}
