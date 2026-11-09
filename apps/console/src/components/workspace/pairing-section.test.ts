import { describe, expect, test } from "bun:test";
import { createElement as h } from "react";
import { renderToString } from "react-dom/server";
import type { Device, PairingView } from "@/lib/pairing";
import { EMPTY_UI, PairingSectionView, type PairingHandlers, type PairingUi } from "./pairing-section";

const NOW = 1_800_000_000_000;
const MIN = 60_000;
const noop = () => {};
const on: PairingHandlers = { onRole: noop, onAskRevoke: noop, onType: noop, onConfirm: noop, onCancel: noop, onToggleLog: noop, onRevokeInvite: noop };

function device(over: Partial<Device>): Device {
  return {
    id: "d1",
    name: "BOREALIS",
    host: "borealis",
    os: "linux",
    hub: false,
    access: "workshop",
    created_ms: NOW - 60 * MIN,
    joined_ms: NOW - 60 * MIN,
    last_seen_ms: NOW - 3 * MIN,
    revoked_ms: null,
    credential: true,
    recent: [{ ts_ms: NOW - 3 * MIN, method: "POST", path: "/api/v1/events", status: 201, write: true }],
    reads: 4,
    writes: 1,
    ...over,
  };
}

const MAC = device({ id: "mac", name: "mac-mini", host: "mac-mini.local", hub: true, access: "operator", recent: [] });
const view = (devices: Device[]): PairingView => ({ devices, now_ms: NOW });
const render = (v: PairingView | null, ui: PairingUi = EMPTY_UI) => renderToString(h(PairingSectionView, { view: v, ui, on }));

/** The markup of one row, by the device's name. */
function row(html: string, name: string): string {
  return html.split('data-testid="device-row"').find((part) => part.includes(`<strong>${name}</strong>`)) ?? "";
}

describe("Preferences › Teams", () => {
  test("each device shows its name, role, last seen and what it did", () => {
    const html = render(view([MAC, device({})]));
    const r = row(html, "BOREALIS");
    expect(r).toContain("Workshop");
    expect(r).toContain("Seen 3 min ago");
    expect(r).toContain("Published events · 3 min ago");
    expect(r).toContain("4 reads, 1 write");
    expect(r).toContain("Revoke");
    expect(r).toContain("Show log");
  });

  test("this Mac is Operator and can be neither demoted nor revoked", () => {
    const r = row(render(view([MAC, device({})])), "mac-mini");
    expect(r).toContain("This Mac");
    expect(r).toContain("Operator");
    expect(r).not.toContain("Revoke");
    expect(r).not.toContain("<select");
  });

  test("Operator waits for the device's name", () => {
    const asking: PairingUi = { ...EMPTY_UI, confirming: { id: "d1", kind: "operator", typed: "ARAK" } };
    const r = row(render(view([MAC, device({})]), asking), "BOREALIS");
    expect(r).toContain("Operator sees everything");
    expect(r).toContain("Type “BOREALIS” to confirm");
    expect(r).toMatch(/<button[^>]*disabled[^>]*>Make Operator/);
    const typed: PairingUi = { ...asking, confirming: { id: "d1", kind: "operator", typed: "borealis" } };
    expect(row(render(view([MAC, device({})]), typed), "BOREALIS")).not.toMatch(/<button[^>]*disabled[^>]*>Make Operator/);
  });

  test("revoking asks once and says the others are untouched", () => {
    const ui: PairingUi = { ...EMPTY_UI, confirming: { id: "d1", kind: "revoke", typed: "" } };
    const r = row(render(view([MAC, device({})]), ui), "BOREALIS");
    expect(r).toContain("It stops working on its next request");
    expect(r).toContain("Other devices are left alone");
  });

  test("a revoked device keeps its log and loses its controls", () => {
    const r = row(render(view([MAC, device({ revoked_ms: NOW - MIN, credential: false })])), "BOREALIS");
    expect(r).toContain("Revoked 1 min ago");
    expect(r).not.toContain(">Revoke<");
    expect(r).not.toContain("<select");
    expect(r).toContain("Show log");
  });

  test("the log lists reads and writes, refusals marked", () => {
    const ui: PairingUi = {
      ...EMPTY_UI,
      logs: {
        d1: [
          { ts_ms: NOW - MIN, method: "GET", path: "/api/v1/profile", status: 403, write: false },
          { ts_ms: NOW - 2 * MIN, method: "POST", path: "/api/v1/events", status: 201, write: true },
        ],
      },
    };
    const r = row(render(view([MAC, device({})]), ui), "BOREALIS");
    expect(r).toContain("Hide log");
    expect(r).toContain("Tried to read your profile · denied");
    expect(r).toContain("GET /api/v1/profile · 403");
    expect(r).toContain("hb-device-log-refused");
  });

  test("only this Mac: it says how to invite one", () => {
    expect(render(view([MAC]))).toContain("harbard hub invite --name &lt;team&gt;");
  });

  test("a pending invite shows its name, role and expiry, and a way to cancel it", () => {
    const v: PairingView = {
      ...view([MAC]),
      invites: [{ ref: "a1b2c3d4e5f6", name: "filtrada", access: "workshop", created_ms: NOW - MIN, expires_ms: NOW + 23 * 60 * MIN }],
    };
    const html = render(v);
    const r = html.split('data-testid="invite-row"')[1] ?? "";
    expect(r).toContain("<strong>filtrada</strong>");
    expect(r).toContain("Workshop");
    expect(r).toContain("Expires in 23 h · a1b2c3d4e5f6");
    expect(r).toContain(">Cancel invite<");
    expect(html).toContain("Unused invitations");
  });

  test("no pending invite, no invite list", () => {
    expect(render({ ...view([MAC, device({})]), invites: [] })).not.toContain("Unused invitations");
    expect(render(view([MAC, device({})]))).not.toContain("Unused invitations");
  });

  test("without serve it says so", () => {
    expect(render(null)).toContain("harbard isn&#x27;t responding");
  });
});
