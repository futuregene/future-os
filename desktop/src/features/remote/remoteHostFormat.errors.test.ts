import type { TFunction } from "i18next";
import type { RemoteDevice } from "./remoteClient";
import { describe, expect, it } from "vitest";
import { addDeviceErrorText, deviceState } from "./remoteHostFormat";

/**
 * What "Add a device" says when it fails, and how a row reads.
 *
 * "Add a device" is two round trips, and the serialized error the frontend gets is the
 * backend's plain message. Collapsing every failure into "try again later" tells the
 * user nothing they can act on — an offline laptop, a stale sign-in and a self-hosted
 * relay that is not up yet all looked the same — so each class that has a different
 * next step gets its own sentence, and anything unrecognised keeps the neutral one.
 */
const t = ((key: string, options?: Record<string, unknown>) => {
  const values = Object.entries(options ?? {})
    .map(([name, value]) => `${name}=${String(value)}`)
    .join(",");
  return values ? `${key}(${values})` : key;
}) as unknown as TFunction<"remote">;

describe("addDeviceErrorText", () => {
  it("sends an exhausted device cap to the limit sentence", () => {
    expect(addDeviceErrorText(new Error("account already holds 5 devices"), t)).toBe("error.limitReached");
  });

  it("separates a stale sign-in from a service that rejects the request", () => {
    expect(addDeviceErrorText(new Error("not signed in"), t)).toBe("failure.account");
    // A relay that refuses the token is not the reader's account: it needs support.
    expect(addDeviceErrorText(new Error("authorization violation: token expired"), t))
      .toBe("failure.serviceSupport(code=AU001)");
  });

  it("calls an unreachable relay a network problem", () => {
    for (const message of ["connection refused", "dial tcp: lookup host failed", "timed out", "deadline exceeded"])
      expect(addDeviceErrorText(new Error(message), t), message).toBe("failure.network(code=NW001)");
  });

  it("reads a gateway error as the service's fault, not the device's", () => {
    // No platform body reaches the frontend through a 502 — the status is all there is.
    expect(addDeviceErrorText(new Error("Failed to create pairing code (HTTP 502)"), t))
      .toBe("failure.serviceLater(code=SV001)");
  });

  it("keeps the neutral sentence for anything it cannot classify", () => {
    // Nothing honest to add: guessing a cause sends the reader after the wrong problem.
    expect(addDeviceErrorText(new Error("something else entirely"), t)).toBe("error.generic");
    expect(addDeviceErrorText(undefined, t)).toBe("error.generic");
  });
});

describe("deviceState", () => {
  const device = (overrides: Partial<RemoteDevice>): RemoteDevice => ({
    connected: true,
    control: "idle",
    pairingCode: null,
    present: true,
    ...overrides,
  } as RemoteDevice);

  it("shows a device that is still waiting for its code as info", () => {
    // The pair exists before the phone connects: that row is a pending invitation, not
    // an offline device, and the difference decides whether the user waits or retries.
    expect(deviceState(device({ pairingCode: "K7QM2Z" }), t)).toEqual({ label: "deviceWaiting", tone: "info" });
  });

  it("shows a paired device that is gone as neutral, whatever the control pointer says", () => {
    expect(deviceState(device({ connected: false, control: "active" }), t))
      .toEqual({ label: "deviceOffline", tone: "neutral" });
  });
});
