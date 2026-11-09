"use client";

/**
 * Preferences › Pairing: every device paired to this hub, its role, when it
 * was last seen and what it did. "Revoke" cuts one device at once; Operator
 * is given on purpose, by typing the device's name. Each device's log lists
 * its remote reads and writes, refusals included. Invites nobody has used
 * yet are listed too, and "Cancel" stops a leaked one at once.
 */

import * as React from "react";
import { createT, richText } from "@/i18n";
import {
  confirmMatches,
  describeAction,
  expiresLine,
  fetchDeviceLog,
  fetchPairing,
  postRevoke,
  postRevokeInvite,
  postRole,
  deviceVersion,
  roleLabel,
  roles,
  statusLine,
  type Device,
  type DeviceAction,
  type PairingView,
  type Invite,
  type Role,
} from "@/lib/pairing";
import { relative } from "@/lib/progress";
import { useUpdate } from "./update-state";

const t = createT("teams");

export type Confirming = { id: string; kind: "operator" | "revoke"; typed: string } | null;

export type PairingUi = {
  confirming: Confirming;
  /** Open logs by device id; `null` while loading. */
  logs: Record<string, DeviceAction[] | null>;
  error: string | null;
  status: string | null;
};

export const EMPTY_UI: PairingUi = { confirming: null, logs: {}, error: null, status: null };

export type PairingHandlers = {
  onRole: (d: Device, role: Role) => void;
  onAskRevoke: (d: Device) => void;
  onType: (typed: string) => void;
  onConfirm: () => void;
  onCancel: () => void;
  onToggleLog: (d: Device) => void;
  onRevokeInvite: (i: Invite) => void;
};

function Log({ actions, now }: { actions: DeviceAction[] | null; now: number }) {
  if (actions === null) return <p className="hb-prefs-hint">{t("logLoading")}</p>;
  if (actions.length === 0) return <p className="hb-prefs-hint">{t("logEmpty")}</p>;
  return (
    <ol className="hb-device-log" aria-label={t("logAria")}>
      {actions.map((a, i) => (
        <li key={`${a.ts_ms}-${i}`} className={a.status === 401 || a.status === 403 ? "hb-device-log-refused" : undefined}>
          <span className="hb-device-log-when">{relative(a.ts_ms, now)}</span>
          <span>{describeAction(a)}</span>
          <span className="hb-mono hb-device-log-path">
            {`${a.method} ${a.path} · ${a.status}`}
          </span>
        </li>
      ))}
    </ol>
  );
}

type HubBuild = { version: string; commit: string | null } | null;

