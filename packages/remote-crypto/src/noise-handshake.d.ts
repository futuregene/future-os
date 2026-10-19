declare module "noise-handshake" {
  interface KeyPair { secretKey: Uint8Array; publicKey: Uint8Array }
  export default class Noise {
    constructor(pattern: "XX" | "XXpsk0" | "IK", initiator: boolean, keyPair?: KeyPair, options?: { psk: Uint8Array });
    s: KeyPair;
    e: KeyPair | null;
    rs: Uint8Array | null;
    psk: Uint8Array | null;
    tx: Uint8Array | null;
    rx: Uint8Array | null;
    hash: Uint8Array | null;
    complete: boolean;
    initialise(prologue: Uint8Array, remoteStatic?: Uint8Array): void;
    send(payload?: Uint8Array): Uint8Array;
    recv(message: Uint8Array): Uint8Array;
  }
}
