import { fromPublic } from "nkeys.js";

const encoder = new TextEncoder();

export interface HandshakeChallenge {
  protocolVersion: number;
  pairId: string;
  desktopId: string;
  desktopPublicKey: string;
  bridgeInstanceId: string;
  deviceId: string;
  clientPublicKey: string;
  clientNonce: string;
  desktopNonce: string;
  desktopSignature: string;
}

export function handshakeTranscript(challenge: HandshakeChallenge): string {
  return [
    "futureos-remote-handshake-v1",
    challenge.pairId,
    challenge.desktopId,
    challenge.desktopPublicKey,
    challenge.bridgeInstanceId,
    challenge.deviceId,
    challenge.clientPublicKey,
    challenge.clientNonce,
    challenge.desktopNonce,
  ].join("\n");
}

export function encodeBase64Url(value: Uint8Array): string {
  const binary = Array.from(value, byte => String.fromCharCode(byte)).join("");
  return globalThis.btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function decodeBase64Url(value: string): Uint8Array | null {
  try {
    let base64 = value.replace(/-/g, "+").replace(/_/g, "/");
    while (base64.length % 4 !== 0) base64 += "=";
    return Uint8Array.from(globalThis.atob(base64), char => char.charCodeAt(0));
  } catch {
    return null;
  }
}

export function verifyDesktopChallenge(
  challenge: HandshakeChallenge,
  expected: {
    pairId: string;
    desktopId: string;
    desktopPublicKey: string;
    deviceId: string;
    clientPublicKey: string;
    clientNonce: string;
  },
): boolean {
  const identityMatches =
    challenge.protocolVersion === 1 &&
    challenge.pairId === expected.pairId &&
    challenge.desktopId === expected.desktopId &&
    challenge.desktopPublicKey === expected.desktopPublicKey &&
    challenge.deviceId === expected.deviceId &&
    challenge.clientPublicKey === expected.clientPublicKey &&
    challenge.clientNonce === expected.clientNonce &&
    Boolean(challenge.bridgeInstanceId) &&
    Boolean(challenge.desktopNonce);
  if (!identityMatches) return false;
  const signature = decodeBase64Url(challenge.desktopSignature);
  return Boolean(
    signature &&
    fromPublic(expected.desktopPublicKey).verify(
      encoder.encode(handshakeTranscript(challenge)),
      signature,
    ),
  );
}
