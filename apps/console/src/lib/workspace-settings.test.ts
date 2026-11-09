import { describe, expect, test } from "bun:test";
import { placementLabel, repoName, reposIn } from "./workspace-settings";

const list = {
  repos: [
    { repo: "/w/tool", workspace: "personal", moved: false, reason: "origin owner me" },
    { repo: "/w/app", workspace: "acme", moved: false, reason: "origin owner Acme" },
    { repo: "/w/notes", workspace: "personal", moved: true, reason: "moved by hand" },
    { repo: "/w/scratch/", workspace: "personal", moved: false, reason: "default workspace" },
  ],
};

describe("a workspace's repositories", () => {
  test("are its own, by name", () => {
    expect(reposIn(list, "personal").map((r) => repoName(r.repo))).toEqual(["notes", "scratch", "tool"]);
    expect(reposIn(list, "acme").map((r) => r.repo)).toEqual(["/w/app"]);
    expect(reposIn(list, "ghost")).toEqual([]);
  });

  test("say why they are there", () => {
    expect(placementLabel(list.repos[0])).toBe("Its origin belongs to me");
    expect(placementLabel(list.repos[2])).toBe("Moved by hand");
    expect(placementLabel(list.repos[3])).toBe("No known owner: default workspace");
    expect(placementLabel({ moved: false, reason: "paired device laptop~1" })).toBe("Comes from a device with this workspace");
  });
});
