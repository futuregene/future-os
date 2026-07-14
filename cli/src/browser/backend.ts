/**
 * Browser backend interfaces and types.
 *
 * Phase 1: defines the contract that both PlaywrightBackend (existing)
 * and future ChromiumBackend / SafariBackend must implement.
 */
import type {
  BrowserKind,
  BrowserProtocol,
  BrowserConnectionConfig,
  BrowserTimeouts,
  PageId,
} from "./types.js";

// ── Evaluate ────────────────────────────────────────────────────────

export type EvaluateRequest =
  | { kind: "expression"; expression: string }
  | { kind: "function"; functionDeclaration: string; arguments?: SerializableValue[] };

export interface EvaluateOptions {
  deadline?: Deadline;
  retryOnContextDestroyed?: boolean;
}

export type SerializableValue =
  | null
  | boolean
  | number
  | string
  | SerializableValue[]
  | { [key: string]: SerializableValue };

// ── Deadline ────────────────────────────────────────────────────────

export interface Deadline {
  readonly startMs: number;
  readonly timeoutMs: number;
  readonly expired: boolean;
  readonly elapsedMs: number;
  remainingMs(): number;
}

export function createDeadline(timeoutMs: number): Deadline {
  const startMs = Date.now();
  return {
    startMs,
    timeoutMs,
    get expired() { return Date.now() - startMs >= timeoutMs; },
    get elapsedMs() { return Date.now() - startMs; },
    remainingMs() { return Math.max(0, timeoutMs - (Date.now() - startMs)); },
  };
}

// ── Discriminated session params ────────────────────────────────────

export type BrowserSessionParams =
  | {
      protocol: "cdp";
      browserKind: "chrome" | "edge" | "chromium";
      endpoint: string;
      timeouts: BrowserTimeouts;
      activePageId?: PageId;
      initTabOrder?: PageId[];
    }
  | {
      protocol: "webdriver";
      browserKind: "safari";
      endpoint: string;
      sessionId: string;
      timeouts: BrowserTimeouts;
      activePageId?: PageId;
    };

// ── Launch options ──────────────────────────────────────────────────

export interface BrowserLaunchOptions {
  port?: number;
  profileDir?: string;
  executablePath?: string;
  url?: string;
}

export interface BrowserStartResult {
  endpoint: string;
  launcher: string;
  profileDir?: string;
  port: number;
  status: "started" | "already_running";
}

export interface BrowserStatusResult {
  endpoint: string;
  reachable: boolean;
  version?: unknown;
  error?: string;
}

// ── Browser Manager ─────────────────────────────────────────────────

export interface BrowserManager {
  readonly kind: BrowserKind;
  readonly protocol: BrowserProtocol;

  /** Start browser process. Returns connection info for persistence. */
  start(options: BrowserLaunchOptions): Promise<InternalBrowserStartResult>;

  /** Check if a running browser's debug endpoint is reachable. */
  status(connection: BrowserConnectionConfig): Promise<BrowserStatusResult>;
}

export interface InternalBrowserStartResult {
  connection: BrowserConnectionConfig;
  launcher: string;
  profileDir?: string;
  port: number;
  status: "started" | "already_running";
}

// ── Browser Session ─────────────────────────────────────────────────

export interface BrowserSession {
  readonly kind: BrowserKind;
  readonly protocol: BrowserProtocol;

  // Page operations
  open(url: string, options?: OpenPageOptions): Promise<InternalPageInfo>;
  click(target: ResolvedTarget, options?: ClickOptions): Promise<InternalActionResult>;
  type(target: ResolvedTarget, text: string, options?: TypeOptions): Promise<InternalTypeResult>;
  press(key: string, target?: ResolvedTarget, options?: PressOptions): Promise<InternalActionResult>;
  tabs(action: TabsAction): Promise<InternalTabsResult>;

  // Raw capabilities
  evaluate<T>(request: EvaluateRequest, options?: EvaluateOptions): Promise<T>;
  captureScreenshot(options: CaptureScreenshotOptions): Promise<Uint8Array>;

  // Lifecycle
  disconnect(): Promise<void>;
}

// ── Options ─────────────────────────────────────────────────────────

export interface OpenPageOptions {
  waitUntil?: "none" | "domcontentloaded" | "load";
}

export interface ClickOptions {
  timeoutMs?: number;
}

export interface TypeOptions {
  clear?: boolean;
  submit?: boolean;
  timeoutMs?: number;
}

export interface PressOptions {
  timeoutMs?: number;
}

export interface CaptureScreenshotOptions {
  fullPage: boolean;
  format: "png" | "jpeg";
  quality?: number;
}

// ── Resolved target ─────────────────────────────────────────────────

export interface ResolvedTarget {
  original: string;
  source: "ref" | "selector";
  selector: string;
  ref?: string;
}

// ── Internal result types (carry pageId) ────────────────────────────

export interface InternalPageInfo {
  pageId: PageId;
  title: string;
  url: string;
}

export interface InternalTabInfo {
  pageId: PageId;
  index: number;
  title: string;
  url: string;
  active: boolean;
}

export interface InternalActionResult {
  pageId: PageId;
  title: string;
  url: string;
  didNavigate: boolean;
}

export interface InternalTypeResult {
  pageId: PageId;
  typed: string;
  submitted: boolean;
}

// ── Tabs ────────────────────────────────────────────────────────────

export type TabsAction =
  | { action: "list" }
  | { action: "new"; url?: string }
  | { action: "select"; index: number }
  | { action: "close"; index: number };

export type InternalTabsResult =
  | { kind: "list"; tabs: InternalTabInfo[] }
  | { kind: "new"; page: InternalPageInfo; index: number }
  | { kind: "select"; page: InternalPageInfo }
  | { kind: "close"; url: string; index: number };

// ── Factory (no global setter) ──────────────────────────────────────

export type BrowserManagerFactory = (kind: BrowserKind) => BrowserManager;
export type BrowserSessionFactory = (params: BrowserSessionParams) => Promise<BrowserSession>;
