import { jwtExpiry } from "./codec";
import type { ClientHost } from "./host";
import type { RemoteCredentials } from "./types";

/** Refresh policy belongs to the role; HTTP/auth/endpoint policy belongs to its
 * explicit host service. Corrupt JWTs never enter a token-refresh loop. */
export async function ensureFreshCredentials(
  credentials: RemoteCredentials,
  signal: AbortSignal | undefined,
  refresh: ClientHost["refreshCredentials"],
): Promise<RemoteCredentials> {
  const expiry = jwtExpiry(credentials.userJwt);
  if (expiry === null) throw new Error("invalid_jwt");
  return expiry * 1000 < Date.now() + 60_000 ? refresh(credentials, signal) : credentials;
}
