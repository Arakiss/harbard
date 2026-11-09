/**
 * Huginn's catalog: every component an agent may put in a view.
 *
 * Huginn is the raven that flies out and brings Odin the news; a view is what
 * an agent brings the operator when a sentence would not do. Each entry is
 * the whole contract of one component: its props, what it may contain, and
 * the one-line description an agent reads in `harbard huginn catalog`.
 *
 * Every string a view carries is written by an agent and is untrusted, so
 * props are strict objects (an unknown prop is a refusal) and every list and
 * string is bounded.
 */

import { z } from "zod";

/**
 * How a value that JSON Schema cannot express is checked. The name travels
 * in the wire schema so the Rust validator applies the same rule.
 */
export const FORMATS = {
  http_url: (value: string) => /^https?:\/\/[^\s]+$/i.test(value),
  file_name: (value: string) => /^[A-Za-z0-9_-][A-Za-z0-9._-]{0,99}$/.test(value),
  iso_date: (value: string) => /^\d{4}-\d{2}-\d{2}$/.test(value),
  answer_id: (value: string) => /^[A-Za-z][A-Za-z0-9_-]{0,63}$/.test(value),
  // Every segment starts with a letter or digit, which already rules out
  // __proto__; the other two names would still reach Object.prototype.
  state_path: (value: string) =>
    /^(\/[A-Za-z0-9][A-Za-z0-9_-]*)+$/.test(value) && value.length <= 128 && !value.split("/").some((segment) => segment === "constructor" || segment === "prototype"),
} as const;

export type FormatName = keyof typeof FORMATS;

const formatted = (format: FormatName, max: number) =>
  z
    .string()
    .max(max)
    .refine(FORMATS[format], { message: `not a valid ${format}` })
    .meta({ "x-huginn-format": format });

const label = z.string().min(1).max(200);
const text = z.string().min(1).max(4000);
const longText = z.string().min(1).max(40_000);
const shortList = <T extends z.ZodType>(item: T) => z.array(item).min(1).max(40);
const longList = <T extends z.ZodType>(item: T) => z.array(item).min(1).max(500);
const tone = z.enum(["neutral", "accent", "success", "warning", "danger", "info"]);
const gap = z.enum(["sm", "md", "lg"]);
const finite = z.number().min(-1e15).max(1e15);
const series = z.strictObject({ name: label, values: longList(finite.nullable()) });

/** What an element may contain. */
export type ChildRule = "none" | "some" | { readonly sameLengthAs: string };

/** A list prop whose items must be as long as another list prop. */
export interface LengthRule {
  /** The list whose items are measured, or the list itself with no `field`. */
  readonly list: string;
  /** Measure `item[field]` instead of the item. */
  readonly field?: string;
  /** Measure the list prop itself instead of each item. */
  readonly whole?: boolean;
  readonly sameLengthAs: string;
}

export type Family = "layout" | "text" | "data" | "chart" | "work" | "control" | "answer";

export const FAMILIES: Readonly<Record<Family, string>> = {
  layout: "arrange other components",
  text: "words, code and links",
  data: "figures, tables and states",
  chart: "plots (at most 8 series)",
  work: "diffs, files, terminals, images attached with the view",
  control: "change the view's own state; nothing leaves the view",
  answer: "ask the operator; the answer comes back as a note",
};

interface Definition<S extends z.ZodObject> {
  readonly family: Family;
  readonly description: string;
  readonly children: ChildRule;
  readonly props: S;
  /** Only valid somewhere inside a Form. */
  readonly insideForm?: true;
  /** Produces an answer; its `id` prop names that answer inside the view. */
  readonly answers?: true;
  /** Props that name a file attached together with the view. */
  readonly files?: readonly string[];
  readonly lengths?: readonly LengthRule[];
}

const stateRef = formatted("state_path", 128);

const define = <S extends z.ZodObject>(definition: Definition<S>): Definition<S> => definition;

const fieldId = formatted("answer_id", 64);
const fieldBase = { id: fieldId, label, hint: label.optional(), required: z.boolean().optional(), state: stateRef.optional() };
const fieldOptions = shortList(label);

