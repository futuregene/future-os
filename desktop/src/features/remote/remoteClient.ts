import { invokeCommand } from "../../integrations/tauri/invoke";

export type RemotePhase
  = | "stopped"
    | "connecting"
    | "ready"
    | "reconnecting"
    | "refreshing"
    | "failed"
    | "revoked";

export type RemoteFailureReason
  = | "network"
    | "system_sleep"
    | "credential_expired"
    | "credential_revoked"
    | "service_authorization"
    | "remote_server"
    | "protocol"
    | "generation_unhealthy"
    | "local";

export interface RecoveryProgress {
  attempt: number;
  maxAttempts: number | null;
  since: number;
  nextRetryAt: number | null;
}

export interface RemoteStatus {
  phase: RemotePhase;
  reason: RemoteFailureReason | null;
  recovery: RecoveryProgress | null;
  natsUrl: string;
  pairId: string;
  /** One-shot pairing code (base64url) returned only by a successful start. */
  pairingCode: string | null;
  /** Unix-seconds expiry of pairingCode (for the countdown); null when no code. */
  pairingCodeExpiresAt: number | null;
  /** Desktop identity authenticated by the signed client/bridge handshake. */
  desktopId: string;
  desktopPublicKey: string;
  /** Test-only web client URL for this machine; null outside the test environment or if bind failed. */
  webUrl: string | null;
  /** Test-only LAN web client URL; null outside the test environment or if unavailable. */
  webLanUrl: string | null;
  /** Non-critical web listener failure; the main Remote link may remain ready. */
  warningCode: "web_bind" | null;
}

export interface RemoteFailurePresentation {
  messageKey: "network" | "pairing" | "serviceLater" | "serviceSupport" | "local";
  supportCode: string;
}

/**
 * Internal failure detail stays precise; the UI deliberately collapses it to
 * a small set of plain-language actions while retaining a stable support code.
 */
export function remoteFailurePresentation(
  reason: RemoteFailureReason,
): RemoteFailurePresentation {
  switch (reason) {
    case "network":
      return { messageKey: "network", supportCode: "NW001" };
    case "credential_revoked":
      return { messageKey: "pairing", supportCode: "PA001" };
    case "service_authorization":
      return { messageKey: "serviceSupport", supportCode: "AU001" };
    case "protocol":
      return { messageKey: "serviceSupport", supportCode: "PT001" };
    case "generation_unhealthy":
      return { messageKey: "serviceLater", supportCode: "RT001" };
    case "remote_server":
      return { messageKey: "serviceLater", supportCode: "SV001" };
    case "local":
      return { messageKey: "local", supportCode: "LC001" };
    case "system_sleep":
      return { messageKey: "network", supportCode: "PW001" };
    case "credential_expired":
      return { messageKey: "serviceLater", supportCode: "AU002" };
  }
}

export interface RemotePairingStatus {
  paired: boolean;
  pairId: string | null;
}

export interface RemoteStartInput {
}

export async function startRemote(input: RemoteStartInput) {
  return invokeCommand<RemoteStatus>("remote_start", { input });
}

export async function stopRemote() {
  return invokeCommand<RemoteStatus>("remote_stop");
}

export async function getRemoteStatus() {
  return invokeCommand<RemoteStatus>("remote_status");
}

export async function getRemotePairingStatus() {
  return invokeCommand<RemotePairingStatus>("remote_pairing_status");
}

export async function unpairRemote() {
  return invokeCommand<RemoteStatus>("remote_unpair");
}

export async function openUrl(url: string) {
  return invokeCommand<void>("open_url", { url });
}
