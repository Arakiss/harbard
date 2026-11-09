"use client";

/**
 * A message's views, rendered through json-render with Huginn's registry.
 *
 * Every view is parsed again here (`parseView`), whatever `harbard` checked
 * when it was attached: this is the wall in front of the DOM. What a view
 * cannot show is said on screen, and the message's text stays readable.
 */

import * as React from "react";
import { defineRegistry, JSONUIProvider, Renderer, type ComponentRegistry } from "@json-render/react";
import type { Spec } from "@json-render/react";
import { COMPONENTS, describeAnswer, huginnCatalog, parseView, readAnswer, resolvedPropsProblem, type Answer, type ComponentName, type DescribedLine, type Issue, type PropsOf, type ViewParse, type ViewSpec } from "@harbard/huginn";
import { attachmentUrl, messageById, type WsAttachment, type WsMessage, type WorkspaceView } from "@/lib/workspace";
import { Attachments } from "../workspace/attachments";
import { useOptionalView } from "../workspace/workspace-context";
import { AnswerContext, Choice, CheckboxField, DateField, Form, MultiSelectField, NumberField, RadioField, SelectField, SliderField, TextField, type AnswerScope, type GivenAnswer } from "./answers";
import { BarChart, Donut, Heatmap, LineChart, Sparkline } from "./charts";
import * as Display from "./display";
import { createT } from "@/i18n";

const t = createT("huginn");

type RegistryMap = { readonly [N in ComponentName]: React.ComponentType<PropsOf<N> & { children?: React.ReactNode }> };

/** Exhaustive by type: a component added to the catalog fails the build until it is here. */
const IMPLEMENTATIONS: RegistryMap = {
  Stack: Display.Stack,
  Grid: Display.Grid,
  Section: Display.Section,
  Card: Display.Card,
  Tabs: Display.Tabs,
  Disclosure: Display.Disclosure,
  Separator: Display.Separator,
  Heading: Display.Heading,
  Text: Display.Text,
  Markdown: Display.Markdown,
  Callout: Display.Callout,
  Quote: Display.Quote,
  Badge: Display.Badge,
  KeyValue: Display.KeyValue,
  List: Display.List,
  Link: Display.Link,
  Code: Display.Code,
  Metric: Display.Metric,
  Table: Display.Table,
  Progress: Display.Progress,
  Timeline: Display.Timeline,
  Steps: Display.Steps,
  Checklist: Display.Checklist,
  BarChart,
  LineChart,
  Sparkline,
  Donut,
  Heatmap,
  Diff: Display.Diff,
  FileTree: Display.FileTree,
  Terminal: Display.Terminal,
  Image: Display.Image,
  BeforeAfter: Display.BeforeAfter,
  Segmented: Display.Segmented,
  Switch: Display.Switch,
  Choice,
  Form,
  TextField,
  NumberField,
  SelectField,
  RadioField,
  CheckboxField,
  MultiSelectField,
  DateField,
  SliderField,
};

function wrap(name: ComponentName): React.ComponentType<{ props: Record<string, unknown>; children?: React.ReactNode }> {
  // Checked just below against this component's own schema; the cast bridges
  // json-render's untyped element props to that fact.
  const Implementation = IMPLEMENTATIONS[name] as React.ComponentType<Record<string, unknown>>;
  return function HuginnElement({ props, children }) {
    // The props are resolved by now, and a value read from state has had no other check.
    const problem = React.useMemo(() => resolvedPropsProblem(name, props), [props]);
    if (problem) return <p className="hb-huginn-error">{t("componentInvalid")}</p>;
    return <Implementation {...props}>{children}</Implementation>;
  };
}

const { registry } = defineRegistry(huginnCatalog, {
  // json-render types this map from the catalog's generics, which our runtime-
  // built catalog widens; `RegistryMap` above is what keeps it exhaustive.
  components: Object.fromEntries((Object.keys(COMPONENTS) as ComponentName[]).map((name) => [name, wrap(name)])) as never,
}) as { registry: ComponentRegistry };

