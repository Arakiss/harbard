"use client";

import type { Route } from "next";
import * as React from "react";
import { activeLocale, createT } from "@/i18n";
import { WORKSPACE_MENU_EVENT, type MenuAnchor } from "@/lib/workspace-menu-bus";
import { currentName, customTile, isMulti, railOrder, setCurrentWorkspace, tileBadge } from "@/lib/current-workspace";
import { palettesFor } from "@/lib/workspace-emblem";
import { WorkspaceEmblem } from "./workspace-emblem";
import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { ArrowLeftRight, Bell, Bookmark, Home, Lightbulb, MessagesSquare, Plus, Search } from "lucide-react";
import { BrandMark } from "@/components/brand/brand-mark";
import { MarkOverlay, MarkUnderlay } from "@/components/brand/living-mark";
import { useFavicon } from "@/components/brand/favicon";
import { WORKING_STATES, logoState, type LogoOneShot, type LogoState } from "@/lib/activity";
import { reportLogo } from "@/lib/desktop-status";
import { bounceRise } from "@/lib/feedback";
import { FeedbackProvider, recordBounce } from "@/components/feedback/feedback";
import { fetchMeta } from "@/lib/api";
import { desktopViewBridge, openDesktopView } from "@/lib/desktop-view";
import { DEFAULT_OPERATOR_INITIALS, destinationOf, operatorName, railItems, type Destination } from "@/lib/workspace";
import { calmOn } from "@/lib/calm";
import { stateOf } from "@/lib/profile";
import { useOsReducedMotion } from "./parts";
import { PreferencesDialog } from "./preferences";
import { ReportPaneProvider } from "./attachments";
import { CalmScreen } from "./calm-screen";
import { Finder } from "./finder";
import { ProfileProvider, useProfile, useProfileState } from "./profile-state";
import { AboutHarbard, MENU_ICONS, ShortcutsList, SmallDialog, WorkspaceMenu, workspaceStatus, type MenuItem } from "./workspace-menu";
import { SeasonDecor, useDecorAwake } from "./season-decor";
import { useThemeAttrs } from "./theme-context";
import { AwayCard } from "./away-card";
import { SuggestCard } from "./suggest-card";
import { UserMenu } from "./user-menu";
import { WorkspaceSettingsDialog } from "./workspace-settings";
import { UpdateBanner } from "./update-banner";
import { UpdateProvider, useUpdate } from "./update-state";
import { useLive, useOptionalView, useOptionalWorkspace, useWorkspace, type LiveCtx } from "./workspace-context";

const t = createT("shell");

/** How long each one-shot of the logo runs (mark.css). */
const ONCE_MS: Record<LogoOneShot, number> = { message: 700, verified: 2_660 };

/**
 * The living logo's state from the live faces: lasting (rest, working,
 * needs you, blocked, connecting, offline) and a one-shot over it when a
 * message arrives or a delivery is verified after this page loaded.
 */
export function useLogo(live: LiveCtx | null, reduce: boolean): { state: LogoState; once: LogoOneShot | null; onceSeq: number } {
  const state = live
    ? logoState({
        connection: live.connection,
        blocked: live.blocked,
        needs: live.needs,
        lastActions: [...live.agents.values()].map((a) => a.last_action_ms),
        working: [...live.agents.values()].filter((a) => WORKING_STATES.has(a.activity.state)).length,
        now: live.serverNow,
      })
    : "idle";
  const [once, setOnce] = React.useState<LogoOneShot | null>(null);
  // Moves for every new one-shot, even one equal to the one still showing.
  const [onceSeq, setOnceSeq] = React.useState(0);
  const seen = React.useRef(new Set<string>());
  const timer = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  React.useEffect(() => {
    if (!live || !Number.isFinite(live.baseline)) return;
    let next: LogoOneShot | null = null;
    for (const [key, a] of live.agents) {
      for (const r of a.reactions) {
        const id = `${key}:${r.kind}:${r.at_ms}`;
        if (r.at_ms <= live.baseline || seen.current.has(id)) continue;
        seen.current.add(id);
        if (r.kind === "hop") next = "verified";
        else if (r.kind === "glance" && next === null) next = "message";
      }
    }
    if (!next || reduce) return;
    clearTimeout(timer.current);
    setOnce(next);
    setOnceSeq((n) => n + 1);
    timer.current = setTimeout(() => setOnce(null), ONCE_MS[next] + 100);
  }, [live, reduce]);
  React.useEffect(() => () => clearTimeout(timer.current), []);
  return { state, once: reduce ? null : once, onceSeq };
}

