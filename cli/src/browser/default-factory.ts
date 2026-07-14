/**
 * Default factory implementations — ChromiumManager/ChromiumSession and SafariManager/SafariSession.
 */
import type { BrowserKind } from "./types.js";
import type {
  BrowserManager,
  BrowserSession,
  BrowserSessionParams,
  BrowserManagerFactory,
  BrowserSessionFactory,
} from "./backend.js";
import { ChromiumManager } from "./chromium/chromium-manager.js";
import { ChromiumSession } from "./chromium/chromium-session.js";
import { SafariManager } from "./safari/safari-manager.js";
import { SafariSession } from "./safari/safari-session.js";

export function createDefaultManager(kind: BrowserKind): BrowserManager {
  if (kind === "safari") return new SafariManager();
  return new ChromiumManager(kind);
}

export function createDefaultSession(
  params: BrowserSessionParams,
): Promise<BrowserSession> {
  if (params.protocol === "webdriver") {
    return Promise.resolve(new SafariSession(params));
  }
  return Promise.resolve(new ChromiumSession(params));
}

export const defaultManagerFactory: BrowserManagerFactory = createDefaultManager;
export const defaultSessionFactory: BrowserSessionFactory = createDefaultSession;
