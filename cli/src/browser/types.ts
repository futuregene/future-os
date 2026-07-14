/**
 * Core types for browser execution layer.
 * Single source of truth — Phase 1+ modules import from here.
 */

// ── Browser identification ──
export type BrowserKind = "chrome" | "edge" | "chromium" | "safari";

// ── Discriminated connection config ──
export type BrowserConnectionConfig =
  | {
      protocol: "cdp";
      browserKind: "chrome" | "edge" | "chromium";
      endpoint: string;
    }
  | {
      protocol: "webdriver";
      browserKind: "safari";
      endpoint: string;
      sessionId: string;
      driverPid?: number;
    };

export type BrowserProtocol = BrowserConnectionConfig["protocol"];

// ── Config version ──
export const CURRENT_CONFIG_VERSION = 2;

export interface BrowserConfig {
  version: typeof CURRENT_CONFIG_VERSION;
  connection: BrowserConnectionConfig;

  activeUrl?: string;
  activePageId?: PageId;
  tabOrder?: PageId[];

  refs?: Record<string, string>;
  refsPageId?: PageId;
  refsUrl?: string;
}

// ── Page identity ──
export type PageId = string;

// ── Timeouts ──
export interface BrowserTimeouts {
  actionTimeoutMs: number;
  navigationTimeoutMs: number;
}

export const DEFAULT_TIMEOUTS: BrowserTimeouts = {
  actionTimeoutMs: 5_000,
  navigationTimeoutMs: 15_000,
};