/**
 * The desktop's Dock shows what the living logo shows: its state, a badge
 * of open "Needs you" plus unread Direct, and each one-shot once. A
 * no-op in a browser tab; the wrapper repeats the state every 20 s as the
 * heartbeat the desktop watches. Under "Do not disturb", Swamped, Away and Fin
 * de jornada the one-shots stay off the Dock (the badge still counts; the
 * desktop holds the bounce and its alerts itself, from profile.json).
 */
function useDesktopStatus(logo: { state: LogoState; once: LogoOneShot | null; onceSeq: number }, live: LiveCtx | null, quiet: boolean, elsewhere = 0) {
  // The Dock is the hub's, not one workspace's: "Needs you" open in the
  // other workspaces counts too (their tiles carry it).
  const needs = (live?.needs ?? 0) + elsewhere;
  const directs = live?.directs ?? 0;
  const once = quiet ? null : logo.once;
  const prevNeeds = React.useRef(needs);
  React.useEffect(() => {
    const rose = bounceRise(prevNeeds.current, needs);
    prevNeeds.current = needs;
    void reportLogo({ state: logo.state, needs, directs, once, onceSeq: logo.onceSeq }).then((r) => {
      // Only the desktop shell has a Dock: there a rise rings it once.
      if (rose && r === "sent") recordBounce({ at_ms: Date.now(), state: logo.state });
    });
  }, [logo.state, once, logo.onceSeq, needs, directs]);
}

const ICONS: Record<Destination, React.ComponentType<{ className?: string; strokeWidth?: number }>> = {
  home: Home,
  direct: MessagesSquare,
  activity: Bell,
  later: Bookmark,
};

/** Closes on Escape or a click outside. */
function usePopover(): [boolean, (open: boolean) => void, React.RefObject<HTMLDivElement | null>] {
  const [open, setOpen] = React.useState(false);
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, [open]);
  return [open, setOpen, ref];
}

