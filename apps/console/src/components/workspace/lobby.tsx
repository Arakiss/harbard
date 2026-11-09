"use client";

import type { Route } from "next";
import * as React from "react";
import Link from "next/link";
import { Search } from "lucide-react";
import { createT } from "@/i18n";
import type { Face } from "@/lib/activity";
import { Robot } from "@/components/robot/robot";
import { robotFor } from "@/components/robot/identity";
import { LOBBY, filterLobby, findChannelOrLobby, harnessName, lobbyGroupTitle, lobbyGroups, lobbyLineup, lobbyPresenceWord, roomComposerNotice, type LobbyEntry, type LobbyPresence } from "@/lib/rooms";
import { hace, robotName, type WorkspaceView, type WsAgent } from "@/lib/workspace";
import { HomeShell } from "./home";
import { AgentAvatar, Composer, useOsReducedMotion } from "./parts";
import { RoomFeed } from "./rooms";
import { useLive, useWorkspace } from "./workspace-context";

const t = createT("rooms");

const RESTING_FACE: Record<LobbyPresence, Face> = { live: "normal", idle: "normal", stale: "asleep", ended: "finished" };

/** How long a card stays lit after the lineup points at it. */
const FLASH_MS = 1_600;

/** Its live robot when the view has the agent; otherwise the same robot at rest, by name. */
function Portrait({ entry, agent, size }: { entry: LobbyEntry; agent: WsAgent | null; size: number }) {
  const live = useLive();
  const os = useOsReducedMotion();
  if (agent) return <AgentAvatar name={entry.name} agent={agent} size={size} dot={false} />;
  return <Robot name={entry.name} size={size} face={RESTING_FACE[entry.presence]} agentKey={entry.key} reduce={os || live?.motion === "reduce"} />;
}

function Presence({ presence }: { presence: LobbyPresence }) {
  return (
    <span className="hb-lobby-presence" data-presence={presence}>
      <span aria-hidden className="hb-lobby-presence-dot" />
      {lobbyPresenceWord(presence)}
    </span>
  );
}

function Card({ entry, agent, now, order, fresh, lit }: { entry: LobbyEntry; agent: WsAgent | null; now: number; order: number; fresh: boolean; lit: boolean }) {
  const identity = robotFor(agent ? robotName(agent) : entry.name);
  const style = { "--lobby-tile": `var(--hb-robot-tile-${identity.tile})`, "--lobby-order": Math.min(order, 12) } as React.CSSProperties;
  const known = !entry.intro && agent?.description ? t("lobbyKnownDoing", { what: agent.description }) : null;
  const label = t("lobbyCardAria", { name: entry.name, harness: harnessName(entry.harness), presence: lobbyPresenceWord(entry.presence) });
  return (
    <li id={`lobby-${entry.key}`} className="hb-lobby-card" data-presence={entry.presence} data-lit={lit ? "true" : undefined} data-fresh={fresh ? "true" : undefined} data-lobby-card={entry.key} style={style}>
      <div className="hb-lobby-band">
        <Presence presence={entry.presence} />
      </div>
      <div className="hb-lobby-portrait">
        <Portrait entry={entry} agent={agent} size={104} />
      </div>
      <div className="hb-lobby-id">
        <h3 className="hb-lobby-name">
          {agent ? (
            <Link href={`/direct/${encodeURIComponent(agent.key)}` as Route} aria-label={label}>
              {entry.name}
            </Link>
          ) : (
            entry.name
          )}
        </h3>
        <div className="hb-lobby-tags">
          <span className="hb-lobby-tag" data-harness={entry.harness}>
            {harnessName(entry.harness)}
          </span>
          {entry.project ? <span className="hb-lobby-tag hb-mono">{entry.project}</span> : null}
        </div>
      </div>
      {entry.intro ? (
        <blockquote className="hb-lobby-bubble">
          {entry.intro}
          <footer>{t("lobbySaid", { when: hace(entry.intro_ts_ms, now) })}</footer>
        </blockquote>
      ) : (
        <div className="hb-lobby-bubble" data-silent="true">
          {t("lobbyNoIntro")}
          {known ? <span className="hb-lobby-known">{known}</span> : null}
        </div>
      )}
      <p className="hb-lobby-seen">
        {entry.presence === "live" ? t("lobbyFirstSeen", { when: hace(entry.first_ms, now) }) : t("lobbyLastSeen", { when: hace(entry.last_ms, now) })}
      </p>
    </li>
  );
}

function Lineup({ entries, agents, onPoint }: { entries: LobbyEntry[]; agents: Map<string, WsAgent>; onPoint: (key: string) => void }) {
  const { shown, more } = lobbyLineup(entries);
  if (shown.length === 0) return null;
  return (
    <ul className="hb-lobby-lineup" aria-label={t("lobbyLineupAria")}>
      {shown.map((entry, i) => (
        <li key={entry.key} style={{ "--lobby-order": Math.min(i, 12) } as React.CSSProperties}>
          <button type="button" className="hb-lobby-stand" onClick={() => onPoint(entry.key)} title={entry.name}>
            <Portrait entry={entry} agent={agents.get(entry.key) ?? null} size={72} />
            <span className="hb-lobby-stand-name">{entry.name}</span>
          </button>
        </li>
      ))}
      {more > 0 ? <li className="hb-lobby-more">{t("lobbyLineupMore", { count: more })}</li> : null}
    </ul>
  );
}

