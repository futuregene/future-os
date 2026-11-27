import { describe, expect, it } from "vitest";
import { formatCredits, windowUsagePercent } from "./subscriptionPresentation";

describe("subscriptionPresentation", () => {
  it("formats credits without floating-point noise or trailing zeros", () => {
    expect(formatCredits(5)).toBe("5");
    expect(formatCredits(1.25)).toBe("1.25");
    expect(formatCredits(0.5)).toBe("0.5");
    expect(formatCredits(5.0000000001)).toBe("5");
    expect(formatCredits(Number.NaN)).toBe("0");
  });

  it("clamps the window bar to 0–100%", () => {
    expect(windowUsagePercent(5, 0)).toBe(0);
    expect(windowUsagePercent(5, 1.5)).toBe(30);
    expect(windowUsagePercent(5, 7)).toBe(100);
    // A window that grants nothing shows an empty bar instead of NaN.
    expect(windowUsagePercent(0, 3)).toBe(0);
  });
});