/** `away`: "While you were away" opens here; off beside a tool of More tools, which loads no workspace. */
function Rail({ away = true }: { away?: boolean }) {
  const pathname = usePathname() ?? "/";
  const router = useRouter();
  const view = useOptionalView();
  const live = useLive();
  const os = useOsReducedMotion();
  const logo = useLogo(live, os || live?.motion === "reduce");
  const profile = useProfile();
  const tiles = view?.workspaces;
  const multi = isMulti(tiles);
  const palettes = palettesFor((tiles ?? []).map((t) => t.id));
  const needsElsewhere = (tiles ?? []).filter((t) => !t.current).reduce((n, t) => n + t.needs, 0);
  useFavicon(logo.state);
  useDesktopStatus(logo, live, !(profile?.oneShots ?? true), needsElsewhere);
  const current = destinationOf(pathname);
  const badges: Partial<Record<Destination, number>> = {
    direct: view?.unread.directs ?? 0,
    activity: view?.unread.activity ?? 0,
  };
  const [plusOpen, setPlusOpen, plusRef] = usePopover();
  const [wsOpen, setWsOpen, wsRef] = usePopover();
  // Under the rail tile by default; under the sidebar's workspace name when opened from there.
  const [wsAt, setWsAt] = React.useState<{ left: number; top: number }>({ left: 80, top: 12 });
  React.useEffect(() => {
    const onOpen = (e: Event) => {
      const anchor = (e as CustomEvent<MenuAnchor>).detail;
      const box = wsRef.current?.getBoundingClientRect();
      if (!box) return;
      setWsAt({ left: anchor.left - box.left, top: anchor.bottom - box.top + 4 });
      setWsOpen(true);
    };
    window.addEventListener(WORKSPACE_MENU_EVENT, onOpen);
    return () => window.removeEventListener(WORKSPACE_MENU_EVENT, onOpen);
  }, [setWsOpen, wsRef]);
  const [dialog, setDialog] = React.useState<"shortcuts" | "acerca" | "espacio" | null>(null);
  const wsName = currentName(tiles);
  const [meta, setMeta] = React.useState<{ version: string; protocol: string | null } | null>(null);
  const post = useOptionalWorkspace()?.post ?? null;
  const update = useUpdate();
  const wsButton = React.useRef<HTMLButtonElement>(null);
  const active = view ? view.agents.filter((a) => !a.temporary && !a.lost && a.presence !== "finished").length : 0;
  const openThemes = () => {
    setPrefsOpen(true);
    setTimeout(() => document.getElementById("hb-themes-title")?.scrollIntoView({ block: "start" }), 50);
  };
  const switchTo = (id: string) => {
    setWsOpen(false);
    setCurrentWorkspace(id);
    if (pathname !== "/") router.push("/" as Route);
  };
  const wsItems: MenuItem[] = [
    // The name at the top of the sidebar is where one looks to change
    // workspace: the others come first.
    ...(multi && tiles
      ? tiles
          .filter((tile) => !tile.current)
          .map((tile) => ({
            id: `espacio-${tile.id}`,
            label: tile.working > 0 ? t("switchToWorking", { name: tile.name, count: tile.working }) : t("switchTo", { name: tile.name }),
            icon: ArrowLeftRight,
            run: () => switchTo(tile.id),
          }))
      : []),
    { id: "preferences", label: t("preferences"), icon: MENU_ICONS.Settings, run: () => setPrefsOpen(true) },
    { id: "themes", label: t("themes"), icon: MENU_ICONS.Palette, run: openThemes },
    { id: "shortcuts", label: t("keyboardShortcuts"), icon: MENU_ICONS.Keyboard, run: () => setDialog("shortcuts") },
    // The workspace's own settings: its name, its tile, and its repositories.
    ...(tiles?.length ? [{ id: "espacio", label: t("workspaceSettingsFor", { name: wsName }), icon: MENU_ICONS.Settings, run: () => setDialog("espacio") }] : []),
    // The session retro: what repeated, and the rules that prevent it.
    { id: "retro", label: "Retro", icon: Lightbulb, run: () => router.push("/retro" as Route) },
    ...(post
      ? [{ id: "read", label: t("markAllRead"), icon: MENU_ICONS.CheckCheck, run: () => void post("read-all", {}).catch(() => {}) }]
      : []),
    // Shown once serve has the update route: a new version, or a way to look for one.
    ...(update && !update.unsupported
      ? [{ id: "update", label: update.available ? t("updateHarbard") : t("checkForUpdates"), icon: MENU_ICONS.ArrowUpCircle, run: update.openDialog, dot: update.available }]
      : []),
    {
      id: "acerca",
      label: t("aboutHarbard"),
      icon: MENU_ICONS.Info,
      run: () => {
        setDialog("acerca");
        fetchMeta()
          .then((m) => setMeta({ version: m.version, protocol: (m as { protocol?: string }).protocol ?? null }))
          .catch(() => setMeta(null));
      },
    },
    {
      id: "strip",
      label: desktopViewBridge().kind === "desktop" ? t("openStrip") : t("openStripHere"),
      icon: MENU_ICONS.Radar,
      run: () => {
        void openDesktopView("strip").then((how) => {
          if (how === "browser") router.push("/pulse" as Route);
        });
      },
    },
  ];
  // Preferences from the workspace menu; the avatar's menu opens its own.
  const [prefsOpen, setPrefsOpen] = React.useState(false);
  const operator = operatorName(view?.operator);
  return (
    <nav className="hb-rail" aria-label={t("destinations")}>
      {multi ? (
        // harbard first and largest; the workspaces it holds come under it.
        <span className="hb-ws-brand">
          <BrandMark
            className="hb-ws-mark hb-living"
            data-logo={logo.state}
            data-logo-once={logo.once ?? undefined}
            underlay={<MarkUnderlay />}
            overlay={<MarkOverlay />}
          />
        </span>
      ) : null}
      <div ref={wsRef} className="relative hb-ws-tiles" data-multi={multi ? "true" : undefined}>
        {multi && tiles ? (
          // Where you are first and largest, the others smaller below it, and
          // a rule before the destinations: which workspace and where in it
          // are two questions. A tile switches; the workspace's name at the
          // top of the sidebar opens its menu.
          railOrder(tiles).map((tile) => {
            const badge = tileBadge(tile);
            const palette = palettes.get(tile.id);
            return (
              <button
                key={tile.id}
                ref={tile.current ? wsButton : undefined}
                type="button"
                className="hb-ws-tile"
                data-current={tile.current ? "true" : undefined}
                aria-current={tile.current ? "true" : undefined}
                aria-label={`${tile.name}${tile.current ? ` (${t("tileCurrent")})` : ""}${tile.needs > 0 ? `, ${tile.needs === 1 ? t("tileNeedsOne") : t("tileNeedsMany", { count: tile.needs })}` : tile.unread > 0 ? `, ${t("tileUnread")}` : ""}`}
                title={tile.working > 0 ? t("tileWorking", { name: tile.name, count: tile.working }) : tile.name}
                onClick={() => {
                  if (!tile.current) switchTo(tile.id);
                }}
              >
                <WorkspaceEmblem id={tile.id} palette={palette} />
                {customTile(tile) ? (
                  <span className="hb-ws-letter hb-ws-letter-over" aria-hidden>
                    {tile.tile}
                  </span>
                ) : null}
                {badge?.kind === "count" ? (
                  <span className="hb-rail-badge" aria-hidden>
                    {badge.text}
                  </span>
                ) : badge?.kind === "dot" ? (
                  <span className="hb-ws-dot" aria-hidden />
                ) : null}
                {!tile.current && tile.working > 0 ? <span className="hb-ws-working" aria-hidden /> : null}
              </button>
            );
          })
        ) : (
          <button
            ref={wsButton}
            type="button"
            className="hb-ws-tile"
            data-brand="true"
            aria-label={`${t("menuOf", { name: wsName })}${update?.available ? `, ${t("newVersionAria")}` : ""}`}
            aria-haspopup="menu"
            aria-expanded={wsOpen}
            title={wsName}
            onClick={() => {
              setWsAt({ left: 80, top: 12 });
              setWsOpen(!wsOpen);
            }}
          >
            <BrandMark
              className="hb-ws-mark hb-living"
              data-logo={logo.state}
              data-logo-once={logo.once ?? undefined}
              underlay={<MarkUnderlay />}
              overlay={<MarkOverlay />}
            />
            {update?.available ? <span className="hb-ws-dot" data-update="true" aria-hidden /> : null}
          </button>
        )}
        {wsOpen ? (
          <WorkspaceMenu
            status={workspaceStatus(live?.connection ?? "connecting", active)}
            items={wsItems}
            style={wsAt}
            onClose={() => {
              setWsOpen(false);
              wsButton.current?.focus();
            }}
          />
        ) : null}
      </div>
      {dialog === "shortcuts" ? (
        <SmallDialog title={t("keyboardShortcuts")} onClose={() => setDialog(null)}>
          <ShortcutsList />
        </SmallDialog>
      ) : null}
      {dialog === "espacio" ? <WorkspaceSettingsDialog onClose={() => setDialog(null)} /> : null}
      {dialog === "acerca" ? (
        <SmallDialog title={t("aboutHarbard")} onClose={() => setDialog(null)}>
          <AboutHarbard
            version={meta?.version ?? null}
            commit={process.env.NEXT_PUBLIC_HARBARD_COMMIT ?? ""}
            protocol={meta?.protocol ?? null}
            update={
              update && !update.unsupported && update.view
                ? {
                    available: update.available,
                    checking: update.checking,
                    check: () => void update.check(),
                    open: () => {
                      setDialog(null);
                      update.openDialog();
                    },
                  }
                : null
            }
          />
        </SmallDialog>
      ) : null}
      {railItems().map((d) => {
        const Icon = ICONS[d.id];
        const badge = badges[d.id] ?? 0;
        return (
          <Link key={d.id} href={d.href as Route} className="hb-dest" aria-current={current === d.id ? "page" : undefined} data-dest={d.id}>
            <span className="hb-dest-icon">
              <Icon className="size-6" strokeWidth={current === d.id ? 2.4 : 1.9} />
              {badge > 0 ? (
                <span className="hb-rail-badge" aria-label={t("unreadCount", { count: badge })}>
                  {badge > 99 ? "99+" : badge}
                </span>
              ) : null}
            </span>
            <span>{d.label}</span>
          </Link>
        );
      })}
      {/* Search is a place you go, like the four above, not a button among the utilities. */}
      <Link href="/search" className="hb-dest" aria-current={pathname.startsWith("/search") ? "page" : undefined} data-dest="search">
        <span className="hb-dest-icon">
          <Search className="size-6" strokeWidth={pathname.startsWith("/search") ? 2.4 : 1.9} />
        </span>
        <span>{t("search")}</span>
      </Link>
      <span className="hb-rail-spacer" />
      <div ref={plusRef} className="relative">
        <button type="button" className="hb-round" aria-label={t("create")} aria-expanded={plusOpen} data-on={plusOpen ? "true" : undefined} onClick={() => setPlusOpen(!plusOpen)}>
          <Plus className="size-5" strokeWidth={2.2} />
        </button>
        {plusOpen ? (
          <div className="hb-pop" style={{ left: 80, bottom: 70 }} role="menu">
            <div className="hb-pop-title">{t("create")}</div>
            <button type="button" role="menuitem" className="hb-pop-item" onClick={() => { setPlusOpen(false); router.push("/direct"); }}>
              {t("messageAgent")}
            </button>
            <button type="button" role="menuitem" className="hb-pop-item" onClick={() => { setPlusOpen(false); router.push("/"); }}>
              {t("messageChannel")}
            </button>
          </div>
        ) : null}
      </div>
      <UserMenu operator={operator} initials={view?.operator_initials ?? DEFAULT_OPERATOR_INITIALS} />
      {away ? <AwayCard /> : null}
      {away ? <SuggestCard /> : null}
      {prefsOpen ? (
        <PreferencesDialog
          onClose={() => {
            setPrefsOpen(false);
            wsButton.current?.focus();
          }}
        />
      ) : null}
    </nav>
  );
}

