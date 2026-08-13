import { createUser } from "nkeys.js";
import {
  decodeBase64Url,
  encodeBase64Url,
  handshakeTranscript,
  type HandshakeChallenge,
  verifyDesktopChallenge,
} from "../handshake";

function signedChallenge(): HandshakeChallenge {
  const desktop = createUser();
  const challenge: HandshakeChallenge = {
    protocolVersion: 1,
    pairId: "pair_1",
    desktopId: "desktop_1",
    desktopPublicKey: desktop.getPublicKey(),
    bridgeInstanceId: "bridge_1",
    deviceId: "dev_1",
    clientPublicKey: createUser().getPublicKey(),
    clientNonce: "challenge_1234567890",
    desktopNonce: "desktop_challenge_1234567890",
    desktopSignature: "",
  };
  challenge.desktopSignature = encodeBase64Url(
    desktop.sign(new TextEncoder().encode(handshakeTranscript(challenge))),
  );
  return challenge;
}

describe("signed pairing handshake", () => {
  it("accepts the exact QR-bound desktop identity and transcript", () => {
    const challenge = signedChallenge();
    expect(
      verifyDesktopChallenge(challenge, {
        pairId: challenge.pairId,
        desktopId: challenge.desktopId,
        desktopPublicKey: challenge.desktopPublicKey,
        deviceId: challenge.deviceId,
        clientPublicKey: challenge.clientPublicKey,
        clientNonce: challenge.clientNonce,
      }),
    ).toBe(true);
  });

  it("rejects a mismatched desktop and a tampered challenge", () => {
    const challenge = signedChallenge();
    expect(
      verifyDesktopChallenge(challenge, {
        pairId: challenge.pairId,
        desktopId: "desktop_other",
        desktopPublicKey: challenge.desktopPublicKey,
        deviceId: challenge.deviceId,
        clientPublicKey: challenge.clientPublicKey,
        clientNonce: challenge.clientNonce,
      }),
    ).toBe(false);
    expect(
      verifyDesktopChallenge(
        { ...challenge, desktopNonce: "tampered_nonce" },
        {
          pairId: challenge.pairId,
          desktopId: challenge.desktopId,
          desktopPublicKey: challenge.desktopPublicKey,
          deviceId: challenge.deviceId,
          clientPublicKey: challenge.clientPublicKey,
          clientNonce: challenge.clientNonce,
        },
      ),
    ).toBe(false);
  });
});

describe("base64url codec", () => {
  it("round-trips bytes", () => {
    const bytes = new Uint8Array([0, 1, 2, 253, 254, 255]);
    expect(decodeBase64Url(encodeBase64Url(bytes))).toEqual(bytes);
  });

  it("returns null for undecodable input instead of throwing", () => {
    expect(decodeBase64Url("!!!")).toBeNull();
  });
});