export const COMPONENTS = {
  Stack: define({
    family: "layout",
    description: "Children one after another.",
    children: "some",
    props: z.strictObject({
      direction: z.enum(["vertical", "horizontal"]).optional(),
      gap: gap.optional(),
      align: z.enum(["start", "center", "end", "stretch"]).optional(),
      wrap: z.boolean().optional(),
    }),
  }),
  Grid: define({
    family: "layout",
    description: "Children in equal columns that collapse on a narrow pane.",
    children: "some",
    props: z.strictObject({ columns: z.number().int().min(1).max(4), gap: gap.optional() }),
  }),
  Section: define({
    family: "layout",
    description: "A titled part of a longer view.",
    children: "some",
    props: z.strictObject({ title: label, description: text.optional() }),
  }),
  Card: define({
    family: "layout",
    description: "A bordered box; tone colours its edge.",
    children: "some",
    props: z.strictObject({ title: label.optional(), subtitle: label.optional(), tone: tone.optional() }),
  }),
  Tabs: define({
    family: "layout",
    description: "One tab per child, in order; labels and children must match in number.",
    children: { sameLengthAs: "labels" },
    props: z.strictObject({ labels: z.array(label).min(2).max(8) }),
  }),
  Disclosure: define({
    family: "layout",
    description: "Content folded behind a label: detail most readers skip.",
    children: "some",
    props: z.strictObject({ label, open: z.boolean().optional() }),
  }),
  Separator: define({
    family: "layout",
    description: "A horizontal rule.",
    children: "none",
    props: z.strictObject({}),
  }),

  Heading: define({
    family: "text",
    description: "A heading.",
    children: "none",
    props: z.strictObject({ text: label, level: z.number().int().min(1).max(3).optional() }),
  }),
  Text: define({
    family: "text",
    description: "A paragraph of plain text.",
    children: "none",
    props: z.strictObject({
      text,
      tone: z.enum(["default", "muted", "success", "warning", "danger"]).optional(),
      size: z.enum(["sm", "md", "lg"]).optional(),
      strong: z.boolean().optional(),
    }),
  }),
  Markdown: define({
    family: "text",
    description: "Headings, lists, tables, code and links in Markdown. Raw HTML stays text.",
    children: "none",
    props: z.strictObject({ source: longText }),
  }),
  Callout: define({
    family: "text",
    description: "A note that must not be missed.",
    children: "none",
    props: z.strictObject({ tone, title: label.optional(), text }),
  }),
  Quote: define({
    family: "text",
    description: "Someone's words, with who said them.",
    children: "none",
    props: z.strictObject({ text, cite: label.optional() }),
  }),
  Badge: define({
    family: "text",
    description: "A short state word.",
    children: "none",
    props: z.strictObject({ text: label, tone: tone.optional() }),
  }),
  KeyValue: define({
    family: "text",
    description: "Label and value pairs.",
    children: "none",
    props: z.strictObject({ items: shortList(z.strictObject({ label, value: text })) }),
  }),
  List: define({
    family: "text",
    description: "A bulleted or numbered list of short lines.",
    children: "none",
    props: z.strictObject({ items: longList(text), ordered: z.boolean().optional() }),
  }),
  Link: define({
    family: "text",
    description: "An http or https link, opened outside the console.",
    children: "none",
    props: z.strictObject({ href: formatted("http_url", 2048), label }),
  }),
  Code: define({
    family: "text",
    description: "A block of code in monospace.",
    children: "none",
    props: z.strictObject({ code: longText, language: label.optional(), filename: label.optional() }),
  }),

  Metric: define({
    family: "data",
    description: "One headline figure. Prefer it to a chart with one number.",
    children: "none",
    props: z.strictObject({
      label,
      value: label,
      delta: label.optional(),
      deltaTone: z.enum(["neutral", "success", "danger"]).optional(),
      hint: label.optional(),
    }),
  }),
  Table: define({
    family: "data",
    description: "Rows and columns; each row has one cell per column. sortable adds sorting by column.",
    children: "none",
    props: z.strictObject({
      caption: label.optional(),
      columns: shortList(z.strictObject({ label, align: z.enum(["start", "end"]).optional() })),
      rows: longList(z.array(z.union([z.string().max(4000), finite, z.null()]))),
      sortable: z.boolean().optional(),
    }),
    lengths: [{ list: "rows", sameLengthAs: "columns" }],
  }),
  Progress: define({
    family: "data",
    description: "How far along something is.",
    children: "none",
    props: z.strictObject({
      label,
      value: finite,
      max: finite.positive().optional(),
      tone: tone.optional(),
      hint: label.optional(),
    }),
  }),
  Timeline: define({
    family: "data",
    description: "What happened, in order.",
    children: "none",
    props: z.strictObject({
      items: longList(z.strictObject({ time: label.optional(), title: label, detail: text.optional(), tone: tone.optional() })),
    }),
  }),
  Steps: define({
    family: "data",
    description: "A sequence with where it stands now.",
    children: "none",
    props: z.strictObject({
      items: shortList(z.strictObject({ label, state: z.enum(["done", "current", "todo", "failed"]), detail: text.optional() })),
    }),
  }),
  Checklist: define({
    family: "data",
    description: "Checks with their verdicts.",
    children: "none",
    props: z.strictObject({
      items: longList(z.strictObject({ label, state: z.enum(["pass", "fail", "warn", "skip"]), detail: text.optional() })),
    }),
  }),

  BarChart: define({
    family: "chart",
    description: "Compare amounts across categories.",
    children: "none",
    props: z.strictObject({
      title: label,
      labels: shortList(label),
      series: z.array(series).min(1).max(8),
      unit: label.optional(),
      stacked: z.boolean().optional(),
    }),
    lengths: [{ list: "series", field: "values", sameLengthAs: "labels" }],
  }),
  LineChart: define({
    family: "chart",
    description: "Change over an ordered axis; null leaves a gap.",
    children: "none",
    props: z.strictObject({
      title: label,
      labels: longList(label),
      series: z.array(series).min(1).max(8),
      unit: label.optional(),
      area: z.boolean().optional(),
    }),
    lengths: [{ list: "series", field: "values", sameLengthAs: "labels" }],
  }),
  Sparkline: define({
    family: "chart",
    description: "A small inline trend beside a label.",
    children: "none",
    props: z.strictObject({ label, values: longList(finite), unit: label.optional() }),
  }),
  Donut: define({
    family: "chart",
    description: "Parts of a whole; at most 6 parts.",
    children: "none",
    props: z.strictObject({
      title: label,
      segments: z.array(z.strictObject({ label, value: finite.nonnegative() })).min(1).max(6),
      unit: label.optional(),
    }),
  }),
  Heatmap: define({
    family: "chart",
    description: "A matrix of amounts, darker for more.",
    children: "none",
    props: z.strictObject({
      title: label,
      rows: shortList(label),
      columns: shortList(label),
      values: z.array(z.array(finite.nullable())).min(1).max(40),
      unit: label.optional(),
    }),
    lengths: [
      { list: "values", sameLengthAs: "columns" },
      { list: "values", whole: true, sameLengthAs: "rows" },
    ],
  }),

  Diff: define({
    family: "work",
    description: "A unified diff with its colours.",
    children: "none",
    props: z.strictObject({ source: longText, filename: label.optional() }),
  }),
  FileTree: define({
    family: "work",
    description: "Paths and what happened to each.",
    children: "none",
    props: z.strictObject({
      entries: longList(
        z.strictObject({
          path: z.string().min(1).max(500),
          status: z.enum(["added", "modified", "deleted", "renamed"]).optional(),
          note: label.optional(),
        }),
      ),
    }),
  }),
  Terminal: define({
    family: "work",
    description: "A command and what it printed.",
    children: "none",
    props: z.strictObject({
      command: z.string().min(1).max(2000).optional(),
      output: z.string().max(40_000),
      exitCode: z.number().int().min(-1).max(255).optional(),
    }),
  }),
  Image: define({
    family: "work",
    description: "An image attached in the same command, by file name.",
    children: "none",
    props: z.strictObject({ file: formatted("file_name", 100), alt: label, caption: label.optional() }),
    files: ["file"],
  }),
  BeforeAfter: define({
    family: "work",
    description: "Two attached images to compare, side by side or under a slider.",
    children: "none",
    props: z.strictObject({
      before: formatted("file_name", 100),
      after: formatted("file_name", 100),
      mode: z.enum(["side", "slider"]).optional(),
    }),
    files: ["before", "after"],
  }),

  Segmented: define({
    family: "control",
    description: "Pick one value for state/path: tabs of a filter.",
    children: "none",
    props: z.strictObject({
      state: stateRef,
      label: label.optional(),
      options: z.array(z.strictObject({ label, value: z.string().max(200) })).min(2).max(8),
    }),
  }),
  Switch: define({
    family: "control",
    description: "Turn a boolean at state/path on or off.",
    children: "none",
    props: z.strictObject({ state: stateRef, label }),
  }),

  Choice: define({
    family: "answer",
    description: "One question with options. A click on an option is the answer; multiple adds a send button.",
    children: "none",
    answers: true,
    props: z.strictObject({
      id: formatted("answer_id", 64),
      question: label,
      detail: text.optional(),
      multiple: z.boolean().optional(),
      options: z
        .array(z.strictObject({ label, detail: label.optional(), tone: z.enum(["neutral", "primary", "danger"]).optional() }))
        .min(2)
        .max(12),
    }),
  }),
  Form: define({
    family: "answer",
    description: "Fields answered together. Its children hold the *Field components.",
    children: "some",
    answers: true,
    props: z.strictObject({
      id: formatted("answer_id", 64),
      title: label,
      detail: text.optional(),
      submitLabel: label.optional(),
    }),
  }),
  TextField: define({
    family: "answer",
    description: "Free text; multiline for a paragraph.",
    children: "none",
    insideForm: true,
    props: z.strictObject({
      ...fieldBase,
      placeholder: label.optional(),
      multiline: z.boolean().optional(),
      defaultValue: z.string().max(1000).optional(),
    }),
  }),
  NumberField: define({
    family: "answer",
    description: "A number within bounds.",
    children: "none",
    insideForm: true,
    props: z.strictObject({
      ...fieldBase,
      min: finite.optional(),
      max: finite.optional(),
      step: finite.positive().optional(),
      unit: label.optional(),
      defaultValue: finite.optional(),
    }),
  }),
  SelectField: define({
    family: "answer",
    description: "One of many options, in a menu.",
    children: "none",
    insideForm: true,
    props: z.strictObject({ ...fieldBase, options: fieldOptions, defaultValue: label.optional() }),
  }),
  RadioField: define({
    family: "answer",
    description: "One of a few options, all visible.",
    children: "none",
    insideForm: true,
    props: z.strictObject({ ...fieldBase, options: fieldOptions, defaultValue: label.optional() }),
  }),
  CheckboxField: define({
    family: "answer",
    description: "Yes or no.",
    children: "none",
    insideForm: true,
    props: z.strictObject({ id: fieldId, label, hint: label.optional(), state: stateRef.optional(), defaultChecked: z.boolean().optional() }),
  }),
  MultiSelectField: define({
    family: "answer",
    description: "Any number of options.",
    children: "none",
    insideForm: true,
    props: z.strictObject({ ...fieldBase, options: fieldOptions, defaultValue: z.array(label).max(40).optional() }),
  }),
  DateField: define({
    family: "answer",
    description: "A day; values are YYYY-MM-DD.",
    children: "none",
    insideForm: true,
    props: z.strictObject({ ...fieldBase, defaultValue: formatted("iso_date", 10).optional() }),
  }),
  SliderField: define({
    family: "answer",
    description: "A value on a range.",
    children: "none",
    insideForm: true,
    props: z.strictObject({
      id: fieldId,
      label,
      hint: label.optional(),
      state: stateRef.optional(),
      min: finite,
      max: finite,
      step: finite.positive().optional(),
      unit: label.optional(),
      defaultValue: finite.optional(),
    }),
  }),
} as const;

