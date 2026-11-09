/**
 * What goes back to the agent when the operator answers a view.
 *
 * An answer carries no text the agent wrote: an option travels as its number
 * and a field as its id and what the operator typed. A view cannot word a
 * button one way and answer another, and nothing an agent put on screen can
 * reach another agent inside a note signed by the operator. The ids are
 * identifiers, too short and too narrow to carry a sentence.
 *
 * The console turns the note back into words against the view it answers
 * (`describeAnswer`), and the first line names that view and component. An
 * answer is an operator note replying to the view's message, so the bus is
 * the only record that a view was answered.
 */

import { definitionOf } from "./catalog";
import type { ViewElement, ViewSpec } from "./parse";

export interface AnswerLine {
  /** The id of the Choice or field. */
  readonly field: string;
  readonly value: string;
}

export interface Answer {
  readonly file: string;
  readonly id: string;
  readonly lines: readonly AnswerLine[];
}

/** One line of an answer as the operator reads it. */
export interface DescribedLine {
  readonly label: string;
  readonly value: string;
}

const HEADER = "Answer · ";
/** Answers stored by older builds open with this header and stay readable. */
const OLD_HEADER = "Respuesta · ";
const NO_VALUE = "(no answer)";
const ARROW = " → ";
export const UNCHANGED = "(proposal unchanged)";
const OLD_UNCHANGED = "(propuesta sin cambios)";
export const YES = "yes";
export const NO = "no";

/** How a set of picked options travels: their 1-based numbers, never their text. */
export function optionsAnswer(indexes: readonly number[]): string {
  const numbers = [...indexes].sort((a, b) => a - b).map((index) => index + 1);
  return numbers.length === 1 ? `option ${numbers[0]}` : `options ${numbers.join(", ")}`;
}

/** Keeps what the operator typed on one line, so it cannot forge another answer line. */
const oneLine = (text: string) => text.replace(/\s+/g, " ").trim();

export function composeAnswer(answer: Answer): string {
  const body = answer.lines.map((line) => `${line.field}${ARROW}${oneLine(line.value) || NO_VALUE}`);
  return [`${HEADER}${answer.file}#${answer.id}`, ...body].join("\n");
}

/** serve stores a reply to an agent as `Para <name>: <text>`. */
const ADDRESSED = /^(?:Para|To) [^\n]*?: (?=(?:Answer|Respuesta) · )/;

/** `null` for every note that is not an answer to a view. */
export function readAnswer(note: string): Answer | null {
  const text = note.replace(ADDRESSED, "");
  const opening = [HEADER, OLD_HEADER].find((candidate) => text.startsWith(candidate));
  if (!opening) return null;
  const [header = "", ...body] = text.split("\n");
  const hash = header.lastIndexOf("#");
  if (hash <= opening.length) return null;
  const lines = body.flatMap((line): AnswerLine[] => {
    const split = line.indexOf(ARROW);
    return split === -1 ? [] : [{ field: line.slice(0, split), value: line.slice(split + ARROW.length) }];
  });
  return { file: header.slice(opening.length, hash), id: header.slice(hash + 1), lines };
}

function picked(value: string, options: readonly string[]): string | null {
  // Answers stored by older builds say "opción" and stay readable.
  const match = /^(?:options?|opci(?:ón|ones)) ([\d, ]+)$/.exec(value);
  if (!match) return null;
  const labels = match[1]!.split(", ").map((number) => options[Number(number) - 1]);
  return labels.every((label) => label !== undefined) ? labels.join(", ") : null;
}

function descendants(spec: ViewSpec, element: ViewElement): ViewElement[] {
  return element.children.flatMap((key) => {
    const child = spec.elements[key];
    return child ? [child, ...descendants(spec, child)] : [];
  });
}

const stringList = (value: unknown): readonly string[] => (Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : []);

function describeField(field: ViewElement, value: string): string {
  if ((value === UNCHANGED || value === OLD_UNCHANGED) && typeof field.props.defaultValue === "string") return field.props.defaultValue;
  return picked(value, stringList(field.props.options)) ?? value;
}

/** An answer in the words of its view; ids stand in for anything the view no longer has. */
export function describeAnswer(spec: ViewSpec, answer: Answer): DescribedLine[] {
  const owner = Object.values(spec.elements).find((element) => definitionOf(element.type)?.answers && element.props.id === answer.id);
  if (!owner) return answer.lines.map((line) => ({ label: line.field, value: line.value }));
  if (owner.type === "Choice") {
    const options = Array.isArray(owner.props.options) ? owner.props.options.map((option: { label: string }) => option.label) : [];
    return answer.lines.map((line) => ({ label: String(owner.props.question), value: picked(line.value, options) ?? line.value }));
  }
  const fields = descendants(spec, owner).filter((element) => definitionOf(element.type)?.insideForm);
  return answer.lines.map((line) => {
    const field = fields.find((element) => element.props.id === line.field);
    return field ? { label: String(field.props.label), value: describeField(field, line.value) } : { label: line.field, value: line.value };
  });
}

export const SEND_LIMIT_CHARS = 1800;
