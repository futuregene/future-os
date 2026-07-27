import type { HistoryMessage, PairingCode } from "./types";

export interface PairingInvitation {
  code: string;
  desktopId: string;
  desktopPublicKey: string;
}

export function decodeBase64UrlJson<T>(value: string): T | null {
  try {
    let base64 = value.trim().replace(/-/g, "+").replace(/_/g, "/");
    while (base64.length % 4 !== 0) base64 += "=";
    return JSON.parse(globalThis.atob(base64)) as T;
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

export function jwtExpiry(jwt: string): number {
  const payload = jwt.split(".")[1];
  if (!payload) return 0;
  return decodeBase64UrlJson<{ exp?: number }>(payload)?.exp ?? 0;
}

export function messageText(content: HistoryMessage["content"]): string {
  if (typeof content === "string") return content;
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