export type ComponentName = keyof typeof COMPONENTS;

/** Props of one component, as a registry receives them. */
export type PropsOf<N extends ComponentName> = z.output<(typeof COMPONENTS)[N]["props"]>;

/** Every definition behind one loose type, for code that walks the whole catalog. */
export interface AnyDefinition {
  readonly family: Family;
  readonly description: string;
  readonly children: ChildRule;
  readonly props: z.ZodObject;
  readonly insideForm?: true;
  readonly answers?: true;
  readonly files?: readonly string[];
  readonly lengths?: readonly LengthRule[];
}

/**
 * Props that must be literal. Answer and control components are static whole,
 * so an answer can be read back against the view; a file name must be known
 * when the view is attached; Tabs needs its labels to count its children.
 */
export function fixedProps(definition: AnyDefinition): "all" | readonly string[] {
  if (definition.answers || definition.insideForm || definition.family === "control") return "all";
  const counted = typeof definition.children === "object" ? [definition.children.sameLengthAs] : [];
  return [...(definition.files ?? []), ...counted];
}

export function definitionOf(type: string): AnyDefinition | null {
  return Object.hasOwn(COMPONENTS, type) ? COMPONENTS[type as ComponentName] : null;
}

export const COMPONENT_NAMES = Object.keys(COMPONENTS) as ComponentName[];

/** The bounds of a whole view, so a hostile one cannot freeze the message list. */
export const LIMITS = {
  bytes: 256 * 1024,
  elements: 400,
  depth: 12,
  expressionDepth: 6,
  stateDepth: 8,
  repeatItems: 200,
  repeatElements: 30,
} as const;
