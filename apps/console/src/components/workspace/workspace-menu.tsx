"use client";

/**
 * The logo's menu, as a team chat client has: the workspace's name and
 * state, Preferences, Themes, Keyboard shortcuts, "Mark everything as read",
 * "About harbard" and "Abrir Strip". A real menu: arrows move between
 * items, Home and End jump, Escape closes and gives focus back.
 */

import * as React from "react";
import { Activity, ArrowUpCircle, CheckCheck, Info, Keyboard, Palette, Radar, Settings, X } from "lucide-react";
import { createT } from "@/i18n";
import { shortcuts } from "@/lib/workspace";

const t = createT("shell");

export interface MenuItem {
  id: string;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
  run: () => void;
  /** A small dot after the label: something new behind this item. */
  dot?: boolean;
}

/** Arrow-key focus inside a menu: the next index, wrapping. */
export function menuStep(at: number, count: number, key: string): number {
  if (count === 0) return 0;
  switch (key) {
    case "ArrowDown":
      return (at + 1) % count;
    case "ArrowUp":
      return (at - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return at;
  }
}

/** "serve is responding · 4 active agents". */
export function workspaceStatus(connection: string, active: number): string {
  const serve = connection === "connected" ? t("serveResponding") : connection === "connecting" ? t("serveConnecting") : t("serveNotResponding");
  return `${serve} · ${active === 1 ? t("oneActiveAgent") : t("activeAgents", { count: active })}`;
}

export function WorkspaceMenu({
  status,
  items,
  onClose,
  style,
}: {
  status: string;
  items: MenuItem[];
  onClose: () => void;
  style?: React.CSSProperties;
}) {
  const refs = React.useRef<(HTMLButtonElement | null)[]>([]);
  const [at, setAt] = React.useState(0);
  React.useEffect(() => refs.current[at]?.focus(), [at]);
  return (
    <div
      className="hb-pop hb-ws-menu"
      style={style}
      role="menu"
      aria-label={t("harbardMenu")}
      onKeyDown={(e) => {
        if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
          e.preventDefault();
          setAt((i) => menuStep(i, items.length, e.key));
        } else if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="hb-pop-head">
        <div>
          <div className="font-black">harbard</div>
          <div className="text-[13px] text-[var(--hb-ink-3)]" data-testid="ws-status">
            {status}
          </div>
        </div>
      </div>
      {items.map((item, i) => {
        const Icon = item.icon;
        return (
          <button
            key={item.id}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="menuitem"
            tabIndex={i === at ? 0 : -1}
            className="hb-pop-item hb-pop-item-icon"
            data-item={item.id}
            onClick={() => {
              onClose();
              item.run();
            }}
          >
            <Icon className="size-4" aria-hidden />
            {item.label}
            {item.dot ? <span className="hb-menu-dot" aria-label={t("newDot")} /> : null}
          </button>
        );
      })}
    </div>
  );
}

export const MENU_ICONS = { Activity, ArrowUpCircle, CheckCheck, Info, Keyboard, Palette, Radar, Settings };

/** A small modal with a title, closed by Escape or its button. */
export function SmallDialog({ title, onClose, children }: { title: string; onClose: () => void; children: React.ReactNode }) {
  const close = React.useRef<HTMLButtonElement>(null);
  React.useEffect(() => {
    close.current?.focus();
    const on = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, [onClose]);
  return (
    <div className="hb-finder-scrim" onMouseDown={onClose}>
      <div className="hb-small-dialog" role="dialog" aria-modal="true" aria-label={title} onMouseDown={(e) => e.stopPropagation()}>
        <div className="hb-pane-head">
          <span className="hb-pane-title">{title}</span>
          <button ref={close} type="button" className="hb-close" onClick={onClose} aria-label={t("close")}>
            <X className="size-5" />
          </button>
        </div>
        <div className="hb-small-body">{children}</div>
      </div>
    </div>
  );
}

export function ShortcutsList() {
  return (
    <dl className="hb-shortcuts">
      {shortcuts().map((s) => (
        <div key={s.keys}>
          <dt>
            <kbd className="hb-kbd">{s.keys}</kbd>
          </dt>
          <dd>{s.what}</dd>
        </div>
      ))}
    </dl>
  );
}

export function AboutHarbard({
  version,
  commit,
  protocol,
  update = null,
}: {
  version: string | null;
  commit: string;
  protocol: string | null;
  /** Whether a newer version exists, and how to see it; null hides the row. */
  update?: { available: boolean; checking: boolean; open: () => void; check: () => void } | null;
}) {
  return (
    <dl className="hb-shortcuts">
      {update ? (
        <div className="hb-about-update">
          <dt>{t("updates")}</dt>
          <dd>
            {update.available ? (
              <span className="hb-chip" data-tone="amber">
                {t("newVersionAvailable")}
              </span>
            ) : (
              <span className="hb-chip" data-tone="green">
                {t("upToDate")}
              </span>
            )}
            {update.available ? (
              <button type="button" className="hb-btn hb-about-update-btn" onClick={update.open}>
                {t("seeWhatsNew")}
              </button>
            ) : (
              <button type="button" className="hb-btn hb-about-update-btn" disabled={update.checking} onClick={update.check}>
                {update.checking ? t("checking") : t("checkNow")}
              </button>
            )}
          </dd>
        </div>
      ) : null}
      <div>
        <dt>{t("version")}</dt>
        <dd className="hb-mono">{version ?? t("serveNotResponding")}</dd>
      </div>
      <div>
        <dt>{t("consoleCommit")}</dt>
        <dd className="hb-mono">{commit || t("noData")}</dd>
      </div>
      <div>
        <dt>{t("protocol")}</dt>
        <dd className="hb-mono">{protocol ?? t("noData")}</dd>
      </div>
    </dl>
  );
}
