"use client";

import * as React from "react";
import { deskOpenNotificationSettings, deskOpenStrip, deskOsNotifications, deskStripMode, type OsNotifications, type StripMode } from "@/lib/desk-prefs";
import { Bell, BellOff, FolderOpen, Monitor, Moon, Palette, PanelTop, Pipette, RefreshCw, SlidersHorizontal, Sparkles, Sun, SunMoon, SwatchBook, Users, X, type LucideIcon } from "lucide-react";
import { ThemeRefused, postTheme, postWorkspace } from "@/lib/api";
import { createT } from "@/i18n";
import { DEFAULT_THEME_ID, describeRefusal, pickerThemes, themeNotice, modes, slugify, swatch, themeLabel, validThemeName, type ThemeEntry, type ThemeMode, type ThemeRefusal, type ThemeState } from "@/lib/theme";
import { LanguageSection } from "./language-section";
import { NotificationsSection } from "./notifications-section";
import { PairingSection } from "./pairing-section";
import { SeasonSection } from "./season-section";
import { UpdateSection } from "./update-section";
import { MemorySinkSection } from "@/components/feedback/memory-sink-section";
import { PrefsLegend } from "./prefs-legend";
import { useOsReducedMotion } from "./parts";
import { useTheme } from "./theme-context";
import { useLive, useOptionalView } from "./workspace-context";

const t = createT("preferences");

/** Says so when macOS is hiding the Dock number or the alerts; silent otherwise. */
function OsNotificationsNotice() {
  const [state, setState] = React.useState<OsNotifications | null>(null);
  React.useEffect(() => {
    void deskOsNotifications().then(setState);
  }, []);
  if (!state || (state.allowed && state.badge)) return null;
  return (
    <fieldset className="hb-prefs-section" data-testid="os-notifications-off">
      <PrefsLegend icon={BellOff}>{t("macNotifications")}</PrefsLegend>
      <p className="hb-prefs-hint">{state.allowed ? t("macNoBadge") : t("macNotificationsOff")}</p>
      <button type="button" className="hb-btn" onClick={() => void deskOpenNotificationSettings()}>
        {t("openSystemSettings")}
      </button>
    </fieldset>
  );
}

/** "Reduce motion": the robots, their reactions and the living logo stand still. */
function MotionSection() {
  const live = useLive();
  const os = useOsReducedMotion();
  const [error, setError] = React.useState<string | null>(null);
  if (!live) return null;
  const on = live.motion === "reduce";
  return (
    <fieldset className="hb-prefs-section">
      <PrefsLegend icon={Sparkles}>{t("motion")}</PrefsLegend>
      <label className="hb-mode">
        <input
          type="checkbox"
          checked={on}
          data-testid="reduce-motion"
          onChange={(e) => {
            setError(null);
            live.setMotion(e.target.checked).catch(() => setError(t("couldNotSave")));
          }}
        />
        {t("reduceMotion")}
      </label>
      <p className="hb-prefs-hint">{os ? t("reduceMotionOsOn") : t("reduceMotionHint")}</p>
      {error ? <p className="hb-error">{error}</p> : null}
    </fieldset>
  );
}

/** What the Strip is, why it is on screen or not, and its night dimming. */
function StripSection() {
  const view = useOptionalView();
  const [error, setError] = React.useState<string | null>(null);
  const [pending, setPending] = React.useState<boolean | null>(null);
  // The app decides at launch; asking once per open of Preferences is enough.
  const [mode, setMode] = React.useState<StripMode | null | undefined>(undefined);
  React.useEffect(() => {
    let alive = true;
    void deskStripMode().then((m) => {
      if (alive) setMode(m);
    });
    return () => {
      alive = false;
    };
  }, []);
  const stored = view ? view.night_dim !== false : null;
  // The choice shows at once; once serve reports it, serve is the truth again.
  React.useEffect(() => {
    if (pending !== null && stored === pending) setPending(null);
  }, [pending, stored]);
  if (!view) return null;
  const on = pending ?? stored === true;
  return (
    <fieldset className="hb-prefs-section">
      <PrefsLegend icon={PanelTop}>{t("strip")}</PrefsLegend>
      <p className="hb-prefs-hint" data-testid="strip-what">
        {t("stripWhat")}{" "}
        {mode === "wide"
          ? t("stripWide")
          : mode === "window"
            ? t("stripWindow")
            : mode === "none"
              ? t("stripNone")
              : mode === null
                ? t("stripBrowser")
                : null}
      </p>
      {mode === "wide" || mode === "window" ? (
        <button type="button" className="hb-btn" data-testid="open-strip" onClick={() => void deskOpenStrip()}>
          {t("openStrip")}
        </button>
      ) : null}
      <label className="hb-mode">
        <input
          type="checkbox"
          checked={on}
          data-testid="night-dim"
          onChange={(e) => {
            const next = e.target.checked;
            setError(null);
            setPending(next);
            postWorkspace("night-dim", { on: next }).catch(() => {
              setPending(null);
              setError(t("couldNotSave"));
            });
          }}
        />
        {t("dimStripAtNight")}
      </label>
      <p className="hb-prefs-hint">{t("dimStripHint")}</p>
      {error ? <p className="hb-error">{error}</p> : null}
    </fieldset>
  );
}