function Row({ d, now, ui, on, hub }: { d: Device; now: number; ui: PairingUi; on: PairingHandlers; hub: HubBuild }) {
  const build = deviceVersion(d, hub);
  const confirming = ui.confirming?.id === d.id ? ui.confirming : null;
  const log = Object.prototype.hasOwnProperty.call(ui.logs, d.id) ? ui.logs[d.id] ?? null : undefined;
  const live = !d.hub && !d.revoked_ms;
  const last = d.recent[0];
  return (
    <li className="hb-device" data-testid="device-row" data-revoked={d.revoked_ms ? "" : undefined}>
      <span className="hb-device-tile" aria-hidden>
        {(d.name.trim()[0] ?? "?").toLocaleUpperCase("es")}
      </span>
      <div className="hb-device-main">
        <div className="hb-device-head">
          <strong>{d.name}</strong>
          <span className="hb-chip hb-device-role" data-role={d.access}>
            {roleLabel(d.access)}
          </span>
          {build?.behind ? (
            <span className="hb-chip" data-tone="amber" title={t("versionBehindTitle")}>
              {t("versionBehind")}
            </span>
          ) : null}
        </div>
        <div className="hb-device-meta">
          {`${statusLine(d, now)}${d.host ? ` · ${d.host}` : ""}`}
        </div>
        {build ? <div className="hb-device-meta hb-mono">{`harbard ${build.text}`}</div> : null}
        {!d.hub ? (
          <div className="hb-device-recent">
            {`${last ? `${describeAction(last)} · ${relative(last.ts_ms, now)}` : t("noRemoteActivity")}${
              d.reads + d.writes > 0
                ? ` · ${d.reads === 1 ? t("readsOne", { count: d.reads }) : t("readsMany", { count: d.reads })}, ${d.writes === 1 ? t("writesOne", { count: d.writes }) : t("writesMany", { count: d.writes })}`
                : ""
            }`}
          </div>
        ) : null}
      </div>
      <div className="hb-device-actions">
        {live ? (
          <label className="hb-field-inline">
            {t("roleField")}
            <select
              className="hb-input"
              value={confirming?.kind === "operator" ? "operator" : d.access}
              aria-label={t("roleAria", { name: d.name })}
              onChange={(e) => on.onRole(d, e.target.value as Role)}
            >
              {roles().map((r) => (
                <option key={r.id} value={r.id}>
                  {r.label}
                </option>
              ))}
            </select>
          </label>
        ) : null}
        {!d.hub ? (
          <button type="button" className="hb-btn" aria-expanded={log !== undefined} onClick={() => on.onToggleLog(d)}>
            {log !== undefined ? t("logHide") : t("logShow")}
          </button>
        ) : null}
        {live ? (
          <button type="button" className="hb-btn hb-btn-danger" onClick={() => on.onAskRevoke(d)}>
            {t("revoke")}
          </button>
        ) : null}
      </div>
      {confirming?.kind === "operator" ? (
        <div className="hb-device-confirm" role="group" aria-label={t("makeOperatorAria", { name: d.name })}>
          <p>{richText(t("operatorWarning"), { strong: <strong>{t("operatorWarningLead")}</strong> })}</p>
          <label className="hb-field-inline hb-grow">
            {t("typeNameToConfirm", { name: d.name })}
            <input className="hb-input" value={confirming.typed} autoComplete="off" spellCheck={false} onChange={(e) => on.onType(e.target.value)} />
          </label>
          <div className="hb-device-confirm-row">
            <button type="button" className="hb-btn hb-btn-primary" disabled={!confirmMatches(confirming.typed, d.name)} onClick={on.onConfirm}>
              {t("makeOperator")}
            </button>
            <button type="button" className="hb-btn" onClick={on.onCancel}>
              {t("cancel")}
            </button>
          </div>
        </div>
      ) : null}
      {confirming?.kind === "revoke" ? (
        <div className="hb-device-confirm" role="group" aria-label={t("revokeAria", { name: d.name })}>
          <p>{richText(t("revokeConfirm"), { name: <strong>{d.name}</strong> })}</p>
          <div className="hb-device-confirm-row">
            <button type="button" className="hb-btn hb-btn-danger" onClick={on.onConfirm}>
              {t("revoke")}
            </button>
            <button type="button" className="hb-btn" onClick={on.onCancel}>
              {t("cancel")}
            </button>
          </div>
        </div>
      ) : null}
      {log !== undefined ? <Log actions={log} now={now} /> : null}
    </li>
  );
}

function Invites({ invites, now, on }: { invites: Invite[]; now: number; on: PairingHandlers }) {
  return (
    <div className="hb-device-invites">
      <h4 className="hb-device-invites-title">{t("invitesTitle")}</h4>
      <ul className="hb-pairing">
        {invites.map((i) => (
          <li key={i.ref} className="hb-device" data-testid="invite-row">
            <div className="hb-device-main">
              <div className="hb-device-head">
                <strong>{i.name}</strong>
                <span className="hb-chip hb-device-role" data-role={i.access}>
                  {roleLabel(i.access)}
                </span>
              </div>
              <div className="hb-device-meta">{`${expiresLine(i, now)} · ${i.ref}`}</div>
            </div>
            <div className="hb-device-actions">
              <button type="button" className="hb-btn hb-btn-danger" onClick={() => on.onRevokeInvite(i)}>
                {t("revokeInvite")}
              </button>
            </div>
          </li>
        ))}
      </ul>
      <p className="hb-prefs-hint">{t("invitesHint")}</p>
    </div>
  );
}

