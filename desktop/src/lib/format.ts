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

/**
 * A spent amount in yuan. Token prices are fractions of a fen, so two decimal
 * places would flatten a whole session to "¥0.00"; keep up to four, and only
 * the ones that carry information (0.0234 → "¥0.0234", 0.02 → "¥0.02").
 * Uses the Latin-digit `en-US` grouping on purpose: a currency figure reads the
 * same in both UI languages.
 */
export function formatCostCny(value: number): string {
  if (!Number.isFinite(value) || value <= 0)
    return "¥0";
  // `toFixed` rather than `Intl`'s default 3 fraction digits: a single request
  // can cost ¥0.0004, and the fourth digit is the one that moves.
  const rounded = value.toFixed(4);
  if (Number(rounded) === 0)
    return "¥<0.0001";
  const trimmed = rounded.replace(/0+$/, "").replace(/\.$/, "");
  const [whole, fraction] = trimmed.split(".");
  const grouped = formatNumber(Number(whole), "en-US");
  return fraction ? `¥${grouped}.${fraction}` : `¥${grouped}`;
}
