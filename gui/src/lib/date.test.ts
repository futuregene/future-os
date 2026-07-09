import { describe, expect, it } from "vitest";
import { formatMessageTimestamp } from "./date";

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
});