const CALM_EXIT = "harbard.calm-exit";

/** Calm mode: on in Swamped until "Exit", which holds for this Swamped (its start). */
function useCalm(): { on: boolean; exit: () => void } {
  const profile = useProfile();
  const view = profile?.view ?? null;
  const state = stateOf(view, Date.now()).id;
  const since = view?.profile.state?.since_ms ?? 0;
  const [exited, setExited] = React.useState<number | null>(null);
  React.useEffect(() => {
    const stored = Number(sessionStorage.getItem(CALM_EXIT));
    setExited(Number.isFinite(stored) && stored > 0 ? stored : null);
  }, []);
  return {
    on: calmOn(state, since, exited),
    exit: () => {
      sessionStorage.setItem(CALM_EXIT, String(since));
      setExited(since);
    },
  };
}

/** `data-hb-motion` (the Preferences toggle) and `data-hb-hidden` (tab out of sight) for a `.hb` root. */
export function useMotionAttrs(live: LiveCtx | null): Record<string, string | undefined> {
  const [hidden, setHidden] = React.useState(false);
  React.useEffect(() => {
    const on = () => setHidden(document.visibilityState !== "visible");
    on();
    document.addEventListener("visibilitychange", on);
    return () => document.removeEventListener("visibilitychange", on);
  }, []);
  return {
    "data-hb-motion": live?.motion === "reduce" ? "reduce" : undefined,
    "data-hb-hidden": hidden ? "" : undefined,
  };
}

