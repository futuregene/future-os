import { describe, expect, it } from "vitest";
import { windowsSandboxAvailable } from "./useSandboxAvailability";

describe("windowsSandboxAvailable", () => {
  it("requires both the rollout gate and a successful host probe", () => {
    expect(windowsSandboxAvailable({ available: true, code: "available", rolloutEnabled: true })).toBe(true);
    expect(windowsSandboxAvailable({ available: true, code: "available", rolloutEnabled: false })).toBe(false);
    expect(windowsSandboxAvailable({ available: false, code: "write_boundary_failed", rolloutEnabled: true })).toBe(false);
  });
});