function LobbyMain({ view }: { view: WorkspaceView }) {
  const { now, post } = useWorkspace();
  const live = useLive();
  const os = useOsReducedMotion();
  const channel = findChannelOrLobby(view);
  const entries = React.useMemo(() => view.lobby ?? [], [view.lobby]);
  const agents = React.useMemo(() => new Map(view.agents.map((a) => [a.key, a])), [view.agents]);
  const [query, setQuery] = React.useState("");
  const [lit, setLit] = React.useState<string | null>(null);
  const timer = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  React.useEffect(() => () => clearTimeout(timer.current), []);
  const seenUnread = React.useRef(0);
  React.useEffect(() => {
    // Open is read, as in any other room.
    if (channel.unread && seenUnread.current !== channel.unread) {
      seenUnread.current = channel.unread;
      void post("read-channel", { channel: LOBBY }).catch(() => {});
    }
  }, [channel.unread, post]);
  const point = (key: string) => {
    const card = document.getElementById(`lobby-${key}`);
    card?.scrollIntoView({ behavior: os || live?.motion === "reduce" ? "auto" : "smooth", block: "center" });
    card?.querySelector<HTMLElement>("a")?.focus({ preventScroll: true });
    setLit(key);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setLit(null), FLASH_MS);
  };
  const groups = lobbyGroups(filterLobby(entries, query));
  const baseline = live?.baseline ?? Number.POSITIVE_INFINITY;
  const totals = lobbyGroups(entries);
  const count = (group: "here" | "around" | "gone") => totals.find((g) => g.group === group)?.entries.length ?? 0;
  let order = 0;
  return (
    <main className="hb-main hb-lobby-main" aria-label={t("lobbyTitle")}>
      <div className="hb-lobby">
        <div className="hb-lobby-stage">
          <header className="hb-lobby-hero">
            <div className="hb-lobby-intro">
              <h1>{t("lobbyTitle")}</h1>
              <p>{t("lobbyTagline")}</p>
              <p className="hb-lobby-counts">
                <span data-group="here">{t("lobbyCountHere", { count: count("here") })}</span>
                <span>{t("lobbyCountAround", { count: count("around") })}</span>
                <span>{t("lobbyCountGone", { count: count("gone") })}</span>
              </p>
            </div>
            <Lineup entries={entries} agents={agents} onPoint={point} />
          </header>
          {entries.length > 0 ? (
            <label className="hb-search hb-lobby-find">
              <Search className="size-5" aria-hidden />
              <input value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t("lobbyFindPlaceholder")} aria-label={t("lobbyFindLabel")} />
            </label>
          ) : (
            <div className="hb-empty">
              <strong>{t("lobbyEmptyTitle")}</strong>
              {t("lobbyEmptyBody")}{" "}
              {/* i18n-exempt: a command, typed the same in every language. */}
              <span className="hb-mono">harbard intro</span>.
            </div>
          )}
          {entries.length > 0 && groups.length === 0 ? <div className="hb-empty">{t("lobbyNoMatch", { query: query.trim() })}</div> : null}
          {groups.map(({ group, entries: list }) => (
            <section key={group} className="hb-lobby-group" aria-labelledby={`lobby-group-${group}`} data-group={group}>
              <h2 id={`lobby-group-${group}`}>
                {lobbyGroupTitle(group)} <span>{list.length}</span>
              </h2>
              <ul className="hb-lobby-grid">
                {list.map((entry) => (
                  <Card key={entry.key} entry={entry} agent={agents.get(entry.key) ?? null} now={now} order={order++} fresh={Number.isFinite(baseline) && entry.intro_ts_ms > baseline} lit={lit === entry.key} />
                ))}
              </ul>
            </section>
          ))}
        </div>
        <aside className="hb-lobby-feed" aria-label={t("lobbyFeedTitle")}>
          <div className="hb-pane-head">
            <span className="hb-pane-title">{t("lobbyFeedTitle")}</span>
          </div>
          <RoomFeed view={view} room={LOBBY} now={now} empty={t("lobbyFeedEmpty")} />
          <Composer target={{ channel: LOBBY }} placeholder={t("writeIn", { room: LOBBY })} blocked={null} notice={roomComposerNotice(channel)} />
        </aside>
      </div>
    </main>
  );
}

/** Who is who: every agent of this workspace, its robot and what it says about itself. */
export function LobbyScreen() {
  return <HomeShell active={null}>{(view) => <LobbyMain view={view} />}</HomeShell>;
}
