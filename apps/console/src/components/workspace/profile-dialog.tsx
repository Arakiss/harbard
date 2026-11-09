"use client";

import * as React from "react";
import { ImageUp, Trash2, X } from "lucide-react";
import { createT, intlTag } from "@/i18n";
import { CONTEXT_MAX, contextCount, contextFits, timeZones, weekdays, type ProfileHours } from "@/lib/profile";
import { operatorTimeZone } from "@/lib/time-zone";
import { refusalText, useProfile } from "./profile-state";
import { OperatorAvatar } from "./operator-avatar";
import { AVATAR_BACKDROP, OPERATOR_AVATARS, avatarLabel, avatarPhotoUrl, avatarThumbUrl, type GalleryAvatar } from "@/lib/avatar-gallery";
import { useModal } from "./use-modal";

const t = createT("profile");

const PHOTO_TYPES = ["image/png", "image/jpeg", "image/webp"];
const DEFAULT_HOURS: ProfileHours = { start: "09:00", end: "18:00", days: [1, 2, 3, 4, 5] };

function fmt(n: number): string {
  return new Intl.NumberFormat(intlTag()).format(n);
}

/**
 * "Profile": who the operator is for the agents. Name, how to address them,
 * time zone and working hours, a photo, and "Context for the agents",
 * which every new session reads in its brief.
 */
