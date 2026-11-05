import { describe, expect, it } from "vitest";
import { formatCheckedAt } from "./updatePresentation";

/**
 * The update page shows a *cached* result — the desktop checks in the
 * background, on a 24-hour schedule — so "you're on the latest version" needs a
 * time next to it. These pin the two forms: a clock time for today, and the date
 * in front of it once the answer is older than that.
 */
describe("formatCheckedAt", () => {
  const now = new Date(2026, 10, 5, 14, 30).getTime();

  it("reads as a padded clock time on the same day", () => {
    expect(formatCheckedAt(new Date(2026, 10, 5, 9, 7).getTime(), now)).toBe("09:07");
    expect(formatCheckedAt(new Date(2026, 10, 5, 14, 30).getTime(), now)).toBe("14:30");
  });

  it("carries the date when the result is from an earlier day", () => {
    expect(formatCheckedAt(new Date(2026, 10, 2, 23, 5).getTime(), now)).toBe("11-02 23:05");
    expect(formatCheckedAt(new Date(2025, 0, 9, 0, 0).getTime(), now)).toBe("01-09 00:00");
  });
});
