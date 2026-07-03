import { invokeCommand } from "../tauri/invoke";

export interface AppSettings {
  autoApprove: boolean;
  hiddenModels: string[];
  /** Remote control: whether it should be running. */
  remoteEnabled: boolean;
  /** Remote control: pairing id (isolation unit / subject prefix). */
  remotePairId: string;
  /** Remote control: NATS client-port URL the GUI backend connects to. */
  remoteNatsUrl: string;
  /** Show the model's thinking/reasoning content in the chat. Off by default. */
  showThinking: boolean;
}

/** Fallback used before the persisted settings load. */
export const DEFAULT_APP_SETTINGS: AppSettings = {
  autoApprove: false,
  hiddenModels: [],
  remoteEnabled: false,
  remotePairId: "DEVPAIR",
  remoteNatsUrl: "nats://localhost:4222",
  showThinking: false,
};

export async function getAppSettings() {
  return invokeCommand<AppSettings>("get_app_settings");
}

export async function updateAppSettings(input: {
  autoApprove?: boolean;
  hiddenModels?: string[];
  remoteEnabled?: boolean;
  remotePairId?: string;
  remoteNatsUrl?: string;
  showThinking?: boolean;
}) {
  return invokeCommand<AppSettings>("update_app_settings", { input });
}