type PageId = "appearance" | "general" | "notifications" | "teams" | "updates";

/** Each page's icon; its title and one-line summary come from the dictionary at render. */
const PAGE_ICONS: Record<PageId, LucideIcon> = {
  appearance: Palette,
  general: SlidersHorizontal,
  notifications: Bell,
  teams: Users,
  updates: RefreshCw,
};

const MODE_ICONS: Record<ThemeMode, LucideIcon> = { light: Sun, dark: Moon, system: Monitor };

function pages(): readonly { id: PageId; title: string; hint: string }[] {
  return [
    { id: "appearance", title: t("pageAppearance"), hint: t("pageAppearanceHint") },
    { id: "general", title: t("pageGeneral"), hint: t("pageGeneralHint") },
    { id: "notifications", title: t("notifications"), hint: t("pageNotificationsHint") },
    { id: "teams", title: t("teams"), hint: t("pageTeamsHint") },
    { id: "updates", title: t("updates"), hint: t("pageUpdatesHint") },
  ];
}

/** The open page's icon, title and what it holds; the dialog's sections are labelled by it. */
function PageHead({ id }: { id: PageId }) {
  const page = pages().find((p) => p.id === id)!;
  const Icon = PAGE_ICONS[id];
  return (
    <header className="hb-prefs-page-head">
      <span className="hb-prefs-page-icon" aria-hidden>
        <Icon />
      </span>
      <span>
        <h3 id="hb-prefs-page-title">{page.title}</h3>
        <span className="hb-prefs-hint">{page.hint}</span>
      </span>
    </header>
  );
}

/** Delay before a colour drag asks serve for the derived family. */
const DRAFT_MS = 120;

function usePrefersDark(): boolean {
  const [dark, setDark] = React.useState(false);
  React.useEffect(() => {
    const q = window.matchMedia("(prefers-color-scheme: dark)");
    setDark(q.matches);
    const on = (e: MediaQueryListEvent) => setDark(e.matches);
    q.addEventListener("change", on);
    return () => q.removeEventListener("change", on);
  }, []);
  return dark;
}

function focusables(root: HTMLElement): HTMLElement[] {
  return [
    ...root.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"]), a[href], textarea:not([disabled])',
    ),
  ].filter((el) => el.offsetParent !== null || el === document.activeElement);
}

function ThemeCard({
  entry,
  mode,
  prefersDark,
  checked,
  focusable,
  onPreview,
  onChoose,
  onKey,
  cardRef,
}: {
  entry: ThemeEntry;
  mode: ThemeMode;
  prefersDark: boolean;
  checked: boolean;
  focusable: boolean;
  onPreview: () => void;
  onChoose: () => void;
  onKey: (e: React.KeyboardEvent) => void;
  cardRef: (el: HTMLButtonElement | null) => void;
}) {
  const s = swatch(entry, mode, prefersDark);
  return (
    <button
      ref={cardRef}
      type="button"
      role="radio"
      aria-checked={checked}
      tabIndex={focusable ? 0 : -1}
      className="hb-theme-card"
      data-theme-card={entry.id}
      onMouseEnter={onPreview}
      onFocus={onPreview}
      onClick={onChoose}
      onKeyDown={onKey}
    >
      <span className="hb-theme-swatch" aria-hidden style={{ background: s.frame }}>
        <span className="hb-theme-swatch-rail">
          <span style={{ background: s.railInk }} />
          <span style={{ background: s.railInk }} />
        </span>
        <span className="hb-theme-swatch-side" style={{ background: s.sidebar }}>
          <span style={{ background: s.ink }} />
          <span className="hb-theme-swatch-on" style={{ background: s.accent }} />
          <span style={{ background: s.ink }} />
        </span>
      </span>
      <span className="hb-theme-name">
        {themeLabel(entry)}
        {entry.builtin ? null : <span className="hb-theme-tag">{entry.stale ? t("themeFileRejected") : t("themeFile")}</span>}
      </span>
    </button>
  );
}

