import type { Language } from "../../i18n";
import { invokeCommand } from "../tauri/invoke";

export interface SessionTitleSettings {
  autoSessionTitle: boolean;
  uiLanguage: string;
}

// Serialize patches so rapid language changes cannot persist out of order.
let writes: Promise<unknown> = Promise.resolve();

export function sessionTitleSettings(input: { autoSessionTitle?: boolean; uiLanguage?: Language } = {}) {
  const result = writes.then(() => invokeCommand<SessionTitleSettings>("session_title_settings", { input }));
  writes = result.catch(() => undefined);
  return result;
}
