import { describe, expect, it } from "vitest";
import { compareVersions, isUpgradeAvailable } from "./skillVersion";

describe("compareVersions", () => {
  it("compares numeric segments numerically, not lexically", () => {
    expect(compareVersions("1.10.0", "1.9.0")).toBeGreaterThan(0);
    expect(compareVersions("1.9.0", "1.10.0")).toBeLessThan(0);
    expect(compareVersions("2.0.0", "1.9.9")).toBeGreaterThan(0);
  });

  it("treats missing trailing segments as zero", () => {
    expect(compareVersions("1.2", "1.2.0")).toBe(0);
    expect(compareVersions("1.2.1", "1.2")).toBeGreaterThan(0);
  });

  it("returns 0 for equal versions", () => {
    expect(compareVersions("1.0.0", "1.0.0")).toBe(0);
  });

  it("falls back to lexical comparison for non-numeric segments", () => {
    expect(compareVersions("1.0.0-beta", "1.0.0-alpha")).toBeGreaterThan(0);
  });
});

describe("isUpgradeAvailable", () => {
  it("is true when latest is newer", () => {
    expect(isUpgradeAvailable("1.0.0", "1.1.0")).toBe(true);
    expect(isUpgradeAvailable("1.9.0", "1.10.0")).toBe(true);
  });

  it("is false when versions are equal or latest is older", () => {
    expect(isUpgradeAvailable("1.0.0", "1.0.0")).toBe(false);
    expect(isUpgradeAvailable("2.0.0", "1.0.0")).toBe(false);
  });

  it("is false when either version is missing or blank", () => {
    expect(isUpgradeAvailable(null, "1.0.0")).toBe(false);
    expect(isUpgradeAvailable("1.0.0", null)).toBe(false);
    expect(isUpgradeAvailable("", "1.0.0")).toBe(false);
    expect(isUpgradeAvailable("1.0.0", "  ")).toBe(false);
    expect(isUpgradeAvailable(undefined, undefined)).toBe(false);
  });
});
