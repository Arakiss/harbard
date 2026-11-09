"use client";

/**
 * What an agent attached (`harbard attach`), shown in the message: images as
 * thumbnails with a lightbox, an before/after pair side by side or under a
 * slider, Markdown reports in a pane on the right, diffs with their colours,
 * logs and text in monospace with "ver completo".
 *
 * Every byte comes from serve's attachments route through the console's
 * proxy, and serve reads only the attachments folder. Text is rendered as
 * text: Markdown goes through `lib/markdown`, which yields data, never HTML.
 */

import * as React from "react";
import { ChevronsLeftRight, Columns2, FileCode2, FileText, ScrollText, X } from "lucide-react";
import { createT, type TranslationKey } from "@/i18n";
import { MARKDOWN_MAX, diffLines, parse, prettyJson, preview, type Block, type Inline } from "@/lib/markdown";
import { attachmentUrl, beforeAfter, size, type WsAttachment } from "@/lib/workspace";

const t = createT("attachments");

const LABEL_KEYS: Record<string, TranslationKey<"attachments"> | undefined> = {
  before: "labelBefore",
  after: "labelAfter",
  report: "labelReport",
  code: "labelCode",
};

/** serve's stored label for an attachment, in the console's language. */
function shownLabel(label: string): string {
  const key = LABEL_KEYS[label];
  return key ? t(key) : label;
}

// ---------------------------------------------------------------------------
// Reading an attachment's text
// ---------------------------------------------------------------------------

type TextState = { kind: "loading" } | { kind: "ready"; text: string } | { kind: "failed" };

/** The text of an attachment, fetched once per id. */
function useAttachmentText(attachment: WsAttachment | null): TextState {
  const [state, setState] = React.useState<TextState>({ kind: "loading" });
  const id = attachment?.id ?? null;
  React.useEffect(() => {
    if (!id) return;
    let live = true;
    setState({ kind: "loading" });
    fetch(attachmentUrl({ id }), { cache: "force-cache" })
      .then((r) => (r.ok ? r.text() : Promise.reject(new Error(String(r.status)))))
      .then((text) => live && setState({ kind: "ready", text }))
      .catch(() => live && setState({ kind: "failed" }));
    return () => {
      live = false;
    };
  }, [id]);
  return state;
}

function Pending({ state, children }: { state: TextState; children: (text: string) => React.ReactNode }) {
  if (state.kind === "ready") return <>{children(state.text)}</>;
  return (
    <p className="hb-att-wait" role="status">
      {state.kind === "failed" ? t("readFailed") : t("loading")}
    </p>
  );
}

// ---------------------------------------------------------------------------
// Markdown
// ---------------------------------------------------------------------------

function Runs({ runs }: { runs: Inline[] }) {
  return (
    <>
      {runs.map((r, i) => {
        switch (r.t) {
          case "text":
            return <React.Fragment key={i}>{r.v}</React.Fragment>;
          case "code":
            return <code key={i}>{r.v}</code>;
          case "strong":
            return (
              <strong key={i}>
                <Runs runs={r.c} />
              </strong>
            );
          case "em":
            return (
              <em key={i}>
                <Runs runs={r.c} />
              </em>
            );
          case "link":
            return r.href ? (
              <a key={i} href={r.href} target="_blank" rel="noopener noreferrer nofollow">
                <Runs runs={r.c} />
              </a>
            ) : (
              <span key={i}>
                <Runs runs={r.c} />
              </span>
            );
        }
      })}
    </>
  );
}

