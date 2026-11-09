"use client";

/**
 * One reader of `GET /api/v1/update` for the whole shell: the banner, the
 * workspace menu's dot, About, Preferences and the update dialog all
 * read the same answer. It reads every ten minutes (serve keeps the check
 * cached; nothing here reaches the network beyond serve) and every 1.5 s
 * while a run goes on. Serve and the console restart mid-run, so a failed
 * read during a run means "reiniciando", never an error.
 */

import * as React from "react";
import { toast } from "sonner";
import { ApiUnavailable, UpdateRefused, fetchUpdate, postUpdateCheck, postUpdateInstall, postUpdatePrefs } from "@/lib/api";
import { createT } from "@/i18n";
import {
  IDLE_POLL_MS,
  RUN_POLL_MS,
  bannerMode,
  errorText,
  readAck,
  readDismissed,
  toastFor,
  writeAck,
  writeDismissed,
  type BannerMode,
  type UpdateChannel,
  type UpdateView,
} from "@/lib/update";
import { OPEN_UPDATE_EVENT } from "@/lib/desktop-view";
import { UpdateDialog } from "./update-dialog";

const t = createT("update");

/** Older than this, opening from the tray or a notification looks again first. */
const FRESH_MS = 5 * 60_000;

export interface UpdateCtx {
  view: UpdateView | null;
  /** serve has no update route (an older serve): every surface stays hidden. */
  unsupported: boolean;
  /** A run was going on and serve stopped answering: it is restarting. */
  restarting: boolean;
  checking: boolean;
  installing: boolean;
  /** The run this page watched going: the dialog keeps showing its outcome. */
  followed: string | null;
  /** The last refusal or failure of a write, in plain words. */
  error: string | null;
  banner: BannerMode;
  /** The workspace menu's dot and About: a newer version is there. */
  available: boolean;
  check: () => Promise<void>;
  install: () => Promise<void>;
  setPrefs: (prefs: { channel?: UpdateChannel; auto_check?: boolean }) => Promise<void>;
  dismiss: () => void;
  openDialog: () => void;
}

const Ctx = React.createContext<UpdateCtx | null>(null);

export function useUpdate(): UpdateCtx | null {
  return React.useContext(Ctx);
}

function storage(): Storage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    return null;
  }
}

const PAGE_COMMIT = process.env.NEXT_PUBLIC_HARBARD_COMMIT ?? "";

export function UpdateProvider({ children }: { children: React.ReactNode }) {
  const [view, setView] = React.useState<UpdateView | null>(null);
  const [unsupported, setUnsupported] = React.useState(false);
  const [restarting, setRestarting] = React.useState(false);
  const [checking, setChecking] = React.useState(false);
  const [installing, setInstalling] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [dismissed, setDismissed] = React.useState<string | null>(null);
  const [dialog, setDialog] = React.useState(false);
  const [followed, setFollowed] = React.useState<string | null>(null);
  const toasted = React.useRef(false);
  // While a run is (or may be) going on, read fast; a failure then is a restart.
  const running = view?.run?.state === "running" || installing || restarting;
  const runningRef = React.useRef(running);
  runningRef.current = running;

  React.useEffect(() => setDismissed(readDismissed(storage())), []);

  const read = React.useCallback(async (signal?: AbortSignal) => {
    try {
      const next = await fetchUpdate(signal);
      setView(next);
      if (next.run?.state === "running") setFollowed(next.run.id);
      setUnsupported(false);
      setRestarting(false);
      if (next.run?.state !== "running") setInstalling(false);
      if (!toasted.current) {
        toasted.current = true;
        const store = storage();
        if (toastFor(next.run, Date.now(), readAck(store)) && next.run) {
          writeAck(store, next.run.id);
          toast.success(t("toastDone"), {
            description: t("toastVersion", { version: next.installed.version }),
            action: { label: t("seeNotes"), onClick: () => setDialog(true) },
          });
        }
      }
    } catch (e) {
      if (signal?.aborted) return;
      if (runningRef.current) {
        setRestarting(true);
        return;
      }
      // An older serve without the route: nothing to show, nothing to say.
      if (e instanceof ApiUnavailable && e.status === 404) setUnsupported(true);
    }
  }, []);

  React.useEffect(() => {
    const ctl = new AbortController();
    void read(ctl.signal);
    const every = running ? RUN_POLL_MS : IDLE_POLL_MS;
    const timer = window.setInterval(() => {
      if (!running && document.visibilityState !== "visible") return;
      void read(ctl.signal);
    }, every);
    const onVisible = () => document.visibilityState === "visible" && void read(ctl.signal);
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      ctl.abort();
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [read, running]);

  const check = React.useCallback(async () => {
    setChecking(true);
    setError(null);
    try {
      setView(await postUpdateCheck());
    } catch (e) {
      setError(e instanceof UpdateRefused ? errorText({ code: e.code ?? "", message: e.message }) : String(e));
    } finally {
      setChecking(false);
    }
  }, []);

  const install = React.useCallback(async () => {
    setInstalling(true);
    setError(null);
    try {
      const next = await postUpdateInstall();
      setView(next);
      if (next.run) setFollowed(next.run.id);
    } catch (e) {
      if (e instanceof UpdateRefused && e.code === "busy") {
        // One already runs: follow it.
        void read();
        return;
      }
      setInstalling(false);
      setError(e instanceof UpdateRefused ? errorText({ code: e.code ?? "", message: e.message }) : String(e));
    }
  }, [read]);

  const setPrefs = React.useCallback(async (prefs: { channel?: UpdateChannel; auto_check?: boolean }) => {
    setError(null);
    if (prefs.channel) setChecking(true);
    try {
      setView(await postUpdatePrefs(prefs));
    } catch (e) {
      setError(e instanceof UpdateRefused ? errorText({ code: e.code ?? "", message: e.message }) : String(e));
      throw e;
    } finally {
      setChecking(false);
    }
  }, []);

  const dismiss = React.useCallback(() => {
    const target = view?.check?.target?.commit;
    if (!target) return;
    writeDismissed(storage(), target);
    setDismissed(target);
  }, [view]);

  // The desktop's tray item and notification (desktop-navigation.tsx): open
  // the dialog, and look again first when the last look is not fresh.
  const viewRef = React.useRef(view);
  viewRef.current = view;
  React.useEffect(() => {
    const on = () => {
      setDialog(true);
      const checked = viewRef.current?.check?.checked_ms ?? 0;
      if (Date.now() - checked > FRESH_MS && viewRef.current?.run?.state !== "running") void check();
    };
    window.addEventListener(OPEN_UPDATE_EVENT, on);
    return () => window.removeEventListener(OPEN_UPDATE_EVENT, on);
  }, [check]);

  const banner = unsupported ? null : bannerMode(view, dismissed, PAGE_COMMIT, restarting || installing);
  const value: UpdateCtx = {
    view,
    unsupported,
    restarting,
    checking,
    installing,
    followed,
    error,
    banner,
    available: !unsupported && view?.check?.status === "available",
    check,
    install,
    setPrefs,
    dismiss,
    openDialog: () => setDialog(true),
  };
  return (
    <Ctx.Provider value={value}>
      {children}
      {dialog ? <UpdateDialog onClose={() => setDialog(false)} /> : null}
    </Ctx.Provider>
  );
}
