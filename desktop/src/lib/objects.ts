/**
 * Small object/string helpers with no business dependencies, shared across
 * features to avoid re-implementing the same guards/normalizers.
 */

/** Narrow to a plain object — not null, not an array. */
export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Collapse every whitespace run to a single space and trim. */
export function singleLine(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

/** Single-line, then hard-truncate to `max` characters with an ellipsis. */
export function truncate(value: string, max: number): string {
  const compact = singleLine(value);
  return compact.length > max ? `${compact.slice(0, max)}...` : compact;
}
