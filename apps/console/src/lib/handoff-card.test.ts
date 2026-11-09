import { expect, test } from "bun:test";
import { cardBody, cardReason, type WsCard } from "./workspace";

// A handoff from an agent to the operator (harbard note --kind handoff --to harness:human)
// shows as "Handover", never as a generic "Notice".
const card = {
  id: "handoff:01X", kind: "handoff", channel: "harbard", agents: ["a1"], since_ms: 0,
  message: "bus:01X", needs: true,
} as unknown as WsCard;

test("a handoff card reads Handover with the channel", () => {
  expect(cardReason(card)).toBe("Handoff in #harbard");
});

test("a handoff card says who passes the work and quotes it", () => {
  const view = { agents: [{ key: "a1", name: "Owen" }], messages: [{ id: "bus:01X", text: "Handing over the contract review" }] } as never;
  const body = cardBody(card, view, 0);
  expect(body.title).toContain("handed you some work");
  expect(body.text).toContain("Handing over the contract review");
});
