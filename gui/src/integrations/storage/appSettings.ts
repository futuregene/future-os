import { invokeCommand } from "../tauri/invoke";

/** Approval tier: ask everything (default), sandbox-protect (macOS only), or fully open. */
export type ApprovalTier = "manual" | "sandbox" | "off";

export interface AppSettings {
  approvalTier: ApprovalTier;
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
  approvalTier: "manual",
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
  approvalTier?: ApprovalTier;
  hiddenModels?: string[];
  remoteEnabled?: boolean;
  remotePairId?: string;
  remoteNatsUrl?: string;
  showThinking?: boolean;
}) {
  return invokeCommand<AppSettings>("update_app_settings", { input });
}
