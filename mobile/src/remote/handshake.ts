import { fromPublic } from "nkeys.js";
import { decodeBase64Url } from "./codec";

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
