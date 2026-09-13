import AsyncStorage from "@react-native-async-storage/async-storage";
import { syncLanguage, type LanguagePreference } from "./index";

const STORAGE_KEY = "future.mobile.language";
let preference: LanguagePreference = "system";
let initialization: Promise<void> | undefined;
const listeners = new Set<() => void>();

export const getLanguagePreference = (): LanguagePreference => preference;
export function subscribeLanguagePreference(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function syncPreferredLanguage(): void {
  syncLanguage(preference);
}

function applyPreference(value: LanguagePreference): void {
  preference = value;
  syncPreferredLanguage();
  listeners.forEach(listener => listener());
}

/** Restore before showing screens so a saved override doesn't flash the OS language. */
export function initializeLanguagePreference(): Promise<void> {
  initialization ??= AsyncStorage.getItem(STORAGE_KEY).then(value => {
    applyPreference(value === "zh" || value === "en" ? value : "system");
  }).catch(() => {
    // Unavailable storage must not prevent launch; follow the OS for this run.
    applyPreference("system");
  });
  return initialization;
}

export async function setLanguagePreference(value: LanguagePreference): Promise<void> {
  await initializeLanguagePreference();
  // Commit only after persistence succeeds, keeping the selection honest on failure.
  await AsyncStorage.setItem(STORAGE_KEY, value);
  applyPreference(value);
}
