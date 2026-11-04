import { formatSentAt } from "../messages/sentAt";

// A fixed reading clock: 2026-11-04 15:20 local time, so a "today" case does
// not depend on when the suite runs and the buckets are unambiguous.
const NOW = new Date(2026, 10, 4, 15, 20, 0).getTime();
const at = (year: number, month: number, day: number, hour: number, minute: number) =>
  new Date(year, month - 1, day, hour, minute, 0).toISOString();
/** ICU puts a narrow no-break space before AM/PM in some Node builds. */
const readable = (value: string) => value.replace(/[\u202f\u00a0]/g, " ");

describe("a prompt's own clock time", () => {
  test("today reads as the clock alone", () => {
    expect(readable(formatSentAt(at(2026, 11, 4, 9, 5), "en-US", NOW))).toBe("09:05 AM");
    expect(formatSentAt(at(2026, 11, 4, 22, 47), "zh-CN", NOW)).toBe("22:47");
  });

  test("an earlier day this year adds the date", () => {
    expect(formatSentAt(at(2026, 11, 3, 22, 47), "zh-CN", NOW)).toBe("11-03 22:47");
    // Same clock day one month back is not "today" either.
    expect(formatSentAt(at(2026, 10, 4, 8, 0), "zh-CN", NOW)).toBe("10-04 08:00");
  });

  test("an earlier year adds the year", () => {
    expect(formatSentAt(at(2025, 12, 31, 23, 59), "zh-CN", NOW)).toBe("2025-12-31 23:59");
  });

  test("a stamp that cannot be read draws nothing", () => {
    expect(formatSentAt("the day before yesterday", "en-US", NOW)).toBe("");
    expect(formatSentAt("", "en-US", NOW)).toBe("");
  });

  test("the calendar day is the reader's own, not a 24-hour window", () => {
    // 00:10 today and 23:50 yesterday are 20 minutes apart and on different
    // days: the second one must not read as a bare clock time.
    const justAfterMidnight = new Date(2026, 10, 4, 0, 10, 0).getTime();
    expect(formatSentAt(at(2026, 11, 3, 23, 50), "zh-CN", justAfterMidnight)).toBe("11-03 23:50");
  });
});
