import * as Device from "expo-device";
import { Platform } from "react-native";
import { createUser, fromSeed } from "nkeys.js";
import { isExpectedClaimUrl, natsWsUrlScheme } from "../config/environment";
import { decodePairingCode, jwtExpiry, parsePairingInvitation, randomId } from "./codec";
import { RemoteApiError } from "./connectionState";
import { loadDeviceId, saveDeviceId, type PendingRevoke } from "./storage";
import type { RemoteCredentials } from "./types";

interface ClaimResponse {
  pair_id: string;
  user_jwt: string;
  refresh_token: string;
  nats_ws_url: string;
}

interface RefreshResponse {
  user_jwt: string;
  nats_ws_url: string;
}

// NATS must be reached over TLS: iOS's ATS rejects plaintext `ws://` and the
// server now hands out `wss://` everywhere. Refuse a non-`wss://` endpoint
// rather than silently degrade to cleartext.
function assertSecureNatsUrl(url: string): void {
  if (natsWsUrlScheme(url) !== "wss") {
    // Server address only (no secret) — surfaces what the platform handed
    // out when a pairing is rejected, in Metro/Xcode consoles.
    console.warn(`[remote] rejecting non-wss NATS endpoint: ${url}`);
    throw new Error("nats_ws_not_tls");
  }
}

async function responseJson<T>(response: Response): Promise<T> {
  const body = (await response.json().catch(() => ({}))) as {
    error?: string;
    message?: string;
  } & T;
  if (!response.ok) {
    // Preserve the server's machine-readable `error` code alongside the human
    // message, so a revocation (`invalid_remote_credential`) is distinguishable
    // from a transient failure instead of being sniffed from prose.
    throw new RemoteApiError(
      body.message ?? `HTTP ${response.status}`,
      body.error,
      response.status,
    );
  }
  return body;
}

// The desktop rejects a server-issued JWT that carries no readable `exp`
// (pairing.rs `jwt_expiry` errors on a missing expiry). Mirror that here so a
// malformed token never reaches the credential store, where a client-side
// `jwtExpiry` that silently returned 0 would otherwise drive a 5s refresh storm.
function assertValidJwt(userJwt: string): void {
  if (jwtExpiry(userJwt) === null) throw new Error("invalid_jwt");
}

async function deviceId(): Promise<string> {
  const stored = await loadDeviceId();
  if (stored) return stored;
  const created = randomId("dev");
  await saveDeviceId(created);
  return created;
}

function deviceName(): string {
  return Device.modelName ?? `${Platform.OS} device`;
}

export async function claimPairingCode(code: string): Promise<RemoteCredentials> {
  const invitation = parsePairingInvitation(code);
  if (!invitation) throw new Error("invalid_pairing_code");
  const pairing = decodePairingCode(invitation.code);
  if (!pairing) throw new Error("invalid_pairing_code");
  if (!isExpectedClaimUrl(pairing.claim_url)) throw new Error("unexpected_pairing_host");

  const keyPair = createUser();
  const seed = new TextDecoder().decode(keyPair.getSeed());
  const id = await deviceId();
  const body = await responseJson<ClaimResponse>(
    await fetch(pairing.claim_url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        nonce: pairing.nonce,
        device_id: id,
        device_public_key: keyPair.getPublicKey(),
        device_name: deviceName(),
      }),
    }),
  );
  assertSecureNatsUrl(body.nats_ws_url);
  assertValidJwt(body.user_jwt);
  const credentials: RemoteCredentials = {
    pairId: body.pair_id,
    deviceId: id,
    seed,
    userJwt: body.user_jwt,
    refreshToken: body.refresh_token,
    natsWsUrl: body.nats_ws_url,
    tokenUrl: pairing.claim_url.replace(/\/pair\/claim$/, "/auth/token"),
    expectedDesktopId: invitation.desktopId,
    expectedDesktopPublicKey: invitation.desktopPublicKey,
  };
  return credentials;
}

export async function refreshCredentials(
  credentials: RemoteCredentials,
): Promise<RemoteCredentials> {
  const keyPair = fromSeed(new TextEncoder().encode(credentials.seed));
  const body = await responseJson<RefreshResponse>(
    await fetch(credentials.tokenUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        pair_id: credentials.pairId,
        device_id: credentials.deviceId,
        public_key: keyPair.getPublicKey(),
        role: "client",
        refresh_token: credentials.refreshToken,
      }),
    }),
  );
  const refreshed = {
    ...credentials,
    userJwt: body.user_jwt,
    natsWsUrl: body.nats_ws_url,
  };
  assertSecureNatsUrl(body.nats_ws_url);
  assertValidJwt(body.user_jwt);
  return refreshed;
}

export async function ensureFreshCredentials(
  credentials: RemoteCredentials,
): Promise<RemoteCredentials> {
  const expiry = jwtExpiry(credentials.userJwt);
  // A JWT we can't read expiry from is corrupt, not expired. The desktop
  // rejects such a token outright; looping a refresh here would hammer the
  // token endpoint on every (re)connect. Fail loudly instead.
  if (expiry === null) throw new Error("invalid_jwt");
  return expiry * 1000 < Date.now() + 60_000 ? refreshCredentials(credentials) : credentials;
}

/**
 * Server-side pair revocation (M7). Always best-effort: a device that is
 * offline (or whose token endpoint is unreachable) must still be able to
 * deregister locally. The caller persists the minimal fields and retries on a
 * later launch; revoking is idempotent on the server (a 404 means "already
 * gone"), so a queued revoke that re-fires after a success is harmless.
 */
export async function serverRevoke(credentials: RemoteCredentials): Promise<void> {
  const keyPair = fromSeed(new TextEncoder().encode(credentials.seed));
  const revokeUrl = credentials.tokenUrl.replace(/\/auth\/token$/, "/pair/revoke");
  const response = await fetch(revokeUrl, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      pair_id: credentials.pairId,
      device_id: credentials.deviceId,
      public_key: keyPair.getPublicKey(),
      refresh_token: credentials.refreshToken,
    }),
  });
  // 401/404 are terminal outcomes (unknown pair / already revoked) — treat
  // them as success so the retry queue drains instead of looping.
  if (!response.ok && response.status !== 401 && response.status !== 404) {
    await responseJson(response);
  }
}

/**
 * Fire the queued server-side revoke (M7). Best-effort: a failure is dropped —
 * the queue entry stays in storage and retries on the next launch. A success
 * (or a terminal 401/404, which serverRevoke treats as success) drains the
 * queue via the caller's clearPendingRevoke.
 */
export async function attemptPendingRevoke(pending: PendingRevoke): Promise<void> {
  await serverRevoke({
    pairId: pending.pairId,
    deviceId: pending.deviceId,
    seed: pending.seed,
    userJwt: "",
    refreshToken: pending.refreshToken,
    natsWsUrl: "",
    tokenUrl: pending.tokenUrl,
    expectedDesktopId: "",
    expectedDesktopPublicKey: "",
  });
}
