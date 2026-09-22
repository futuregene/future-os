import { describe, expect, it } from "vitest";
import { formatBytes, formatCostCny, formatNumber } from "./format";

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

describe("formatCostCny", () => {
  it("keeps the sub-fen precision a session actually reaches", () => {
    expect(formatCostCny(0.0234)).toBe("¥0.0234");
    expect(formatCostCny(0.02)).toBe("¥0.02");
    expect(formatCostCny(12.5)).toBe("¥12.5");
    expect(formatCostCny(1234.56789)).toBe("¥1,234.5679");
  });

  it("never renders a negative or non-finite amount", () => {
    expect(formatCostCny(0)).toBe("¥0");
    expect(formatCostCny(-1)).toBe("¥0");
    expect(formatCostCny(Number.NaN)).toBe("¥0");
  });

  it("does not round a real amount down to nothing", () => {
    expect(formatCostCny(0.00001)).toBe("¥<0.0001");
  });
});
