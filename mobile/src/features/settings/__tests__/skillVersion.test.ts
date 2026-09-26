import { isSkillUpgrade } from "../skillVersion";

test.each([
  [null, "1.0.0"],
  ["1.0.0", null],
  [undefined, undefined],
  ["", "1.0.0"],
  ["   ", "1.0.0"],
  ["1.0.0", ""],
])("a version that is missing on either side (%s -> %s) never upgrades", (installed, latest) => {
  expect(isSkillUpgrade(installed, latest)).toBe(false);
});

test.each([
  ["1.0.0", "1.0.0", false],
  ["1.0.0", "1.0.1", true],
  ["1.0.1", "1.0.0", false],
  ["1.9.9", "2.0.0", true],
  ["2.0.0", "1.9.9", false],
  ["1.9.0", "1.10.0", true],
  ["1.10.0", "1.9.0", false],
  ["1.0.9", "1.0.10", true],
  ["1.0.10", "1.0.9", false],
  // A missing segment is a zero, so 1.2 and 1.2.0 are the same release.
  ["1.2", "1.2.0", false],
  ["1.2.0", "1.2", false],
  ["1.0.0", "1.0.0.1", true],
  [" 1.0.0 ", "1.0.0", false],
])("installed %s, latest %s is an upgrade: %s", (installed, latest, expected) => {
  expect(isSkillUpgrade(installed, latest)).toBe(expected);
});

test.each([
  // Side-loaded skills carry non-numeric tags; those compare as text rather
  // than being coerced into NaN comparisons.
  ["1.0.0-alpha", "1.0.0-beta", true],
  ["1.0.0-beta", "1.0.0-alpha", false],
  ["1.0.rc", "1.0.1", false],
  ["1.0.1", "1.0.rc", true],
  ["nightly-2", "nightly-10", false],
])("non-numeric segments in installed %s, latest %s compare as text: %s", (installed, latest, expected) => {
  expect(isSkillUpgrade(installed, latest)).toBe(expected);
});
