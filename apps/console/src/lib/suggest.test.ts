import { describe, expect, test } from "bun:test";
import { suggestionFor } from "./suggest";

describe("the away suggestion", () => {
  test("offers Away after an absence and Available on the way back", () => {
    const away = suggestionFor({ state: "away" }, "available", true);
    expect(away?.target).toBe("away");
    expect(away?.accept).toBe("Set me to Away");
    expect(away?.dismiss).toBe("Not now");
    const back = suggestionFor({ state: "available" }, "away", true);
    expect(back?.target).toBe("available");
    expect(back?.accept).toBe("Set me to Available");
  });

  test("never offers the state already on, nor Away over Away or Fin", () => {
    expect(suggestionFor({ state: "away" }, "away", true)).toBeNull();
    expect(suggestionFor({ state: "away" }, "done", true)).toBeNull();
    expect(suggestionFor({ state: "available" }, "available", true)).toBeNull();
  });

  test("says nothing when turned off or for anything it does not know", () => {
    expect(suggestionFor({ state: "away" }, "available", false)).toBeNull();
    expect(suggestionFor({ state: "swamped" }, "available", true)).toBeNull();
    expect(suggestionFor({ nope: 1 }, "available", true)).toBeNull();
    expect(suggestionFor(null, "available", true)).toBeNull();
  });
});
