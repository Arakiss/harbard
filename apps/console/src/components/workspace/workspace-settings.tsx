"use client";

import * as React from "react";
import { createT, richText } from "@/i18n";
import { fetchWorkspaces, postWorkspaces, type WorkspacesList } from "@/lib/api";
import { currentWorkspace } from "@/lib/current-workspace";
import { placementLabel, reposIn, repoName } from "@/lib/workspace-settings";
import { SmallDialog } from "./workspace-menu";
import { useOptionalWorkspace } from "./workspace-context";

const t = createT("shell");

/**
 * "<workspace> settings": the workspace's name, its rail tile, and the
 * repositories it holds, each with where it can be moved. A move takes
 * effect for the console at once and for every session at its next prompt;
 * nothing already written is rewritten, the history goes with the
 * repository.
 */
export function WorkspaceSettingsDialog({ onClose }: { onClose: () => void }) {
  const refresh = useOptionalWorkspace()?.refresh;
  const [list, setList] = React.useState<WorkspacesList | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [name, setName] = React.useState("");
  const [icon, setIcon] = React.useState("");

  const load = React.useCallback(async () => {
    try {
      const l = await fetchWorkspaces();
      setList(l);
      const id = currentWorkspace() ?? l.default;
      const me = l.workspaces.find((w) => w.id === id) ?? l.workspaces.find((w) => w.default);
      if (me) {
        setName(me.name);
        setIcon(me.icon);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);
  React.useEffect(() => void load(), [load]);

  const id = list ? (currentWorkspace() ?? list.default) : null;
  const me = list?.workspaces.find((w) => w.id === id) ?? list?.workspaces.find((w) => w.default) ?? null;
  const others = list && me ? list.workspaces.filter((w) => w.id !== me.id) : [];
  const repos = list && me ? reposIn(list, me.id) : [];

  const run = async (action: "move" | "reset" | "rename" | "icon", body: Record<string, unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await postWorkspaces(action, body);
      await load();
      refresh?.();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <SmallDialog title={me ? t("workspaceSettingsFor", { name: me.name }) : t("workspaceSettings")} onClose={onClose}>
      {!list && !error ? <p className="hb-prefs-hint">{t("loading")}</p> : null}
      {error ? (
        <p className="hb-prefs-hint" role="alert">
          {error}
        </p>
      ) : null}
      {me ? (
        <div className="hb-ws-settings">
          <form
            className="hb-field-block"
            onSubmit={(e) => {
              e.preventDefault();
              void run("rename", { workspace: me.id, name });
            }}
          >
            <span>{t("name")}</span>
            <div className="hb-ws-settings-row">
              <input className="hb-input" value={name} maxLength={40} onChange={(e) => setName(e.target.value)} aria-label={t("workspaceName")} />
              <button type="submit" className="hb-btn" disabled={busy || name.trim() === "" || name.trim() === me.name}>
                {t("save")}
              </button>
            </div>
          </form>
          <form
            className="hb-field-block"
            onSubmit={(e) => {
              e.preventDefault();
              void run("icon", { workspace: me.id, icon });
            }}
          >
            <span>{t("railIcon")}</span>
            <div className="hb-ws-settings-row">
              <input className="hb-input hb-ws-settings-icon" value={icon} maxLength={2} placeholder={me.tile} onChange={(e) => setIcon(e.target.value)} aria-label={t("railIconLetters")} />
              <button type="submit" className="hb-btn" disabled={busy || icon === me.icon}>
                {t("save")}
              </button>
            </div>
            <small>{t("railIconHint")}</small>
          </form>
          <div className="hb-field-block">
            <span>{t("repositories")}</span>
            {me.owners.length ? (
              <small>
                {richText(t("reposJoinByOrigin"), { origin: <code>{"origin"}</code>, owners: me.owners.join(", ") })}
                {me.default ? ` ${t("reposJoinByDefault")}` : ""}
              </small>
            ) : null}
            {repos.length === 0 ? <p className="hb-prefs-hint">{t("noReposYet")}</p> : null}
            <ul className="hb-ws-repos">
              {repos.map((r) => (
                <li key={r.repo}>
                  <div className="hb-ws-repo-name" title={r.repo}>
                    {repoName(r.repo)}
                    <small>{placementLabel(r)}</small>
                  </div>
                  {others.length ? (
                    <select
                      className="hb-input"
                      value=""
                      disabled={busy}
                      aria-label={t("moveRepoAria", { repo: repoName(r.repo) })}
                      onChange={(e) => {
                        if (e.target.value) void run("move", { repo: r.repo, workspace: e.target.value });
                      }}
                    >
                      <option value="">{t("moveTo")}</option>
                      {others.map((w) => (
                        <option key={w.id} value={w.id}>
                          {w.name}
                        </option>
                      ))}
                    </select>
                  ) : null}
                  {r.moved ? (
                    <button type="button" className="hb-btn" disabled={busy} onClick={() => void run("reset", { repo: r.repo })}>
                      {t("returnToRule")}
                    </button>
                  ) : null}
                </li>
              ))}
            </ul>
          </div>
        </div>
      ) : null}
    </SmallDialog>
  );
}
