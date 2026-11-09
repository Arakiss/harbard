"use client";

/**
 * The shell's update notice: a pill in the title bar's right end, where a
 * desktop app says it has a new version. It offers the changelog and the
 * button, follows a run while it goes, and asks for a reload once the page
 * runs an older build than the one just installed. "Later" hides it
 * until a newer target appears.
 */

import { ArrowUpCircle, Loader2, RefreshCw, X } from "lucide-react";
import { createT } from "@/i18n";
import { bannerLine, type BannerMode } from "@/lib/update";
import { useUpdate } from "./update-state";

const t = createT("update");

export function UpdateBanner() {
  const ctx = useUpdate();
  const mode: BannerMode = ctx?.banner ?? null;
  if (!ctx || !mode) return null;
  if (mode === "running") {
    return (
      <div className="hb-update-pill" data-mode="running" role="status">
        <button type="button" className="hb-update-pill-text" onClick={ctx.openDialog}>
          <Loader2 className="hb-spin size-4" aria-hidden />
          {ctx.restarting ? t("restarting") : t("bannerRunning")}
        </button>
      </div>
    );
  }
  if (mode === "done") {
    return (
      <div className="hb-update-pill" data-mode="done" role="status">
        <span className="hb-update-pill-text">{t("bannerDone")}</span>
        <button type="button" className="hb-update-pill-action" onClick={() => window.location.reload()}>
          <RefreshCw className="size-3.5" aria-hidden />
          {t("reload")}
        </button>
      </div>
    );
  }
  const check = ctx.view?.check;
  if (!check) return null;
  return (
    <div className="hb-update-pill" data-mode="available" role="status" data-testid="update-banner">
      <button type="button" className="hb-update-pill-text" onClick={ctx.openDialog} title={t("seeNotes")}>
        <ArrowUpCircle className="size-4" aria-hidden />
        {bannerLine(check)}
      </button>
      <button type="button" className="hb-update-pill-link" onClick={ctx.openDialog}>
        {t("seeNotes")}
      </button>
      <button
        type="button"
        className="hb-update-pill-action"
        onClick={() => {
          ctx.openDialog();
          // With a checkout that cannot fast-forward, the dialog says why instead.
          if (!ctx.view?.source.problem) void ctx.install();
        }}
      >
        {t("update")}
      </button>
      <button type="button" className="hb-update-pill-close" aria-label={t("dismissAria")} title={t("later")} onClick={ctx.dismiss}>
        <X className="size-3.5" aria-hidden />
      </button>
    </div>
  );
}
