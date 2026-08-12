import { IS_RELEASE } from "../version.generated";

export const DEVELOPMENT_PLATFORM_URL = "https://test.future-os.cn";
export const PRODUCTION_PLATFORM_URL = "https://future-os.cn";

// Channel policy mirrors the desktop (desktop/src-tauri/src/build_info.rs +
// future_platform.rs): a release build (plain `X.Y.Z`) is production-locked,
// every other build (`0.0.2-<hash>…`) targets the test environment. The flag is
// derived from the version string by scripts/version.mjs — NOT from `__DEV__` —
// so a local Gradle release build is still a dev-channel package and must reach
// the test host (the production host has remote control disabled).
export const PLATFORM_URL = IS_RELEASE ? PRODUCTION_PLATFORM_URL : DEVELOPMENT_PLATFORM_URL;

/**
 * NATS WebSocket scheme of an endpoint URL — `wss`, `ws`, or unrecognized.
 * Deliberately a prefix check, not `new URL(...)`: scheme parsing must behave
 * identically on Hermes (mobile) and V8/browsers (web test client), and engine
 * URL parsers disagree on non-http(s) schemes.
 */
export function natsWsUrlScheme(url: string): "wss" | "ws" | "other" {
  const lower = url.trim().toLowerCase();
  if (lower.startsWith("wss://")) return "wss";
  if (lower.startsWith("ws://")) return "ws";
  return "other";
}

export function isExpectedClaimUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    const expected = new URL(PLATFORM_URL);
    return (
      parsed.protocol === "https:" &&
      parsed.host === expected.host &&
      parsed.pathname.endsWith("/client/v1/remote/pair/claim")
    );
  } catch {
    return false;
  }
}
