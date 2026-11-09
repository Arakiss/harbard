import { expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { OPERATOR_AVATARS, avatarPhotoUrl, avatarThumbUrl, defaultAvatarId } from "./avatar-gallery";

const pub = join(import.meta.dir, "../../public");

test("twenty-four avatars, each with a unique id, a label and a gesture", () => {
  expect(OPERATOR_AVATARS).toHaveLength(24);
  expect(new Set(OPERATOR_AVATARS.map((a) => a.id)).size).toBe(24);
  for (const a of OPERATOR_AVATARS) {
    expect(a.label.length).toBeGreaterThan(3);
    expect(a.motion.length).toBeGreaterThan(0);
  }
});

test("a longer gallery does not change anybody's default face", () => {
  const firstTwelve = OPERATOR_AVATARS.slice(0, 12).map((a) => a.id);
  for (const name of ["Dana", "Ana", "x", "", "operator", "acme42"]) {
    expect(firstTwelve).toContain(defaultAvatarId(name));
  }
});

test("every avatar ships both of its files", () => {
  for (const a of OPERATOR_AVATARS) {
    expect(existsSync(join(pub, avatarThumbUrl(a.id)))).toBe(true);
    expect(existsSync(join(pub, avatarPhotoUrl(a.id)))).toBe(true);
  }
});

test("an operator without a photo gets a stable gallery avatar", () => {
  const ids = OPERATOR_AVATARS.map((a) => a.id);
  expect(ids).toContain(defaultAvatarId("Dana"));
  expect(defaultAvatarId("Dana")).toBe(defaultAvatarId(" dana "));
  expect(ids).toContain(defaultAvatarId(""));
});
