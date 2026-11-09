"use client";

import * as React from "react";
import { BellOff, Camera, Check, ChevronRight, Info, MessageSquareText, Settings, UserRound, X } from "lucide-react";
import { createT } from "@/i18n";
import {
  customUntil,
  dndChoices,
  dndUntil,
  doneUntil,
  quietLabel,
  stateOf,
  states,
  statusExpiry,
  type DndChoice,
  type StateId,
} from "@/lib/profile";
import { passwordNotice } from "@/lib/workspace";
import { OperatorAvatar } from "./operator-avatar";
import { PreferencesDialog } from "./preferences";
import { ProfileDialog } from "./profile-dialog";
import { refusalText, useProfile } from "./profile-state";
import { StateIcon } from "./state-icon";
import { StatusDialog } from "./status-dialog";

const t = createT("profile");

/** Arrow keys walk the menu's items; Home and End jump. */
function onMenuKey(e: React.KeyboardEvent<HTMLDivElement>) {
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
  const items = [...e.currentTarget.querySelectorAll<HTMLElement>("[data-menu-item]:not([disabled])")];
  if (items.length === 0) return;
  e.preventDefault();
  const at = items.indexOf(document.activeElement as HTMLElement);
  const next =
    e.key === "Home" ? 0 : e.key === "End" ? items.length - 1 : (at + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
  items[next]?.focus();
}

type Open = null | "status" | "profile" | "preferences";

function dndShort(choice: DndChoice): string {
  if (choice === "30m") return t("dndShort30Minutes");
  return choice === "1h" ? t("dndShort1Hour") : t("dndShortTomorrow");
}

/**
 * The menu behind the operator's avatar at the bottom of the rail (sketch
 * 01): who and in which state, the six coordination states, Do not disturb,
 * Profile and context, Preferences, and the fixed password notice.
 */
export function UserMenu({ operator, initials }: { operator: string; initials: string }) {
  const profile = useProfile();
  const [open, setOpen] = React.useState(false);
  const [dialog, setDialog] = React.useState<Open>(null);
  const [durations, setDurations] = React.useState(false);
  const [custom, setCustom] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const button = React.useRef<HTMLButtonElement>(null);
  const root = React.useRef<HTMLDivElement>(null);
  const menu = React.useRef<HTMLDivElement>(null);
  const view = profile?.view ?? null;
  const name = view?.profile.name ?? operator;
  const quiet = profile?.quiet ?? false;
  const status = profile?.status ?? null;
  const quietUntil = view?.quiet_until_ms ?? null;
  const preferred = view?.profile.dnd_default ?? "1h";
  const nowMs = Date.now();
  const now = new Date(nowMs);
  const state = stateOf(view, nowMs);
  const stateUntil = state.id === "available" ? null : (view?.state_until_ms ?? null);

  const close = React.useCallback((refocus: boolean) => {
    setOpen(false);
    setDurations(false);
    setError(null);
    if (refocus) button.current?.focus();
  }, []);

  React.useEffect(() => {
    if (!open) return;
    // The state in force, or the first item that can be used: without serve
    // the states are disabled, and focus must still land inside the menu.
    (menu.current?.querySelector<HTMLElement>("[data-menu-item][aria-checked='true']:not([disabled])") ??
      menu.current?.querySelector<HTMLElement>("[data-menu-item]:not([disabled])"))?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close(true);
      }
    };
    const onDown = (e: MouseEvent) => {
      if (root.current && !root.current.contains(e.target as Node)) close(false);
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, [open, close]);

  const openDialog = (d: Open) => {
    setOpen(false);
    setDurations(false);
    setDialog(d);
  };
  const closeDialog = () => {
    setDialog(null);
    button.current?.focus();
  };
  const write = (run: () => Promise<unknown>) => {
    setError(null);
    run().catch((e) => setError(refusalText(e)));
  };
  const choose = (id: StateId) =>
    write(async () => {
      // Done for the day lasts until the next start of the working hours.
      const until = id === "done" ? doneUntil(view?.profile.hours, new Date()) : null;
      await profile?.update({ state: { id, until_ms: until } });
      close(true);
    });
  const quietFor = (choice: DndChoice) =>
    write(async () => {
      await profile?.update({ dnd: { until_ms: dndUntil(choice, new Date()) } });
      close(true);
    });
  const quietUntilCustom = () => {
    const at = customUntil(custom, new Date());
    if (at === null) {
      setError(t("pickMoment"));
      return;
    }
    write(async () => {
      await profile?.update({ dnd: { until_ms: at } });
      close(true);
    });
  };

  const presence = quiet ? "do-not-disturb" : state.id;
  const headline = `${state.label}${stateUntil ? ` · ${quietLabel(stateUntil, now)}` : ""}`;
  return (
    <div ref={root} className="hb-me">
      <button
        ref={button}
        type="button"
        className="hb-me-button"
        aria-label={t("menuOfState", { name, state: quiet ? t("doNotDisturb") : state.label })}
        aria-expanded={open}
        aria-haspopup="dialog"
        data-testid="me-button"
        onClick={() => (open ? close(false) : setOpen(true))}
      >
        <OperatorAvatar name={name} initials={initials} size={42} presence={presence} />
      </button>
      {open ? (
        <div ref={menu} className="hb-pop hb-me-menu" role="dialog" aria-label={t("menuOf", { name })} onKeyDown={onMenuKey}>
          <div className="hb-me-head">
            <button
              type="button"
              data-menu-item
              className="hb-me-photo"
              title={t("changePhoto")}
              aria-label={t("changePhoto")}
              data-testid="me-photo"
              onClick={() => openDialog("profile")}
            >
              <OperatorAvatar name={name} initials={initials} size={56} presence={presence} />
              <span className="hb-me-photo-hint" aria-hidden>
                <Camera className="size-5" />
              </span>
            </button>
            <div className="hb-me-who">
              <div className="hb-me-name">{name}</div>
              <div className="hb-me-presence" data-tone={state.tone} data-testid="me-state">
                <StateIcon id={state.id} className="size-4" />
                {headline}
              </div>
            </div>
          </div>

          <button type="button" data-menu-item className="hb-me-message" onClick={() => openDialog("status")}>
            {status ? (
              <>
                <span className="hb-me-status-emoji" aria-hidden>
                  {status.emoji || "💬"}
                </span>
                <span className="hb-me-status-text">
                  <span>{status.text}</span>
                  {statusExpiry(status.clear_at_ms, now) ? <small>{statusExpiry(status.clear_at_ms, now)}</small> : null}
                </span>
              </>
            ) : (
              <>
                <MessageSquareText className="size-4" aria-hidden />
                <span className="hb-me-status-empty">{t("addStatusMessage")}</span>
              </>
            )}
          </button>
          {status ? (
            <button type="button" data-menu-item className="hb-pop-item hb-pop-item-icon hb-me-clear" onClick={() => write(() => profile!.update({ status: null }))}>
              <X className="size-4" aria-hidden />
              {t("clearStatusMessage")}
            </button>
          ) : null}

          <div className="hb-pop-sep" />
          <div className="hb-me-section" id="hb-me-states">
            {t("yourStatus")}
          </div>
          <div role="radiogroup" aria-labelledby="hb-me-states" className="hb-me-states">
            {states().map((s) => {
              const on = s.id === state.id;
              return (
                <button
                  key={s.id}
                  type="button"
                  role="radio"
                  aria-checked={on}
                  data-menu-item
                  data-state={s.id}
                  className="hb-me-state"
                  disabled={!view}
                  onClick={() => (on ? close(true) : choose(s.id))}
                >
                  <span className="hb-me-state-icon" data-tone={s.tone}>
                    <StateIcon id={s.id} className="size-[18px]" />
                  </span>
                  <span className="hb-me-state-text">
                    <span className="hb-me-state-label">{s.label}</span>
                    <span className="hb-me-state-hint">{s.hint}</span>
                  </span>
                  {on ? <Check className="hb-me-state-check size-5" aria-hidden /> : null}
                </button>
              );
            })}
          </div>

          <div className="hb-pop-sep" />
          {quiet ? (
            <button type="button" data-menu-item className="hb-pop-item hb-pop-item-icon" data-testid="dnd-off" onClick={() => write(() => profile!.update({ dnd: null }))}>
              <BellOff className="size-[18px]" aria-hidden />
              <span>
                {t("doNotDisturb")} · {quietUntil ? quietLabel(quietUntil, now) : t("dndActive")}
                <small className="hb-me-row-sub">{t("dndResumeHint")}</small>
              </span>
            </button>
          ) : (
            <>
              <button
                type="button"
                data-menu-item
                className="hb-pop-item hb-pop-item-icon"
                aria-expanded={durations}
                data-testid="dnd-open"
                disabled={!view}
                onClick={() => setDurations(!durations)}
              >
                <BellOff className="size-[18px]" aria-hidden />
                {t("doNotDisturb")} · {dndShort(preferred)}
                <ChevronRight className="hb-me-chevron size-4" data-open={durations || undefined} aria-hidden />
              </button>
              {durations ? (
                <div className="hb-me-durations" role="group" aria-label={t("dndDurationAria")}>
                  {dndChoices().map((c) => (
                    <button key={c.id} type="button" data-menu-item className="hb-pop-item hb-pop-sub" onClick={() => quietFor(c.id)}>
                      {c.label}
                      {c.id === preferred ? <Check className="ml-auto size-4" aria-label={t("dndYourDefault")} /> : null}
                    </button>
                  ))}
                  <div className="hb-me-custom">
                    <label htmlFor="hb-dnd-custom">{t("customMoment")}</label>
                    <input id="hb-dnd-custom" type="datetime-local" className="hb-input" data-menu-item value={custom} onChange={(e) => setCustom(e.target.value)} />
                    <button type="button" className="hb-btn" data-menu-item disabled={!custom} onClick={quietUntilCustom}>
                      {t("apply")}
                    </button>
                  </div>
                  <p className="hb-me-hint">{t("dndMenuHint")}</p>
                </div>
              ) : null}
            </>
          )}
          <button type="button" data-menu-item className="hb-pop-item hb-pop-item-icon" onClick={() => openDialog("profile")}>
            <UserRound className="size-[18px]" aria-hidden />
            {t("profileAndContext")}
            <ChevronRight className="hb-me-chevron size-4" aria-hidden />
          </button>
          <button type="button" data-menu-item className="hb-pop-item hb-pop-item-icon" onClick={() => openDialog("preferences")}>
            <Settings className="size-[18px]" aria-hidden />
            {t("preferences")}
            <ChevronRight className="hb-me-chevron size-4" aria-hidden />
          </button>
          {error ? (
            <p className="hb-error hb-me-error" role="alert">
              {error}
            </p>
          ) : null}
          <div className="hb-pop-sep" />
          <p className="hb-me-notice" data-testid="password-notice">
            <Info className="size-4 flex-none" aria-hidden />
            <span>{passwordNotice()}</span>
          </p>
        </div>
      ) : null}
      {dialog === "status" ? <StatusDialog onClose={closeDialog} /> : null}
      {dialog === "profile" ? <ProfileDialog operator={operator} initials={initials} onClose={closeDialog} /> : null}
      {dialog === "preferences" ? <PreferencesDialog onClose={closeDialog} /> : null}
    </div>
  );
}