export function ProfileDialog({ operator, initials, onClose }: { operator: string; initials: string; onClose: () => void }) {
  const profile = useProfile();
  const stored = profile?.view?.profile ?? null;
  const ref = React.useRef<HTMLDivElement>(null);
  const file = React.useRef<HTMLInputElement>(null);
  useModal(ref, onClose);
  const [name, setName] = React.useState(stored?.name ?? "");
  const [address, setAddress] = React.useState(stored?.address ?? "");
  const [zone, setZone] = React.useState(stored?.time_zone ?? "");
  const [hasHours, setHasHours] = React.useState(Boolean(stored?.hours));
  const [hours, setHours] = React.useState<ProfileHours>(stored?.hours ?? DEFAULT_HOURS);
  const [context, setContext] = React.useState(stored?.context ?? "");
  const [status, setStatus] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState(false);
  const here = React.useMemo(() => operatorTimeZone(), []);
  const zones = React.useMemo(() => timeZones(here), [here]);
  const count = contextCount(context);
  const over = count > CONTEXT_MAX;
  // Under the character cap but too heavy for the brief (many lines or emoji).
  const heavy = !over && !contextFits(context);
  const limits = profile?.view?.limits;

  const toggleDay = (day: number) =>
    setHours((h) => ({
      ...h,
      days: h.days.includes(day) ? h.days.filter((d) => d !== day) : [...h.days, day].sort((a, b) => a - b),
    }));

  const pickPhoto = async (picked: File | undefined) => {
    if (!picked || !profile) return;
    setError(null);
    if (!PHOTO_TYPES.includes(picked.type)) {
      setError(t("photoWrongType"));
      return;
    }
    if (picked.size > (limits?.photo_bytes ?? 2 * 1024 * 1024)) {
      setError(t("photoTooLarge"));
      return;
    }
    try {
      await profile.uploadPhoto(picked);
      setStatus(t("photoSaved"));
    } catch (e) {
      setError(refusalText(e));
    }
  };

  const [chosen, setChosen] = React.useState<string | null>(null);
  const pickAvatar = async (avatar: GalleryAvatar) => {
    setChosen(avatar.id);
    if (!profile) return;
    setError(null);
    try {
      const res = await fetch(avatarPhotoUrl(avatar.id));
      if (!res.ok) throw new Error(`avatar ${avatar.id}: ${res.status}`);
      const blob = await res.blob();
      await profile.uploadPhoto(new File([blob], `${avatar.id}.webp`, { type: "image/webp" }));
      setStatus(t("avatarChosen", { label: avatarLabel(avatar) }));
    } catch (e) {
      setError(refusalText(e));
    }
  };

  const save = async () => {
    if (!profile) return;
    if (hasHours && hours.days.length === 0) {
      setError(t("pickWorkDay"));
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await profile.update({
        name: name.trim() || null,
        address: address.trim() || null,
        time_zone: zone || null,
        hours: hasHours ? hours : null,
        context: context.trim() || null,
      });
      onClose();
    } catch (e) {
      setError(refusalText(e));
      setSaving(false);
    }
  };

  return (
    <div className="hb-scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div ref={ref} className="hb-modal hb-profile-modal" role="dialog" aria-modal="true" aria-labelledby="hb-profile-title">
        <header className="hb-prefs-head">
          <h2 id="hb-profile-title">{t("profile")}</h2>
          <button type="button" className="hb-close" aria-label={t("closeProfile")} onClick={onClose}>
            <X className="size-5" aria-hidden />
          </button>
        </header>
        {!profile?.view ? (
          <p className="hb-banner" role="status">
            {t("profileUnavailable")}
          </p>
        ) : null}
        <form
          className="hb-modal-body hb-profile-body"
          onSubmit={(e) => {
            e.preventDefault();
            void save();
          }}
        >
          <div className="hb-profile-side">
            <OperatorAvatar name={name || operator} initials={initials} size={160} presence={null} />
            <input
              ref={file}
              type="file"
              accept={PHOTO_TYPES.join(",")}
              className="hb-visually-hidden"
              tabIndex={-1}
              aria-hidden
              data-testid="photo-input"
              onChange={(e) => {
                void pickPhoto(e.target.files?.[0]);
                e.target.value = "";
              }}
            />
            <button type="button" className="hb-btn" disabled={!profile?.view} onClick={() => file.current?.click()}>
              <ImageUp className="mr-2 inline size-4" aria-hidden />
              {t("uploadPhoto")}
            </button>
            {profile?.photoUrl ? (
              <button
                type="button"
                className="hb-btn hb-btn-quiet"
                onClick={() =>
                  profile
                    .update({ photo: null })
                    .then(() => setStatus(t("photoRemoved")))
                    .catch((e) => setError(refusalText(e)))
                }
              >
                <Trash2 className="mr-2 inline size-4" aria-hidden />
                {t("removePhoto")}
              </button>
            ) : null}
            <p className="hb-prefs-hint">{t("photoHint")}</p>
            <p className="hb-prefs-hint hb-av-gallery-title">{t("pickFromGallery")}</p>
            <div className="hb-av-gallery" role="group" aria-label={t("galleryAvatars")}>
              {OPERATOR_AVATARS.map((a) => (
                <button
                  key={a.id}
                  type="button"
                  className="hb-av-pick"
                  style={{ background: AVATAR_BACKDROP }}
                  data-motion={a.motion}
                  data-picked={chosen === a.id ? "true" : undefined}
                  title={avatarLabel(a)}
                  aria-label={t("useAsAvatar", { label: avatarLabel(a) })}
                  disabled={!profile?.view}
                  data-testid={`avatar-${a.id}`}
                  onClick={() => void pickAvatar(a)}
                >
                  <img src={avatarThumbUrl(a.id)} alt="" width={44} height={44} loading="lazy" />
                </button>
              ))}
            </div>
          </div>

          <div className="hb-profile-fields">
            <label className="hb-field-block">
              <span>{t("name")}</span>
              <input className="hb-input" data-autofocus data-testid="profile-name" value={name} maxLength={limits?.name ?? 60} placeholder={operator} onChange={(e) => setName(e.target.value)} />
            </label>
            <label className="hb-field-block">
              <span>{t("howToAddress")}</span>
              <input
                className="hb-input"
                value={address}
                maxLength={limits?.address ?? 120}
                placeholder={t("addressPlaceholder")}
                onChange={(e) => setAddress(e.target.value)}
              />
              <small>{t("addressHint")}</small>
            </label>
            <label className="hb-field-block">
              <span>{t("timeZone")}</span>
              <select className="hb-input" value={zone} onChange={(e) => setZone(e.target.value)}>
                <option value="">{t("timeZoneNotSet")}</option>
                {zones.map((z) => (
                  <option key={z} value={z}>
                    {z === here ? t("timeZoneThisMac", { zone: z }) : z}
                  </option>
                ))}
              </select>
            </label>
            <fieldset className="hb-field-block hb-hours">
              <legend>{t("workingHours")}</legend>
              <label className="hb-check">
                <input type="checkbox" checked={hasHours} onChange={(e) => setHasHours(e.target.checked)} />
                {t("tellAgentsWhenIWork")}
              </label>
              {hasHours ? (
                <>
                  <div className="hb-hours-row">
                    <label>
                      {t("hoursFrom")}
                      <input type="time" className="hb-input" value={hours.start} onChange={(e) => setHours({ ...hours, start: e.target.value })} />
                    </label>
                    <label>
                      {t("hoursTo")}
                      <input type="time" className="hb-input" value={hours.end} onChange={(e) => setHours({ ...hours, end: e.target.value })} />
                    </label>
                  </div>
                  <div className="hb-days" role="group" aria-label={t("workDays")}>
                    {weekdays().map((d) => (
                      <button
                        key={d.day}
                        type="button"
                        className="hb-weekday"
                        aria-pressed={hours.days.includes(d.day)}
                        aria-label={d.long}
                        title={d.long}
                        onClick={() => toggleDay(d.day)}
                      >
                        {d.short}
                      </button>
                    ))}
                  </div>
                </>
              ) : null}
            </fieldset>
            <label className="hb-field-block">
              <span>{t("contextForAgents")}</span>
              <textarea
                className="hb-input hb-context"
                value={context}
                rows={8}
                placeholder={t("contextPlaceholder")}
                aria-describedby="hb-context-count hb-context-hint"
                aria-invalid={over || heavy || undefined}
                onChange={(e) => setContext(e.target.value)}
              />
              <span className="hb-context-meta">
                <small id="hb-context-hint">{t("contextHint")}</small>
                <small id="hb-context-count" className="hb-context-count" data-over={over || undefined} data-testid="context-count">
                  {fmt(count)} / {fmt(CONTEXT_MAX)}
                </small>
              </span>
            </label>
            <div role="status" aria-live="polite" className="hb-prefs-status">
              {error ? (
                <span className="hb-error">{error}</span>
              ) : over ? (
                <span className="hb-error">{t("charactersOver", { count: fmt(count - CONTEXT_MAX) })}</span>
              ) : heavy ? (
                <span className="hb-error">{t("contextTooHeavy")}</span>
              ) : (
                status
              )}
            </div>
          </div>
          <footer className="hb-modal-foot">
            <span className="hb-grow" />
            <button type="button" className="hb-btn" onClick={onClose}>
              {t("cancel")}
            </button>
            <button type="submit" className="hb-btn hb-btn-primary" disabled={saving || over || heavy || !profile?.view}>
              {t("save")}
            </button>
          </footer>
        </form>
      </div>
    </div>
  );
}
