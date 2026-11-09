"use client";

import * as React from "react";
import type { LogoState } from "@/lib/activity";
import { MARK } from "./mark-geometry";
import type { Mark, MarkGeometry, MarkShape } from "./parse-mark";

/** Which token paints the favicon's dot in each logo state. */
function faviconDotToken(state: LogoState): string {
  if (state === "blocked") return "mark-dot-red";
  if (state === "needs") return "mark-dot-amber";
  if (state === "disconnected" || state === "connecting") return "mark-dot-grey";
  return "mark-dot";
}

/** Only what a colour token can hold reaches the SVG. */
function safe(v: string): string {
  return v.replace(/[^#a-zA-Z0-9(),.% -]/g, "");
}

function svgShape(sh: MarkGeometry | MarkShape, fill: string, mask?: string): string {
  const m = mask ? ` mask="url(#${mask})"` : "";
  if (sh.kind === "path") return `<path d="${sh.d}"${sh.fillRule ? ` fill-rule="${sh.fillRule}"` : ""} fill="${fill}"${m}/>`;
  if (sh.kind === "circle") return `<circle cx="${sh.cx}" cy="${sh.cy}" r="${sh.r}" fill="${fill}"${m}/>`;
  return `<rect x="${sh.x}" y="${sh.y}" width="${sh.width}" height="${sh.height}" fill="${fill}"${m}/>`;
}

/**
 * The tab's icon: the mark on its tile, the bubble in the rail ink and the
 * dot in the state's colour. Still: a favicon never animates.
 */
function faviconSvg(colours: { tile: string; ink: string; dot: string }, mark: Mark = MARK): string {
  // Mask luminance, not paint: white keeps, black cuts.
  const masks = mark.masks
    .map((m) => `<mask id="${m.id}" maskUnits="userSpaceOnUse">${m.shapes.map((sh) => svgShape(sh, sh.keep ? "white" : "black")).join("")}</mask>`)
    .join("");
  const shapes = mark.shapes.map((sh) => svgShape(sh, safe(sh.tone === "presence" ? colours.dot : colours.ink), sh.mask)).join("");
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><defs>${masks}</defs><rect width="64" height="64" rx="14" fill="${safe(colours.tile)}"/><g transform="translate(6 6) scale(.8125)">${shapes}</g></svg>`;
}

/** Keeps the page's icon on the logo's state colour, read from the shell's tokens. */
export function useFavicon(state: LogoState) {
  React.useEffect(() => {
    const root = document.querySelector<HTMLElement>(".hb");
    if (!root) return;
    const css = getComputedStyle(root);
    const token = (n: string) => css.getPropertyValue(`--hb-${n}`).trim();
    const colours = { tile: token("ws-tile"), ink: token("rail-ink"), dot: token(faviconDotToken(state)) };
    if (!colours.tile || !colours.ink || !colours.dot) return;
    const href = `data:image/svg+xml,${encodeURIComponent(faviconSvg(colours))}`;
    let link = document.querySelector<HTMLLinkElement>("link[data-hb-favicon]");
    if (!link) {
      // Appended last, so the browser prefers it over the static icon.
      link = document.createElement("link");
      link.rel = "icon";
      link.type = "image/svg+xml";
      link.dataset.hbFavicon = "";
      document.head.appendChild(link);
    }
    if (link.getAttribute("href") !== href) link.setAttribute("href", href);
  }, [state]);
}
