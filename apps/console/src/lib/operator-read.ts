/** Whether a confirmed read cursor covers an event, in retained append order. */
export function readCoversEvent(
  events: readonly { id: string }[],
  eventId: string,
  readThrough: string | undefined,
): boolean {
  if (!readThrough) return false;
  if (readThrough === eventId) return true;
  const eventIndex = events.findIndex((event) => event.id === eventId);
  const readIndex = events.findIndex((event) => event.id === readThrough);
  if (eventIndex >= 0 && readIndex >= 0) return readIndex >= eventIndex;
  // Outside the retained batch only a strictly later timestamp is evidence.
  // A ULID's random suffix cannot order events from the same millisecond.
  const ulid = /^[0-7][0-9A-HJKMNP-TV-Z]{25}$/;
  return ulid.test(eventId) && ulid.test(readThrough) &&
    readThrough.slice(0, 10) > eventId.slice(0, 10);
}
