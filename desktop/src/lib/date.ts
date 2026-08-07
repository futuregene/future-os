/**
 * Short hour:minute label. `locale` is passed through to `Intl` — leave it
 * undefined for the host default; UI callers should pass the active i18n
 * language so a zh UI doesn't show English 12-hour times. `lib/` stays
 * dependency-free, so the language is threaded in rather than imported here.
 */
export function formatTime(value: string, locale?: string) {
  return new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

/**
 * Full date + time label ("2026/07/09 22:51"). Like `formatTime`, `locale` is
 * threaded through to `Intl` so a zh UI gets a localized year-month-day order.
 */
export function formatDateTime(value: string, locale?: string) {
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;
/**
 * Relative-time cutoffs use fixed day counts (not calendar months/years) so the
 * boundaries are deterministic and don't shift with month length.
 */
const MONTH_MS = 30 * DAY_MS;
const YEAR_MS = 365 * DAY_MS;

function pad2(n: number) {
  return String(n).padStart(2, "0");
}

/**
 * Chat-message timestamp with graduated resolution:
 *   - < 1 minute            → `justNowLabel` ("刚刚" / "just now")
 *   - < 1 month (30 days)   → relative ("3 分钟前" / "3 minutes ago") via Intl
 *   - < 1 year  (365 days)  → "MM-dd HH:mm"
 *   - ≥ 1 year              → "YYYY-MM-dd"
 *
 * Relative labels come from `Intl.RelativeTimeFormat`, which localizes the
 * wording/plural for the passed `locale` — no extra i18n strings needed. The
 * absolute buckets use fixed numeric patterns on purpose (a localized `Intl`
 * date would reorder zh to 年/月/日 and drop the requested `MM-dd` / `YYYY-MM-dd`
 * shape). `lib/` stays i18n-free, so the "just now" wording and `now` (for tests
 * / a shared ticker) are threaded in by the caller.
 */
export function formatMessageTimestamp(
  value: string,
  locale?: string,
  options?: { now?: number; justNowLabel?: string },
): string {
  const date = new Date(value);
  const time = date.getTime();
  if (Number.isNaN(time))
    return "";

  const now = options?.now ?? Date.now();
  // Clamp future timestamps (clock skew) to 0 so they read as "just now".
  const diff = Math.max(0, now - time);

  if (diff < MONTH_MS) {
    if (diff < MINUTE_MS) {
      if (options?.justNowLabel)
        return options.justNowLabel;
      return new Intl.RelativeTimeFormat(locale, { numeric: "auto" }).format(0, "second");
    }
    const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "always" });
    if (diff < HOUR_MS)
      return rtf.format(-Math.floor(diff / MINUTE_MS), "minute");
    if (diff < DAY_MS)
      return rtf.format(-Math.floor(diff / HOUR_MS), "hour");
    return rtf.format(-Math.floor(diff / DAY_MS), "day");
  }

  if (diff < YEAR_MS)
    return `${pad2(date.getMonth() + 1)}-${pad2(date.getDate())} ${pad2(date.getHours())}:${pad2(date.getMinutes())}`;

  return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())}`;
}

/**
 * Compact elapsed-time label: "12s", "1m 4s" (integer seconds). Pass
 * `subSecond` for finer resolution below a minute ("640ms", "1.5s"), used by
 * the run inspector where tool durations are often sub-second.
 */
export function formatDuration(ms: number, options?: { subSecond?: boolean }) {
  if (options?.subSecond) {
    if (ms < 1000)
      return `${ms}ms`;
    if (ms < 60_000)
      return `${(ms / 1000).toFixed(1)}s`;
  }
  const totalSeconds = Math.max(0, Math.round(ms / 1000));
  if (totalSeconds < 60) {
    return `${totalSeconds}s`;
  }
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}m ${seconds}s`;
}
