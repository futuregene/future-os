import { invokeCommand } from "../tauri/invoke";

/** Approval tier: fully open (default), ask everything, or sandbox-protect (macOS only). */
export type ApprovalTier = "off" | "manual" | "sandbox";

export interface AppSettings {
  approvalTier: ApprovalTier;
  hiddenModels: string[];
  /** Show the model's thinking/reasoning content in the chat. Off by default. */
  showThinking: boolean;
  /**
   * Silently upgrade installed skills to their latest version on app open (and
   * immediately when toggled on). Off by default.
   */
  autoUpgradeSkills: boolean;
  /**
   * Auto-connect the single paired remote device on app launch (the user can
   * still disconnect by hand). Off by default. The Remote feature is dev-only,
   * so this only takes effect on non-release builds.
   */
  autoConnectRemote: boolean;
}

/** Fallback used before the persisted settings load. */
export const DEFAULT_APP_SETTINGS: AppSettings = {
  approvalTier: "off",
  hiddenModels: [],
  showThinking: false,
  autoUpgradeSkills: false,
  autoConnectRemote: false,
};

export async function getAppSettings() {
  return invokeCommand<AppSettings>("get_app_settings");
}

export async function updateAppSettings(input: {
  approvalTier?: ApprovalTier;
  hiddenModels?: string[];
  showThinking?: boolean;
  autoUpgradeSkills?: boolean;
  autoConnectRemote?: boolean;
}) {
  return invokeCommand<AppSettings>("update_app_settings", { input });
}
