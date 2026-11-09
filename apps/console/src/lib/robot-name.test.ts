import { expect, test } from "bun:test";
import { robotFor } from "@/components/robot/identity";
import { leadKeyOf, robotName } from "./workspace";

const lead = (id: string) => ({ kind: "session" as const, handle: "coordinator", key: `session:${id}`, name: "Claude · lead" });

test("two lead sessions share a name but not a robot", () => {
  const a = lead("ae9926b1-c26c-47fb-a211-b48165c70ea1");
  const b = lead("31f3de51-e07f-4483-acba-949639b27a6c");
  expect(robotName(a)).toBe(a.key);
  expect(robotFor(robotName(a)).model.id).not.toBe(robotFor(robotName(b)).model.id);
});

test("a subagent keeps the robot of its own name", () => {
  expect(robotName({ kind: "subagent", handle: "stella-walsh", key: "a4eadbf2994e5e7ba", name: "Stella Walsh" })).toBe("Stella Walsh");
});

test("a lead message is drawn as the session that wrote it", () => {
  const agents = [{ key: "a4eadbf2994e5e7ba", lead_session: "fae4f51f" }];
  expect(leadKeyOf(agents, "session:31f3de51")).toBe("session:31f3de51");
  expect(leadKeyOf(agents, "a4eadbf2994e5e7ba")).toBe("session:fae4f51f");
  expect(leadKeyOf(agents, "unknown")).toBeNull();
  expect(leadKeyOf(agents, null)).toBeNull();
});
