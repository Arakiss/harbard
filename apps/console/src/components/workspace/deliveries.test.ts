import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { WsShipment } from "@/lib/deliveries";
import { atDate, type WorkspaceView, type WsAgent, type WsDelivery } from "@/lib/workspace";
import { DeliveriesView } from "./deliveries";

const NOW = Date.UTC(2026, 10, 7, 19, 0);
const MIN = 60_000;
const agent = (key: string, name: string) =>
  ({ key, name, presence: "finished", dormant: null, lost: false, reactions: [], activity: { state: "finished", since_ms: NOW, detail: null, source_ms: NOW } }) as unknown as WsAgent;
const ship = (id: string, agentKey: string, extra: Partial<WsShipment>): WsShipment => ({
  id,
  agent: agentKey,
  ts_ms: NOW - 30 * MIN,
  text: `Entrega ${id}`,
  channel: "harbard",
  sha: "abcdef1",
  commit: null,
  state: "delivered",
  last_state: "installed",
  pending: true,
  untracked: null,
  review: [],
  ...extra,
});
const view = (shipments: WsShipment[], deliveries: WsDelivery[] = []) =>
  ({
    now_ms: NOW,
    agents: [agent("spring", "Marcus"), agent("lima", "Laura")],
    shipments,
    deliveries,
    running_commit: "59ee93d000000000000000000000000000000000",
  }) as unknown as WorkspaceView;

describe("the Deliveries view", () => {
  test("its header says in one sentence what it is", () => {
    const out = renderToStaticMarkup(h(DeliveriesView, { view: view([ship("a", "spring", {})]), commit: "59ee93d" }));
    expect(out).toContain("What the agents have finished and how far each commit has got");
    expect(out).toContain("You have commit 59ee93d installed (serve and console).");
  });

  test("outside harbard's own repository it never talks about installing", () => {
    const elsewhere = ship("a", "spring", { channel: "storefront", last_state: "integrated" });
    const out = renderToStaticMarkup(h(DeliveriesView, { view: view([elsewhere]), commit: "59ee93d" }));
    expect(out).not.toContain("hb-ship-installed");
    expect(out).not.toContain("Not installed yet");
    expect(out).toContain("Not on their branch yet (1)");
  });

  test("pending deliveries first, each with robot, name, line, state and a link to its conversation", () => {
    const out = renderToStaticMarkup(
      h(DeliveriesView, {
        view: view([ship("a", "spring", { state: "installed", pending: false }), ship("b", "lima", { state: "with_fixes" })]),
        commit: "59ee93d",
      }),
    );
    const links = [...out.matchAll(/href="\/direct\/([^"]+)"/g)].map((m) => m[1]);
    expect(links).toEqual(["lima", "spring"]);
    expect(out).toContain("Not installed yet (1)");
    expect(out).toMatch(/class="hb-ship-chip" data-state="with_fixes">With fixes</);
    expect(out).toMatch(/class="hb-ship-chip" data-state="installed">Installed</);
    expect(out).toContain(">Laura<");
    expect(out).toContain("Entrega b");
    expect(out).toContain(`delivered ${atDate(NOW - 30 * MIN, NOW)}`);
    expect(out).toContain('aria-current="step"');
  });

  test("the integration queue: hidden when empty, a secondary section with its line when not", () => {
    expect(renderToStaticMarkup(h(DeliveriesView, { view: view([]), commit: "" }))).not.toContain("Integration queue");
    const queued = { id: "d1", channel: "harbard", sha: "abcdef1234", branch: "main", verdict: "landed", created_ms: NOW, decided_ms: NOW, session: "s" } as WsDelivery;
    const out = renderToStaticMarkup(h(DeliveriesView, { view: view([], [queued]), commit: "" }));
    expect(out).toContain("Integration queue (1)");
    expect(out).toContain("the automatic queue that checks a commit and moves the branch");
  });
});
