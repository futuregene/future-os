/**
 * Browser pipeline state persistence.
 *
 * Reads/writes ~/.future/agent/browser/config.json.
 * Handles v1 → v2 migration and runtime validation.
 */
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import {
  type BrowserConfig,
  CURRENT_CONFIG_VERSION,
} from "./types.js";
import { isRecord } from "../utils/object.js";

// ── Paths ───────────────────────────────────────────────────────────

const FUTURE_HOME = process.env["FUTURE_HOME"] ?? join(homedir(), ".future");
const BROWSER_DIR = join(FUTURE_HOME, "agent", "browser");
const CONFIG_FILE = join(BROWSER_DIR, "config.json");

// ── Public API ──────────────────────────────────────────────────────

export async function loadBrowserConfig(): Promise<BrowserConfig> {
  try {
    const raw = await readFile(CONFIG_FILE, "utf8");
    const parsed = JSON.parse(raw) as unknown;
    return parseBrowserConfig(parsed);
  } catch (err) {
    if (isNodeError(err) && err.code === "ENOENT") {
      return defaultBrowserConfig();
    }
    throw err;
  }
}

export async function saveBrowserConfig(config: BrowserConfig): Promise<void> {
  await mkdir(BROWSER_DIR, { recursive: true });
  await writeFile(CONFIG_FILE, `${JSON.stringify(config, null, 2)}\n`);
}

export function defaultBrowserConfig(): BrowserConfig {
  return {
    version: CURRENT_CONFIG_VERSION,
    connection: {
      protocol: "cdp",
      browserKind: "chromium",
      endpoint: "http://127.0.0.1:9222",
    },
  };
}

export function getBrowserDir(): string {
  return BROWSER_DIR;
}

// ── Parse & Validate ────────────────────────────────────────────────

export function parseBrowserConfig(raw: unknown): BrowserConfig {
  if (!isRecord(raw)) {
    throw new InvalidBrowserConfigError("Browser config must be a JSON object");
  }

  const version = raw.version;

  // Missing version or 1 → migrate
  if (version === undefined || version === 1) {
    return migrateV1Config(raw);
  }

  // Unknown future version
  if (typeof version === "number" && version > CURRENT_CONFIG_VERSION) {
    throw new InvalidBrowserConfigError(
      `Unsupported browser config version: ${version}. Expected ≤ ${CURRENT_CONFIG_VERSION}.`,
    );
  }

  // version === 2
  if (version === CURRENT_CONFIG_VERSION) {
    return validateV2Config(raw);
  }

  // version === 0, -1, 1.5, "2"
  throw new InvalidBrowserConfigError(
    `Unsupported browser config version: ${String(version)}`,
  );
}

function migrateV1Config(raw: Record<string, unknown>): BrowserConfig {
  // V1 config had: { endpoint?, activeUrl?, refs? }
  const endpointRaw = typeof raw.endpoint === "string" && raw.endpoint.trim().length > 0
    ? raw.endpoint
    : "http://127.0.0.1:9222";

  // Validate endpoint is http(s) URL
  if (!/^https?:\/\/.+/.test(endpointRaw)) {
    throw new InvalidBrowserConfigError(
      `Invalid V1 endpoint: "${endpointRaw}". Must be an http(s) URL.`,
    );
  }

  return {
    version: CURRENT_CONFIG_VERSION,
    connection: {
      protocol: "cdp",
      browserKind: "chromium", // Generic CDP — refined after /json/version
      endpoint: endpointRaw,
    },
    activeUrl: optionalString(raw.activeUrl),
    refs: validateRefsMap(raw.refs),
  };
}

