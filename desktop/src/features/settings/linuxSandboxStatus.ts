const LINUX_UNAVAILABLE_REASON_KEYS: Record<string, string> = {
  binary_missing: "binaryMissing",
  path_rejected: "pathRejected",
  binary_invalid: "binaryInvalid",
  version_unreadable: "versionUnreadable",
  version_too_old: "versionTooOld",
  required_feature_missing: "requiredFeatureMissing",
  user_namespace_disabled: "userNamespaceDisabled",
  proc_mount_restricted: "procMountRestricted",
  probe_timeout: "probeTimeout",
  probe_failed: "probeFailed",
  binary_identity_changed: "binaryIdentityChanged",
  probe_transport_error: "probeTransportError",
};

/** Map stable Agent diagnostics to user-facing reason/remediation copy. */
export function linuxUnavailableReasonKey(code?: string): string {
  return LINUX_UNAVAILABLE_REASON_KEYS[code ?? ""] ?? "unknown";
}
