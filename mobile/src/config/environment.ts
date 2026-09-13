// The invitation selects the platform for each desktop, independently of the
// APK's release channel. Keep a first-party allowlist: scanning a QR must not
// make the app send pairing requests to arbitrary servers.
const TRUSTED_PLATFORM_ORIGINS = ["https://future-os.cn", "https://test.future-os.cn"];

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
    return (
      TRUSTED_PLATFORM_ORIGINS.includes(parsed.origin) &&
      parsed.username === "" &&
      parsed.password === "" &&
      parsed.pathname === "/client/v1/remote/pair/claim" &&
      parsed.search === "" &&
      parsed.hash === ""
    );
  } catch {
    return false;
  }
}
