/**
 * The one door into a view. `harbard` checks a view when it is attached so
 * the agent hears about a mistake at once, and the console checks it again
 * here before rendering, because the store is a file anyone on the machine
 * can write. Nothing past this module validates a view again.
 *
 * json-render's own `catalog.validate` only checks component names: it let
 * through wrong prop types, unknown props, `javascript:` links and unknown
 * actions (checked on 0.22.0). That is why this exists.
 */

import { z } from "zod";
import { definitionOf, fixedProps, FORMATS, LIMITS, type AnyDefinition, type LengthRule } from "./catalog";
import { checkCondition, checkExpression, checkState, isExpression } from "./expression";

export type IssueCode =
  | "too_large"
  | "invalid_json"
  | "bad_envelope"
  | "too_many_elements"
  | "bad_key"
  | "unsupported_field"
  | "unknown_component"
  | "bad_props"
  | "hidden_characters"
  | "children_not_allowed"
  | "children_required"
  | "children_mismatch"
  | "length_mismatch"
  | "root_not_found"
  | "missing_child"
  | "shared_child"
  | "orphan"
  | "too_deep"
  | "outside_form"
  | "nested_answer"
  | "duplicate_answer_id"
  | "duplicate_field_id"
  | "bad_state"
  | "bad_expression"
  | "fixed_prop"
  | "bad_condition"
  | "bad_repeat"
  | "item_outside_repeat"
  | "nested_repeat"
  | "interactive_in_repeat"
  | "repeat_too_large";

export interface Issue {
  readonly code: IssueCode;
  /** The element it is about, when it is about one. */
  readonly element: string | null;
  readonly message: string;
}

export interface ViewRepeat {
  readonly statePath: string;
  readonly key?: string;
}

export interface ViewElement {
  readonly type: string;
  readonly props: Readonly<Record<string, unknown>>;
  readonly children: readonly string[];
  readonly visible?: unknown;
  readonly repeat?: ViewRepeat;
}

export interface ViewSpec {
  readonly root: string;
  readonly elements: Readonly<Record<string, ViewElement>>;
  readonly state?: Readonly<Record<string, unknown>>;
}

export type ViewParse = { readonly ok: true; readonly spec: ViewSpec } | { readonly ok: false; readonly issues: readonly Issue[] };

const ELEMENT_FIELDS = new Set(["type", "props", "children", "visible", "repeat"]);
const MAX_ISSUES = 20;

/**
 * The same characters serve refuses in a note (`brief::hidden_char`): format
 * and unassigned code points, variation selectors, and the blanks Unicode
 * does not class as format. A reply quoting such a label would be refused and
 * the operator could not see why.
 */
const HIDDEN = /[\p{Cf}\p{Cn}\uFE00-\uFE0F\u{E0100}-\u{E01EF}\u115F\u1160\u3164\uFFA0\u034F\u17B4\u17B5\u2800]/u;

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

function firstHidden(value: unknown): string | null {
  if (typeof value === "string") {
    const found = HIDDEN.exec(value);
    return found ? `U+${found[0].codePointAt(0)!.toString(16).toUpperCase().padStart(4, "0")}` : null;
  }
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = firstHidden(item);
      if (found) return found;
    }
    return null;
  }
  if (isRecord(value)) return firstHidden(Object.values(value));
  return null;
}

function lengthsAgree(rule: LengthRule, props: Readonly<Record<string, unknown>>): boolean {
  const target = props[rule.sameLengthAs];
  const list = props[rule.list];
  if (!Array.isArray(target) || !Array.isArray(list)) return false;
  if (rule.whole) return list.length === target.length;
  return list.every((item: unknown) => {
    const measured = rule.field && isRecord(item) ? item[rule.field] : item;
    return Array.isArray(measured) && measured.length === target.length;
  });
}

/** An element with the one fact about it the walk still needs. */
type CheckedElement = ViewElement & { readonly usesItem: boolean };

