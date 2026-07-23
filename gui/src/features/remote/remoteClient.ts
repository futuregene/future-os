import { invokeCommand } from "../../integrations/tauri/invoke";

export interface RemoteStatus {
  running: boolean;
  connected: boolean;
  natsUrl: string;
  pairId: string;
  /** One-shot pairing code (base64url) returned only by a successful start. */
  pairingCode: string | null;
  /** Unix-seconds expiry of pairingCode (for the countdown); null when no code. */
  pairingCodeExpiresAt: number | null;
  /** Web client URL for this machine (localhost); null if the web server failed to bind. */
  webUrl: string | null;
  /** Web client URL a phone on the same LAN can reach; null if unavailable. */
  webLanUrl: string | null;
  error: string | null;
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
