import { describe, expect, it } from "vitest";
import {
  probeWindowsSandboxWithRetry,
  shouldPersistSandboxFallback,
  windowsSandboxAvailable,
} from "./useSandboxAvailability";

describe("windowsSandboxAvailable", () => {
  it("reflects the native host probe without a separate product switch", () => {
    expect(
      windowsSandboxAvailable({ available: true, code: "available" }),
    ).toBe(true);
    expect(
      windowsSandboxAvailable({
        available: false,
        code: "write_boundary_failed",
      }),
    ).toBe(false);
  });
});

describe("shouldPersistSandboxFallback", () => {
  it("falls back only for an authoritative unavailable result", () => {
    expect(
      shouldPersistSandboxFallback(
        { available: false, definitive: true, resolved: true },
        "sandbox",
      ),
    ).toBe(true);
  });

  it("preserves the saved tier after a transient probe error", () => {
    expect(
      shouldPersistSandboxFallback(
        { available: false, definitive: false, resolved: true },
        "sandbox",
      ),
    ).toBe(false);
  });
});

describe("probeWindowsSandboxWithRetry", () => {
  it("retries transient Agent connection failures", async () => {
    let attempts = 0;
    const waits: number[] = [];

    const available = await probeWindowsSandboxWithRetry(
      async () => {
        attempts += 1;
        if (attempts < 3)
          throw new Error("connection refused");
        return { available: true, code: "available" };
      },
      [0, 100, 250],
      async milliseconds => void waits.push(milliseconds),
    );

    expect(available).toBe(true);
    expect(attempts).toBe(3);
    expect(waits).toEqual([100, 250]);
  });

  it("does not retry an explicit unavailable result", async () => {
    let attempts = 0;

    const available = await probeWindowsSandboxWithRetry(
      async () => {
        attempts += 1;
        return { available: false, code: "write_boundary_failed" };
      },
      [0, 100],
      async () => {},
    );

    expect(available).toBe(false);
    expect(attempts).toBe(1);
  });

  it("rejects after exhausting transient failures", async () => {
    let attempts = 0;

    await expect(
      probeWindowsSandboxWithRetry(
        async () => {
          attempts += 1;
          throw new Error("connection refused");
        },
        [0, 100, 250],
        async () => {},
      ),
    ).rejects.toThrow("connection refused");

    expect(attempts).toBe(3);
  });
});
