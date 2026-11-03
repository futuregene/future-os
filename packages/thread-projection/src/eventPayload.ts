/** Decoded values are process-local: a wire field cannot impersonate this
 * symbol. Spreading an event retains it; JSON serialization keeps the original
 * wire payload. Hydration never serializes a complete body back into a string. */
const decoded = Symbol("decodedEventPayload");
type Decoded = { [decoded]?: unknown };
export function hasEventPayload(event: object): boolean {
  return Object.prototype.hasOwnProperty.call(event, decoded);
}

export function withEventPayload<T extends object>(event: T, value: unknown): T {
  return Object.assign({ ...event }, { [decoded]: value });
}

/** Carry the decoded value when adapting an envelope into a projector event. */
export function copyEventPayload<T extends object>(source: object, target: T): T {
  return hasEventPayload(source)
    ? Object.assign(target, { [decoded]: (source as Decoded)[decoded] }) : target;
}

export function eventPayload(event: object, raw?: string | null): unknown {
  if (hasEventPayload(event)) return (event as Decoded)[decoded];
  if (!raw) return null;
  try { return JSON.parse(raw) as unknown; } catch { return null; }
}