function checkElement(key: string, raw: unknown, issues: Issue[]): CheckedElement | null {
  const refuse = (code: IssueCode, message: string) => {
    issues.push({ code, element: key, message });
    return null;
  };
  if (!FORMATS.answer_id(key)) return refuse("bad_key", "an element key starts with a letter and holds letters, digits, - and _ (64 at most)");
  if (!isRecord(raw)) return refuse("bad_envelope", "an element is an object with type, props and children");
  const extra = Object.keys(raw).filter((field) => !ELEMENT_FIELDS.has(field));
  if (extra.length > 0) {
    return refuse("unsupported_field", `${extra.join(", ")} is not supported: a view has no actions or watchers; see harbard huginn catalog`);
  }
  if (typeof raw.type !== "string") return refuse("bad_envelope", "type must be a component name");
  const definition = definitionOf(raw.type);
  if (!definition) return refuse("unknown_component", `${raw.type} is not in the catalog; see harbard huginn catalog`);
  if (!isRecord(raw.props)) return refuse("bad_props", `${raw.type}: props must be an object`);
  const fixed = fixedProps(definition);
  const dynamic = Object.entries(raw.props).filter(([name, value]) => name in definition.props.shape && isExpression(value));
  let usesItem = false;
  for (const [name, value] of dynamic) {
    if (fixed === "all" || fixed.includes(name)) return refuse("fixed_prop", `${raw.type}.${name} must be a plain value`);
    const checked = checkExpression(value);
    if (!checked.ok) return refuse("bad_expression", `${raw.type}.${name}: ${checked.why}`);
    usesItem ||= checked.usesItem;
  }
  const dynamicNames = new Set(dynamic.map(([name]) => name));
  const literal = Object.fromEntries(Object.entries(raw.props).filter(([name]) => !dynamicNames.has(name)));
  const schema = dynamicNames.size > 0 ? z.strictObject(Object.fromEntries(Object.entries(definition.props.shape).filter(([name]) => !dynamicNames.has(name)))) : definition.props;
  const parsed = schema.safeParse(literal);
  if (!parsed.success) {
    const first = parsed.error.issues[0];
    return refuse("bad_props", `${raw.type}.${first?.path.join(".") ?? ""}: ${first?.message ?? "invalid"}`);
  }
  const props = { ...(parsed.data as Record<string, unknown>), ...Object.fromEntries(dynamic) };
  const hidden = firstHidden(props) ?? firstHidden(raw.visible);
  if (hidden) return refuse("hidden_characters", `${raw.type} carries a character you cannot see (${hidden})`);
  const children = raw.children ?? [];
  if (!Array.isArray(children) || !children.every((child): child is string => typeof child === "string")) {
    return refuse("bad_envelope", "children must be a list of element keys");
  }
  if (!checkChildRule(definition, raw.type, props, children, refuse)) return null;
  for (const rule of definition.lengths ?? []) {
    if (dynamicNames.has(rule.list) || dynamicNames.has(rule.sameLengthAs)) continue;
    if (!lengthsAgree(rule, props)) {
      return refuse("length_mismatch", `${raw.type}.${rule.list} must match ${rule.sameLengthAs} in length`);
    }
  }
  let visibleUsesItem = false;
  if (raw.visible !== undefined) {
    const checked = checkCondition(raw.visible);
    if (!checked.ok) return refuse("bad_condition", `${raw.type}.visible: ${checked.why}`);
    visibleUsesItem = checked.usesItem;
  }
  const repeat = checkRepeat(raw.repeat, definition);
  if (repeat === "bad") return refuse("bad_repeat", `${raw.type}: repeat is { statePath, key? } on a component that takes children`);
  const element: CheckedElement = { type: raw.type, props, children, usesItem: usesItem || (visibleUsesItem && repeat === null) };
  return {
    ...element,
    ...(raw.visible === undefined ? {} : { visible: raw.visible }),
    ...(repeat === null ? {} : { repeat }),
  };
}

