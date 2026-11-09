"use client";

import * as React from "react";
import { createT } from "@/i18n";
import { ProfileRefused, fetchDigests, fetchPendingDigest, fetchProfile, postDigestRead, postProfile, postProfilePhoto, profilePhotoUrl } from "@/lib/api";
import { invisibleRefusal } from "@/lib/invisible";
import {
  effectiveStatus,
  isQuiet,
  oneShotsAllowed,
  stateOf,
  utcOffsetMin,
  type DigestItem,
  type ProfilePatch,
  type ProfileStatus,
  type ProfileView,
  type StateId,
} from "@/lib/profile";

const t = createT("profile");

/** Away this long before coming back opens "While you were away" while still Away. */
const RETURN_AFTER_MS = 2 * 60_000;

/** What "While you were away" shows: a cut digest, or what Away holds so far. */
export interface Away {
  /** The digest's id when it was cut (it is marked read on close). */
  digest: string | null;
  state: StateId;
  since: number;
  until: number;
  items: DigestItem[];
  /** Still in that state: the card offers "Make me available". */
  stillAway: boolean;
}

/** A status or Do not disturb can end while nothing else moves: look again this often. */
const REFRESH_MS = 30_000;

export interface ProfileCtx {
  /** Null until serve answers (or while it does not). */
  view: ProfileView | null;
  /** The status in force, on this browser's clock. */
  status: ProfileStatus | null;
  quiet: boolean;
  /** The Dock's one-shots may play: not under Do not disturb or a quiet state. */
  oneShots: boolean;
  photoUrl: string | null;
  /** All or nothing; rejects with serve's reason, translated when it is ours. */
  update: (patch: ProfilePatch) => Promise<ProfileView>;
  uploadPhoto: (file: File) => Promise<ProfileView>;
  /** "While you were away", when there is something to show. */
  away: Away | null;
  closeAway: () => void;
}

const Context = React.createContext<ProfileCtx | null>(null);

/** Serve's refusals in the operator's language; the rest as serve said it. */
export function refusalText(e: unknown): string {
  if (e instanceof ProfileRefused) {
    if (e.code === "too_large") return t("photoTooLarge");
    if (e.code === "unsupported_type") return t("photoWrongType");
    if (e.code === "symlink") return t("profileFolderSymlink");
    if (e.code === "too_long") return t("tooLong");
    if (e.code === "invisible_chars") return invisibleRefusal(e.message) ?? e.message;
    if (e.status === undefined) return t("couldNotSave");
    return e.message;
  }
  return t("couldNotSave");
}

/** The rail's profile: read on mount, on focus and every 30 s, written through serve. */
export function useProfileState(): ProfileCtx {
  const [view, setView] = React.useState<ProfileView | null>(null);
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    let alive = true;
    const ac = new AbortController();
    const load = () => {
      setNow(Date.now());
      fetchProfile(ac.signal)
        .then((v) => alive && setView(v))
        .catch(() => undefined);
    };
    load();
    const timer = window.setInterval(load, REFRESH_MS);
    window.addEventListener("focus", load);
    return () => {
      alive = false;
      ac.abort();
      window.clearInterval(timer);
      window.removeEventListener("focus", load);
    };
  }, []);
  const [away, setAway] = React.useState<Away | null>(null);
  // Digests already shown on this page, and the Away already greeted.
  const shown = React.useRef(new Set<string>());
  const greeted = React.useRef<number | null>(null);
  const showDigest = React.useCallback((d: { id: string; state: StateId; from_ms: number; to_ms: number; items: DigestItem[] }) => {
    if (shown.current.has(d.id)) return;
    shown.current.add(d.id);
    setAway({ digest: d.id, state: d.state, since: d.from_ms, until: d.to_ms, items: d.items, stillAway: false });
  }, []);
  const update = React.useCallback(
    async (patch: ProfilePatch) => {
      // Every write tells serve the browser's offset, so an agent reads times in this clock.
      const next = await postProfile({ utc_offset_min: utcOffsetMin(new Date()), ...patch });
      setView(next);
      setNow(Date.now());
      if (next.digest) showDigest(next.digest);
      else if (patch.state !== undefined) setAway((a) => (a?.stillAway ? null : a));
      return next;
    },
    [showDigest],
  );
  // An unread digest (an hourly one, or one cut by another window) opens on
  // its own, except in Swamped: calm mode shows one thing only, and the
  // digests wait, unread, until they leave it.
  const swamped = stateOf(view, now).id === "swamped";
  // Only once the profile is known: before it, a Swamped would look like
  // Available and a digest would open over calm mode.
  const known = view !== null;
  React.useEffect(() => {
    if (!known || swamped) return;
    const ac = new AbortController();
    fetchDigests(ac.signal)
      .then((all) => {
        const unread = all.items.filter((d) => !d.read && Date.now() - d.to_ms < 24 * 3_600_000);
        const last = unread[unread.length - 1];
        if (last) showDigest(last);
      })
      .catch(() => undefined);
    return () => ac.abort();
  }, [now, showDigest, swamped, known]);
  // Back at the console while still Away: what it holds so far, once per Away.
  const since = view?.profile.state?.since_ms ?? null;
  const inAway = stateOf(view, now).id === "away";
  React.useEffect(() => {
    if (!inAway || since === null) return;
    const greet = () => {
      if (document.visibilityState !== "visible" || greeted.current === since || Date.now() - since < RETURN_AFTER_MS) return;
      greeted.current = since;
      fetchPendingDigest()
        .then((p) => setAway({ digest: null, state: "away", since, until: Date.now(), items: p.items, stillAway: true }))
        .catch(() => undefined);
    };
    window.addEventListener("focus", greet);
    document.addEventListener("visibilitychange", greet);
    return () => {
      window.removeEventListener("focus", greet);
      document.removeEventListener("visibilitychange", greet);
    };
  }, [inAway, since]);
  const closeAway = React.useCallback(() => {
    setAway((a) => {
      if (a?.digest) void postDigestRead(a.digest).catch(() => undefined);
      return null;
    });
  }, []);
  const uploadPhoto = React.useCallback(async (file: File) => {
    const next = await postProfilePhoto(file);
    setView(next);
    return next;
  }, []);
  const rev = view?.profile.photo ? view.profile.photo_rev : null;
  return React.useMemo(
    () => ({
      view,
      status: effectiveStatus(view, now),
      quiet: isQuiet(view, now),
      oneShots: oneShotsAllowed(stateOf(view, now).id, isQuiet(view, now)),
      photoUrl: rev === null ? null : profilePhotoUrl(rev),
      update,
      uploadPhoto,
      away,
      closeAway,
    }),
    [view, now, rev, update, uploadPhoto, away, closeAway],
  );
}

export function ProfileProvider({ value, children }: { value: ProfileCtx; children: React.ReactNode }) {
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

/** Null outside the rail. */
export function useProfile(): ProfileCtx | null {
  return React.useContext(Context);
}
