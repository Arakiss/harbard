import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WsAttachment } from "@/lib/workspace";
import { Attachments, Compare, DiffView, Lightbox, MarkdownView, ReportPaneView, TextView } from "./attachments";
import { TechDetails } from "./parts";

// Synthetic attachments only: ids and names as `harbard attach` writes them.
function att(id: string, kind: WsAttachment["kind"], label: WsAttachment["label"] = null): WsAttachment {
  return { id, name: id.split("/")[1]!, kind, label, bytes: 2048, path: `/h/.harbard/attachments/${id}` };
}
const SHOT = att("01K9Z8Y7X6W5V4T3S2R1Q0P9N8/capture.png", "image");
const BEFORE = att("01K9Z8Y7X6W5V4T3S2R1Q0P9N1/before.png", "image", "before");
const AFTER = att("01K9Z8Y7X6W5V4T3S2R1Q0P9N2/after.png", "image", "after");
const REPORT = att("01K9Z8Y7X6W5V4T3S2R1Q0P9N3/report.md", "markdown", "report");

const html = (el: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(el);

describe("images", () => {
  test("an image is an inline thumbnail served through the console's proxy", () => {
    const out = html(h(Attachments, { attachments: [SHOT] }));
    expect(out).toContain('class="hb-thumb"');
    expect(out).toContain('src="/api/harbard/attachments/01K9Z8Y7X6W5V4T3S2R1Q0P9N8/capture.png"');
    expect(out).not.toContain('role="dialog"');
  });

  test("a click opens the lightbox: the full image over the page, closable", () => {
    const out = html(h(Attachments, { attachments: [SHOT], lightbox: SHOT.id }));
    expect(out).toContain('class="hb-lightbox"');
    expect(out).toContain('role="dialog"');
    expect(out).toContain('aria-modal="true"');
    expect(out).toContain('aria-label="Close"');
    const alone = html(h(Lightbox, { attachment: BEFORE, onClose: () => {} }));
    expect(alone).toContain('alt="before: before.png"');
    expect(alone).toContain("<strong>before · </strong>before.png");
  });

  test("a before and an after show side by side, and under a slider", () => {
    const out = html(h(Attachments, { attachments: [AFTER, BEFORE] }));
    expect(out).toContain('class="hb-compare" data-mode="side"');
    expect(out).toContain("Side by side");
    expect(out).toContain("Slider");
    // Antes first, whatever the order in the message.
    expect(out.indexOf("before.png")).toBeLessThan(out.indexOf("after.png"));
    expect(out).toContain("<figcaption>Before</figcaption>");
    expect(out).toContain("<figcaption>After</figcaption>");
    // The pair is not repeated as thumbnails.
    expect(out).not.toContain('class="hb-thumb"');
    const slider = html(h(Compare, { before: BEFORE, after: AFTER, mode: "slider" }));
    expect(slider).toContain('type="range"');
    expect(slider).toContain('aria-label="Slide to go from before to after"');
    expect(slider).toContain("clip-path:inset(0 0 0 50%)");
  });
});

describe("reports and text", () => {
  test("a Markdown report is a card that opens the pane on the right", () => {
    const out = html(h(Attachments, { attachments: [REPORT] }));
    expect(out).toContain("hb-report-card");
    expect(out).toContain("report.md");
    expect(out).toContain("read here");
  });

  test("the pane renders the report, sanitised: a script tag in it never runs", () => {
    const source = [
      "# Console report",
      "",
      "All **good**, with `code` and a [link](https://example.com).",
      "",
      "<script>alert('x')</script>",
      "",
      "<img src=x onerror=alert(1)>",
      "",
      "[bad](javascript:alert(1)) ![photo](https://tracker.example/p.png)",
      "",
      "| case | state |",
      "| --- | --- |",
      "| before | broken |",
    ].join("\n");
    const out = html(h(ReportPaneView, { attachment: REPORT, source: { kind: "ready", text: source }, onClose: () => {} }));
    expect(out).toContain('class="hb-report-pane"');
    expect(out).toContain("<h2>Console report</h2>");
    expect(out).toContain("<strong>good</strong>");
    expect(out).toContain("<code>code</code>");
    expect(out).toContain('href="https://example.com"');
    expect(out).toContain("&lt;script&gt;alert(&#x27;x&#x27;)&lt;/script&gt;");
    expect(out).not.toContain("<script");
    expect(out).not.toContain("<img");
    expect(out).not.toContain("javascript:");
    expect(out).not.toContain("tracker.example");
    expect(out).toContain("<th>case</th>");
    expect(html(h(MarkdownView, { source: "<b onclick=x>hello</b>" }))).toBe('<div class="hb-md"><p>&lt;b onclick=x&gt;hello&lt;/b&gt;</p></div>');
  });

  test("a diff shows additions and removals in their colours", () => {
    const out = html(h(DiffView, { source: "--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-old\n+new\n same" }));
    expect(out).toContain('data-kind="file">--- a/x');
    expect(out).toContain('data-kind="hunk">@@ -1,2 +1,2 @@');
    expect(out).toContain('data-kind="del">-old');
    expect(out).toContain('data-kind="add">+new');
    expect(out).toContain('data-kind="context"> same');
  });

  test("a log is monospace, its first lines first, with ver completo", () => {
    const log = Array.from({ length: 30 }, (_, i) => `line ${i + 1}`).join("\n");
    const out = html(h(TextView, { source: log }));
    expect(out).toContain('class="hb-log hb-mono"');
    expect(out).toContain("line 24");
    expect(out).not.toContain("line 25");
    expect(out).toContain("Show everything (6 more lines)");
    const full = html(h(TextView, { source: log, full: true }));
    expect(full).toContain("line 30");
    expect(full).not.toContain("Show everything");
    expect(html(h(TextView, { source: '{"a":1}', json: true }))).toContain("{\n  &quot;a&quot;: 1\n}");
  });
});

describe("the technical body", () => {
  test("sits under a collapsed Technical details with the file cards", () => {
    const out = html(h(TechDetails, { body: "Rebased onto main; head 1582702.", files: h("span", { className: "card" }, "x.md") }));
    expect(out).toContain('<details class="hb-tech"><summary>Technical details</summary>');
    expect(out).not.toContain("<details open");
    expect(out).toContain("Rebased onto main; head 1582702.");
    expect(out).toContain('<span class="card">x.md</span>');
  });
});
