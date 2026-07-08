/**
 * Human-readable byte size using binary units (B / KiB / MiB). `null`/`undefined`
 * renders as an em dash so callers can pass optional sizes directly.
 */
export function formatBytes(size?: number | null): string {
  if (size == null)
    return "—";
  if (size < 1024)
    return `${size} B`;
  if (size < 1024 * 1024)
    return `${(size / 1024).toFixed(1)} KiB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MiB`;
}
