/**
 * What leaves the TypeScript side: the catalog as data for the Rust validator
 * (`harbard` refuses a bad view at attach time) and as text for the agents
 * (`harbard huginn catalog`). Both are generated, committed, and pinned by a
 * test, so a catalog change cannot reach one side only.
 */

import { z } from "zod";
import { COMPONENT_NAMES, COMPONENTS, FAMILIES, LIMITS, definitionOf, fixedProps, type AnyDefinition, type Family } from "./catalog";

interface JsonSchema {
  readonly type?: string;
  readonly properties?: Readonly<Record<string, JsonSchema>>;
  readonly required?: readonly string[];
  readonly items?: JsonSchema;
  readonly enum?: readonly unknown[];
  readonly anyOf?: readonly JsonSchema[];
  readonly [keyword: string]: unknown;
}

function schemaOf(definition: AnyDefinition): JsonSchema {
  // The cast names what z.toJSONSchema returns: a draft-2020 schema object.
  const { $schema: _dialect, ...schema } = z.toJSONSchema(definition.props, { io: "input" }) as JsonSchema;
  return schema;
}

export function wireCatalog(): object {
  const components = Object.fromEntries(
    COMPONENT_NAMES.map((name) => {
      const definition: AnyDefinition = COMPONENTS[name];
      return [
        name,
        {
          family: definition.family,
          children: definition.children,
          insideForm: definition.insideForm ?? false,
          answers: definition.answers ?? false,
          files: definition.files ?? [],
          lengths: definition.lengths ?? [],
          fixed: fixedProps(definition),
          props: schemaOf(definition),
        },
      ];
    }),
  );
  return { version: 1, limits: LIMITS, components };
}

function typeOf(schema: JsonSchema): string {
  if (schema.enum) return schema.enum.map((value) => JSON.stringify(value)).join("|");
  if (schema.anyOf) return schema.anyOf.map(typeOf).join("|");
  if (schema.type === "array") return `${schema.items ? typeOf(schema.items) : "any"}[]`;
  if (schema.type === "object") return `{ ${propsLine(schema)} }`;
  if (schema["x-huginn-format"]) return String(schema["x-huginn-format"]);
  return schema.type === "integer" ? "int" : (schema.type ?? "any");
}

function propsLine(schema: JsonSchema): string {
  const required = new Set(schema.required ?? []);
  return Object.entries(schema.properties ?? {})
    .map(([name, prop]) => `${name}${required.has(name) ? "" : "?"}: ${typeOf(prop)}`)
    .join(", ");
}

const HEADER = `# Huginn: show the operator a view instead of a sentence

A view is a JSON file with the extension .huginn, attached to a note. The note's
text stays the message; the view renders under it in the console.

  harbard huginn check view.huginn
  harbard note "..." --attach view.huginn --attach chart.png

{ "root": "card", "elements": {
    "card":  { "type": "Card", "props": { "title": "Deploy" }, "children": ["state"] },
    "state": { "type": "Metric", "props": { "label": "Errors", "value": "0" } } } }

Rules: root plus a flat map of elements; each child has one parent. An unknown prop
is refused. There are no actions and no watchers.
Answers: Choice, Form and each *Field need an id. The note you get holds no text of
yours, only ids and numbers: "plan → option 2" is the second option of Choice plan;
"reason → text" is what the operator typed; "(proposal unchanged)" means your
defaultValue was left alone. Options count from 1.
Files: Image and BeforeAfter name files attached in the same command.
Limits: ${LIMITS.elements} elements, ${LIMITS.depth} levels, ${LIMITS.bytes} bytes.

State is optional and stays inside the view:
  { "root": "s", "state": { "env": "dev", "rows": [{ "name": "api", "ok": true }] },
    "elements": { "s": { "type": "Stack", "props": {}, "children": ["pick", "list"] },
      "pick": { "type": "Segmented", "props": { "state": "/env", "options": [
        { "label": "Dev", "value": "dev" }, { "label": "Prod", "value": "prod" }] } },
      "list": { "type": "Stack", "props": {}, "children": ["row"],
        "repeat": { "statePath": "/rows", "key": "name" },
        "visible": { "$item": "ok", "eq": true } },
      "row": { "type": "Text", "props": { "text": { "$template": "\${name} en \${/env}" } } } } }
A prop may be a whole expression: $state, $item, $index, $template, or
$cond/$then/$else. Not inside lists, and not in control, answer or file props.
visible hides an element: { "$state": "/x", "eq": 1 }, gt, gte, lt, lte, neq, not,
$and, $or. On a repeat it filters items. A *Field with state writes its value there,
so one field can show another. One repeat per branch, ${LIMITS.repeatElements} elements inside,
${LIMITS.repeatItems} items. Answer and control components cannot repeat.

Use a view when the shape carries meaning: a table, a figure, a decision. A short
answer stays text.
`;

export function agentGuide(): string {
  const families = Object.keys(FAMILIES) as Family[];
  const sections = families.map((family) => {
    const lines = COMPONENT_NAMES.filter((name) => definitionOf(name)?.family === family).map((name) => {
      const definition: AnyDefinition = COMPONENTS[name];
      const children = definition.children === "none" ? "" : " +children";
      return `  ${name} { ${propsLine(schemaOf(definition))} }${children}\n    ${definition.description}`;
    });
    return `## ${family}: ${FAMILIES[family]}\n${lines.join("\n")}`;
  });
  return `${HEADER}\n${sections.join("\n\n")}\n`;
}
