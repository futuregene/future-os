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
