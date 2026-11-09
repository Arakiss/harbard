"use client";

/**
 * The components that ask the operator. What they send holds ids, numbers and
 * what the operator typed (`composeAnswer`), never a word the agent wrote. An
 * answer is an ordinary operator note replying to the view's message, so it
 * is the bus that remembers a view was answered.
 */

import * as React from "react";
import { Check } from "lucide-react";
import { Radio } from "@base-ui/react/radio";
import { RadioGroup } from "@base-ui/react/radio-group";
import { Slider } from "@base-ui/react/slider";
import { useStateStore } from "@json-render/react";
import { NO, SEND_LIMIT_CHARS, UNCHANGED, YES, composeAnswer, describeAnswer, optionsAnswer, type AnswerLine, type DescribedLine, type PropsOf, type ViewSpec } from "@harbard/huginn";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { WorkspaceRefused } from "@/lib/api";
import { invisibleRefusal } from "@/lib/invisible";
import { replyBody, replyError } from "@/lib/workspace";
import { useOptionalWorkspace } from "../workspace/workspace-context";
import { formatNumber } from "./scale";
import { createT, intlTag } from "@/i18n";
import { operatorTimeZone } from "@/lib/time-zone";

const t = createT("huginn");

export interface GivenAnswer {
  readonly lines: readonly DescribedLine[];
  readonly tsMs: number | null;
}

export interface AnswerScope {
  readonly file: string;
  readonly spec: ViewSpec | null;
  /** Who gets the answer; `null` when this message cannot be answered. */
  readonly target: { readonly agent: string; readonly replyTo: string; readonly author: string } | null;
  readonly given: ReadonlyMap<string, GivenAnswer>;
}

export const AnswerContext = React.createContext<AnswerScope>({ file: "", spec: null, target: null, given: new Map() });

const clock = (tsMs: number) => new Date(tsMs).toLocaleTimeString(intlTag(), { hour: "2-digit", minute: "2-digit", timeZone: operatorTimeZone() });