function BlockView({ block }: { block: Block }) {
  switch (block.t) {
    case "heading": {
      const Tag = `h${Math.min(block.level + 1, 6)}` as "h2";
      return (
        <Tag>
          <Runs runs={block.c} />
        </Tag>
      );
    }
    case "paragraph":
      return (
        <p>
          <Runs runs={block.c} />
        </p>
      );
    case "list": {
      const items = block.items.map((item, i) => (
        <li key={i}>
          <Runs runs={item} />
        </li>
      ));
      return block.ordered ? <ol>{items}</ol> : <ul>{items}</ul>;
    }
    case "quote":
      return (
        <blockquote>
          <Runs runs={block.c} />
        </blockquote>
      );
    case "code":
      return (
        <pre className="hb-mono" data-lang={block.lang ?? undefined}>
          {block.v}
        </pre>
      );
    case "rule":
      return <hr />;
    case "table":
      return (
        <div className="hb-md-table">
          <table>
            <thead>
              <tr>
                {block.head.map((c, i) => (
                  <th key={i}>
                    <Runs runs={c} />
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((row, r) => (
                <tr key={r}>
                  {row.map((c, i) => (
                    <td key={i}>
                      <Runs runs={c} />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
  }
}

/** A Markdown report, rendered from data: no HTML in it ever reaches the page. */
export function MarkdownView({ source }: { source: string }) {
  const huge = source.length > MARKDOWN_MAX;
  const blocks = React.useMemo(() => (huge ? [] : parse(source)), [source, huge]);
  if (huge) {
    return (
      <div className="hb-md">
        <p>{t("reportTooLong")}</p>
        <TextView source={source} />
      </div>
    );
  }
  return (
    <div className="hb-md">
      {blocks.map((b, i) => (
        <BlockView key={i} block={b} />
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Diffs, logs, text
// ---------------------------------------------------------------------------

/** A unified diff with its add and remove colours. */
export function DiffView({ source, full: initialFull = false }: { source: string; full?: boolean }) {
  const [full, setFull] = React.useState(initialFull);
  const lines = diffLines(source);
  const shown = full ? lines : lines.slice(0, 60);
  return (
    <div>
      <div className="hb-diff hb-mono" role="figure" aria-label={t("changes")}>
        {shown.map((l, i) => (
          <div key={i} className="hb-diff-line" data-kind={l.kind}>
            {l.text || " "}
          </div>
        ))}
      </div>
      {lines.length > shown.length ? (
        <button type="button" className="hb-att-more" onClick={() => setFull(true)}>
          {t("showFull", { n: lines.length - shown.length })}
        </button>
      ) : null}
    </div>
  );
}

/** A log or a text file in monospace: the first lines, then "ver completo". */
export function TextView({ source, json = false, full: initialFull = false }: { source: string; json?: boolean; full?: boolean }) {
  const [full, setFull] = React.useState(initialFull);
  const text = json ? prettyJson(source) : source;
  const head = preview(text);
  return (
    <div>
      <pre className="hb-log hb-mono">{full ? text : head.text}</pre>
      {!full && head.more > 0 ? (
        <button type="button" className="hb-att-more" onClick={() => setFull(true)}>
          {t("showFull", { n: head.more })}
        </button>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Images: thumbnails, lightbox, before/after
// ---------------------------------------------------------------------------

/** One image, full size, over the page. Escape or the button closes it. */
export function Lightbox({ attachment, onClose }: { attachment: WsAttachment; onClose: () => void }) {
  const close = React.useRef<HTMLButtonElement>(null);
  React.useEffect(() => {
    close.current?.focus();
    const on = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, [onClose]);
  return (
    <div className="hb-lightbox" role="dialog" aria-modal="true" aria-label={attachment.name} onClick={onClose}>
      <figure onClick={(e) => e.stopPropagation()}>
        {/* eslint-disable-next-line @next/next/no-img-element -- served by harbard, not optimisable */}
        <img src={attachmentUrl(attachment)} alt={attachment.label ? `${shownLabel(attachment.label)}: ${attachment.name}` : attachment.name} />
        <figcaption>
          {attachment.label ? <strong>{shownLabel(attachment.label)} · </strong> : null}
          {attachment.name}
        </figcaption>
      </figure>
      <button ref={close} type="button" className="hb-lightbox-close" onClick={onClose} aria-label={t("close")}>
        <X className="size-6" aria-hidden />
      </button>
    </div>
  );
}

function Thumb({ attachment, onOpen }: { attachment: WsAttachment; onOpen: (a: WsAttachment) => void }) {
  return (
    <button type="button" className="hb-thumb" onClick={() => onOpen(attachment)} title={t("enlarge", { name: attachment.name })} data-attachment={attachment.id}>
      {/* eslint-disable-next-line @next/next/no-img-element -- served by harbard, not optimisable */}
      <img src={attachmentUrl(attachment)} alt={attachment.label ? `${shownLabel(attachment.label)}: ${attachment.name}` : attachment.name} loading="lazy" />
      {attachment.label ? <span className="hb-thumb-label">{shownLabel(attachment.label)}</span> : null}
    </button>
  );
}

export type CompareMode = "side" | "slider";

/** An before/after pair: side by side, or one over the other under a slider. */
export function Compare({
  before,
  after,
  mode: initialMode = "side",
  onOpen,
}: {
  before: WsAttachment;
  after: WsAttachment;
  mode?: CompareMode;
  onOpen?: (a: WsAttachment) => void;
}) {
  const [mode, setMode] = React.useState<CompareMode>(initialMode);
  const [at, setAt] = React.useState(50);
  return (
    <div className="hb-compare" data-mode={mode}>
      <div className="hb-compare-tabs" role="group" aria-label={t("compare")}>
        <button type="button" aria-pressed={mode === "side"} onClick={() => setMode("side")}>
          <Columns2 className="size-4" aria-hidden /> {t("sideBySide")}
        </button>
        <button type="button" aria-pressed={mode === "slider"} onClick={() => setMode("slider")}>
          <ChevronsLeftRight className="size-4" aria-hidden /> {t("slider")}
        </button>
      </div>
      {mode === "side" ? (
        <div className="hb-compare-side">
          {[before, after].map((a) => (
            <figure key={a.id}>
              <button type="button" onClick={() => onOpen?.(a)} title={t("enlarge", { name: a.name })}>
                {/* eslint-disable-next-line @next/next/no-img-element -- served by harbard, not optimisable */}
                <img src={attachmentUrl(a)} alt={`${a.label ? shownLabel(a.label) : ""}: ${a.name}`} />
              </button>
              <figcaption>{a.label === "before" ? t("before") : t("after")}</figcaption>
            </figure>
          ))}
        </div>
      ) : (
        <div className="hb-compare-slider">
          <div className="hb-compare-stage">
            {/* eslint-disable-next-line @next/next/no-img-element -- served by harbard, not optimisable */}
            <img src={attachmentUrl(before)} alt={`${t("labelBefore")}: ${before.name}`} />
            {/* eslint-disable-next-line @next/next/no-img-element -- served by harbard, not optimisable */}
            <img className="hb-compare-top" src={attachmentUrl(after)} alt={`${t("labelAfter")}: ${after.name}`} style={{ clipPath: `inset(0 0 0 ${at}%)` }} />
            <span className="hb-compare-handle" style={{ left: `${at}%` }} aria-hidden />
          </div>
          <div className="hb-compare-legend">
            <span>{t("before")}</span>
            <input
              type="range"
              min={0}
              max={100}
              value={at}
              onChange={(e) => setAt(Number(e.target.value))}
              aria-label={t("sliderHint")}
            />
            <span>{t("after")}</span>
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// The report pane (Markdown on the right)
// ---------------------------------------------------------------------------

const ReportPaneContext = React.createContext<((a: WsAttachment) => void) | null>(null);

/** The pane's content: a Markdown report, given its text. */
export function ReportPaneView({ attachment, source, onClose }: { attachment: WsAttachment; source: TextState; onClose: () => void }) {
  return (
    <aside className="hb-report-pane" aria-label={t("reportNamed", { name: attachment.name })}>
      <div className="hb-pane-head">
        <FileText className="size-5" aria-hidden />
        <span className="hb-pane-title truncate">{attachment.name}</span>
        {attachment.label ? <span className="hb-pane-sub">{shownLabel(attachment.label)}</span> : null}
        <button type="button" className="hb-close" onClick={onClose} aria-label={t("closeReport")}>
          <X className="size-5" />
        </button>
      </div>
      <div className="hb-report-body">
        <Pending state={source}>{(text) => <MarkdownView source={text} />}</Pending>
      </div>
    </aside>
  );
}

function ReportPane({ attachment, onClose }: { attachment: WsAttachment; onClose: () => void }) {
  const source = useAttachmentText(attachment);
  React.useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, [onClose]);
  return <ReportPaneView attachment={attachment} source={source} onClose={onClose} />;
}

/** Holds the open report; one at a time, over the right side of the shell. */
export function ReportPaneProvider({ children }: { children: React.ReactNode }) {
  const [open, setOpen] = React.useState<WsAttachment | null>(null);
  const close = React.useCallback(() => setOpen(null), []);
  return (
    <ReportPaneContext.Provider value={setOpen}>
      {children}
      {open ? <ReportPane key={open.id} attachment={open} onClose={close} /> : null}
    </ReportPaneContext.Provider>
  );
}

// ---------------------------------------------------------------------------
// The list in a message
// ---------------------------------------------------------------------------

function TextAttachment({ attachment, full }: { attachment: WsAttachment; full?: boolean }) {
  const state = useAttachmentText(attachment);
  return (
    <div className="hb-att-text">
      <div className="hb-att-name">
        {attachment.kind === "diff" ? <FileCode2 className="size-4" aria-hidden /> : <ScrollText className="size-4" aria-hidden />}
        {attachment.name}
        <span className="hb-att-size">{size(attachment.bytes)}</span>
      </div>
      <Pending state={state}>
        {(text) => (attachment.kind === "diff" ? <DiffView source={text} full={full} /> : <TextView source={text} json={attachment.kind === "json"} full={full} />)}
      </Pending>
    </div>
  );
}

/** Every attachment of one message. `lightbox` opens one image at first (tests, links). */
export function Attachments({ attachments, lightbox: initial = null }: { attachments: WsAttachment[]; lightbox?: string | null }) {
  const openReport = React.useContext(ReportPaneContext);
  const [big, setBig] = React.useState<WsAttachment | null>(() => attachments.find((a) => a.id === initial) ?? null);
  const close = React.useCallback(() => setBig(null), []);
  if (attachments.length === 0) return null;
  const pair = beforeAfter(attachments);
  const images = attachments.filter((a) => a.kind === "image" && a !== pair?.before && a !== pair?.after);
  const reports = attachments.filter((a) => a.kind === "markdown");
  const texts = attachments.filter((a) => a.kind === "diff" || a.kind === "text" || a.kind === "json");
  return (
    <div className="hb-atts">
      {pair ? <Compare before={pair.before} after={pair.after} onOpen={setBig} /> : null}
      {images.length > 0 ? (
        <div className="hb-thumbs">
          {images.map((a) => (
            <Thumb key={a.id} attachment={a} onOpen={setBig} />
          ))}
        </div>
      ) : null}
      {reports.map((a) => (
        <button key={a.id} type="button" className="hb-file hb-report-card" onClick={() => openReport?.(a)} title={t("readInPane", { name: a.name })}>
          <FileText className="size-6 shrink-0" strokeWidth={1.6} aria-hidden />
          <span className="min-w-0">
            <span className="block truncate">{a.name}</span>
            <span className="hb-file-sub truncate">{t("reportSummary", { size: size(a.bytes) })}</span>
          </span>
        </button>
      ))}
      {texts.map((a) => (
        <TextAttachment key={a.id} attachment={a} />
      ))}
      {big ? <Lightbox attachment={big} onClose={close} /> : null}
    </div>
  );
}
