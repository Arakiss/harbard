import { describe, expect, test } from "bun:test";
import { invisibleRefusal } from "./invisible";

describe("serve's refusal of invisible characters, in the console's language", () => {
  test("names the code points serve found", () => {
    const text = invisibleRefusal(
      "`context` carries characters you cannot see (U+200B, U+E0049); they would reach the agents as text. Remove them, or retype it, and save again",
    );
    expect(text).toContain("invisible characters (U+200B, U+E0049)");
  });

  test("leaves any other refusal alone", () => {
    expect(invisibleRefusal("a status holds at most 100 characters")).toBeNull();
  });
});
