/**
 * CDP endpoint resolver: HTTP /json/version → WebSocket URL.
 *
 * Responsibilities:
 * 1. GET <httpEndpoint>/json/version
 * 2. Validate HTTP status + JSON structure
 * 3. Read webSocketDebuggerUrl
 * 4. Identify browser from Browser/version fields
 */

export interface CdpEndpointInfo {
  httpEndpoint: string;
  webSocketDebuggerUrl: string;
  browserKind: "chrome" | "edge" | "chromium";
  browserVersion?: string;
}

/**
 * Resolve a CDP HTTP endpoint to WebSocket URL and browser identity.
 *
 * Config persists HTTP endpoint (never WebSocket URL — it may change on restart).
 * This function is called at the start of each CLI command to get a fresh
 * WebSocket connection URL.
 */
export async function resolveCdpEndpoint(
  httpEndpoint: string,
  timeoutMs: number = 5000,
): Promise<CdpEndpointInfo> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);

  try {
    const response = await fetch(`${httpEndpoint}/json/version`, {
      signal: controller.signal,
    });

    if (!response.ok) {
      throw new Error(
        `CDP /json/version returned HTTP ${response.status}: ${response.statusText}`,
      );
    }

    const data = (await response.json()) as Record<string, unknown>;

    const webSocketDebuggerUrl = typeof data.webSocketDebuggerUrl === "string"
      ? data.webSocketDebuggerUrl
      : undefined;

    if (!webSocketDebuggerUrl || !webSocketDebuggerUrl.startsWith("ws")) {
      throw new Error(
        `Invalid webSocketDebuggerUrl in /json/version: ${JSON.stringify(data)}`,
      );
    }

    const browserInfo = identifyBrowser(data);
    const browserVersion = typeof data.Browser === "string"
      ? data.Browser
      : typeof data.browser === "string"
        ? data.browser
        : undefined;

    return {
      httpEndpoint,
      webSocketDebuggerUrl,
      browserKind: browserInfo,
      browserVersion,
    };
  } finally {
    clearTimeout(timer);
  }
}

function identifyBrowser(data: Record<string, unknown>): "chrome" | "edge" | "chromium" {
  const browser = typeof data.Browser === "string" ? data.Browser.toLowerCase() : "";

  if (browser.includes("edg") || browser.includes("edge")) return "edge";
  if (browser.includes("chrome")) return "chrome";
  if (browser.includes("chromium")) return "chromium";

  // Fallback: check User-Agent style fields
  const ua = String(data["User-Agent"] ?? data["user-agent"] ?? "").toLowerCase();
  if (ua.includes("edg/")) return "edge";
  if (ua.includes("chrome/")) return "chrome";
  if (ua.includes("chromium/")) return "chromium";

  return "chromium";
}
