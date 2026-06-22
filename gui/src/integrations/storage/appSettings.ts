import { invoke } from "@tauri-apps/api/core";

export interface AppSettings {
  autoApprove: boolean;
  hiddenModels: string[];
}

export async function getAppSettings() {
  return invoke<AppSettings>("get_app_settings");
}

export async function updateAppSettings(input: { autoApprove?: boolean; hiddenModels?: string[] }) {
  return invoke<AppSettings>("update_app_settings", { input });
}
