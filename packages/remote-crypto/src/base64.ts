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
