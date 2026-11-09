/**
 * A small Markdown reader for agent reports, sanitised by construction: it
 * returns plain data (blocks of inline runs), never HTML. The renderer turns
 * that data into React elements, so every character of the report reaches
 * the page as text: a `<script>` in a report is shown, never run, and no
 * raw HTML passes through.
 *
 * What it reads: headings, paragraphs, bullet and numbered lists, quotes,
 * fenced code, rules and pipe tables; inline code, bold, italics and links.
 * A link keeps its URL only for http(s) and mailto; an image is its alt
 * text (a report never makes the console fetch anything).
 */

export type Inline =
  | { t: "text"; v: string }
  | { t: "code"; v: string }
  | { t: "strong"; c: Inline[] }
  | { t: "em"; c: Inline[] }
  | { t: "link"; href: string | null; c: Inline[] };

export type Block =
  | { t: "heading"; level: 1 | 2 | 3 | 4 | 5 | 6; c: Inline[] }
  | { t: "paragraph"; c: Inline[] }
  | { t: "list"; ordered: boolean; items: Inline[][] }
  | { t: "quote"; c: Inline[] }
  | { t: "code"; lang: string | null; v: string }
  | { t: "rule" }
  | { t: "table"; head: Inline[][]; rows: Inline[][][] };

/** A link target the console may follow: http(s) or mailto, else null. */
export function safeHref(raw: string): string | null {
  const url = raw.trim();
  return /^(https?:\/\/|mailto:)/i.test(url) ? url : null;
}

/** Longest report parsed as Markdown; past it the pane shows it as text. */
export const MARKDOWN_MAX = 1_000_000;

/**
 * `indexOf` that remembers: the next `token` at or after `from`. A paragraph
 * full of openers with no closer would otherwise rescan to its end for each
 * one; with the last answer kept per token it is linear.
 */
function nextFinder(text: string): (token: string, from: number) => number {
  const memo = new Map<string, { from: number; at: number }>();
  return (token, from) => {
    const m = memo.get(token);
    if (m && from >= m.from && (m.at === -1 || m.at >= from)) return m.at;
    const at = text.indexOf(token, from);
    memo.set(token, { from, at });
    return at;
  };
}

