import { describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { createElement as h } from "react";
import { renderToString } from "react-dom/server";
import { FACES } from "@/lib/activity";
import { assetSize, robotFor } from "./identity";
import { Robot } from "./robot";
import { ROBOT_MODELS, ROBOT_SIZES } from "./robots";

const TILES: Record<string, number> = { "#dfe8d8": 1, "#e6eef2": 2, "#f2e6dc": 3, "#e8e3f2": 4 };
// The pick expected for these names: model, light, tile.
const EXPECTED: [string, string, string, string][] = [
  ["keychain", "antenna", "mint", "#e8e3f2"],
  ["harbard-themes", "e-off", "mint", "#f2e6dc"],
  ["identity", "propeller", "warm", "#f2e6dc"],
  ["parser", "flower", "ice", "#e8e3f2"],
  ["kc-review-2", "bowler", "ice", "#f2e6dc"],
  ["mind-backup", "flower", "lilac", "#e6eef2"],
  ["harbard-chat", "lantern", "ice", "#f2e6dc"],
  ["keychain-review", "e-off", "lilac", "#f2e6dc"],
  ["harbard-view", "dials", "ice", "#dfe8d8"],
  ["recall-perf", "lantern", "warm", "#e6eef2"],
  ["ñandú-ü", "lantern", "mint", "#f2e6dc"],
];

describe("one robot per agent, stable", () => {
  test("the same name always picks the same robot", () => {
    for (const [name, model, glow, tile] of EXPECTED) {
      const r = robotFor(name);
      expect([name, r.model.id, r.glow, r.tile]).toEqual([name, model, glow, TILES[tile]!]);
    }
  });

  test("the same name renders the same robot every time, in every view size", () => {
    for (const [name] of EXPECTED) {
      const shots = [24, 36, 48, 56, 64].map((size) => {
        const html = renderToString(h(Robot, { name, size }));
        return [html.match(/data-robot="([^"]+)"/)![1], html.match(/--tile:([^;"]+)/)![1], html.match(/--g:([^;"]+)/)![1]];
      });
      expect(new Set(shots.map((s) => s.join("|"))).size).toBe(1);
      expect(renderToString(h(Robot, { name, size: 36 }))).toBe(renderToString(h(Robot, { name, size: 36 })));
    }
  });

  test("models are twelve, in order, and every asset exists", () => {
    expect(ROBOT_MODELS.map((m) => m.id)).toEqual(["e-off", "antenna", "dome", "dials", "radio", "teapot", "headset", "propeller", "lantern", "bowler", "flower", "stripes"]);
    const pub = join(import.meta.dir, "..", "..", "..", "public", "robots");
    for (const m of ROBOT_MODELS) {
      for (const s of ROBOT_SIZES) {
        expect(existsSync(join(pub, `${m.id}-${s}.webp`))).toBe(true);
        expect(existsSync(join(pub, `${m.id}-mask-${s}.png`))).toBe(true);
      }
    }
  });

  test("idle breathing and blinking are desynchronised per agent", () => {
    const timings = new Set(EXPECTED.map(([n]) => { const r = robotFor(n); return `${r.blinkS}|${r.blinkDelayS}|${r.breatheDelayS}`; }));
    expect(timings.size).toBeGreaterThan(EXPECTED.length - 2);
  });

  test("a 36 px robot uses the 72 px body and mask at 2x, a 150 px one the 432", () => {
    expect([assetSize(24), assetSize(36), assetSize(48), assetSize(64), assetSize(108), assetSize(150)]).toEqual([72, 72, 144, 144, 216, 432]);
    const html = renderToString(h(Robot, { name: "keychain", size: 36 }));
    expect(html).toContain("antenna-mask-72.png");
    expect(html).toContain('sizes="36px"');
    expect(html).toContain("antenna-72.webp 72w");
  });
});

describe("faces", () => {
  test("every face renders, clipped to the model's screen box, with the reflection layer kept", () => {
    for (const face of FACES) {
      const html = renderToString(h(Robot, { name: "harbard-view", size: 150, face }));
      expect(html).toContain(`rb-s-${face}`);
      expect(html).toContain("hb-bot-glass");
      expect(html).toContain("--mask:url(/robots/dials-mask-432.png)");
      const m = ROBOT_MODELS.find((x) => x.id === "dials")!;
      expect(html).toContain(`--x:${m.screen.x}`);
    }
  });

  test("at 40 px and below the tiny variant, above it the full face", () => {
    const cls = (size: number) => renderToString(h(Robot, { name: "a", size })).match(/class="(hb-bot [^"]*)"/)![1]!.split(" ");
    expect(cls(40)).toContain("tiny");
    expect(cls(41)).not.toContain("tiny");
  });

  test("below 56 px the robot holds still (a breath would be under a device pixel) and still blinks", () => {
    const cls = (size: number) => renderToString(h(Robot, { name: "a", size })).match(/class="(hb-bot [^"]*)"/)![1]!.split(" ");
    expect(cls(48)).toContain("still");
    expect(cls(56)).not.toContain("still");
    expect(renderToString(h(Robot, { name: "a", size: 36 }))).toContain('class="rb-blink"');
  });

  test("blocked and waiting faces turn amber; the rest keep the agent's light", () => {
    const g = (state: "blocked" | "waiting" | "thinking") => renderToString(h(Robot, { name: "keychain", activity: { state, since_ms: 0 } })).match(/--g:([^;"]+)/)![1];
    expect(g("blocked")).toBe("var(--hb-robot-glow-amber)");
    expect(g("waiting")).toBe("var(--hb-robot-glow-amber)");
    expect(g("thinking")).toBe("var(--hb-robot-glow-mint)");
  });

  test("the verified face follows serve's state only", () => {
    // A finished agent whose DONE claims verification still shows finished::
    // the face reads activity.state, which serve sets from receipts.
    const claim = renderToString(h(Robot, { name: "keychain", activity: { state: "finished", since_ms: 0 } }));
    expect(claim).toContain('data-face="finished"');
    expect(claim).not.toContain("rb-check");
    const proven = renderToString(h(Robot, { name: "keychain", activity: { state: "verified", since_ms: 0 } }));
    expect(proven).toContain('data-face="verified"');
    expect(proven).toContain("rb-check");
  });
});
