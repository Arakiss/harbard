/**
 * The dynamic part of a view: expressions in props, conditions in `visible`,
 * and the initial `state`. It is a closed subset of what json-render accepts.
 * `$computed` is left out on purpose (it calls named functions), as are
 * `$bindState`, `on` and `watch`: state changes only through the controls and
 * fields a view contains, and it never leaves the view.
 */

import { FORMATS, LIMITS } from "./catalog";

export type Checked = { readonly ok: true; readonly usesItem: boolean } | { readonly ok: false; readonly why: string };

const fine = (usesItem = false): Checked => ({ ok: true, usesItem });
const bad = (why: string): Checked => ({ ok: false, why });
const both = (a: Checked, b: Checked): Checked => (!a.ok ? a : !b.ok ? b : fine(a.usesItem || b.usesItem));

const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
const isScalar = (value: unknown) => value === null || ["string", "number", "boolean"].includes(typeof value);

/** Anything else with a `$` key is refused later, by the prop's own schema. */
export const isExpression = (value: unknown): value is Record<string, unknown> => isRecord(value) && Object.keys(value).some((key) => key.startsWith("$"));

const isStatePath = (value: unknown) => typeof value === "string" && FORMATS.state_path(value);
const isItemPath = (value: unknown) => typeof value === "string" && (value === "" || FORMATS.state_path(`/${value}`));
const onlyKey = (value: Record<string, unknown>) => Object.keys(value).length === 1;

const COMPARISONS = new Set(["eq", "neq", "gt", "gte", "lt", "lte", "not"]);

function checkSingle(value: unknown): Checked {
  if (!isRecord(value)) return bad("a condition is an object");
  const sources = ["$state", "$item", "$index"].filter((key) => key in value);
  if (sources.length !== 1) return bad("a condition reads one of $state, $item or $index");
  const source = sources[0]!;
  if (source === "$state" && !isStatePath(value.$state)) return bad("$state must be a path like /form/plan");
  if (source === "$item" && !isItemPath(value.$item)) return bad("$item must be a field of the item");
  if (source === "$index" && value.$index !== true) return bad("$index must be true");
  for (const [key, operand] of Object.entries(value)) {
    if (key === source) continue;
    if (!COMPARISONS.has(key)) return bad(`${key} is not a comparison`);
    const isRef = isRecord(operand) && onlyKey(operand) && isStatePath(operand.$state);
    if (key === "not" ? operand !== true : key === "eq" || key === "neq" ? !isScalar(operand) && !isRef : typeof operand !== "number" && !isRef) {
      return bad(`${key} has a value it cannot compare`);
    }
  }
  return fine(source !== "$state");
}

export function checkCondition(value: unknown, depth = 1): Checked {
  if (depth > LIMITS.expressionDepth) return bad("a condition nests too deep");
  if (typeof value === "boolean") return fine();
  if (Array.isArray(value)) {
    if (value.length === 0) return bad("a list of conditions cannot be empty");
    return value.map(checkSingle).reduce(both);
  }
  if (isRecord(value) && onlyKey(value) && ("$and" in value || "$or" in value)) {
    const list = value.$and ?? value.$or;
    if (!Array.isArray(list) || list.length === 0) return bad("$and and $or take a list of conditions");
    return list.map((item) => checkCondition(item, depth + 1)).reduce(both);
  }
  return checkSingle(value);
}

export function checkExpression(value: unknown, depth = 1): Checked {
  if (depth > LIMITS.expressionDepth) return bad("an expression nests too deep");
  if (!isRecord(value)) return bad("an expression is an object");
  if (onlyKey(value) && "$state" in value) return isStatePath(value.$state) ? fine() : bad("$state must be a path like /form/plan");
  if (onlyKey(value) && "$item" in value) return isItemPath(value.$item) ? fine(true) : bad("$item must be a field of the item");
  if (onlyKey(value) && "$index" in value) return value.$index === true ? fine(true) : bad("$index must be true");
  if (onlyKey(value) && "$template" in value) {
    const template = value.$template;
    if (typeof template !== "string" || template.length > 4000) return bad("$template must be a string");
    return fine(/\$\{(?!\/)/.test(template));
  }
  if ("$cond" in value && "$then" in value && "$else" in value && Object.keys(value).length === 3) {
    const branch = (operand: unknown) => (isScalar(operand) ? fine() : checkExpression(operand, depth + 1));
    return [checkCondition(value.$cond, depth + 1), branch(value.$then), branch(value.$else)].reduce(both);
  }
  return bad(`only $state, $item, $index, $template and $cond are allowed, not ${Object.keys(value).join(", ")}`);
}

/** The initial state: plain JSON, bounded, with keys a path can name. */
export function checkState(value: unknown, depth = 1): string | null {
  if (depth > LIMITS.stateDepth) return "state nests too deep";
  if (Array.isArray(value)) {
    if (value.length > LIMITS.repeatItems) return `a list in state holds ${LIMITS.repeatItems} items at most`;
    for (const item of value) {
      const problem = checkState(item, depth + 1);
      if (problem) return problem;
    }
    return null;
  }
  if (!isRecord(value)) return null;
  for (const [key, item] of Object.entries(value)) {
    if (!FORMATS.state_path(`/${key}`)) return `${key} cannot be a key in state`;
    const problem = checkState(item, depth + 1);
    if (problem) return problem;
  }
  return null;
}
