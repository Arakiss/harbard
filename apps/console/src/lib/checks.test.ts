import { describe, expect, test } from "bun:test";
import { checkLine } from "./checks";

const DAY = Date.UTC(2026, 10, 7, 12);

describe("recent 'Was it you?' checks", () => {
  test("say what changed and what the operator answered", () => {
    expect(checkLine({ id: "a", what: "rule", text: "Guard rm", since_ms: DAY, outcome: "kept", answered_ms: DAY })).toBe(
      "Rule approved: “Guard rm” · 7 November · kept on 7 November",
    );
    expect(checkLine({ id: "b", what: "memory_sink", since_ms: DAY, outcome: "revoked", answered_ms: DAY })).toBe(
      "Saving to external memory allowed · 7 November · revoked on 7 November",
    );
    expect(checkLine({ id: "c", what: "context", text: "", since_ms: DAY })).toBe("Context deleted · 7 November · unanswered, in Needs you");
  });
});