export function PairingSectionView({ view, ui, on, hub = null }: { view: PairingView | null; ui: PairingUi; on: PairingHandlers; hub?: HubBuild }) {
  const devices = view?.devices ?? [];
  const others = devices.filter((d) => !d.hub);
  return (
    <section className="hb-prefs-group" id="hb-prefs-pairing" aria-labelledby="hb-pairing-title">
      <h3 id="hb-pairing-title" className="hb-prefs-group-title">
        {t("sectionTitle")}
      </h3>
      <div className="hb-prefs-section">
        <p className="hb-prefs-hint">{t("sectionHint")}</p>
        {!view ? (
          <p className="hb-banner" role="status">
            {t("unreachable")}
          </p>
        ) : (
          <ul className="hb-pairing">
            {[...devices.filter((d) => d.hub), ...others].map((d) => (
              <Row key={d.id} d={d} now={view.now_ms} ui={ui} on={on} hub={hub} />
            ))}
          </ul>
        )}
        {view && (view.invites?.length ?? 0) > 0 ? <Invites invites={view.invites ?? []} now={view.now_ms} on={on} /> : null}
        {view && others.length === 0 ? (
          <p className="hb-prefs-hint">{richText(t("onlyThisMac"), { command: <span className="hb-mono">{t("inviteCommand")}</span> })}</p>
        ) : null}
        <div role="status" aria-live="polite" className="hb-prefs-status">
          {ui.error ? <span className="hb-error">{ui.error}</span> : ui.status}
        </div>
      </div>
    </section>
  );
}

export function PairingSection() {
  // The hub's own build, to say which devices run another one.
  const installed = useUpdate()?.view?.installed;
  const [view, setView] = React.useState<PairingView | null>(null);
  const [ui, setUi] = React.useState<PairingUi>(EMPTY_UI);
  const uiRef = React.useRef(ui);
  uiRef.current = ui;
  const viewRef = React.useRef(view);
  viewRef.current = view;

  const reload = React.useCallback((signal?: AbortSignal) => {
    return fetchPairing(signal)
      .then(setView)
      .catch(() => {
        if (!signal?.aborted) setView(null);
      });
  }, []);

  React.useEffect(() => {
    const c = new AbortController();
    void reload(c.signal);
    const timer = window.setInterval(() => void reload(), 15_000);
    return () => {
      c.abort();
      window.clearInterval(timer);
    };
  }, [reload]);

  const fail = (e: unknown) => setUi((u) => ({ ...u, error: e instanceof Error ? e.message : String(e), status: null }));
  const done = (status: string) => {
    setUi((u) => ({ ...u, confirming: null, error: null, status }));
    void reload();
  };
  const byId = (id: string) => viewRef.current?.devices.find((d) => d.id === id);

  const on: PairingHandlers = {
    onRole: (d, role) => {
      if (role === d.access) return;
      if (role === "operator") {
        setUi((u) => ({ ...u, confirming: { id: d.id, kind: "operator", typed: "" }, error: null, status: null }));
        return;
      }
      postRole(d.id, role).then(() => done(t("statusRoleSet", { name: d.name, role: roleLabel(role) })), fail);
    },
    onAskRevoke: (d) => setUi((u) => ({ ...u, confirming: { id: d.id, kind: "revoke", typed: "" }, error: null, status: null })),
    onType: (typed) => setUi((u) => (u.confirming ? { ...u, confirming: { ...u.confirming, typed } } : u)),
    onCancel: () => setUi((u) => ({ ...u, confirming: null })),
    onConfirm: () => {
      const c = uiRef.current.confirming;
      const d = c ? byId(c.id) : undefined;
      if (!c || !d) return;
      if (c.kind === "operator") {
        if (!confirmMatches(c.typed, d.name)) return;
        postRole(d.id, "operator", c.typed).then(() => done(t("statusRoleSet", { name: d.name, role: roleLabel("operator") })), fail);
      } else {
        postRevoke(d.id).then(() => done(t("statusDeviceRevoked", { name: d.name })), fail);
      }
    },
    onRevokeInvite: (i) => {
      postRevokeInvite(i.ref).then(() => done(t("statusInviteCancelled", { name: i.name })), fail);
    },
    onToggleLog: (d) => {
      if (Object.prototype.hasOwnProperty.call(uiRef.current.logs, d.id)) {
        setUi((u) => {
          const logs = { ...u.logs };
          delete logs[d.id];
          return { ...u, logs };
        });
        return;
      }
      setUi((u) => ({ ...u, logs: { ...u.logs, [d.id]: null } }));
      fetchDeviceLog(d.id).then(
        (actions) => setUi((u) => (Object.prototype.hasOwnProperty.call(u.logs, d.id) ? { ...u, logs: { ...u.logs, [d.id]: actions } } : u)),
        (e) => {
          setUi((u) => {
            const logs = { ...u.logs };
            delete logs[d.id];
            return { ...u, logs };
          });
          fail(e);
        },
      );
    },
  };

  return <PairingSectionView view={view} ui={ui} on={on} hub={installed ?? null} />;
}