function checkRepeat(raw: unknown, definition: AnyDefinition): ViewRepeat | null | "bad" {
  if (raw === undefined) return null;
  if (!isRecord(raw) || Object.keys(raw).some((key) => key !== "statePath" && key !== "key") || definition.children !== "some") return "bad";
  if (typeof raw.statePath !== "string" || !FORMATS.state_path(raw.statePath)) return "bad";
  if (raw.key === undefined) return { statePath: raw.statePath };
  if (typeof raw.key !== "string" || !FORMATS.state_path(`/${raw.key}`)) return "bad";
  return { statePath: raw.statePath, key: raw.key };
}

function checkChildRule(
  definition: AnyDefinition,
  type: string,
  props: Readonly<Record<string, unknown>>,
  children: readonly string[],
  refuse: (code: IssueCode, message: string) => null,
): boolean {
  const rule = definition.children;
  if (rule === "none") {
    if (children.length === 0) return true;
    refuse("children_not_allowed", `${type} takes no children`);
    return false;
  }
  if (rule === "some") {
    if (children.length > 0) return true;
    refuse("children_required", `${type} needs at least one child`);
    return false;
  }
  const expected = props[rule.sameLengthAs];
  if (Array.isArray(expected) && expected.length === children.length) return true;
  refuse("children_mismatch", `${type} needs one child per item of ${rule.sameLengthAs}`);
  return false;
}

interface WalkState {
  readonly elements: ReadonlyMap<string, CheckedElement>;
  readonly seen: Set<string>;
  readonly answerIds: Set<string>;
  readonly issues: Issue[];
}

function countBelow(element: ViewElement, elements: ReadonlyMap<string, ViewElement>): number {
  return element.children.reduce((total, key) => {
    const child = elements.get(key);
    return child ? total + 1 + countBelow(child, elements) : total;
  }, 0);
}

function walk(
  key: string,
  state: WalkState,
  context: { depth: number; insideAnswer: boolean; formFields: Set<string> | null; inRepeat: boolean },
): void {
  const element = state.elements.get(key);
  if (!element) return;
  state.seen.add(key);
  const issue = (code: IssueCode, message: string) => state.issues.push({ code, element: key, message });
  const definition = definitionOf(element.type);
  if (context.depth > LIMITS.depth) {
    issue("too_deep", `a view nests ${LIMITS.depth} levels at most`);
    return;
  }
  if (element.usesItem && !context.inRepeat) issue("item_outside_repeat", "$item and $index only work inside a repeat");
  if (definition && context.inRepeat && fixedProps(definition) === "all") issue("interactive_in_repeat", `${element.type} cannot be repeated: it needs a place of its own`);
  if (element.repeat) {
    if (context.inRepeat) issue("nested_repeat", "a repeat cannot sit inside another");
    if (countBelow(element, state.elements) > LIMITS.repeatElements) issue("repeat_too_large", `a repeat holds ${LIMITS.repeatElements} elements at most`);
  }
  if (definition?.insideForm) {
    const id = String(element.props.id);
    if (!context.formFields) issue("outside_form", `${element.type} only works inside a Form`);
    else if (context.formFields.has(id)) issue("duplicate_field_id", `two fields of one Form use the id ${id}`);
    else context.formFields.add(id);
  }
  if (definition?.answers) {
    if (context.insideAnswer) issue("nested_answer", `${element.type} cannot sit inside another Form`);
    const id = String(element.props.id);
    if (state.answerIds.has(id)) issue("duplicate_answer_id", `two components answer as ${id}`);
    state.answerIds.add(id);
  }
  const next = {
    depth: context.depth + 1,
    insideAnswer: context.insideAnswer || Boolean(definition?.answers),
    formFields: element.type === "Form" ? new Set<string>() : context.formFields,
    inRepeat: context.inRepeat || Boolean(element.repeat),
  };
  for (const child of element.children) walk(child, state, next);
}

/**
 * Children are checked before the walk so the walk may assume a tree: each
 * key has one parent, and nothing points back at the root.
 */
