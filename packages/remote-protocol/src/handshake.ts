/** Producer envelope for the existing Remote v2 Noise exchange. */
export interface HandshakeRequest {
  type: string;
  message: string;
  id?: string;
  pairing?: boolean;
}

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
