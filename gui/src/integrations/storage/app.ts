import { invoke } from "@tauri-apps/api/core";

export async function initializeAppStore() {
  await invoke("initialize_app_store");
}

export function storedTimeToIso(value: number) {
  return new Date(value).toISOString();
}
