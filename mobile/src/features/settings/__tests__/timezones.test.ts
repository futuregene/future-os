import { allTimezones, matchTimezones, timezoneOptions } from "../timezones";

// A task's trigger times are read on the desktop's clock, so the picker has to
// offer the names that clock accepts — not the four the form used to hard-code.

test("offers the runtime's zone list, and never an empty one", () => {
  const zones = allTimezones();
  expect(zones.length).toBeGreaterThan(1);
  // The list is read once and shared: the form asks for it on every keystroke.
  expect(allTimezones()).toBe(zones);
});

test("keeps a stored zone the list does not contain", () => {
  const zones = timezoneOptions("Mars/Olympus_Mons");
  expect(zones[0]).toBe("Mars/Olympus_Mons");
  expect(zones.filter(zone => zone === "Mars/Olympus_Mons")).toHaveLength(1);
});

test("a blank stored zone adds nothing", () => {
  expect(timezoneOptions("  ")).toBe(allTimezones());
});

test("matches on a fragment, case-insensitively, and underscores for spaces", () => {
  const berlin = matchTimezones("berlin", "");
  expect(berlin.every(zone => zone.toLowerCase().includes("berlin"))).toBe(true);

  // A phone keyboard types a space where the IANA name has an underscore.
  const newYork = matchTimezones("new york", "");
  expect(newYork).toContain("America/New_York");
});

test("an empty query is the whole list, not no matches", () => {
  expect(matchTimezones("   ", "")).toBe(timezoneOptions(""));
});

test("a query nobody matches answers with nothing, so the form can say so", () => {
  expect(matchTimezones("not a zone at all", "")).toEqual([]);
});
