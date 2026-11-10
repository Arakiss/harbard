/**
 * Push from the bus: serve's `GET /api/v1/stream` sends one `bus` frame per
 * appended event. The console used to learn of a new message only at its next
 * two-second pull of the faces' slice; with the stream it looks at once.
 *
 * The stream says only that something changed. What changed is still read
 * from the slice and the full view, so a dropped connection costs speed, never
 * correctness: the pulls keep running underneath as before.
 */

/** The part of `EventSource` this module uses, so a test can script it. */
export interface BusSource {
  addEventListener(type: "bus", listener: () => void): void;
  close(): void;
}

export interface BusPushOptions {
  /** Opens the stream; `null` where the platform has no `EventSource`. */
  open: () => BusSource | null;
  /** Called once per settled burst of appends. */
  onChange: () => void;
  /**
   * How long a burst may keep arriving before the console looks. A lane
   * handover writes two notes in the same millisecond; one look covers both.
   */
  settleMs?: number;
}

/** Under serve's own 150 ms grouping of a burst (`STREAM_DEBOUNCE`), so a look never waits on two bursts. */
export const PUSH_SETTLE_MS = 120;

/** Opens the stream and returns the function that closes it. */
export function createBusPush({ open, onChange, settleMs = PUSH_SETTLE_MS }: BusPushOptions): () => void {
  const source = open();
  let pending: ReturnType<typeof setTimeout> | undefined;
  source?.addEventListener("bus", () => {
    if (pending !== undefined) return;
    pending = setTimeout(() => {
      pending = undefined;
      onChange();
    }, settleMs);
  });
  return () => {
    if (pending !== undefined) clearTimeout(pending);
    pending = undefined;
    source?.close();
  };
}

/** The browser's stream through the console's proxy, or `null` without `EventSource`. */
export function openBusStream(url: string): BusSource | null {
  return typeof EventSource === "undefined" ? null : new EventSource(url);
}
