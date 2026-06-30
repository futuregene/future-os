import { invokeCommand } from "../tauri/invoke";

export interface AppSettings {
  autoApprove: boolean;
  hiddenModels: string[];
}

/** Fallback used before the persisted settings load. */
export const DEFAULT_APP_SETTINGS: AppSettings = { autoApprove: false, hiddenModels: [] };

export async function getAppSettings() {
  return invokeCommand<AppSettings>("get_app_settings");
}

export async function updateAppSettings(input: { autoApprove?: boolean; hiddenModels?: string[] }) {
  return invokeCommand<AppSettings>("update_app_settings", { input });
}
