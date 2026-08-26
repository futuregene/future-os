import * as SecureStore from "expo-secure-store";
import type { RemoteCredentials } from "./types";

// `deviceId` is deliberately excluded from the credential bundle: it is the
// stable device identity that must survive an unpair, so it lives under
// DEVICE_ID_KEY alone (single source of truth) rather than being duplicated
// inside the credential set.
const CREDENTIAL_KEYS: { [Key in Exclude<keyof RemoteCredentials, "deviceId">]: string } = {
  pairId: "futureos.remote.pair-id.v1",
  seed: "futureos.remote.seed.v1",
  userJwt: "futureos.remote.user-jwt.v1",
  refreshToken: "futureos.remote.refresh-token.v1",
  natsWsUrl: "futureos.remote.nats-ws-url.v1",
  tokenUrl: "futureos.remote.token-url.v1",
  expectedDesktopId: "futureos.remote.desktop-id.v1",
  expectedDesktopPublicKey: "futureos.remote.desktop-public-key.v1",
};
const DEVICE_ID_KEY = "futureos.remote.device-id.v1";
const LAST_MODEL_KEY = "futureos.remote.last-model.v1";
const LAST_THINKING_KEY = "futureos.remote.last-thinking.v1";
const PENDING_REVOKE_KEY = "futureos.remote.pending-revoke.v1";

const secureOptions: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
};

// SecureStore has no multi-key transaction. Keep credential bundle operations
// in call order so readers never observe an in-process partial write, and an
// older clear cannot race a newer pair and delete fields that were just written.
let credentialOperationQueue: Promise<void> = Promise.resolve();

function enqueueCredentialOperation<T>(operation: () => Promise<T>): Promise<T> {
  const result = credentialOperationQueue.then(operation, operation);
  credentialOperationQueue = result.then(
    () => undefined,
    () => undefined,
  );
  return result;
}

async function deleteCredentialFields(): Promise<void> {
  await Promise.all(
    Object.values(CREDENTIAL_KEYS).map(key => SecureStore.deleteItemAsync(key, secureOptions)),
  );
}

export async function loadCredentials(): Promise<RemoteCredentials | null> {
  return enqueueCredentialOperation(async () => {
    const entries = await Promise.all(
      Object.entries(CREDENTIAL_KEYS).map(async ([field, key]) => [
        field,
        await SecureStore.getItemAsync(key, secureOptions),
      ]),
    );
    if (entries.every(([, value]) => value == null)) return null;
    if (entries.some(([, value]) => value == null)) {
      await deleteCredentialFields();
      return null;
    }
    const deviceId = await loadDeviceId();
    if (deviceId == null) {
      // A credential bundle without its device identity is corrupt; clear it so a
      // fresh pair re-establishes both.
      await deleteCredentialFields();
      return null;
    }
    return { ...Object.fromEntries(entries), deviceId } as unknown as RemoteCredentials;
  });
}

export async function saveCredentials(credentials: RemoteCredentials): Promise<void> {
  return enqueueCredentialOperation(async () => {
    const { deviceId, ...rest } = credentials;
    const fields = Object.keys(CREDENTIAL_KEYS) as (keyof typeof rest)[];
    await Promise.all([
      ...fields.map(field =>
        SecureStore.setItemAsync(CREDENTIAL_KEYS[field], rest[field], secureOptions),
      ),
      saveDeviceId(deviceId),
    ]);
  });
}

export async function clearCredentials(): Promise<void> {
  return enqueueCredentialOperation(deleteCredentialFields);
}

export async function loadDeviceId(): Promise<string | null> {
  return SecureStore.getItemAsync(DEVICE_ID_KEY, secureOptions);
}

export async function saveDeviceId(deviceId: string): Promise<void> {
  await SecureStore.setItemAsync(DEVICE_ID_KEY, deviceId, secureOptions);
}

export async function loadLastModel(): Promise<string | null> {
  return SecureStore.getItemAsync(LAST_MODEL_KEY, secureOptions);
}

export async function saveLastModel(modelId: string): Promise<void> {
  await SecureStore.setItemAsync(LAST_MODEL_KEY, modelId, secureOptions);
}

export async function loadLastThinking(): Promise<string | null> {
  return SecureStore.getItemAsync(LAST_THINKING_KEY, secureOptions);
}

export async function saveLastThinking(level: string): Promise<void> {
  await SecureStore.setItemAsync(LAST_THINKING_KEY, level, secureOptions);
}

/** Minimal payload needed to retry a server-side pair revocation later. */
export interface PendingRevoke {
  pairId: string;
  deviceId: string;
  seed: string;
  refreshToken: string;
  tokenUrl: string;
}

/**
 * The unpair retry queue (M7): an offline unpair must succeed locally, and the
 * server-side revoke is queued here to fire on a later launch. Store only the
 * revoke-relevant fields — never the full credential set.
 */
export async function savePendingRevoke(revoke: PendingRevoke): Promise<void> {
  await SecureStore.setItemAsync(PENDING_REVOKE_KEY, JSON.stringify(revoke), secureOptions);
}

export async function loadPendingRevoke(): Promise<PendingRevoke | null> {
  const raw = await SecureStore.getItemAsync(PENDING_REVOKE_KEY, secureOptions);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as PendingRevoke;
  } catch {
    await clearPendingRevoke();
    return null;
  }
}

export async function clearPendingRevoke(): Promise<void> {
  await SecureStore.deleteItemAsync(PENDING_REVOKE_KEY, secureOptions);
}
