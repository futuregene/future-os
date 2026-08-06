import { IS_RELEASE } from "../version.generated";

export const DEVELOPMENT_PLATFORM_URL = "https://test.future-os.cn";
export const PRODUCTION_PLATFORM_URL = "https://future-os.cn";

// Channel policy mirrors the desktop (gui/src-tauri/src/build_info.rs +
// future_platform.rs): a release build (plain `X.Y.Z`) is production-locked,
// every other build (`0.0.0-<hash>…`) targets the test environment. The flag is
// derived from the version string by scripts/version.mjs — NOT from `__DEV__` —
// so a local Gradle release build is still a dev-channel package and must reach
// the test host (the production host has remote control disabled).
export const PLATFORM_URL = IS_RELEASE ? PRODUCTION_PLATFORM_URL : DEVELOPMENT_PLATFORM_URL;

/** NATS WebSocket scheme of an endpoint URL — `wss`, `ws`, or unrecognized. */
export function natsWsUrlScheme(url: string): "wss" | "ws" | "other" {
  try {
    const parsed = new URL(url);
    if (parsed.protocol === "wss:") return "wss";
    if (parsed.protocol === "ws:") return "ws";
    return "other";
  } catch {
    return "other";
  }
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
