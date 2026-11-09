"use client";

import { useEffect } from "react";
import { desktopDragHandler } from "@/lib/desktop-drag";

/**
 * Marks the document when the console is hosted inside the Tauri desktop
 * shell. Overlay titlebars put macOS traffic lights on top of the webview;
 * CSS keyed off `data-shell="desktop"` reserves that strip so the brand is
 * not painted under the buttons. It also moves the window from empty chrome,
 * since an Overlay titlebar leaves nothing native to drag.
 */
export function DesktopShellMark() {
  useEffect(() => {
    const isDesktop =
      new URLSearchParams(window.location.search).get("shell") === "desktop" ||
      "__TAURI_INTERNALS__" in window ||
      "__TAURI__" in window ||
      /Tauri/i.test(navigator.userAgent);
    if (!isDesktop) return;
    document.documentElement.dataset.shell = "desktop";
    const onPress = desktopDragHandler(window as Parameters<typeof desktopDragHandler>[0]);
    document.addEventListener("mousedown", onPress);
    return () => {
      document.removeEventListener("mousedown", onPress);
      delete document.documentElement.dataset.shell;
    };
  }, []);
  return null;
}
