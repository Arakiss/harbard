"use client";

import * as React from "react";
import { X } from "lucide-react";
import { createT } from "@/i18n";
import { stateOf } from "@/lib/profile";
import { suggestionFor, type Suggestion } from "@/lib/suggest";
import { refusalText, useProfile } from "./profile-state";
import { StateIcon } from "./state-icon";

const t = createT("workspace");

type Host = Window & { __HARBARD_PENDING_SUGGESTION__?: unknown };

/**
 * The desktop's away suggestion, as a card in the corner: "Shall I set you to
 * Away?" after an absence, "You are back" on the way back. Not modal, never
 * a switch: the state changes on "Ponerme…" and nothing else.
 */
export function SuggestCard() {
  const profile = useProfile();
  const [detail, setDetail] = React.useState<unknown>(null);
  const [error, setError] = React.useState<string | null>(null);
  React.useEffect(() => {
    const host = window as Host;
    const take = (d: unknown) => {
      delete host.__HARBARD_PENDING_SUGGESTION__;
      setError(null);
      setDetail(d);
    };
    const onEvent = (e: Event) => e instanceof CustomEvent && take(e.detail);
    window.addEventListener("harbard:suggest", onEvent);
    if (host.__HARBARD_PENDING_SUGGESTION__ !== undefined) take(host.__HARBARD_PENDING_SUGGESTION__);
    return () => window.removeEventListener("harbard:suggest", onEvent);
  }, []);
  const view = profile?.view ?? null;
  const current = stateOf(view, Date.now()).id;
  const enabled = view?.profile.away_suggest !== false;
  const s: Suggestion | null = view ? suggestionFor(detail, current, enabled) : null;
  if (!s || !profile) return null;
  const close = () => setDetail(null);
  return (
    <div className="hb-suggest" role="dialog" aria-modal="false" aria-labelledby="hb-suggest-title" data-testid="suggest-card">
      <span className="hb-suggest-icon" data-tone={s.tone} aria-hidden>
        <StateIcon id={s.target} className="size-5" />
      </span>
      <div className="hb-suggest-text">
        <strong id="hb-suggest-title">{s.title}</strong>
        <p>{s.body}</p>
        {error ? <p className="hb-error">{error}</p> : null}
        <div className="hb-suggest-actions">
          <button
            type="button"
            className="hb-btn hb-btn-primary"
            onClick={() =>
              profile.update({ state: { id: s.target } }).then(close, (e) => setError(refusalText(e)))
            }
          >
            {s.accept}
          </button>
          <button type="button" className="hb-btn" onClick={close}>
            {s.dismiss}
          </button>
        </div>
      </div>
      <button type="button" className="hb-close" aria-label={t("suggestClose")} onClick={close}>
        <X className="size-4" aria-hidden />
      </button>
    </div>
  );
}
