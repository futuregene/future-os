import { describe, expect, it } from "vitest";
import {
  filterTimezoneChoices,
  formatOffset,
  timezoneChoices,
  timezoneLabel,
  timezoneName,
  timezoneOffsetMinutes,
} from "./timezoneChoice";

/**
 * The picker's own facts, away from the form: what a row says and what a query
 * finds.
 *
 * The one thing these tests pin hardest is the offset, because it is what the
 * reader asked for and what a wrong sign would quietly break: a zone displayed
 * on the wrong side of UTC still looks like a plausible row.
 */

const WINTER = new Date("2026-01-15T12:00:00Z");
const SUMMER = new Date("2026-07-15T12:00:00Z");

describe("formatOffset", () => {
  it("writes the sign, the hours, and the minutes only when there are some", () => {
    expect(formatOffset(0)).toBe("UTC");
    expect(formatOffset(480)).toBe("UTC+8");
    expect(formatOffset(-300)).toBe("UTC-5");
    expect(formatOffset(330)).toBe("UTC+5:30");
    expect(formatOffset(-210)).toBe("UTC-3:30");
    expect(formatOffset(345)).toBe("UTC+5:45");
  });
});

describe("timezoneOffsetMinutes", () => {
  it("reads the zone's distance from UTC at that instant", () => {
    expect(timezoneOffsetMinutes("Asia/Shanghai", WINTER)).toBe(480);
    expect(timezoneOffsetMinutes("Asia/Kolkata", WINTER)).toBe(330);
    expect(timezoneOffsetMinutes("UTC", WINTER)).toBe(0);
  });

  it("follows daylight saving, which is why the list is built for one instant", () => {
    // The same zone, two sides of its own DST change.
    expect(timezoneOffsetMinutes("Europe/Berlin", WINTER)).toBe(60);
    expect(timezoneOffsetMinutes("Europe/Berlin", SUMMER)).toBe(120);
    expect(timezoneOffsetMinutes("America/New_York", WINTER)).toBe(-300);
    expect(timezoneOffsetMinutes("America/New_York", SUMMER)).toBe(-240);
  });
});

describe("timezoneName", () => {
  it("names a zone in the reader's language instead of quoting the identifier", () => {
    const english = timezoneName("Asia/Shanghai", "en");
    expect(english).not.toBe("Asia/Shanghai");
    expect(english.toLowerCase()).toContain("china");

    const chinese = timezoneName("Asia/Shanghai", "zh");
    expect(chinese).not.toBe("Asia/Shanghai");
    // Han characters, not the Latin identifier.
    expect(/[\u4E00-\u9FFF]/.test(chinese)).toBe(true);
  });
});

describe("timezoneChoices", () => {
  it("leads with this machine's zone, then the common ones, each with an offset", () => {
    const choices = timezoneChoices("en", WINTER, "This machine's zone");
    expect(choices[0]).toEqual({ zone: "", label: "This machine's zone", offset: "" });
    expect(choices[1]!.zone).toBe("Asia/Shanghai");
    expect(choices[1]!.offset).toBe("UTC+8");
    // The full database follows, well past the common list.
    expect(choices.length).toBeGreaterThan(50);
  });

  it("offers a zone the runtime does not know when a task already stores it", () => {
    const choices = timezoneChoices("en", WINTER, "This machine's zone", "Mars/Olympus");
    expect(choices.some(choice => choice.zone === "Mars/Olympus")).toBe(true);
    // …and keeps the identifier, because there is no name to give it.
    expect(choices.find(choice => choice.zone === "Mars/Olympus")!.label).toBe("Mars/Olympus");
  });

  it("orders the full list west to east, so a reader can find their band", () => {
    const zones = timezoneChoices("en", WINTER, "This machine's zone").map(choice => choice.zone);
    // Past the common block the list is offset-ordered: the far west comes
    // before the far east, which is what makes scrolling a usable way to reach
    // a zone in between. `Asia/Tokyo` is in the common block and is selected as
    // a point *before* the tail, so the comparison is against a zone that is
    // only reachable by scrolling.
    expect(zones.indexOf("Pacific/Midway")).toBeLessThan(zones.indexOf("Pacific/Kiritimati"));
    expect(zones.indexOf("America/Anchorage")).toBeLessThan(zones.indexOf("Pacific/Kiritimati"));
    expect(zones.indexOf("Pacific/Pago_Pago")).toBeLessThan(zones.indexOf("Pacific/Kiritimati"));
  });
});

describe("filterTimezoneChoices", () => {
  const choices = timezoneChoices("en", WINTER, "This machine's zone");

  it("keeps everything for an empty query", () => {
    expect(filterTimezoneChoices(choices, "   ")).toHaveLength(choices.length);
  });

  it("matches the identifier, the display name and the offset", () => {
    const byZone = filterTimezoneChoices(choices, "berlin").map(choice => choice.zone);
    expect(byZone).toContain("Europe/Berlin");
    expect(byZone).not.toContain("Asia/Tokyo");

    // `+8` is how the reader says it, `UTC+8` how it is written.
    const spoken = filterTimezoneChoices(choices, "+8").map(choice => choice.zone);
    expect(spoken).toContain("Asia/Shanghai");
    const written = filterTimezoneChoices(choices, "UTC+8").map(choice => choice.zone);
    expect(written).toContain("Asia/Shanghai");
  });

  it("matches a city the display name leaves out", () => {
    // The localized name for Asia/Shanghai is the standard's name, not "上海".
    expect(filterTimezoneChoices(choices, "上海").map(choice => choice.zone)).toContain("Asia/Shanghai");
  });

  it("keeps this machine's own zone reachable", () => {
    expect(filterTimezoneChoices(choices, "machine").map(choice => choice.zone)).toEqual([""]);
  });
});

describe("timezoneLabel", () => {
  it("puts the offset beside the name, and says nothing extra for the machine's zone", () => {
    expect(timezoneLabel("Asia/Shanghai", "en", WINTER, "This machine's zone")).toContain("UTC+8");
    expect(timezoneLabel("", "en", WINTER, "This machine's zone")).toBe("This machine's zone");
  });
});
