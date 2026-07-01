import { invokeCommand } from "../../integrations/tauri/invoke";

export interface RemoteStatus {
  running: boolean;
  connected: boolean;
  natsUrl: string;
  pairId: string;
  error: string | null;
}

export async function startRemote(input: { natsUrl: string; pairId: string }) {
  return invokeCommand<RemoteStatus>("remote_start", { input });
}

export async function stopRemote() {
  return invokeCommand<RemoteStatus>("remote_stop");
}

export async function getRemoteStatus() {
  return invokeCommand<RemoteStatus>("remote_status");
}