function asRenderable(spec: ViewSpec): Spec {
  return {
    root: spec.root,
    elements: Object.fromEntries(
      Object.entries(spec.elements).map(([key, element]) => [
        key,
        {
          type: element.type,
          props: { ...element.props },
          children: [...element.children],
          // `parseView` admitted only conditions and repeats json-render reads;
          // the cast bridges its generic element type to that fact.
          ...(element.visible === undefined ? {} : { visible: element.visible as never }),
          ...(element.repeat ? { repeat: element.repeat } : {}),
        },
      ]),
    ),
    ...(spec.state ? { state: { ...spec.state } } : {}),
  };
}

/** Answers already on the bus, by the id of the component they answer. */
export function answersOf(file: string, spec: ViewSpec, message: WsMessage, view: WorkspaceView | null): ReadonlyMap<string, GivenAnswer> {
  const given = new Map<string, GivenAnswer>();
  if (!view) return given;
  for (const id of message.replies) {
    const reply = messageById(view, id);
    if (reply?.from !== "operator") continue;
    const answer = readAnswer(reply.text);
    if (answer?.file === file) given.set(answer.id, { lines: describeAnswer(spec, answer), tsMs: reply.ts_ms });
  }
  return given;
}

function RefusedView({ file, issues }: { file: string; issues: readonly Issue[] }) {
  return (
    <div className="hb-huginn-refused" role="status">
      <p>{t("viewRefused")}</p>
      <details>
        <summary>{t("why")}</summary>
        <ul className="hb-mono">
          {issues.map((issue, index) => (
            <li key={index}>
              {file}: {issue.code}
              {issue.element ? ` (${issue.element})` : ""}: {issue.message}
            </li>
          ))}
        </ul>
      </details>
    </div>
  );
}

function RenderedView({ spec, scope, files }: { spec: ViewSpec; scope: AnswerScope; files: readonly WsAttachment[] }) {
  const renderable = React.useMemo(() => asRenderable(spec), [spec]);
  return (
    <section className="hb-huginn" aria-label={t("viewAria")}>
      <Display.FilesContext.Provider value={files}>
        <AnswerContext.Provider value={scope}>
          <JSONUIProvider registry={registry} initialState={spec.state ? { ...spec.state } : undefined}>
            <Renderer spec={renderable} registry={registry} />
          </JSONUIProvider>
        </AnswerContext.Provider>
      </Display.FilesContext.Provider>
    </section>
  );
}

/** One view, given its text. */
export function HuginnView({ file, source, message, files = [] }: { file: string; source: string; message: WsMessage | null; files?: readonly WsAttachment[] }) {
  const view = useOptionalView();
  const parsed = React.useMemo(() => parseView(source), [source]);
  if (!parsed.ok) return <RefusedView file={file} issues={parsed.issues} />;
  const target = message && message.from !== "operator" ? { agent: message.agent, replyTo: message.id, author: message.author } : null;
  const scope: AnswerScope = { file, spec: parsed.spec, target, given: message ? answersOf(file, parsed.spec, message, view) : new Map() };
  return <RenderedView spec={parsed.spec} scope={scope} files={files} />;
}

type Loaded = { readonly id: string; readonly name: string; readonly parsed: ViewParse; readonly source: string };

function useViews(views: readonly WsAttachment[]): { loaded: readonly Loaded[]; failed: readonly string[]; pending: boolean } {
  const key = views.map((view) => view.id).join("\n");
  const [state, setState] = React.useState<{ key: string; loaded: readonly Loaded[]; failed: readonly string[] } | null>(null);
  React.useEffect(() => {
    if (!key) return;
    let live = true;
    const list = key.split("\n");
    void Promise.all(
      list.map(async (id): Promise<Loaded | string> => {
        try {
          const response = await fetch(attachmentUrl({ id }), { cache: "force-cache" });
          if (!response.ok) return id;
          const source = await response.text();
          return { id, name: id.split("/")[1] ?? id, source, parsed: parseView(source) };
        } catch {
          return id;
        }
      }),
    ).then((results) => {
      if (live) setState({ key, loaded: results.filter((r): r is Loaded => typeof r !== "string"), failed: results.filter((r): r is string => typeof r === "string") });
    });
    return () => {
      live = false;
    };
  }, [key]);
  const current = state?.key === key ? state : null;
  return { loaded: current?.loaded ?? [], failed: current?.failed ?? [], pending: key !== "" && current === null };
}

