import type { RemoteStatus } from "./remoteClient";
import { describe, expect, it } from "vitest";
import { remoteConnectionPresentation, shouldShowRemoteConnectionHint } from "./remoteClient";

describe("customer remote status", () => {
  const status = (phase: RemoteStatus["phase"], agentAvailable = true, pairId = "pair_test") => ({ phase, agentAvailable, pairId }) as RemoteStatus;
  it("uses yellow for every active connection stage, including local service loss", () => {
    for (const phase of ["connecting", "reconnecting", "refreshing"] as const)
      expect(remoteConnectionPresentation(status(phase))?.level).toBe("connecting");
    expect(remoteConnectionPresentation(status("ready", false))).toMatchObject({
      level: "connecting",
      customerState: "devicePreparing",
      action: "restartDesktop",
      supportCode: "LC003",
      titleKey: "statusDevicePreparing",
    });
    expect(remoteConnectionPresentation(status("ready"))?.level).toBe("connected");
  });
  it("uses red for stopped paired access and terminal errors; hides unpaired idle", () => {
    for (const phase of ["stopped", "failed", "revoked"] as const)
      expect(remoteConnectionPresentation(status(phase))?.level).toBe("disconnected");
    expect(remoteConnectionPresentation(status("stopped", true, ""))).toBeNull();
    expect(remoteConnectionPresentation(null)).toBeNull();
  });
  it("keeps customer category, action, and technical code as separate layers", () => {
    expect(remoteConnectionPresentation({
      ...status("failed"),
      reason: "protocol",
    })).toMatchObject({
      level: "disconnected",
      customerState: "serviceUnavailable",
      action: "contactSupport",
      supportCode: "PT001",
      titleKey: "statusServiceUnavailable",
    });
    expect(remoteConnectionPresentation({
      ...status("reconnecting"),
      reason: "network",
    })).toMatchObject({
      level: "connecting",
      customerState: "networkUnavailable",
      action: "checkNetwork",
      supportCode: "NW001",
      titleKey: "statusNetworkUnavailable",
    });
  });
  it("does not describe the first pairing connection as recovery", () => {
    expect(shouldShowRemoteConnectionHint(status("connecting", true, ""))).toBe(false);
    expect(shouldShowRemoteConnectionHint(status("connecting"))).toBe(true);
    expect(shouldShowRemoteConnectionHint(status("reconnecting"))).toBe(true);
    expect(shouldShowRemoteConnectionHint(status("ready", false))).toBe(true);
  });
});
