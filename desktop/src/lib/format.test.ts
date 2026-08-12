import { describe, expect, it } from "vitest";
import { formatBytes, formatNumber } from "./format";

describe("formatBytes", () => {
  it("renders null/undefined as an em dash", () => {
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(undefined)).toBe("—");
  });

  it("renders bytes below 1 KiB as-is", () => {
    expect(formatBytes(512)).toBe("512 B");
  });

  it("renders KiB below 1 MiB with one decimal", () => {
    expect(formatBytes(2048)).toBe("2.0 KiB");
  });

  it("renders MiB at and above 1 MiB with one decimal", () => {
    expect(formatBytes(3 * 1024 * 1024)).toBe("3.0 MiB");
  });
});

describe("formatNumber", () => {
  it("groups digits per locale and reuses the cached formatter", () => {
    expect(formatNumber(1234567, "en-US")).toBe("1,234,567");
    // Second call with the same locale hits the cache path.
    expect(formatNumber(42, "en-US")).toBe("42");
  });
});
