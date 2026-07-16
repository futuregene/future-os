import { invokeCommand } from "../tauri/invoke";

/** Approval tier: fully open (default), ask everything, or sandbox-protect (macOS only). */
export type ApprovalTier = "off" | "manual" | "sandbox";

export interface AppSettings {
  approvalTier: ApprovalTier;
  hiddenModels: string[];
  /** Remote control: whether it should be running. */
  remoteEnabled: boolean;
  /** Remote control: pairing id (isolation unit / subject prefix). */
  remotePairId: string;
  /** Remote control: NATS client-port URL the GUI backend connects to. */
  remoteNatsUrl: string;
  /** Show the model's thinking/reasoning content in the chat. On by default. */
  showThinking: boolean;
  /**
   * Silently upgrade installed skills to their latest version on app open (and
   * immediately when toggled on). Off by default.
   */
  autoUpgradeSkills: boolean;
}

/** Fallback used before the persisted settings load. */
export const DEFAULT_APP_SETTINGS: AppSettings = {
  approvalTier: "off",
  hiddenModels: [],
  remoteEnabled: false,
  remotePairId: "DEVPAIR",
  remoteNatsUrl: "nats://localhost:4222",
  showThinking: true,
  autoUpgradeSkills: false,
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
  autoUpgradeSkills?: boolean;
}) {
  return invokeCommand<AppSettings>("update_app_settings", { input });
}