/** The file names a view shows itself, so they are not shown twice. */
function filesNamedBy(spec: ViewSpec): Set<string> {
  const named = new Set<string>();
  for (const element of Object.values(spec.elements)) {
    const definition = Object.hasOwn(COMPONENTS, element.type) ? (COMPONENTS[element.type as ComponentName] as { files?: readonly string[] }) : null;
    for (const prop of definition?.files ?? []) {
      const value = element.props[prop];
      if (typeof value === "string") named.add(value);
    }
  }
  return named;
}

/** Everything attached to one message: its views first, then the other files. */
export function MessageAttachments({ message }: { message: WsMessage }) {
  const attachments = React.useMemo(() => message.attachments ?? [], [message.attachments]);
  const views = React.useMemo(() => attachments.filter((a) => a.kind === "huginn"), [attachments]);
  const { loaded, failed, pending } = useViews(views);
  if (views.length === 0) return <Attachments attachments={attachments} />;
  const named = new Set(loaded.flatMap((view) => (view.parsed.ok ? [...filesNamedBy(view.parsed.spec)] : [])));
  // While the views load, their images are held back so none flashes as a thumbnail.
  const rest = attachments.filter((a) => a.kind !== "huginn" && !named.has(a.name) && !(pending && a.kind === "image"));
  return (
    <>
      {loaded.map((view) => (
        <HuginnView key={view.id} file={view.name} source={view.source} message={message} files={attachments} />
      ))}
      {failed.length > 0 ? <p className="hb-att-wait" role="status">{t("viewUnreadable")}</p> : null}
      {pending ? <p className="hb-att-wait" role="status">{t("viewLoading")}</p> : null}
      <Attachments attachments={rest} />
    </>
  );
}

const rawLines = (answer: Answer): readonly DescribedLine[] => answer.lines.map((line) => ({ label: line.field, value: line.value }));

/** The operator's answer as it reads in the conversation: the note itself only holds ids. */
function useAnswerWords(answer: Answer, message: WsMessage): readonly DescribedLine[] {
  const view = useOptionalView();
  const parent = view ? (view.messages.find((candidate) => candidate.replies.includes(message.id)) ?? null) : null;
  const id = parent?.attachments?.find((attachment) => attachment.kind === "huginn" && attachment.name === answer.file)?.id ?? null;
  const [lines, setLines] = React.useState<readonly DescribedLine[]>(() => rawLines(answer));
  // `answer` is rebuilt on every render of the message; its content is what matters.
  const content = JSON.stringify(answer);
  React.useEffect(() => {
    if (!id) return;
    let live = true;
    void fetch(attachmentUrl({ id }), { cache: "force-cache" })
      .then((response) => (response.ok ? response.text() : Promise.reject(new Error(String(response.status)))))
      .then((source) => {
        const parsed = parseView(source);
        if (live && parsed.ok) setLines(describeAnswer(parsed.spec, answer));
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `content` is `answer`
  }, [id, content]);
  return lines;
}

export function AnswerEcho({ answer, message }: { answer: Answer; message: WsMessage }) {
  const lines = useAnswerWords(answer, message);
  return (
    <dl className="hb-huginn-echo">
      {lines.map((line, index) => (
        <div key={index}>
          <dt>{line.label}</dt>
          <dd>{line.value}</dd>
        </div>
      ))}
    </dl>
  );
}

/** The note as an answer to a view, when it is one and the operator wrote it. */
export function operatorAnswer(message: WsMessage): Answer | null {
  return message.from === "operator" ? readAnswer(message.text) : null;
}
