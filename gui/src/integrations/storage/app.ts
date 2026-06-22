import { invokeCommand } from "../tauri/invoke";

export async function initializeAppStore() {
  await invokeCommand("initialize_app_store");
}

export function storedTimeToIso(value: number) {
  return new Date(value).toISOString();
}
