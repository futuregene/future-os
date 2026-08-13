import type { HistoryMessage, PairingCode } from "./types";

/**
 * Client-side cap for a `prompt` message. The desktop caps relayed event data
 * at 900KB and pages history at 512KB; a prompt bigger than the NATS user-JWT
 * payload limit never reaches the desktop, so without a local guard the send
 * degrades into an opaque 10s x3 timeout. Stay well under the 1MB wire budget —
 * the rest of the envelope (session, model, attachments) shares it.
 */
export const MAX_PROMPT_MESSAGE_BYTES = 512 * 1024;

/** Serialized UTF-8 byte length of a string (JS `.length` counts UTF-16 code
 * units, which over/under-counts for CJK and surrogate pairs). */
export function utf8Bytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

export interface PairingInvitation {
  code: string;
  desktopId: string;
  desktopPublicKey: string;
}

export function encodeBase64Url(value: Uint8Array): string {
  const binary = Array.from(value, byte => String.fromCharCode(byte)).join("");
  return globalThis.btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function decodeBase64Url(value: string): Uint8Array | null {
  try {
    let base64 = value.trim().replace(/-/g, "+").replace(/_/g, "/");
    while (base64.length % 4 !== 0) base64 += "=";
    return Uint8Array.from(globalThis.atob(base64), char => char.charCodeAt(0));
  } catch {
    return null;
  }
}

export function decodeBase64UrlJson<T>(value: string): T | null {
  const bytes = decodeBase64Url(value);
  if (!bytes) return null;
  try {
    return JSON.parse(new TextDecoder().decode(bytes)) as T;
  } catch {
    return null;
  }
}

export function decodePairingCode(
  value: string,
  nowSeconds = Date.now() / 1000,
): PairingCode | null {
  const decoded = decodeBase64UrlJson<Partial<PairingCode>>(value);
  if (
    decoded?.v !== 2 ||
    typeof decoded.nonce !== "string" ||
    typeof decoded.claim_url !== "string" ||
    typeof decoded.exp !== "number" ||
    decoded.exp < Math.floor(nowSeconds)
  ) {
    return null;
  }
  return decoded as PairingCode;
}

export function pairingCodeFromQr(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (!trimmed.startsWith("futureos://")) return null;
  return parsePairingInvitation(trimmed) ? trimmed : null;
}

export function parsePairingInvitation(value: string): PairingInvitation | null {
  try {
    const url = new URL(value.trim());
    if (url.host !== "remote" || url.pathname !== "/pair") return null;
    const code = url.searchParams.get("code");
    const desktopId = url.searchParams.get("desktopId");
    const desktopPublicKey = url.searchParams.get("desktopKey");
    if (!code || !desktopId?.startsWith("desktop_") || !desktopPublicKey?.startsWith("U")) {
      return null;
    }
    return { code, desktopId, desktopPublicKey };
  } catch {
    return null;
  }
}

/**
 * The JWT's `exp` (unix seconds), or `null` when it can't be read (missing
 * payload segment, undecodable, or absent `exp`). The desktop treats such a
 * token as invalid outright; callers must treat `null` as a terminal
 * credential failure rather than an "already expired" that would drive a
 * refresh storm.
 */
export function jwtExpiry(jwt: string): number | null {
  const payload = jwt.split(".")[1];
  if (!payload) return null;
  const exp = decodeBase64UrlJson<{ exp?: unknown }>(payload)?.exp;
  return typeof exp === "number" && Number.isFinite(exp) && exp > 0 ? exp : null;
}

export function messageText(content: HistoryMessage["content"]): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .filter(block => block?.type === "text")
    .map(block => block.text ?? "")
    .join("");
}

export function randomId(prefix: string): string {
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  const value = Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
  return `${prefix}_${value}`;
}
