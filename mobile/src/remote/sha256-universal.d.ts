declare module "sha256-universal" {
  interface Hash {
    update(bytes: Uint8Array): Hash;
    digest(encoding: "hex"): string;
  }
  export default function sha256(): Hash;
}
