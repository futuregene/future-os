import type { HistoryMessage } from "./types";
export { decodeBase64Url, encodeBase64Url } from "@future-os/remote-crypto/base64";
export { jwtExpiry } from "@future-os/remote-protocol";

export function messageText(content: HistoryMessage["blocks"] | undefined | null): string {
  if (!Array.isArray(content)) return "";
  return content
    .filter(block => block?.kind === "text")
    .map(block => block.text ?? "")
    .join("");
}

export function randomId(prefix: string): string {
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  const value = Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
  return `${prefix}_${value}`;
}
