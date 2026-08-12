import { describe, expect, it } from "vitest";
import { formatDateTime, formatDuration, formatMessageTimestamp, formatTime } from "./date";

// Fixed reference "now": 2026-07-09 12:00:00 local time.
const NOW = new Date(2026, 6, 9, 12, 0, 0).getTime();

function at(msAgo: number): string {
  return new Date(NOW - msAgo).toISOString();
}

const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

describe("formatMessageTimestamp", () => {
  it("shows the just-now label under one minute", () => {
    expect(formatMessageTimestamp(at(30_000), "en", { now: NOW, justNowLabel: "just now" }))
      .toBe("just now");
  });

  it("clamps future timestamps (clock skew) to just-now", () => {
    expect(formatMessageTimestamp(at(-5_000), "en", { now: NOW, justNowLabel: "just now" }))
      .toBe("just now");
  });

  it("shows relative minutes/hours/days within a month", () => {
    expect(formatMessageTimestamp(at(3 * MIN), "en", { now: NOW })).toBe("3 minutes ago");
    expect(formatMessageTimestamp(at(2 * HOUR), "en", { now: NOW })).toBe("2 hours ago");
    expect(formatMessageTimestamp(at(5 * DAY), "en", { now: NOW })).toBe("5 days ago");
  });

  it("localizes relative labels for zh", () => {
    expect(formatMessageTimestamp(at(3 * MIN), "zh", { now: NOW })).toBe("3分钟前");
  });

  it("shows MM-dd HH:mm between one month and one year", () => {
    // 40 days before 2026-07-09 12:00 → 2026-05-30 12:00.
    expect(formatMessageTimestamp(at(40 * DAY), "en", { now: NOW })).toBe("05-30 12:00");
  });

  it("shows YYYY-MM-dd beyond one year", () => {
    // 400 days before 2026-07-09 → 2025-06-04.
    expect(formatMessageTimestamp(at(400 * DAY), "en", { now: NOW })).toBe("2025-06-04");
  });

  it("returns empty string for an invalid date", () => {
    expect(formatMessageTimestamp("not-a-date", "en", { now: NOW })).toBe("");
  });

  it("falls back to a relative 'now' label when no justNowLabel is given", () => {
    expect(formatMessageTimestamp(at(30_000), "en", { now: NOW })).toBe("now");
  });
});

describe("formatTime", () => {
  it("formats hour:minute per locale", () => {
    expect(formatTime("2026-07-09T22:51:00", "en-US")).toMatch(/10:51/);
  });
});

describe("formatDateTime", () => {
  it("formats full date + time per locale", () => {
    const label = formatDateTime("2026-07-09T22:51:00", "en-US");
    expect(label).toMatch(/07\/09\/2026|2026/);
    expect(label).toMatch(/10:51/);
  });
});

describe("formatDuration", () => {
  it("renders sub-second and sub-minute durations with subSecond resolution", () => {
    expect(formatDuration(640, { subSecond: true })).toBe("640ms");
    expect(formatDuration(1500, { subSecond: true })).toBe("1.5s");
  });

  it("renders integer seconds below a minute", () => {
    expect(formatDuration(12_000)).toBe("12s");
  });

  it("renders minutes and seconds at and above a minute", () => {
    expect(formatDuration(64_000)).toBe("1m 4s");
  });
});