function Answered({ title, given }: { title: string; given: GivenAnswer }) {
  return (
    <div className="hb-huginn-answered" role="status">
      <p>
        <Check className="size-4" aria-hidden />
        <strong>{title}</strong>
        <span>{given.tsMs === null ? t("answered") : t("answeredAt", { time: clock(given.tsMs) })}</span>
      </p>
      <dl>
        {given.lines.map((line) => (
          <div key={line.label}>
            <dt>{line.label}</dt>
            <dd>{line.value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

type Sent = { readonly state: "idle" } | { readonly state: "sending" } | { readonly state: "failed"; readonly why: string };

function refusalText(error: unknown): string {
  if (error instanceof WorkspaceRefused) {
    // By here api.ts has already put the invisible-characters refusal in the
    // operator's language, so it is recognised by the code points it names.
    return invisibleRefusal(error.message) ?? (/U\+[0-9A-F]{4,6}/.test(error.message) ? error.message : replyError(error.status, error.reason));
  }
  return replyError(undefined, undefined);
}

/** Send one answer, and remember it locally until the bus shows the note. */
function useAnswer(id: string) {
  const scope = React.useContext(AnswerContext);
  const workspace = useOptionalWorkspace();
  const [sent, setSent] = React.useState<Sent>({ state: "idle" });
  const [local, setLocal] = React.useState<GivenAnswer | null>(null);
  const send = async (lines: readonly AnswerLine[]) => {
    if (!scope.target || !workspace) return;
    const text = composeAnswer({ file: scope.file, id, lines });
    if (text.length > SEND_LIMIT_CHARS) {
      setSent({ state: "failed", why: t("answerTooLong") });
      return;
    }
    setSent({ state: "sending" });
    try {
      await workspace.post("reply", replyBody({ agent: scope.target.agent, replyTo: scope.target.replyTo }, text));
      const described = scope.spec ? describeAnswer(scope.spec, { file: scope.file, id, lines }) : lines.map((line) => ({ label: line.field, value: line.value }));
      setLocal({ lines: described, tsMs: Date.now() });
      setSent({ state: "idle" });
    } catch (error) {
      setSent({ state: "failed", why: refusalText(error) });
    }
  };
  return { given: scope.given.get(id) ?? local, send, sent, canSend: Boolean(scope.target && workspace), author: scope.target?.author ?? null };
}

function AnswerFooter({ sent, author, canSend }: { sent: Sent; author: string | null; canSend: boolean }) {
  if (sent.state === "failed") return <p className="hb-huginn-error" role="alert">{sent.why}</p>;
  if (!canSend) return <p className="hb-huginn-foot">{t("noReplyHere")}</p>;
  return author ? <p className="hb-huginn-foot">{t("answerReaches", { author })}</p> : null;
}

export function Choice({ id, question, detail, multiple, options }: PropsOf<"Choice">) {
  const { given, send, sent, canSend, author } = useAnswer(id);
  const [picked, setPicked] = React.useState<readonly number[]>([]);
  if (given) return <Answered title={question} given={given} />;
  const busy = sent.state === "sending" || !canSend;
  const answer = (indexes: readonly number[]) => void send([{ field: id, value: optionsAnswer(indexes) }]);
  return (
    <div className="hb-huginn-ask">
      <p className="hb-huginn-question">{question}</p>
      {detail ? <p className="hb-huginn-detail">{detail}</p> : null}
      {multiple ? (
        <>
          <div className="hb-huginn-options" data-multiple="true">
            {options.map((option, index) => (
              <label key={index} className="hb-huginn-option">
                <Checkbox disabled={busy} checked={picked.includes(index)} onCheckedChange={(on) => setPicked((now) => (on ? [...now, index] : now.filter((one) => one !== index)))} />
                <span>
                  {option.label}
                  {option.detail ? <small>{option.detail}</small> : null}
                </span>
              </label>
            ))}
          </div>
          <button type="button" className="hb-btn hb-btn-primary" disabled={busy || picked.length === 0} onClick={() => answer(picked)}>
            {t("send")}
          </button>
        </>
      ) : (
        <div className="hb-huginn-options">
          {options.map((option, index) => (
            <button key={index} type="button" className="hb-huginn-pick" data-tone={option.tone ?? "neutral"} disabled={busy} onClick={() => answer([index])}>
              {option.label}
              {option.detail ? <small>{option.detail}</small> : null}
            </button>
          ))}
        </div>
      )}
      <AnswerFooter sent={sent} author={author} canSend={canSend} />
    </div>
  );
}

type FieldValue = string | readonly string[] | boolean;

interface FieldState {
  readonly value: FieldValue;
  readonly required: boolean;
  /** What the note says for this value: never the text of an option. */
  readonly toAnswer: (value: FieldValue) => string;
}

interface FormScope {
  readonly fields: Readonly<Record<string, FieldState>>;
  readonly register: (id: string, field: FieldState) => void;
  readonly unregister: (id: string) => void;
  readonly change: (id: string, value: FieldValue) => void;
  readonly disabled: boolean;
}

const FormContext = React.createContext<FormScope | null>(null);

const filled = (field: FieldState) => (typeof field.value === "boolean" ? true : field.value.length > 0);

export function Form({ id, title, detail, submitLabel, children }: PropsOf<"Form"> & { children?: React.ReactNode }) {
  const { given, send, sent, canSend, author } = useAnswer(id);
  const [fields, setFields] = React.useState<Readonly<Record<string, FieldState>>>({});
  const register = React.useCallback((fieldId: string, field: FieldState) => {
    setFields((now) => (fieldId in now ? now : { ...now, [fieldId]: field }));
  }, []);
  // A field hidden by a condition leaves the form, so it is neither required nor sent.
  const unregister = React.useCallback((fieldId: string) => {
    setFields(({ [fieldId]: _gone, ...rest }) => rest);
  }, []);
  const change = React.useCallback((fieldId: string, value: FieldValue) => {
    setFields((now) => {
      const field = now[fieldId];
      return field ? { ...now, [fieldId]: { ...field, value } } : now;
    });
  }, []);
  const disabled = sent.state === "sending" || !canSend;
  const scope = React.useMemo(() => ({ fields, register, unregister, change, disabled }), [fields, register, unregister, change, disabled]);
  if (given) return <Answered title={title} given={given} />;
  const entries = Object.values(fields);
  const ready = entries.length > 0 && entries.every((field) => !field.required || filled(field));
  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!ready || disabled) return;
    // A field that came back from a condition registers last; the page has the real order.
    const order = [...event.currentTarget.querySelectorAll<HTMLElement>("[data-huginn-field]")].flatMap((node) => node.dataset.huginnField ?? []);
    void send(order.flatMap((fieldId) => (fields[fieldId] ? [{ field: fieldId, value: fields[fieldId].toAnswer(fields[fieldId].value) }] : [])));
  };
  return (
    <form className="hb-huginn-ask" onSubmit={submit}>
      <p className="hb-huginn-question">{title}</p>
      {detail ? <p className="hb-huginn-detail">{detail}</p> : null}
      <FormContext.Provider value={scope}>{children}</FormContext.Provider>
      <div>
        <button type="submit" className="hb-btn hb-btn-primary" disabled={disabled || !ready}>
          {submitLabel ?? t("send")}
        </button>
      </div>
      <AnswerFooter sent={sent} author={author} canSend={canSend} />
    </form>
  );
}

interface FieldOptions {
  readonly required?: boolean;
  readonly state?: string;
  readonly toAnswer: (value: FieldValue) => string;
  /** The value conditions read, when it is not the field's own text. */
  readonly forState?: (value: FieldValue) => unknown;
}

function useField(id: string, initial: FieldValue, options: FieldOptions) {
  const form = React.useContext(FormContext);
  const store = useStateStore();
  const register = form?.register;
  const unregister = form?.unregister;
  const latest = React.useRef({ initial, options, store });
  latest.current = { initial, options, store };
  const required = Boolean(options.required);
  const publish = (value: FieldValue) => {
    const { state, forState } = latest.current.options;
    if (state) latest.current.store.set(state, forState ? forState(value) : value);
  };
  React.useEffect(() => {
    const now = latest.current;
    register?.(id, { value: now.initial, required, toAnswer: now.options.toAnswer });
    if (now.options.state && now.initial !== "") now.store.set(now.options.state, now.options.forState ? now.options.forState(now.initial) : now.initial);
    return () => unregister?.(id);
  }, [register, unregister, id, required]);
  return {
    value: form?.fields[id]?.value ?? initial,
    set: (value: FieldValue) => {
      form?.change(id, value);
      publish(value);
    },
    disabled: form?.disabled ?? true,
  };
}

function FieldShell({ field, label, hint, required, control, children }: { field: string; label: string; hint?: string; required?: boolean; control: string; children: React.ReactNode }) {
  return (
    <div className="hb-huginn-field" data-huginn-field={field}>
      <label htmlFor={control}>
        {label}
        {required ? <span className="hb-huginn-required"> {t("required")}</span> : null}
      </label>
      {children}
      {hint ? <small>{hint}</small> : null}
    </div>
  );
}

function GroupShell({ field, label, hint, required, children }: { field: string; label: string; hint?: string; required?: boolean; children: React.ReactNode }) {
  return (
    <fieldset className="hb-huginn-field" data-huginn-field={field}>
      <legend>
        {label}
        {required ? <span className="hb-huginn-required"> {t("required")}</span> : null}
      </legend>
      {children}
      {hint ? <small>{hint}</small> : null}
    </fieldset>
  );
}

const asText = (value: FieldValue) => (typeof value === "string" ? value : "");

const textAnswer = (value: FieldValue) => (typeof value === "string" ? value : "");
const asNumber = (value: FieldValue) => (typeof value === "string" && value !== "" ? Number(value) : null);
const optionAnswer = (options: readonly string[]) => (value: FieldValue) => {
  const index = options.indexOf(asText(value));
  return index < 0 ? "" : optionsAnswer([index]);
};

export function TextField({ id, label, hint, required, state, placeholder, multiline, defaultValue }: PropsOf<"TextField">) {
  const control = React.useId();
  // Left alone, a proposal is the agent's words, so the note only says it stood.
  const toAnswer = (value: FieldValue) => (defaultValue !== undefined && value === defaultValue ? UNCHANGED : textAnswer(value));
  const field = useField(id, defaultValue ?? "", { required, state, toAnswer });
  const common = { id: control, placeholder, required, disabled: field.disabled, value: asText(field.value), maxLength: 1000 };
  return (
    <FieldShell field={id} label={label} hint={hint} required={required} control={control}>
      {multiline ? <Textarea {...common} onChange={(e) => field.set(e.target.value)} /> : <Input {...common} onChange={(e) => field.set(e.target.value)} />}
    </FieldShell>
  );
}

export function NumberField({ id, label, hint, required, state, min, max, step, unit, defaultValue }: PropsOf<"NumberField">) {
  const control = React.useId();
  const field = useField(id, defaultValue === undefined ? "" : String(defaultValue), { required, state, toAnswer: textAnswer, forState: asNumber });
  return (
    <FieldShell field={id} label={label} hint={unit ? (hint ? `${hint} (${unit})` : unit) : hint} required={required} control={control}>
      <Input id={control} type="number" min={min} max={max} step={step ?? "any"} required={required} disabled={field.disabled} value={asText(field.value)} onChange={(e) => field.set(e.target.value)} />
    </FieldShell>
  );
}

export function SelectField({ id, label, hint, required, state, options, defaultValue }: PropsOf<"SelectField">) {
  const control = React.useId();
  const field = useField(id, defaultValue ?? "", { required, state, toAnswer: optionAnswer(options) });
  return (
    <FieldShell field={id} label={label} hint={hint} required={required} control={control}>
      <Select value={asText(field.value) || null} disabled={field.disabled} onValueChange={(value) => field.set(typeof value === "string" ? value : "")}>
        <SelectTrigger id={control} className="w-full">
          <SelectValue placeholder={t("chooseOption")} />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option} value={option}>
              {option}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </FieldShell>
  );
}

export function RadioField({ id, label, hint, required, state, options, defaultValue }: PropsOf<"RadioField">) {
  const field = useField(id, defaultValue ?? "", { required, state, toAnswer: optionAnswer(options) });
  return (
    <GroupShell field={id} label={label} hint={hint} required={required}>
      <RadioGroup className="hb-huginn-group" value={asText(field.value)} disabled={field.disabled} onValueChange={(value) => field.set(typeof value === "string" ? value : "")}>
        {options.map((option) => (
          <label key={option} className="hb-huginn-option">
            <Radio.Root value={option} className="hb-huginn-radio">
              <Radio.Indicator className="hb-huginn-radio-dot" />
            </Radio.Root>
            <span>{option}</span>
          </label>
        ))}
      </RadioGroup>
    </GroupShell>
  );
}

export function CheckboxField({ id, label, hint, state, defaultChecked }: PropsOf<"CheckboxField">) {
  const field = useField(id, defaultChecked ?? false, { state, toAnswer: (value) => (value === true ? YES : NO) });
  return (
    <div className="hb-huginn-field" data-huginn-field={id}>
      <label className="hb-huginn-option">
        <Checkbox disabled={field.disabled} checked={field.value === true} onCheckedChange={(on) => field.set(on)} />
        <span>{label}</span>
      </label>
      {hint ? <small>{hint}</small> : null}
    </div>
  );
}

export function MultiSelectField({ id, label, hint, required, state, options, defaultValue }: PropsOf<"MultiSelectField">) {
  const toAnswer = (value: FieldValue) => {
    const indexes = Array.isArray(value) ? options.flatMap((option, index) => (value.includes(option) ? [index] : [])) : [];
    return indexes.length === 0 ? "" : optionsAnswer(indexes);
  };
  const field = useField(id, defaultValue ?? [], { required, state, toAnswer });
  const picked = Array.isArray(field.value) ? (field.value as readonly string[]) : [];
  return (
    <GroupShell field={id} label={label} hint={hint} required={required}>
      <div className="hb-huginn-group">
        {options.map((option) => (
          <label key={option} className="hb-huginn-option">
            <Checkbox
              disabled={field.disabled}
              checked={picked.includes(option)}
              onCheckedChange={(on) => field.set(on ? options.filter((o) => o === option || picked.includes(o)) : picked.filter((o) => o !== option))}
            />
            <span>{option}</span>
          </label>
        ))}
      </div>
    </GroupShell>
  );
}

export function DateField({ id, label, hint, required, state, defaultValue }: PropsOf<"DateField">) {
  const control = React.useId();
  const field = useField(id, defaultValue ?? "", { required, state, toAnswer: textAnswer });
  return (
    <FieldShell field={id} label={label} hint={hint} required={required} control={control}>
      <Input id={control} type="date" required={required} disabled={field.disabled} value={asText(field.value)} onChange={(e) => field.set(e.target.value)} />
    </FieldShell>
  );
}

export function SliderField({ id, label, hint, state, min, max, step, unit, defaultValue }: PropsOf<"SliderField">) {
  const field = useField(id, String(defaultValue ?? min), { state, toAnswer: textAnswer, forState: asNumber });
  const current = Number(asText(field.value));
  return (
    <GroupShell field={id} label={label} hint={hint}>
      <div className="hb-huginn-slider">
        <Slider.Root value={current} min={min} max={max} step={step ?? 1} disabled={field.disabled} onValueChange={(value) => field.set(String(Array.isArray(value) ? value[0] : value))}>
          <Slider.Control className="hb-huginn-slider-control">
            <Slider.Track className="hb-huginn-slider-track">
              <Slider.Indicator className="hb-huginn-slider-fill" />
              <Slider.Thumb className="hb-huginn-slider-thumb" aria-label={label} />
            </Slider.Track>
          </Slider.Control>
        </Slider.Root>
        <output>{formatNumber(current, unit)}</output>
      </div>
    </GroupShell>
  );
}
