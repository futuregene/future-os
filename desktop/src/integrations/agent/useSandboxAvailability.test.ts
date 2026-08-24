import { describe, expect, it } from "vitest";
import { windowsSandboxAvailable } from "./useSandboxAvailability";

describe("windowsSandboxAvailable", () => {
  it("reflects the native host probe without a separate product switch", () => {
    expect(windowsSandboxAvailable({ available: true, code: "available" })).toBe(true);
    expect(windowsSandboxAvailable({ available: false, code: "write_boundary_failed" })).toBe(false);
  });
});
