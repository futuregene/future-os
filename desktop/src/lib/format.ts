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

// Intl.NumberFormat construction is comparatively expensive, and the hot
// callers (the streaming message meta, once per push and once per tick) only
// ever format against the current UI locale — cache one formatter per locale.
const numberFormatters = new Map<string, Intl.NumberFormat>();

/** Locale-grouped number formatting with a per-locale formatter cache. */
export function formatNumber(value: number, locale: string): string {
  let formatter = numberFormatters.get(locale);
  if (!formatter) {
    formatter = new Intl.NumberFormat(locale);
    numberFormatters.set(locale, formatter);
  }
  return formatter.format(value);
}
