import { Lato } from "next/font/google";

/**
 * Lato for the workspace shell and its rail: the humanist sans of desktop
 * team chat, at its sizes. Self-hosted at build time like the console's
 * other faces.
 */
export const lato = Lato({
  subsets: ["latin", "latin-ext"],
  weight: ["400", "700", "900"],
  variable: "--font-lato",
  display: "swap",
});
