import type { RemoteCredentials } from "../src/types";
export function deviceName(): string { return "Pixel 8"; }
export async function refreshCredentials(credentials: RemoteCredentials): Promise<RemoteCredentials> { return credentials; }
