"use client";

/** The components that only show: layout, text, data and the agent's work. */

import * as React from "react";
import { AlertTriangle, Check, Circle, CircleDot, Info, Minus, X } from "lucide-react";
import { Switch as BaseSwitch } from "@base-ui/react/switch";
import { Tabs as BaseTabs } from "@base-ui/react/tabs";
import { useStateBinding } from "@json-render/react";
import type { PropsOf } from "@harbard/huginn";
import { attachmentUrl, type WsAttachment } from "@/lib/workspace";
import { Compare, DiffView, MarkdownView, TextView } from "../workspace/attachments";
import { formatNumber } from "./scale";
import { createT } from "@/i18n";

const t = createT("huginn");

type WithChildren<T> = T & { children?: React.ReactNode };

/** The attachments of the message, for the components that show a file by name. */
export const FilesContext = React.createContext<readonly WsAttachment[]>([]);

export function Stack({ direction, gap, align, wrap, children }: WithChildren<PropsOf<"Stack">>) {
  return (
    <div className="hb-huginn-stack" data-direction={direction ?? "vertical"} data-gap={gap ?? "md"} data-align={align} data-wrap={wrap || undefined}>
      {children}
    </div>
  );
}

export function Grid({ columns, gap, children }: WithChildren<PropsOf<"Grid">>) {
  return (
    <div className="hb-huginn-grid" data-gap={gap ?? "md"} style={{ "--huginn-columns": columns } as React.CSSProperties}>
      {children}
    </div>
  );
}

export function Section({ title, description, children }: WithChildren<PropsOf<"Section">>) {
  return (
    <section className="hb-huginn-section">
      <h3>{title}</h3>
      {description ? <p>{description}</p> : null}
      {children}
    </section>
  );
}

export function Card({ title, subtitle, tone, children }: WithChildren<PropsOf<"Card">>) {
  return (
    <div className="hb-huginn-card" data-tone={tone ?? "neutral"}>
      {title ? <p className="hb-huginn-card-title">{title}</p> : null}
      {subtitle ? <p className="hb-huginn-card-sub">{subtitle}</p> : null}
      {children}
    </div>
  );
}

export function Tabs({ labels, children }: WithChildren<PropsOf<"Tabs">>) {
  const panels = React.Children.toArray(children);
  return (
    <BaseTabs.Root defaultValue={0} className="hb-huginn-tabs">
      <BaseTabs.List className="hb-huginn-tablist">
        {labels.map((label, index) => (
          <BaseTabs.Tab key={`${index}-${label}`} value={index} className="hb-huginn-tab">
            {label}
          </BaseTabs.Tab>
        ))}
      </BaseTabs.List>
      {panels.map((panel, index) => (
        // Panels stay mounted so a Form spanning tabs keeps every field.
        <BaseTabs.Panel key={index} value={index} keepMounted className="hb-huginn-panel">
          {panel}
        </BaseTabs.Panel>
      ))}
    </BaseTabs.Root>
  );
}

export function Disclosure({ label, open, children }: WithChildren<PropsOf<"Disclosure">>) {
  return (
    <details className="hb-huginn-disclosure" open={open}>
      <summary>{label}</summary>
      <div>{children}</div>
    </details>
  );
}

export function Separator() {
  return <hr className="hb-huginn-rule" />;
}

export function Heading({ text, level }: PropsOf<"Heading">) {
  const Tag = `h${(level ?? 2) + 1}` as "h2" | "h3" | "h4";
  return <Tag className="hb-huginn-heading">{text}</Tag>;
}

export function Text({ text, tone, size, strong }: PropsOf<"Text">) {
  return (
    <p className="hb-huginn-text" data-tone={tone} data-size={size} data-strong={strong || undefined}>
      {text}
    </p>
  );
}

export function Markdown({ source }: PropsOf<"Markdown">) {
  return <MarkdownView source={source} />;
}

const CALLOUT_ICON = { neutral: Info, accent: Info, info: Info, success: Check, warning: AlertTriangle, danger: AlertTriangle } as const;

export function Callout({ tone, title, text }: PropsOf<"Callout">) {
  const Icon = CALLOUT_ICON[tone];
  return (
    <div className="hb-huginn-callout" data-tone={tone} role="note">
      <Icon className="size-4 shrink-0" aria-hidden />
      <div>
        {title ? <strong>{title}</strong> : null}
        <p>{text}</p>
      </div>
    </div>
  );
}

export function Quote({ text, cite }: PropsOf<"Quote">) {
  return (
    <blockquote className="hb-huginn-quote">
      <p>{text}</p>
      {cite ? <cite>{cite}</cite> : null}
    </blockquote>
  );
}

export function Badge({ text, tone }: PropsOf<"Badge">) {
  return (
    <span className="hb-huginn-badge" data-tone={tone ?? "neutral"}>
      {text}
    </span>
  );
}

