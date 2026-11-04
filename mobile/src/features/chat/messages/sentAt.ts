/**
 * When a prompt was sent, as the line under its bubble reads it.
 *
 * The desktop puts the same fact in a message's author line
 * (`desktop/src/lib/date.ts` `formatMessageTimestamp`), with a relative label
 * for a recent message. This one stays absolute on purpose: a transcript row
 * here is memoized and nothing ticks it, so "3 minutes ago" would freeze at
 * whatever it said when the row first mounted — a wrong time, silently. A
 * clock time ages correctly without a re-render.
 *
 * The graduation is the desktop's: the clock alone for today, the date added
 * once the message is older than that, the year once it is older than this
 * year. The clock comes from `Intl`, so an English UI reads 12-hour and a
 * Chinese one 24-hour like the rest of the app's times; the date buckets stay
 * numeric on purpose (a localized date would reorder year-month-day out of the
 * shape they ask for). `locale` and `now` are threaded in rather than read
 * here — `lib`-style helpers in this app take their surroundings as arguments.
 */
export function formatSentAt(value: string, locale?: string, now: number = Date.now()): string {
  const date = new Date(value);
  const time = date.getTime();
  // An unparseable stamp draws nothing rather than an "Invalid Date" line.
  if (Number.isNaN(time)) return "";

  const reference = new Date(now);
  const clock = new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
  if (sameDay(date, reference)) return clock;

  const day = `${pad2(date.getMonth() + 1)}-${pad2(date.getDate())}`;
  if (date.getFullYear() === reference.getFullYear()) return `${day} ${clock}`;
  return `${date.getFullYear()}-${day} ${clock}`;
}

/** Same calendar day on the reader's own clock, not within 24 hours. */
function sameDay(date: Date, reference: Date): boolean {
  return date.getFullYear() === reference.getFullYear()
    && date.getMonth() === reference.getMonth()
    && date.getDate() === reference.getDate();
}

function pad2(value: number): string {
  return String(value).padStart(2, "0");
}
