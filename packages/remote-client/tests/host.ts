import type { RemoteCredentials } from "../src/types";
export function deviceName(): string { return "Pixel 8"; }
/**
 * A build a test can recognise in the `secure_ready` payload.
 *
 * The real phone answers with the version baked in at build time; a test that
 * made one up would stop being a test of the *declaration* and start being a
 * test of a literal, so the value is only ever compared against itself.
 */
export function deviceVersion(): string { return "2.0.1-test"; }
export async function refreshCredentials(credentials: RemoteCredentials): Promise<RemoteCredentials> { return credentials; }
