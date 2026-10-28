import type { TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import { deviceSyncStageLabel, deviceVersionLabel } from "./remoteHostFormat";

/**
 * A `t` that answers the way i18next does: with the key when a translator is
 * absent, and with the interpolation applied. The two helpers under test are
 * only choosing *whether* to say something, so the exact sentence is the
 * translator's business and asserted through the key.
 */
const t = ((key: string, options?: Record<string, unknown>) => {
  const values = Object.entries(options ?? {})
    .map(([name, value]) => `${name}=${String(value)}`)
    .join(",");
  return values ? `${key}(${values})` : key;
}) as unknown as TFunction<"remote">;

describe("what a device says about itself", () => {
  it("names the build a device reported", () => {
    expect(deviceVersionLabel({ client: { name: "PJZ110", kind: "mobile", version: "2.0.1" } }, t))
      .toBe("clientVersion(version=2.0.1)");
  });

  // An older client sends nothing at all, and that is an answer worth printing:
  // "unknown" is what tells a reader the field is absent rather than the device
  // being current. A blank string means the same thing, because the wire spells
  // "not said" both ways.
  it("says unknown when the device never said, or said nothing", () => {
    expect(deviceVersionLabel({ client: { name: "PJZ110", kind: "mobile" } }, t))
      .toBe("clientVersionUnknown");
    expect(deviceVersionLabel({ client: { name: "PJZ110", kind: "mobile", version: "   " } }, t))
      .toBe("clientVersionUnknown");
    expect(deviceVersionLabel({ client: null }, t)).toBe("clientVersionUnknown");
  });

  it("names the step a device's sync is stuck at, and nothing when it is healthy", () => {
    expect(deviceSyncStageLabel({ client: { name: "PJZ110", kind: "mobile", syncStage: "history" } }, t))
      .toBe("clientSyncStalled(stage=history)");
    expect(deviceSyncStageLabel({ client: { name: "PJZ110", kind: "mobile" } }, t)).toBeNull();
    expect(deviceSyncStageLabel({ client: { name: "PJZ110", kind: "mobile", syncStage: "" } }, t)).toBeNull();
    expect(deviceSyncStageLabel({ client: null }, t)).toBeNull();
  });
});
