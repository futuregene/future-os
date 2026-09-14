import type { AlertButton, AlertOptions } from "react-native";

export interface AppAlertRequest {
  title: string;
  message?: string;
  buttons?: AlertButton[];
  options?: AlertOptions;
}

// Non-React callers (updates, notifications, download confirmations) use the
// same app-styled dialog as screens. Queue concurrent errors instead of
// replacing a confirmation and leaving its promise unresolved.
let requests: AppAlertRequest[] = [];
const listeners = new Set<() => void>();
function emit() { for (const listener of listeners) listener(); }

export const AppAlert = {
  alert(title: string, message?: string, buttons?: AlertButton[], options?: AlertOptions): void {
    requests = [...requests, { title, message, buttons, options }];
    emit();
  },
};

export function currentAppAlert(): AppAlertRequest | null { return requests[0] ?? null; }
export function subscribeAppAlerts(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** Called only after the native Modal has dismissed. Ignore duplicate events. */
export function finishAppAlert(request: AppAlertRequest): void {
  if (requests[0] !== request) return;
  requests = requests.slice(1);
  emit();
}