function checkShape(root: string, elements: ReadonlyMap<string, CheckedElement>, issues: Issue[]): boolean {
  if (!elements.has(root)) {
    issues.push({ code: "root_not_found", element: root, message: `root ${root} is not among the elements` });
    return false;
  }
  const parents = new Map<string, string>();
  let sound = true;
  for (const [key, element] of elements) {
    for (const child of element.children) {
      if (!elements.has(child)) {
        issues.push({ code: "missing_child", element: key, message: `child ${child} is not among the elements` });
        sound = false;
      } else if (parents.has(child) || child === root) {
        issues.push({ code: "shared_child", element: child, message: `${child} has more than one parent, or is the root and a child` });
        sound = false;
      } else {
        parents.set(child, key);
      }
    }
  }
  return sound;
}

/** Parse the text of an attached view. Every outcome is a value; this never throws. */
export function parseView(source: string): ViewParse {
  const fail = (code: IssueCode, message: string): ViewParse => ({ ok: false, issues: [{ code, element: null, message }] });
  if (new TextEncoder().encode(source).length > LIMITS.bytes) return fail("too_large", `a view holds ${LIMITS.bytes} bytes at most`);
  let raw: unknown;
  try {
    raw = JSON.parse(source);
  } catch {
    return fail("invalid_json", "a view is one JSON document");
  }
  if (!isRecord(raw) || typeof raw.root !== "string" || !isRecord(raw.elements) || Object.keys(raw).some((field) => field !== "root" && field !== "elements" && field !== "state")) {
    return fail("bad_envelope", "a view is { root, elements, state? } and nothing else");
  }
  if (raw.state !== undefined) {
    if (!isRecord(raw.state)) return fail("bad_state", "state is an object");
    const problem = checkState(raw.state);
    if (problem) return fail("bad_state", problem);
    const hidden = firstHidden(raw.state);
    if (hidden) return fail("hidden_characters", `state carries a character you cannot see (${hidden})`);
  }
  const entries = Object.entries(raw.elements);
  if (entries.length === 0) return fail("bad_envelope", "a view has at least one element");
  if (entries.length > LIMITS.elements) return fail("too_many_elements", `a view holds ${LIMITS.elements} elements at most`);

  const issues: Issue[] = [];
  const elements = new Map<string, CheckedElement>();
  for (const [key, element] of entries) {
    const checked = checkElement(key, element, issues);
    if (checked) elements.set(key, checked);
  }
  if (issues.length > 0) return { ok: false, issues: issues.slice(0, MAX_ISSUES) };

  if (!checkShape(raw.root, elements, issues)) return { ok: false, issues: issues.slice(0, MAX_ISSUES) };
  const state: WalkState = { elements, seen: new Set(), answerIds: new Set(), issues };
  walk(raw.root, state, { depth: 1, insideAnswer: false, formFields: null, inRepeat: false });
  for (const key of elements.keys()) {
    if (!state.seen.has(key)) issues.push({ code: "orphan", element: key, message: `${key} is not reachable from the root` });
  }
  if (issues.length > 0) return { ok: false, issues: issues.slice(0, MAX_ISSUES) };
  const plain = [...elements].map(([key, { usesItem: _usesItem, ...element }]) => [key, element] as const);
  return { ok: true, spec: { root: raw.root, elements: Object.fromEntries(plain), ...(raw.state === undefined ? {} : { state: raw.state as Record<string, unknown> }) } };
}

/**
 * Props after the renderer resolved their expressions. State can hold anything
 * the view put there, so a value read from it passes the same schema a literal
 * does. `null` means the props are fine.
 */
export function resolvedPropsProblem(type: string, props: Readonly<Record<string, unknown>>): string | null {
  const definition = definitionOf(type);
  if (!definition) return `${type} is not in the catalog`;
  const parsed = definition.props.safeParse(props);
  if (!parsed.success) return parsed.error.issues[0]?.message ?? "invalid";
  const hidden = firstHidden(props);
  if (hidden) return `a character you cannot see (${hidden})`;
  return (definition.lengths ?? []).every((rule) => lengthsAgree(rule, props)) ? null : "lists of different lengths";
}