/**
 * Preferences › Themes: mode, built-in and file themes, and a custom theme
 * from two colours. Hovering or focusing previews; clicking, Enter or Space
 * chooses. Closing without choosing (Esc, the X, a click outside) restores
 * what was chosen, and focus goes back to whoever opened the dialog.
 */
export function PreferencesDialog({ onClose }: { onClose: () => void }) {
  const theme = useTheme();
  const prefersDark = usePrefersDark();
  const dialogRef = React.useRef<HTMLDivElement>(null);
  const cards = React.useRef<(HTMLButtonElement | null)[]>([]);
  const state = theme?.state ?? null;
  const chosenId = theme?.chosen.entry?.id ?? state?.theme ?? DEFAULT_THEME_ID;
  const chosenMode = theme?.chosen.mode ?? "light";
  const themes = pickerThemes(state?.themes ?? []);
  const [active, setActive] = React.useState(() => Math.max(0, themes.findIndex((t) => t.id === chosenId)));
  const [status, setStatus] = React.useState<string | null>(null);
  const [refusal, setRefusal] = React.useState<ThemeRefusal | null>(null);
  const base = theme?.chosen.entry?.light;
  // Seeded from the chosen theme; without serve there is nothing to edit.
  const [frame, setFrame] = React.useState(base?.frame ?? "");
  const [accent, setAccent] = React.useState(base?.accent ?? "");
  const [draft, setDraft] = React.useState<ThemeEntry | null>(null);
  const [name, setName] = React.useState("");
  const [code, setCode] = React.useState("");
  const draftSeq = React.useRef(0);
  const draftTimer = React.useRef<number | undefined>(undefined);
  const preview = theme?.preview;
  const [page, setPage] = React.useState<PageId>("appearance");
  const contentRef = React.useRef<HTMLDivElement>(null);

  // A draft still waiting on its timer or on serve must never paint after
  // the dialog is gone.
  const dropDraft = React.useCallback(() => {
    draftSeq.current += 1;
    window.clearTimeout(draftTimer.current);
  }, []);
  React.useEffect(() => dropDraft, [dropDraft]);

  const close = React.useCallback(() => {
    dropDraft();
    preview?.(null);
    onClose();
  }, [dropDraft, preview, onClose]);

  // The parent re-renders on every preview; the handlers read the latest
  // close through a ref so focus is placed once, on open, and never again.
  const closeRef = React.useRef(close);
  closeRef.current = close;

  React.useEffect(() => {
    const root = dialogRef.current;
    if (root) (root.querySelector<HTMLElement>('input[name="hb-mode"]:checked') ?? focusables(root)[0])?.focus();
  }, []);

  // Esc closes; Tab stays inside.
  React.useEffect(() => {
    const root = dialogRef.current;
    if (!root) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        closeRef.current();
        return;
      }
      if (e.key !== "Tab") return;
      const items = focusables(root);
      if (items.length === 0) return;
      const first = items[0]!;
      const last = items[items.length - 1]!;
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, []);

  const fail = (e: unknown) => {
    if (e instanceof ThemeRefused) {
      setRefusal(e.refusal ?? null);
      setStatus(e.refusal ? null : e.message);
    } else {
      setStatus(e instanceof Error ? e.message : String(e));
    }
  };

  const choose = async (entry: ThemeEntry) => {
    setRefusal(null);
    try {
      await theme?.choose(entry.id);
      preview?.(null);
      setStatus(t("themeChosen", { name: themeLabel(entry) }));
    } catch (e) {
      fail(e);
    }
  };

  const onCardKey = (i: number) => (e: React.KeyboardEvent) => {
    const cols = 4;
    const moves: Record<string, number> = { ArrowRight: 1, ArrowDown: cols, ArrowLeft: -1, ArrowUp: -cols };
    if (e.key in moves) {
      e.preventDefault();
      const next = Math.min(themes.length - 1, Math.max(0, i + moves[e.key]!));
      setActive(next);
      cards.current[next]?.focus();
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      const next = e.key === "Home" ? 0 : themes.length - 1;
      setActive(next);
      cards.current[next]?.focus();
    }
  };

  // The custom theme: serve derives and checks the family as the colours move.
  const editDraft = (nextFrame: string, nextAccent: string) => {
    setFrame(nextFrame);
    setAccent(nextAccent);
    const seq = ++draftSeq.current;
    window.clearTimeout(draftTimer.current);
    draftTimer.current = window.setTimeout(async () => {
      if (seq !== draftSeq.current) return;
      try {
        const out = await postTheme<{ ok: boolean; theme?: ThemeEntry; refusal?: ThemeRefusal }>("theme-preview", {
          frame: nextFrame,
          accent: nextAccent,
        });
        if (seq !== draftSeq.current) return;
        if (out.ok && out.theme) {
          setDraft(out.theme);
          setRefusal(null);
          preview?.({ entry: out.theme });
        } else {
          setDraft(null);
          setRefusal(out.refusal ?? null);
          preview?.(null);
        }
      } catch (e) {
        fail(e);
      }
    }, DRAFT_MS);
  };

  const saveAs = async () => {
    const slug = slugify(name);
    if (!validThemeName(slug)) {
      setStatus(t("enterThemeName"));
      return;
    }
    try {
      const save = () => postTheme<ThemeState>("theme-save", { name: slug, frame, accent });
      await (theme ? theme.write(save) : save());
      preview?.(null);
      setDraft(null);
      setRefusal(null);
      setStatus(t("savedAs", { file: `${slug}.toml` }));
    } catch (e) {
      fail(e);
    }
  };

  const shown = draft ?? theme?.applied.entry ?? null;
  const copy = async () => {
    if (!shown) return;
    try {
      await navigator.clipboard.writeText(shown.share);
      setStatus(t("codeCopied", { name: themeLabel(shown) }));
    } catch {
      setCode(shown.share);
      setStatus(t("couldNotCopy"));
    }
  };

  const importCode = async () => {
    try {
      const load = () => postTheme<ThemeState>("theme-import", { code: code.trim() });
      const next = await (theme ? theme.write(load) : load());
      preview?.(null);
      setRefusal(null);
      setCode("");
      setStatus(t("imported", { name: next.theme }));
    } catch (e) {
      fail(e);
    }
  };

  const openFolder = async () => {
    try {
      await postTheme("theme-open-folder", {});
      setStatus(t("themesFolderOpened"));
    } catch (e) {
      fail(e);
    }
  };

  const previewMode = (theme?.applied.mode ?? chosenMode) as ThemeMode;
  return (
    <div className="hb-scrim" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div ref={dialogRef} className="hb-prefs" role="dialog" aria-modal="true" aria-labelledby="hb-prefs-title">
        <header className="hb-prefs-head">
          <h2 id="hb-prefs-title">{t("preferences")}</h2>
          <button type="button" className="hb-close" aria-label={t("closePreferences")} onClick={close}>
            <X className="size-5" aria-hidden />
          </button>
        </header>
        <div className="hb-prefs-body">
          <nav className="hb-prefs-nav" aria-label={t("sections")}>
            {pages().map((p) => {
              const Icon = PAGE_ICONS[p.id];
              return (
                <button
                  key={p.id}
                  type="button"
                  className="hb-prefs-nav-item"
                  data-prefs-page={p.id}
                  aria-current={page === p.id ? "page" : undefined}
                  onClick={() => {
                    setPage(p.id);
                    contentRef.current?.scrollTo({ top: 0 });
                  }}
                >
                  <span className="hb-prefs-nav-icon" aria-hidden>
                    <Icon />
                  </span>
                  {p.title}
                </button>
              );
            })}
          </nav>
          <div ref={contentRef} className="hb-prefs-content">
            <PageHead id={page} />
            {page === "appearance" ? (
              <>
            {!state ? (
              <p className="hb-banner" role="status">
                {t("themesUnavailable")}
              </p>
            ) : null}
            <fieldset className="hb-prefs-section">
              <PrefsLegend icon={SunMoon}>{t("mode")}</PrefsLegend>
              <p className="hb-prefs-hint">{t("modeHint")}</p>
              <div className="hb-mode-row">
                {modes().map((m) => {
                  const ModeIcon = MODE_ICONS[m.id];
                  return (
                  <label key={m.id} className="hb-mode">
                    <input
                      type="radio"
                      name="hb-mode"
                      value={m.id}
                      checked={chosenMode === m.id}
                      disabled={!state}
                      onChange={() => {
                        setRefusal(null);
                        theme?.setMode(m.id).then(() => setStatus(t("modeChosen", { label: m.label })), fail);
                      }}
                    />
                    <ModeIcon className="hb-mode-icon" aria-hidden />
                    {m.label}
                  </label>
                  );
                })}
              </div>
            </fieldset>

            <section className="hb-prefs-section" aria-labelledby="hb-themes-title">
              <h3 id="hb-themes-title" className="hb-prefs-legend">
                <SwatchBook className="hb-prefs-legend-icon" aria-hidden />
                {t("themes")}
              </h3>
              <p className="hb-prefs-hint">{t("themesHint")}</p>
              <div className="hb-theme-grid" role="radiogroup" aria-labelledby="hb-themes-title" onMouseLeave={() => !draft && preview?.(null)}>
                {themes.map((entry, i) => (
                  <ThemeCard
                    key={entry.id}
                    entry={entry}
                    mode={previewMode}
                    prefersDark={prefersDark}
                    checked={entry.id === chosenId}
                    focusable={i === active}
                    cardRef={(el) => {
                      cards.current[i] = el;
                    }}
                    onPreview={() => {
                      setActive(i);
                      preview?.({ entry });
                    }}
                    onChoose={() => void choose(entry)}
                    onKey={onCardKey(i)}
                  />
                ))}
              </div>
              {state && state.errors.length > 0 ? (
                <div className="hb-prefs-errors">
                  <strong>{t("rejectedFiles")}</strong>
                  <ul>
                    {state.errors.map((e) => (
                      <li key={`${e.file}:${e.code}`} className="hb-mono">
                        {describeRefusal(e)}
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
              {state && themeNotice(state) ? <p className="hb-prefs-hint">{themeNotice(state)}</p> : null}
            </section>

            {state?.season ? <SeasonSection season={state.season} /> : null}

            <section className="hb-prefs-section" aria-labelledby="hb-custom-title">
              <h3 id="hb-custom-title" className="hb-prefs-legend">
                <Pipette className="hb-prefs-legend-icon" aria-hidden />
                {t("customTheme")}
              </h3>
              <p className="hb-prefs-hint">{t("customThemeHint")}</p>
              {frame && accent ? (
              <div className="hb-custom-row">
                <label className="hb-colour">
                  <input type="color" value={frame} disabled={!state} onChange={(e) => editDraft(e.target.value, accent)} />
                  <span>
                    {t("frame")} <span className="hb-mono">{frame}</span>
                  </span>
                </label>
                <label className="hb-colour">
                  <input type="color" value={accent} disabled={!state} onChange={(e) => editDraft(frame, e.target.value)} />
                  <span>
                    {t("accent")} <span className="hb-mono">{accent}</span>
                  </span>
                </label>
              </div>
              ) : null}
              <div className="hb-custom-row">
                <label className="hb-field-inline">
                  {t("name")}
                  <input className="hb-input" value={name} placeholder={t("themeNamePlaceholder")} onChange={(e) => setName(e.target.value)} />
                </label>
                <button type="button" className="hb-btn hb-btn-primary" disabled={!state} onClick={() => void saveAs()}>
                  {t("saveAs")}
                </button>
                <button type="button" className="hb-btn" disabled={!shown} onClick={() => void copy()}>
                  {t("copyShareCode")}
                </button>
              </div>
              <div className="hb-custom-row">
                <label className="hb-field-inline hb-grow">
                  {t("pasteCode")}
                  <input className="hb-input hb-mono" value={code} placeholder={t("themeCodePlaceholder")} onChange={(e) => setCode(e.target.value)} />
                </label>
                <button type="button" className="hb-btn" disabled={!state || !code.trim()} onClick={() => void importCode()}>
                  {t("importTheme")}
                </button>
              </div>
              <div className="hb-custom-row">
                <button type="button" className="hb-btn" disabled={!state} onClick={() => void openFolder()}>
                  <FolderOpen className="mr-2 inline size-4" aria-hidden />
                  {t("openThemesFolder")}
                </button>
                {state ? <span className="hb-mono hb-prefs-hint">{state.dir}</span> : null}
              </div>
              <div role="status" aria-live="polite" className="hb-prefs-status">
                {refusal ? <span className="hb-error">{describeRefusal(refusal)}</span> : status}
              </div>
            </section>
              </>
            ) : null}
            {page === "general" ? (
              <>
                <LanguageSection />
                <MotionSection />
                <StripSection />
                <MemorySinkSection />
              </>
            ) : null}
            {page === "notifications" ? (
              <>
                <OsNotificationsNotice />
                <NotificationsSection />
              </>
            ) : null}
            {page === "teams" ? <PairingSection /> : null}
            {page === "updates" ? <UpdateSection /> : null}
          </div>
        </div>
        <footer className="hb-prefs-foot">
          <button type="button" className="hb-btn" onClick={close}>
            {t("done")}
          </button>
        </footer>
      </div>
    </div>
  );
}