/** Shift+Esc anywhere in the shell: everything read (not "Needs you"). */
function useReadAllKey() {
  const { post } = useWorkspace();
  React.useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if (e.key === "Escape" && e.shiftKey) {
        e.preventDefault();
        void post("read-all", {}).catch(() => {});
      }
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, [post]);
}

export function Shell({ children }: { children: React.ReactNode }) {
  const profile = useProfileState();
  // Feedback asks nothing in Focused, Swamped and Away (`feedbackAllowed`).
  return (
    <ProfileProvider value={profile}>
      <FeedbackProvider operatorState={stateOf(profile?.view ?? null, Date.now()).id}>
        <ShellBody>{children}</ShellBody>
      </FeedbackProvider>
    </ProfileProvider>
  );
}

function ShellBody({ children }: { children: React.ReactNode }) {
  const { state } = useWorkspace();
  const attrs = useThemeAttrs();
  const motion = useMotionAttrs(useLive());
  useReadAllKey();
  const view = useOptionalView();
  const calm = useCalm();
  const decor = attrs["data-hb-decor"] !== undefined;
  const decorAwake = useDecorAwake(decor);
  return (
    <div className="hb" lang={activeLocale()} {...attrs} {...motion} data-hb-calm={calm.on ? "" : undefined} data-hb-decor-still={decor && !decorAwake ? "" : undefined}>
      <UpdateProvider>
        <div className="hb-topbar" aria-hidden />
        {decor ? <SeasonDecor /> : null}
        <UpdateBanner />
        <Rail />
        <div className="hb-panel">
          {calm.on ? <CalmScreen onExit={calm.exit} /> : <ReportPaneProvider>{children}</ReportPaneProvider>}
          <Finder view={view} />
          {state.kind === "ready" && state.error ? (
            <div className="hb-banner" role="status" style={{ position: "fixed", right: 20, bottom: 20, zIndex: 40 }}>
              {t("harbardNotResponding")}
            </div>
          ) : null}
        </div>
      </UpdateProvider>
    </div>
  );
}

/**
 * A tool of "More tools" (`/setup`) in the whole shell: the rail
 * and a panel, with the theme's tokens, so it looks like the rest of
 * harbard. No badges: the tool loads no workspace.
 */
export function MoreFrame({ children }: { children: React.ReactNode }) {
  const attrs = useThemeAttrs();
  const profile = useProfileState();
  return (
    <ProfileProvider value={profile}>
      <div className="hb" lang={activeLocale()} {...attrs}>
        <UpdateProvider>
          <div className="hb-topbar" aria-hidden />
          <UpdateBanner />
          <Rail away={false} />
          <div className="hb-panel">{children}</div>
        </UpdateProvider>
      </div>
    </ProfileProvider>
  );
}
