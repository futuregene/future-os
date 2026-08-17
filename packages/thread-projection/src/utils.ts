/**
 * Small object/string helpers with no business dependencies. Platform-neutral:
 * path handling splits on both `/` and `\` so Windows paths work too.
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

/**
 * Last path segment, splitting on both `/` and `\` so Windows paths work too.
 * Returns "" when the path has no segment; callers supply their own fallback.
 */
export function pathBasename(path: string): string {
  const segments = path.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] ?? "";
}

/**
 * Lowercase extension (without the dot) of a path's last segment, or "" when
 * there's none. Derived from `pathBasename` so a dot in a parent directory
 * never leaks into the result, and a leading-dot name (`.bashrc`) has no
 * extension.
 */
export function pathExtension(path: string): string {
  const base = pathBasename(path);
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(dot + 1).toLowerCase() : "";
}
