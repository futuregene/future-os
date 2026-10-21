import { formatRelativeTime } from "../relativeTime";

/** A `t` that keeps the key and its count, so a test reads what it was given. */
const t = (key: string, options?: { count?: number }) =>
  options?.count === undefined ? key : `${key}:${options.count}`;
const en = (at: number, now: number) => formatRelativeTime(at, now, "en", t);
const zh = (at: number, now: number) => formatRelativeTime(at, now, "zh-Hans", t);

const minute = 60_000;
const hour = 60 * minute;
// A fixed Tuesday, mid-afternoon, so the calendar cases are not clock-dependent.
const now = new Date(2026, 9, 21, 15, 0, 0).getTime();
const at = (day: number, hours: number, minutes = 0) =>
  new Date(2026, 9, day, hours, minutes, 0).getTime();

describe("how old a conversation reads", () => {
  it("says nothing more precise than 'just now' under a minute", () => {
    expect(en(now - 20_000, now)).toBe("sessions.relativeNow");
    expect(zh(now - 20_000, now)).toBe("sessions.relativeNow");
  });

  it("counts minutes, then hours within the same day", () => {
    expect(en(now - 5 * minute, now)).toBe("sessions.relativeMinutes:5");
    expect(zh(now - 5 * minute, now)).toBe("sessions.relativeMinutes:5");
    expect(en(now - 3 * hour, now)).toBe("sessions.relativeHours:3");
  });

  it("says yesterday for yesterday, however few hours ago it was", () => {
    // 00:30 today, 22:00 yesterday: two and a half hours, and still "yesterday" —
    // an age in hours is what made the list lie about the day it happened on.
    const justAfterMidnight = new Date(2026, 9, 21, 0, 30, 0).getTime();
    expect(en(at(20, 22), justAfterMidnight)).toBe("sessions.relativeYesterday");
    expect(zh(at(20, 22), justAfterMidnight)).toBe("sessions.relativeYesterday");
    // Inside an hour the minutes are the more precise truth, and the two agree.
    expect(en(at(20, 23, 50), justAfterMidnight)).toBe("sessions.relativeMinutes:40");
  });

  it("counts days for the rest of the week, then prints the date", () => {
    expect(en(at(19, 12), now)).toBe("sessions.relativeDays:2");
    expect(en(at(15, 12), now)).toBe("sessions.relativeDays:6");
    // A week old is a date, and the year is left off while it is this one.
    expect(en(at(14, 12), now)).toBe("Oct 14");
    expect(zh(at(14, 12), now)).toBe("10月14日");
    expect(en(at(3, 12), now)).toBe("Oct 3");
  });

  it("prints the year when the conversation is older than this one", () => {
    expect(en(new Date(2025, 11, 30, 12, 0, 0).getTime(), now)).toBe("Dec 30, 2025");
    expect(zh(new Date(2025, 11, 30, 12, 0, 0).getTime(), now)).toBe("2025年12月30日");
  });

  it("reads a stamp in the future as 'just now'", () => {
    // Two machines' clocks disagree by seconds; a negative age is not a thing
    // a list can show.
    expect(en(now + 30_000, now)).toBe("sessions.relativeNow");
  });
});
