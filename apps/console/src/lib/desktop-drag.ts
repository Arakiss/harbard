/**
 * Window dragging for the desktop shell.
 *
 * The main window uses an Overlay titlebar, so macOS draws no title bar to
 * grab. WKWebView ignores CSS `app-region`, and Tauri's own
 * `data-tauri-drag-region` only fires when the attribute is on the exact
 * element pressed, so every child of a header would need it. Instead, one
 * listener moves the window when a press starts on empty chrome (the top band, the rail or
 * a screen header) and never when it starts on something you can click.
 */

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

type Host = {
  __TAURI_INTERNALS__?: { invoke?: Invoke; metadata?: { currentWindow?: { label?: string } } };
};

/** Chrome that moves the window when pressed on empty space. */
export const DRAG_ZONES = [".hb-topbar", ".hb-rail", ".hb-head", ".hb-dm-head", ".hb-away-head", ".hb-calm-head", "[data-desk-drag]"].join(",");

/** Anything a press on should act on, not move the window. */
export const NO_DRAG = [
  "a",
  "button",
  "input",
  "textarea",
  "select",
  "label",
  "summary",
  "[contenteditable]",
  "[role='button']",
  "[role='menu']",
  "[role='menuitem']",
  "[role='dialog']",
  "[role='tab']",
  "[data-no-drag]",
  ".hb-pop",
].join(",");

type Closest = { closest(selector: string): unknown };

/** True when a press on `target` should drag the window. */
export function startsDrag(target: Closest | null): boolean {
  if (!target || typeof target.closest !== "function") return false;
  if (target.closest(NO_DRAG)) return false;
  return Boolean(target.closest(DRAG_ZONES));
}

type Press = { button: number; detail: number; target: unknown; preventDefault(): void };

/** Handles one mousedown: drag on a single press, zoom on a double one. */
export function desktopDragHandler(host: Host) {
  const invoke = host.__TAURI_INTERNALS__?.invoke;
  return (event: Press): "drag" | "zoom" | "ignored" => {
    if (!invoke || event.button !== 0) return "ignored";
    if (!startsDrag(event.target as Closest | null)) return "ignored";
    const label = host.__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
    const args = label ? { label } : undefined;
    event.preventDefault();
    if (event.detail === 2) {
      void invoke("plugin:window|internal_toggle_maximize", args).catch(() => {});
      return "zoom";
    }
    void invoke("plugin:window|start_dragging", args).catch(() => {});
    return "drag";
  };
}