function validateV2Config(raw: Record<string, unknown>): BrowserConfig {
  const conn = raw.connection;
  if (!isRecord(conn)) {
    throw new InvalidBrowserConfigError("connection field is required in v2 config");
  }

  const protocol = validateEnum(conn.protocol, ["cdp", "webdriver"] as const, "protocol");
  const endpoint = requireHttpUrl(requireNonEmptyString(conn.endpoint, "connection.endpoint"), "connection.endpoint");

  if (protocol === "cdp") {
    return {
      version: CURRENT_CONFIG_VERSION,
      connection: {
        protocol: "cdp",
        browserKind: validateEnum(conn.browserKind, ["chrome", "edge", "chromium"] as const, "browser kind"),
        endpoint,
      },
      activeUrl: optionalString(raw.activeUrl),
      activePageId: optionalString(raw.activePageId),
      tabOrder: validateOptionalStringArray(raw.tabOrder),
      refs: validateRefsMap(raw.refs),
      refsPageId: optionalString(raw.refsPageId),
      refsUrl: optionalString(raw.refsUrl),
    };
  }

  // Early Safari builds read only the root-level WebDriver sessionId, while
  // safaridriver returns it under value.sessionId. JSON serialization omitted
  // that undefined field, leaving a config no browser command could load.
  // Recover only that historical missing-field shape; malformed values should
  // still fail validation instead of being silently discarded.
  const browserKind = validateEnum(conn.browserKind, ["safari"] as const, "browser kind");
  if (conn.sessionId === undefined) {
    return defaultBrowserConfig();
  }

  return {
    version: CURRENT_CONFIG_VERSION,
    connection: {
      protocol: "webdriver",
      browserKind,
      endpoint,
      sessionId: requireNonEmptyString(conn.sessionId, "connection.sessionId"),
      driverPid: optionalPositiveInteger(conn.driverPid),
    },
    activeUrl: optionalString(raw.activeUrl),
    activePageId: optionalString(raw.activePageId),
    tabOrder: validateOptionalStringArray(raw.tabOrder),
    refs: validateRefsMap(raw.refs),
    refsPageId: optionalString(raw.refsPageId),
    refsUrl: optionalString(raw.refsUrl),
  };
}

// ── Validation Helpers ──────────────────────────────────────────────

function requireNonEmptyString(value: unknown, field: string): string {
  if (typeof value === "string" && value.trim().length > 0) return value;
  throw new InvalidBrowserConfigError(`${field} must be a non-empty string`);
}

function requireHttpUrl(value: string, field: string): string {
  if (/^https?:\/\/.+/.test(value)) return value;
  throw new InvalidBrowserConfigError(`${field} must be an http(s) URL, got: ${JSON.stringify(value)}`);
}

function optionalString(value: unknown): string | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value === "string" && value.trim().length > 0) return value;
  return undefined;
}

function optionalPositiveInteger(value: unknown): number | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value === "number" && Number.isFinite(value) && value > 0 && Number.isInteger(value)) {
    return value;
  }
  throw new InvalidBrowserConfigError(`expected positive integer, got ${String(value)}`);
}

function validateEnum<const T extends readonly string[]>(
  value: unknown,
  allowed: T,
  field: string,
): T[number] {
  if (typeof value === "string" && (allowed as readonly string[]).includes(value)) {
    return value as T[number];
  }
  throw new InvalidBrowserConfigError(
    `Invalid ${field}: "${String(value)}". Expected one of: ${allowed.join(", ")}`,
  );
}

function validateOptionalStringArray(value: unknown): string[] | undefined {
  if (value === undefined || value === null) return undefined;
  if (!Array.isArray(value)) {
    throw new InvalidBrowserConfigError("tabOrder must be an array of strings");
  }
  for (const item of value) {
    if (typeof item !== "string" || item.trim().length === 0) {
      throw new InvalidBrowserConfigError("tabOrder must contain only non-empty strings");
    }
  }
  return value;
}

function validateRefsMap(value: unknown): Record<string, string> | undefined {
  if (value === undefined || value === null) return undefined;
  if (!isRecord(value)) {
    throw new InvalidBrowserConfigError("refs must be a JSON object (string → string)");
  }
  for (const [key, val] of Object.entries(value)) {
    if (typeof val !== "string") {
      throw new InvalidBrowserConfigError(`refs["${key}"] must be a string selector`);
    }
  }
  return value as Record<string, string>;
}

// ── Error ───────────────────────────────────────────────────────────

export class InvalidBrowserConfigError extends Error {
  constructor(message: string) {
    super(`Invalid browser config: ${message}`);
    this.name = "InvalidBrowserConfigError";
  }
}

function isNodeError(err: unknown): err is NodeJS.ErrnoException {
  return err instanceof Error && "code" in err;
}
