import { describe, expect, it } from "vitest";
import { linuxUnavailableReasonKey } from "./linuxSandboxStatus";

describe("linuxUnavailableReasonKey", () => {
  it("maps every stable Linux remediation branch", () => {
    expect(linuxUnavailableReasonKey("binary_missing")).toBe("binaryMissing");
    expect(linuxUnavailableReasonKey("version_too_old")).toBe("versionTooOld");
    expect(linuxUnavailableReasonKey("required_feature_missing")).toBe("requiredFeatureMissing");
    expect(linuxUnavailableReasonKey("user_namespace_disabled")).toBe("userNamespaceDisabled");
    expect(linuxUnavailableReasonKey("proc_mount_restricted")).toBe("procMountRestricted");
    expect(linuxUnavailableReasonKey("probe_transport_error")).toBe("probeTransportError");
  });

  it("uses safe generic guidance for a missing or future diagnostic", () => {
    expect(linuxUnavailableReasonKey()).toBe("unknown");
    expect(linuxUnavailableReasonKey("future_code")).toBe("unknown");
  });
});