export function KeyValue({ items }: PropsOf<"KeyValue">) {
  return (
    <dl className="hb-huginn-kv">
      {items.map((item, index) => (
        <div key={`${index}-${item.label}`}>
          <dt>{item.label}</dt>
          <dd>{item.value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function List({ items, ordered }: PropsOf<"List">) {
  const Tag = ordered ? "ol" : "ul";
  return (
    <Tag className="hb-huginn-list">
      {items.map((item, index) => (
        <li key={index}>{item}</li>
      ))}
    </Tag>
  );
}

export function Link({ href, label }: PropsOf<"Link">) {
  return (
    <a className="hb-huginn-link" href={href} target="_blank" rel="noopener noreferrer nofollow">
      {label}
    </a>
  );
}

export function Code({ code, language, filename }: PropsOf<"Code">) {
  return (
    <div className="hb-huginn-code">
      {filename || language ? <div className="hb-huginn-code-head">{filename ?? language}</div> : null}
      <pre className="hb-mono">{code}</pre>
    </div>
  );
}

export function Metric({ label, value, delta, deltaTone, hint }: PropsOf<"Metric">) {
  return (
    <div className="hb-huginn-metric">
      <span>{label}</span>
      <strong>{value}</strong>
      {delta ? (
        <em data-tone={deltaTone ?? "neutral"}>{delta}</em>
      ) : null}
      {hint ? <small>{hint}</small> : null}
    </div>
  );
}

type Cell = PropsOf<"Table">["rows"][number][number];

const compareCells = (a: Cell | undefined, b: Cell | undefined): number => {
  if (a == null || b == null) return a == null ? (b == null ? 0 : 1) : -1;
  if (typeof a === "number" && typeof b === "number") return a - b;
  return String(a).localeCompare(String(b), "es", { numeric: true });
};

const TABLE_PAGE = 50;

export function Table({ caption, columns, rows, sortable }: PropsOf<"Table">) {
  const [sort, setSort] = React.useState<{ column: number; down: boolean } | null>(null);
  const [all, setAll] = React.useState(false);
  const ordered = sort ? [...rows].sort((a, b) => compareCells(a[sort.column], b[sort.column]) * (sort.down ? -1 : 1)) : rows;
  const shownRows = all ? ordered : ordered.slice(0, TABLE_PAGE);
  const flip = (column: number) => setSort((now) => (now?.column === column ? { column, down: !now.down } : { column, down: false }));
  return (
    <div className="hb-huginn-table">
      <table>
        {caption ? <caption>{caption}</caption> : null}
        <thead>
          <tr>
            {columns.map((column, index) => (
              <th key={`${index}-${column.label}`} data-align={column.align} aria-sort={sort?.column === index ? (sort.down ? "descending" : "ascending") : undefined}>
                {sortable ? (
                  <button type="button" onClick={() => flip(index)}>
                    {column.label}
                    {sort?.column === index ? <span aria-hidden>{sort.down ? " ↓" : " ↑"}</span> : null}
                  </button>
                ) : (
                  column.label
                )}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {shownRows.map((row, r) => (
            <tr key={r}>
              {row.map((cell, c) => (
                <td key={c} data-align={columns[c]?.align}>
                  {typeof cell === "number" ? formatNumber(cell) : (cell ?? "")}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {ordered.length > shownRows.length ? (
        <button type="button" className="hb-att-more" onClick={() => setAll(true)}>
          {t("showRemaining", { count: ordered.length - shownRows.length })}
        </button>
      ) : null}
    </div>
  );
}

export function Progress({ label, value, max, tone, hint }: PropsOf<"Progress">) {
  const top = max ?? 100;
  const percent = Math.max(0, Math.min(100, (value / top) * 100));
  return (
    <div className="hb-huginn-progress" data-tone={tone ?? "accent"}>
      <div>
        <span>{label}</span>
        <b>{t("percent", { value: Math.round(percent) })}</b>
      </div>
      <div className="hb-huginn-bar" role="progressbar" aria-label={label} aria-valuenow={value} aria-valuemin={0} aria-valuemax={top}>
        <i style={{ width: `${percent}%` }} />
      </div>
      {hint ? <small>{hint}</small> : null}
    </div>
  );
}

export function Timeline({ items }: PropsOf<"Timeline">) {
  return (
    <ol className="hb-huginn-timeline">
      {items.map((item, index) => (
        <li key={index} data-tone={item.tone ?? "neutral"}>
          {item.time ? <time>{item.time}</time> : null}
          <strong>{item.title}</strong>
          {item.detail ? <p>{item.detail}</p> : null}
        </li>
      ))}
    </ol>
  );
}

const STEP = {
  done: { Icon: Check, word: "stepDone" },
  current: { Icon: CircleDot, word: "stepCurrent" },
  todo: { Icon: Circle, word: "stepTodo" },
  failed: { Icon: X, word: "stepFailed" },
} as const;

export function Steps({ items }: PropsOf<"Steps">) {
  return (
    <ol className="hb-huginn-steps">
      {items.map((item, index) => {
        const { Icon, word } = STEP[item.state];
        return (
          <li key={index} data-state={item.state}>
            <Icon className="size-4 shrink-0" aria-hidden />
            <div>
              <strong>{item.label}</strong> <span>{t(word)}</span>
              {item.detail ? <p>{item.detail}</p> : null}
            </div>
          </li>
        );
      })}
    </ol>
  );
}

const CHECK = {
  pass: { Icon: Check, word: "checkPass" },
  fail: { Icon: X, word: "checkFail" },
  warn: { Icon: AlertTriangle, word: "checkWarn" },
  skip: { Icon: Minus, word: "checkSkip" },
} as const;

export function Checklist({ items }: PropsOf<"Checklist">) {
  return (
    <ul className="hb-huginn-steps">
      {items.map((item, index) => {
        const { Icon, word } = CHECK[item.state];
        return (
          <li key={index} data-state={item.state}>
            <Icon className="size-4 shrink-0" aria-hidden />
            <div>
              <strong>{item.label}</strong> <span>{t(word)}</span>
              {item.detail ? <p>{item.detail}</p> : null}
            </div>
          </li>
        );
      })}
    </ul>
  );
}

export function Diff({ source, filename }: PropsOf<"Diff">) {
  return (
    <div className="hb-huginn-code">
      {filename ? <div className="hb-huginn-code-head">{filename}</div> : null}
      <DiffView source={source} />
    </div>
  );
}

const FILE_STATUS = { added: "fileAdded", modified: "fileModified", deleted: "fileDeleted", renamed: "fileRenamed" } as const;

export function FileTree({ entries }: PropsOf<"FileTree">) {
  return (
    <ul className="hb-huginn-files hb-mono">
      {entries.map((entry, index) => (
        <li key={`${index}-${entry.path}`} data-status={entry.status}>
          <span>{entry.path}</span>
          {entry.status ? <em>{t(FILE_STATUS[entry.status])}</em> : null}
          {entry.note ? <small>{entry.note}</small> : null}
        </li>
      ))}
    </ul>
  );
}

export function Terminal({ command, output, exitCode }: PropsOf<"Terminal">) {
  return (
    <div className="hb-huginn-code">
      {command || exitCode !== undefined ? (
        <div className="hb-huginn-code-head hb-mono">
          {command ? <span>$ {command}</span> : <span />}
          {exitCode !== undefined ? <b data-ok={exitCode === 0}>{t("exitCode", { code: exitCode })}</b> : null}
        </div>
      ) : null}
      <TextView source={output} />
    </div>
  );
}

function MissingFile({ file }: { file: string }) {
  return <p className="hb-huginn-error">{t("missingAttachment", { file })}</p>;
}

const useFile = (name: string) => React.useContext(FilesContext).find((file) => file.name === name && file.kind === "image") ?? null;

export function Image({ file, alt, caption }: PropsOf<"Image">) {
  const found = useFile(file);
  if (!found) return <MissingFile file={file} />;
  return (
    <figure className="hb-huginn-image">
      {/* eslint-disable-next-line @next/next/no-img-element -- served by harbard, not optimisable */}
      <img src={attachmentUrl(found)} alt={alt} loading="lazy" />
      {caption ? <figcaption>{caption}</figcaption> : null}
    </figure>
  );
}

export function BeforeAfter({ before, after, mode }: PropsOf<"BeforeAfter">) {
  const beforeFile = useFile(before);
  const afterFile = useFile(after);
  if (!beforeFile) return <MissingFile file={before} />;
  if (!afterFile) return <MissingFile file={after} />;
  return <Compare before={beforeFile} after={afterFile} mode={mode === "slider" ? "slider" : "side"} />;
}

export function Segmented({ state, label, options }: PropsOf<"Segmented">) {
  const [current, set] = useStateBinding<string>(state);
  return (
    <div className="hb-huginn-control">
      {label ? <span>{label}</span> : null}
      <div className="hb-huginn-segmented" role="group" aria-label={label}>
        {options.map((option) => (
          <button key={option.value} type="button" aria-pressed={current === option.value} onClick={() => set(option.value)}>
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}

export function Switch({ state, label }: PropsOf<"Switch">) {
  const [current, set] = useStateBinding<boolean>(state);
  return (
    <label className="hb-huginn-control hb-huginn-switch">
      <BaseSwitch.Root className="hb-huginn-switch-root" checked={current === true} onCheckedChange={(on) => set(on)}>
        <BaseSwitch.Thumb className="hb-huginn-switch-thumb" />
      </BaseSwitch.Root>
      <span>{label}</span>
    </label>
  );
}
