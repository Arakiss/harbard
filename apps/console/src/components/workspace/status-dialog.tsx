"use client";

import * as React from "react";
import { X } from "lucide-react";
import { createT } from "@/i18n";
import { clearAfterChoices, clearAt, statusExpiry, statusPresets, type ClearAfter } from "@/lib/profile";
import { refusalText, useProfile } from "./profile-state";
import { useModal } from "./use-modal";

const t = createT("profile");

/** "Your status": an emoji, a few words and when it clears. */
export function StatusDialog({ onClose }: { onClose: () => void }) {
  const profile = useProfile();
  const current = profile?.status ?? null;
  const [emoji, setEmoji] = React.useState(current?.emoji ?? "");
  const [text, setText] = React.useState(current?.text ?? "");
  // i18n-exempt: the choice's id, not copy.
  const [after, setAfter] = React.useState<ClearAfter>(current?.clear_at_ms ? "today" : "nunca");
  const [keepCurrentEnd, setKeepCurrentEnd] = React.useState(Boolean(current?.clear_at_ms));
  const [error, setError] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState(false);
  const ref = React.useRef<HTMLDivElement>(null);
  useModal(ref, onClose);
  const max = profile?.view?.limits.status_text ?? 100;

  const save = async (next: { emoji: string; text: string } | null) => {
    if (!profile) return;
    setSaving(true);
    setError(null);
    try {
      const clear = keepCurrentEnd && current?.clear_at_ms ? current.clear_at_ms : clearAt(after, new Date());
      await profile.update({ status: next && (next.emoji.trim() || next.text.trim()) ? { ...next, clear_at_ms: clear } : null });
      onClose();
    } catch (e) {
      setError(refusalText(e));
      setSaving(false);
    }
  };

  return (
    <div className="hb-scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={ref} className="hb-modal hb-status-modal" role="dialog" aria-modal="true" aria-labelledby="hb-status-title">
        <header className="hb-prefs-head">
          <h2 id="hb-status-title">{t("yourStatus")}</h2>
          <button type="button" className="hb-close" aria-label={t("close")} onClick={onClose}>
            <X className="size-5" aria-hidden />
          </button>
        </header>
        <form
          className="hb-modal-body"
          onSubmit={(e) => {
            e.preventDefault();
            void save({ emoji, text });
          }}
        >
          <div className="hb-status-field">
            <input
              className="hb-input hb-status-emoji"
              aria-label={t("emoji")}
              value={emoji}
              maxLength={8}
              placeholder="💬"
              onChange={(e) => setEmoji(e.target.value)}
            />
            <input
              className="hb-input"
              data-autofocus
              aria-label={t("whatAreYouDoingAria")}
              value={text}
              maxLength={max}
              placeholder={t("whatAreYouDoing")}
              onChange={(e) => setText(e.target.value)}
            />
          </div>
          <fieldset className="hb-status-presets">
            <legend>{t("suggestions")}</legend>
            {statusPresets().map((p) => (
              <button
                key={p.text}
                type="button"
                className="hb-status-preset"
                aria-pressed={emoji === p.emoji && text === p.text}
                onClick={() => {
                  setEmoji(p.emoji);
                  setText(p.text);
                }}
              >
                <span aria-hidden>{p.emoji}</span> {p.text}
              </button>
            ))}
          </fieldset>
          <label className="hb-status-after">
            {t("clearAfter")}
            <select
              className="hb-input"
              value={keepCurrentEnd ? "actual" : after}
              onChange={(e) => {
                if (e.target.value === "actual") return;
                setKeepCurrentEnd(false);
                setAfter(e.target.value as ClearAfter);
              }}
            >
              {keepCurrentEnd && current?.clear_at_ms ? (
                <option value="actual">{statusExpiry(current.clear_at_ms, new Date())}</option>
              ) : null}
              {clearAfterChoices().map((c) => (
                <option key={c.id} value={c.id}>
                  {c.label}
                </option>
              ))}
            </select>
          </label>
          {error ? (
            <p className="hb-error" role="alert">
              {error}
            </p>
          ) : null}
          <footer className="hb-modal-foot">
            {current ? (
              <button type="button" className="hb-btn" disabled={saving} onClick={() => void save(null)}>
                {t("removeStatus")}
              </button>
            ) : null}
            <span className="hb-grow" />
            <button type="button" className="hb-btn" onClick={onClose}>
              {t("cancel")}
            </button>
            <button type="submit" className="hb-btn hb-btn-primary" disabled={saving || !profile?.view}>
              {t("save")}
            </button>
          </footer>
        </form>
      </div>
    </div>
  );
}
