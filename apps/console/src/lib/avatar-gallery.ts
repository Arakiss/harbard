/**
 * The operator's avatar gallery: vinyl-toy busts drawn in the robots' style
 * (same material, light and palette), so the human reads as one of the
 * workshop. Picking one uploads it as the profile photo, so it shows wherever
 * the photo does; the files live in `public/avatars/`.
 */
import { type TranslationKey, createT } from "@/i18n";

const t = createT("avatars");

/** Each character's gesture when you reach for it: played by CSS, once. */
export type AvatarMotion = "hop" | "tilt" | "sway" | "turn" | "nod" | "pop" | "float";

export interface GalleryAvatar {
  id: string;
  label: TranslationKey<"avatars">;
  motion: AvatarMotion;
}

/**
 * The avatars an operator can be given before choosing. Fixed at the first
 * twelve: the default is picked by position, so a longer gallery must not
 * change anybody's face.
 */
const DEFAULT_POOL = 12;

export const OPERATOR_AVATARS: readonly GalleryAvatar[] = [
  { id: "operadora", label: "operatorHeadset", motion: "nod" },
  { id: "zorro", label: "mechanicalFox", motion: "tilt" },
  { id: "piloto", label: "pilotGoggles", motion: "sway" },
  { id: "gato", label: "captainCat", motion: "tilt" },
  { id: "inventora", label: "inventorMagnifier", motion: "nod" },
  { id: "electricista", label: "electricianHardHat", motion: "pop" },
  { id: "buho", label: "engineerOwl", motion: "turn" },
  { id: "nutria", label: "otterWrench", motion: "sway" },
  { id: "panda", label: "constructionPanda", motion: "pop" },
  { id: "conejo", label: "welderRabbit", motion: "hop" },
  { id: "pinguino", label: "technicianPenguin", motion: "sway" },
  { id: "mapache", label: "handyRaccoon", motion: "tilt" },
  { id: "astronauta", label: "astronaut", motion: "float" },
  { id: "jardinera", label: "gardenerHat", motion: "sway" },
  { id: "chef", label: "chefMoustache", motion: "nod" },
  { id: "musica", label: "musicianAfro", motion: "nod" },
  { id: "bombera", label: "firefighter", motion: "pop" },
  { id: "exploradora", label: "explorer", motion: "tilt" },
  { id: "lobo", label: "wolfScarf", motion: "turn" },
  { id: "erizo", label: "hedgehogGlasses", motion: "hop" },
  { id: "rana", label: "frogRainHat", motion: "hop" },
  { id: "koala", label: "koala", motion: "sway" },
  { id: "dragon", label: "littleDragon", motion: "pop" },
  { id: "yeti", label: "yetiHat", motion: "float" },
];

/** An avatar's name, in the language in force. */
export function avatarLabel(avatar: Pick<GalleryAvatar, "label">): string {
  return t(avatar.label);
}

/** The flat backdrop every bust is drawn on: behind a gesture, the tile's edge never shows. */
export const AVATAR_BACKDROP = "#f4efe6";

/** The small file the gallery shows. */
export function avatarThumbUrl(id: string): string {
  return `/avatars/${id}-192.webp`;
}

/** The file uploaded as the photo: big enough for the 160 px Profile at 2x. */
export function avatarPhotoUrl(id: string): string {
  return `/avatars/${id}-512.webp`;
}

/**
 * The avatar an operator has before choosing one: a gallery bust picked by
 * their name, so a new install never shows bare initials. Stable for a name.
 */
export function defaultAvatarId(name: string): string {
  let h = 0;
  for (const ch of name.trim().toLowerCase()) h = (h * 31 + (ch.codePointAt(0) ?? 0)) >>> 0;
  return OPERATOR_AVATARS[h % DEFAULT_POOL]!.id;
}
