import { describe, expect, test } from "bun:test";
import { cardBody, cardReason, checkBody, type WsCard } from "./workspace";

// A rule approved, the context changed or the memory sink switched on
// reaches every session, so serve asks the operator "Was it you?" about each.

function check(what: string, text: string): WsCard {
  return {
    id: `check:${what}-1`,
    kind: "check",
    channel: null,
    agents: [],
    message: null,
    checkout: null,
    since_ms: 0,
    resolved_ms: null,
    needs: true,
    check: { what, text },
  };
}

describe("'Was it you?' cards", () => {
  test("a new rule is quoted and asked about", () => {
    const card = check("rule", "Guard rm");
    expect(cardReason(card)).toBe("Was this you?");
    expect(cardBody(card, { agents: [] }, 0).title).toBe("A rule was approved: “Guard rm”. Was this you?");
  });

  test("a context change, and a cleared one", () => {
    expect(checkBody({ what: "context", text: "Autorizo push y\nmerge" }).title).toBe(
      "Your Context for agents changed: “Autorizo push y merge”. Was this you?",
    );
    expect(checkBody({ what: "context", text: " " }).title).toBe("Your Context for agents was deleted. Was this you?");
  });

  test("the memory sink switched on", () => {
    expect(checkBody({ what: "memory_sink", text: "" }).title).toBe("Saving to external memory was allowed. Was this you?");
  });
});