/** Inline runs of one line or paragraph. */
export function inline(text: string): Inline[] {
  const next = nextFinder(text);
  const out: Inline[] = [];
  let buf = "";
  const flush = () => {
    if (buf) out.push({ t: "text", v: buf });
    buf = "";
  };
  let i = 0;
  while (i < text.length) {
    const ch = text[i]!;
    if (ch === "\\" && i + 1 < text.length && /[\\`*_[\]()#!>-]/.test(text[i + 1]!)) {
      buf += text[i + 1];
      i += 2;
      continue;
    }
    if (ch === "`") {
      const end = next("`", i + 1);
      if (end > i) {
        flush();
        out.push({ t: "code", v: text.slice(i + 1, end) });
        i = end + 1;
        continue;
      }
    }
    if ((ch === "*" || ch === "_") && text[i + 1] === ch) {
      const end = next(ch + ch, i + 2);
      if (end > i + 2) {
        flush();
        out.push({ t: "strong", c: inline(text.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    }
    if (ch === "*" || (ch === "_" && (i === 0 || /\W/.test(text[i - 1]!)))) {
      const end = next(ch, i + 1);
      if (end > i + 1 && text[i + 1] !== " ") {
        flush();
        out.push({ t: "em", c: inline(text.slice(i + 1, end)) });
        i = end + 1;
        continue;
      }
    }
    const image = ch === "!" && text[i + 1] === "[";
    if (ch === "[" || image) {
      const open = image ? i + 1 : i;
      const close = next("]", open + 1);
      if (close > open && text[close + 1] === "(") {
        const end = next(")", close + 2);
        if (end > close) {
          flush();
          const label = text.slice(open + 1, close);
          if (image) out.push({ t: "text", v: label || "image" });
          else out.push({ t: "link", href: safeHref(text.slice(close + 2, end)), c: inline(label) });
          i = end + 1;
          continue;
        }
      }
    }
    buf += ch;
    i += 1;
  }
  flush();
  return out;
}

function cells(line: string): string[] {
  let row = line.trim();
  if (row.startsWith("|")) row = row.slice(1);
  if (row.endsWith("|")) row = row.slice(0, -1);
  return row.split("|").map((c) => c.trim());
}

const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const DIVIDER = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

/** The blocks of a Markdown document. */
export function parse(source: string): Block[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let para: string[] = [];
  const endPara = () => {
    if (para.length) blocks.push({ t: "paragraph", c: inline(para.join(" ")) });
    para = [];
  };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i]!;
    const fence = line.match(/^\s*(```|~~~)\s*([\w+-]*)/);
    if (fence) {
      endPara();
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !lines[i]!.trim().startsWith(fence[1]!)) {
        body.push(lines[i]!);
        i += 1;
      }
      blocks.push({ t: "code", lang: fence[2] || null, v: body.join("\n") });
      i += 1;
      continue;
    }
    if (!line.trim()) {
      endPara();
      i += 1;
      continue;
    }
    const heading = line.match(/^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/);
    if (heading) {
      endPara();
      blocks.push({ t: "heading", level: heading[1]!.length as 1 | 2 | 3 | 4 | 5 | 6, c: inline(heading[2]!) });
      i += 1;
      continue;
    }
    if (RULE.test(line)) {
      endPara();
      blocks.push({ t: "rule" });
      i += 1;
      continue;
    }
    if (line.includes("|") && i + 1 < lines.length && DIVIDER.test(lines[i + 1]!)) {
      endPara();
      const head = cells(line).map(inline);
      const rows: Inline[][][] = [];
      i += 2;
      while (i < lines.length && lines[i]!.includes("|") && lines[i]!.trim()) {
        rows.push(cells(lines[i]!).map(inline));
        i += 1;
      }
      blocks.push({ t: "table", head, rows });
      continue;
    }
    if (/^\s{0,3}>/.test(line)) {
      endPara();
      const body: string[] = [];
      while (i < lines.length && /^\s{0,3}>/.test(lines[i]!)) {
        body.push(lines[i]!.replace(/^\s{0,3}>\s?/, ""));
        i += 1;
      }
      blocks.push({ t: "quote", c: inline(body.join(" ")) });
      continue;
    }
    const item = line.match(/^\s{0,3}([-*+]|\d{1,3}[.)])\s+(.*)$/);
    if (item) {
      endPara();
      const ordered = /\d/.test(item[1]!);
      const items: Inline[][] = [];
      while (i < lines.length) {
        const m = lines[i]!.match(/^\s{0,3}([-*+]|\d{1,3}[.)])\s+(.*)$/);
        if (m && /\d/.test(m[1]!) === ordered) {
          items.push(inline(m[2]!));
          i += 1;
        } else if (lines[i]!.trim() && /^\s{2,}\S/.test(lines[i]!) && items.length) {
          // A continuation line of the last item.
          items[items.length - 1] = [...items[items.length - 1]!, { t: "text", v: ` ${lines[i]!.trim()}` }];
          i += 1;
        } else break;
      }
      blocks.push({ t: "list", ordered, items });
      continue;
    }
    para.push(line.trim());
    i += 1;
  }
  endPara();
  return blocks;
}

/** The lines of a unified diff, by what they do. */
export type DiffLine = { kind: "add" | "del" | "hunk" | "file" | "context"; text: string };

export function diffLines(source: string): DiffLine[] {
  return source
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((text): DiffLine => {
      if (text.startsWith("+++") || text.startsWith("---") || text.startsWith("diff ") || text.startsWith("index ")) return { kind: "file", text };
      if (text.startsWith("@@")) return { kind: "hunk", text };
      if (text.startsWith("+")) return { kind: "add", text };
      if (text.startsWith("-")) return { kind: "del", text }; // i18n-exempt: a CSS data-kind, not a word.
      return { kind: "context", text };
    });
}

/** Lines shown before "ver completo". */
const PREVIEW_LINES = 24;

/** The first lines of a text, and whether there is more. */
export function preview(source: string, lines = PREVIEW_LINES): { text: string; more: number } {
  const all = source.replace(/\r\n?/g, "\n").replace(/\n$/, "").split("\n");
  return { text: all.slice(0, lines).join("\n"), more: Math.max(0, all.length - lines) };
}

/** JSON pretty-printed when it parses, as it is otherwise. */
export function prettyJson(source: string): string {
  try {
    return JSON.stringify(JSON.parse(source), null, 2);
  } catch {
    return source;
  }
}
