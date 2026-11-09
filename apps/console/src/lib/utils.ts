import { clsx, type ClassValue } from "clsx"
import { extendTailwindMerge } from "tailwind-merge"

/**
 * The type scale in globals.css is named by ROLE — text-ui, text-body,
 * text-meta — and tailwind-merge cannot know those are sizes. Its default
 * rules read an unrecognised `text-*` as a colour, so a size token and a
 * colour token landed in the same conflict group and whichever came last
 * silently deleted the other.
 *
 * That is not theoretical. The Edge strip's Acknowledge is
 * `bg-primary text-primary-foreground` plus `text-ui`, and the merge dropped
 * the foreground: the label inherited the strip's cream ink and painted itself
 * on a cream button. A white pill with no text in it, in the one control the
 * operator is meant to reach for. Thirty-three other call sites lost their
 * size the same way, in the other order.
 *
 * Declaring the scale belongs here because every component reaches the merge
 * through this one function.
 */
export const TYPE_SCALE = [
  "micro",
  "meta",
  "small",
  "ui",
  "nav",
  "body",
  "title",
  "display",
] as const

const twMerge = extendTailwindMerge({
  extend: {
    classGroups: {
      "font-size": [{ text: [...TYPE_SCALE] }],
    },
  },
})

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
