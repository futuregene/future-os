/**
 * Default factory implementations — ChromiumManager and ChromiumSession.
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

export function createDefaultManager(kind: BrowserKind): BrowserManager {
  if (kind === "safari") {
    throw new Error("Safari backend not yet implemented. Use --browser chrome or --browser edge.");
  }
  return new ChromiumManager(kind);
}

export function createDefaultSession(
  params: BrowserSessionParams,
): Promise<BrowserSession> {
  if (params.protocol === "webdriver") {
    throw new Error("Safari backend not yet implemented.");
  }
  return Promise.resolve(new ChromiumSession(params));
}

export const defaultManagerFactory: BrowserManagerFactory = createDefaultManager;
export const defaultSessionFactory: BrowserSessionFactory = createDefaultSession;
