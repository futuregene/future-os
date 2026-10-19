// Legacy challenge verification is an explicit subpath, keeping NKey loading
// out of the ordinary protocol codecs and v2 handshake type exports.
import { fromPublic } from "nkeys.js";
import { decodeBase64Url } from "@future-os/remote-crypto/base64";
import { handshakeTranscript, type HandshakeChallenge } from "./handshake";
const encoder = new TextEncoder();

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
