/**
 * How long ago a conversation last moved, as the recency list says it.
 *
 * The catalogue hands out `lastMessageAt` (a unix-millis stamp) and the list
 * sorts by it; a stamp is not readable, so the row says it in words. Nothing
 * here rounds in the reader's favour: "2 min ago" may be 2m59s, and a value in
 * the future — two machines' clocks can disagree by seconds — reads as "just
 * now" rather than as a negative age.
 *
 * The calendar cases (yesterday, a date) are decided on local midnight, not on
 * 24-hour spans: something from 22:00 yesterday, read at 00:30, is "yesterday"
 * rather than "2 h ago" — and something from 23:50 read at 00:30 is "40 min
 * ago", because inside an hour the minutes are the more precise truth and both
 * readings agree.
 */

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
/** Past this many days the row prints the date instead of an age. */
const DAYS_BEFORE_DATE = 7;

const MONTHS_EN = [
  "Jan", "Feb", "Mar", "Apr", "May", "Jun",
  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/** Midnight of `at`'s own local day, so two stamps compare as calendar days. */
function startOfDay(at: number): number {
  const date = new Date(at);
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/** `Oct 3` / `2025年10月3日`, with the year only when it is not this one. */
function dateLabel(at: number, now: number, chinese: boolean): string {
  const date = new Date(at);
  const month = date.getMonth();
  const day = date.getDate();
  const year = date.getFullYear();
  const thisYear = new Date(now).getFullYear();
  if (chinese)
    return year === thisYear
      ? `${month + 1}月${day}日`
      : `${year}年${month + 1}月${day}日`;
  return year === thisYear
    ? `${MONTHS_EN[month]} ${day}`
    : `${MONTHS_EN[month]} ${day}, ${year}`;
}

export type RelativeTimeTranslate = (
  key: string,
  options?: { count?: number },
) => string;

export function formatRelativeTime(
  at: number,
  now: number,
  language: string,
  t: RelativeTimeTranslate,
): string {
  const chinese = language.startsWith("zh");
  const elapsed = now - at;
  if (elapsed < MINUTE) return t("sessions.relativeNow");
  if (elapsed < HOUR)
    return t("sessions.relativeMinutes", { count: Math.floor(elapsed / MINUTE) });
  const days = Math.round((startOfDay(now) - startOfDay(at)) / (24 * HOUR));
  // Same calendar day: an age in hours (midnight may cut a "same day" span in
  // half, which is why the day count and the hour count are computed apart).
  if (days <= 0) return t("sessions.relativeHours", { count: Math.floor(elapsed / HOUR) });
  if (days === 1) return t("sessions.relativeYesterday");
  if (days < DAYS_BEFORE_DATE) return t("sessions.relativeDays", { count: days });
  return dateLabel(at, now, chinese);
}
