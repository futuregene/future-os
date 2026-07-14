/**
 * Unified error classes for browser operations.
 *
 * All errors carry enough context for the CLI facade to produce
 * user-actionable messages without exposing protocol internals.
 */
import type { PageId } from "./types.js";

// ── Base ────────────────────────────────────────────────────────────

export class BrowserError extends Error {
  constructor(
    message: string,
    public readonly code: string,
  ) {
    super(message);
    this.name = "BrowserError";
  }
}

// ── Config ──────────────────────────────────────────────────────────

export class InvalidBrowserConfigError extends BrowserError {
  constructor(message: string) {
    super(`Invalid browser config: ${message}`, "invalid_config");
    this.name = "InvalidBrowserConfigError";
  }
}

export class UnsupportedBrowserConfigVersionError extends BrowserError {
  constructor(version: unknown) {
    super(
      `Unsupported browser config version: ${String(version)}. Expected 1 or ${2}.`,
      "unsupported_config_version",
    );
    this.name = "UnsupportedBrowserConfigVersionError";
  }
}

// ── Launch ──────────────────────────────────────────────────────────

export class BrowserNotFoundError extends BrowserError {
  constructor(detail?: string) {
    const msg = detail
      ? `Browser not found: ${detail}`
      : "Could not find Chrome or Edge. Install Chrome, or pass executablePath to browser start.";
    super(msg, "browser_not_found");
    this.name = "BrowserNotFoundError";
  }
}

export class BrowserLaunchError extends BrowserError {
  constructor(reason: string) {
    super(`Failed to launch browser: ${reason}`, "browser_launch_error");
    this.name = "BrowserLaunchError";
  }
}

export class BrowserConnectionError extends BrowserError {
  constructor(endpoint: string, reason: string) {
    super(
      `Cannot connect to browser at ${endpoint}: ${reason}`,
      "browser_connection_error",
    );
    this.name = "BrowserConnectionError";
  }
}

export class BrowserPermissionError extends BrowserError {
  public readonly remedyCommand: string;

  constructor(browser: string, remedy: string) {
    super(
      `${browser} remote automation is disabled.`,
      "browser_permission_error",
    );
    this.name = "BrowserPermissionError";
    this.remedyCommand = remedy;
  }
}

// ── Protocol ────────────────────────────────────────────────────────

export class BrowserProtocolError extends BrowserError {
  constructor(
    message: string,
    public readonly protocolCode?: number,
    public readonly protocolData?: unknown,
  ) {
    super(message, "browser_protocol_error");
    this.name = "BrowserProtocolError";
  }
}

// ── Page ────────────────────────────────────────────────────────────

export class PageNavigationError extends BrowserError {
  constructor(url: string, reason: string) {
    super(`Failed to navigate to ${url}: ${reason}`, "page_navigation_error");
    this.name = "PageNavigationError";
  }
}

// ── Selector ────────────────────────────────────────────────────────

export class SelectorError extends BrowserError {
  constructor(
    message: string,
    code: string,
    public readonly selector?: string,
  ) {
    super(message, code);
    this.name = "SelectorError";
  }
}

export class UnknownRefError extends SelectorError {
  constructor(ref: string) {
    super(
      `Unknown browser ref "${ref}". Run browser command snapshot first.`,
      "unknown_ref",
      ref,
    );
    this.name = "UnknownRefError";
  }
}

export class ElementNotFoundError extends SelectorError {
  constructor(selector: string) {
    super(
      `Element not found: "${selector}"`,
      "element_not_found",
      selector,
    );
    this.name = "ElementNotFoundError";
  }
}

export class ElementNotInteractableError extends SelectorError {
  constructor(selector: string, reason: string) {
    super(
      `Element not interactable: "${selector}" — ${reason}`,
      "element_not_interactable",
      selector,
    );
    this.name = "ElementNotInteractableError";
  }
}

export class StrictModeViolationError extends SelectorError {
  constructor(selector: string, count: number) {
    super(
      `Strict mode violation: "${selector}" resolved to ${count} elements. Use a more specific selector.`,
      "strict_mode_violation",
      selector,
    );
    this.name = "StrictModeViolationError";
  }
}

// ── Timeout ─────────────────────────────────────────────────────────

export class OperationTimeoutError extends BrowserError {
  constructor(
    operation: string,
    timeoutMs: number,
    public readonly context?: string,
  ) {
    const extra = context ? ` (${context})` : "";
    super(
      `Timed out after ${timeoutMs}ms waiting for ${operation}${extra}`,
      "operation_timeout",
    );
    this.name = "OperationTimeoutError";
  }
}

// ── Capability ──────────────────────────────────────────────────────

export class UnsupportedCapabilityError extends BrowserError {
  constructor(
    browserKind: string,
    operation: string,
    alternative?: string,
  ) {
    const extra = alternative ? `\nAlternative: ${alternative}` : "";
    super(
      `${operation} is not supported on ${browserKind}.${extra}`,
      "unsupported_capability",
    );
    this.name = "UnsupportedCapabilityError";
  }
}

// ── Lifecycle ───────────────────────────────────────────────────────

export class BrowserClosedError extends BrowserError {
  constructor(pageId?: PageId) {
    const detail = pageId ? ` (page: ${pageId})` : "";
    super(
      `Browser or page was closed during operation${detail}`,
      "browser_closed",
    );
    this.name = "BrowserClosedError";
  }
}
