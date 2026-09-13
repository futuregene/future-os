import * as SecureStore from "expo-secure-store";
import type { RemoteCredentials } from "./types";

// `deviceId` is deliberately excluded from the credential bundle: it is the
// stable device identity that must survive an unpair, so it lives under
// DEVICE_ID_KEY alone (single source of truth) rather than being duplicated
// inside the credential set.
const CREDENTIAL_KEYS: {
  [Key in Exclude<keyof RemoteCredentials, "deviceId">]: string;
} = {
  pairId: "futureos.remote.pair-id.v1",
  seed: "futureos.remote.seed.v1",
  userJwt: "futureos.remote.user-jwt.v1",
  refreshToken: "futureos.remote.refresh-token.v1",
  natsWsUrl: "futureos.remote.nats-ws-url.v1",
  tokenUrl: "futureos.remote.token-url.v1",
  expectedDesktopId: "futureos.remote.desktop-id.v1",
  expectedDesktopPublicKey: "futureos.remote.desktop-public-key.v1",
};
const CREDENTIAL_COMMIT_KEY = "futureos.remote.credential-commit.v2";
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

async function settleWrites(writes: Promise<void>[]): Promise<void> {
  const results = await Promise.allSettled(writes);
  const failure = results.find((result) => result.status === "rejected");
  if (failure?.status === "rejected") throw failure.reason;
}

async function deleteCredentialFields(): Promise<void> {
  // Persist the tombstone before deleting fields, including legacy credentials.
  await SecureStore.setItemAsync(CREDENTIAL_COMMIT_KEY, "cleared", secureOptions);
  await settleWrites(
    Object.values(CREDENTIAL_KEYS).flatMap((key) =>
      [key, key + ".a", key + ".b"].map((item) => SecureStore.deleteItemAsync(item, secureOptions)),
    ),
  );
}

async function loadLegacyCredentials(): Promise<RemoteCredentials | null> {
  const commit = await SecureStore.getItemAsync(CREDENTIAL_COMMIT_KEY, secureOptions);
  if (commit === "cleared") return null;
  if (commit !== null && commit !== "a" && commit !== "b")
    throw new Error("invalid_credential_commit");
  const entries = await Promise.all(
    Object.entries(CREDENTIAL_KEYS).map(async ([field, key]) => [
      field,
      await SecureStore.getItemAsync(commit ? key + "." + commit : key, secureOptions),
    ]),
  );
  if (entries.every(([, value]) => value == null)) return null;
  const deviceId = await loadDeviceId();
  if (entries.some(([, value]) => !value) || !deviceId) {
    await deleteCredentialFields();
    return null;
  }
  return {
    ...Object.fromEntries(entries),
    deviceId,
  } as unknown as RemoteCredentials;
}

export interface PairedDesktop {
  desktopId: string;
  pairId: string;
}

interface DesktopEntry extends PairedDesktop {
  slot: "a" | "b";
}
interface DesktopRegistry {
  activeDesktopId: string | null;
  desktops: DesktopEntry[];
}
const DESKTOP_REGISTRY_KEY = "futureos.remote.desktops.v3";

function desktopFieldKey(desktopId: string, field: string, slot: string): string {
  // SecureStore keys accept only alphanumeric characters, '.', '-' and '_'.
  const encoded = Array.from(desktopId, (char) => char.codePointAt(0)!.toString(16)).join("-");
  return `futureos.remote.desktop.v3.${encoded}.${field}.${slot}`;
}

async function commitRegistry(registry: DesktopRegistry): Promise<void> {
  await SecureStore.setItemAsync(DESKTOP_REGISTRY_KEY, JSON.stringify(registry), secureOptions);
}

async function writeDesktop(
  registry: DesktopRegistry,
  credentials: RemoteCredentials,
): Promise<void> {
  const { deviceId, expectedDesktopId: desktopId } = credentials;
  const storedDeviceId = await loadDeviceId();
  if (storedDeviceId && storedDeviceId !== deviceId)
    throw new Error("credential_device_mismatch");
  if (!storedDeviceId) await saveDeviceId(deviceId);
  const previous = registry.desktops.find((entry) => entry.desktopId === desktopId);
  const slot = previous?.slot === "a" ? "b" : "a";
  const fields = Object.keys(CREDENTIAL_KEYS) as (keyof typeof CREDENTIAL_KEYS)[];
  await settleWrites(fields.map((field) => SecureStore.setItemAsync(
    desktopFieldKey(desktopId, field, slot), credentials[field], secureOptions,
  )));
  const entry: DesktopEntry = { desktopId, pairId: credentials.pairId, slot };
  // One commit switches the complete bundle and active selection together.
  await commitRegistry({
    activeDesktopId: desktopId,
    desktops: previous
      ? registry.desktops.map((item) => item.desktopId === desktopId ? entry : item)
      : [...registry.desktops, entry],
  });
}

async function readRegistry(): Promise<DesktopRegistry> {
  const raw = await SecureStore.getItemAsync(DESKTOP_REGISTRY_KEY, secureOptions);
  if (raw !== null) {
    const parsed = JSON.parse(raw) as DesktopRegistry;
    if (!parsed || !Array.isArray(parsed.desktops) ||
      !parsed.desktops.every((entry) => entry && typeof entry.desktopId === "string" &&
        entry.desktopId && typeof entry.pairId === "string" && entry.pairId &&
        (entry.slot === "a" || entry.slot === "b")) ||
      new Set(parsed.desktops.map((entry) => entry.desktopId)).size !== parsed.desktops.length ||
      (parsed.activeDesktopId !== null &&
        !parsed.desktops.some((entry) => entry.desktopId === parsed.activeDesktopId)))
      throw new Error("invalid_desktop_registry");
    return parsed;
  }
  const registry: DesktopRegistry = { activeDesktopId: null, desktops: [] };
  const legacy = await loadLegacyCredentials();
  if (legacy) await writeDesktop(registry, legacy);
  else await commitRegistry(registry);
  // Never destroy the old bundle until the new registry is durable. The
  // registry is authoritative thereafter, so old credentials cannot resurrect.
  await deleteCredentialFields();
  return legacy ? readRegistry() : registry;
}

export async function loadPairedDesktops(): Promise<PairedDesktop[]> {
  return enqueueCredentialOperation(async () =>
    (await readRegistry()).desktops.map(({ desktopId, pairId }) => ({ desktopId, pairId })),
  );
}

export async function loadCredentials(desktopId?: string): Promise<RemoteCredentials | null> {
  return enqueueCredentialOperation(async () => {
    const registry = await readRegistry();
    const entry = registry.desktops.find((item) =>
      item.desktopId === (desktopId ?? registry.activeDesktopId));
    if (!entry) return null;
    const fields = await Promise.all(Object.keys(CREDENTIAL_KEYS).map(async (field) => [
      field, await SecureStore.getItemAsync(desktopFieldKey(entry.desktopId, field, entry.slot), secureOptions),
    ]));
    const deviceId = await loadDeviceId();
    if (!deviceId || fields.some(([, value]) => !value)) throw new Error("incomplete_desktop_credentials");
    const credentials = { ...Object.fromEntries(fields), deviceId } as unknown as RemoteCredentials;
    if (credentials.pairId !== entry.pairId || credentials.expectedDesktopId !== entry.desktopId)
      throw new Error("desktop_credential_mismatch");
    return credentials;
  });
}

export async function saveCredentials(credentials: RemoteCredentials): Promise<void> {
  return enqueueCredentialOperation(async () => writeDesktop(await readRegistry(), credentials));
}

/** Remove only the matching pair (or the active desktop), never other desktops. */
export async function clearCredentials(expectedPairId?: string): Promise<void> {
  return enqueueCredentialOperation(async () => {
    const registry = await readRegistry();
    const entry = registry.desktops.find((item) => expectedPairId
      ? item.pairId === expectedPairId : item.desktopId === registry.activeDesktopId);
    if (!entry) return;
    await commitRegistry({
      activeDesktopId: registry.activeDesktopId === entry.desktopId ? null : registry.activeDesktopId,
      desktops: registry.desktops.filter((item) => item !== entry),
    });
    await settleWrites(Object.keys(CREDENTIAL_KEYS).flatMap((field) =>
      ["a", "b"].map((slot) => SecureStore.deleteItemAsync(
        desktopFieldKey(entry.desktopId, field, slot), secureOptions,
      )),
    ));
  });
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
async function readPendingRevokes(): Promise<PendingRevoke[]> {
  const raw = await SecureStore.getItemAsync(PENDING_REVOKE_KEY, secureOptions);
  if (!raw) return [];
  const parsed: unknown = JSON.parse(raw);
  const entries = Array.isArray(parsed) ? parsed : [parsed];
  if (
    !entries.every(
      (entry) =>
        entry &&
        ["pairId", "deviceId", "seed", "refreshToken", "tokenUrl"].every(
          (field) => typeof entry[field] === "string" && entry[field].length > 0,
        ),
    )
  )
    throw new Error("invalid_pending_revoke");
  return entries as PendingRevoke[];
}

export async function savePendingRevoke(revoke: PendingRevoke): Promise<void> {
  return enqueueCredentialOperation(async () => {
    const pending = await readPendingRevokes();
    const { pairId, deviceId, seed, refreshToken, tokenUrl } = revoke;
    await SecureStore.setItemAsync(
      PENDING_REVOKE_KEY,
      JSON.stringify([
        ...pending.filter((entry) => entry.pairId !== revoke.pairId),
        { pairId, deviceId, seed, refreshToken, tokenUrl },
      ]),
      secureOptions,
    );
  });
}

export async function loadPendingRevoke(
  excluded: ReadonlySet<string> = new Set(),
): Promise<PendingRevoke | null> {
  return enqueueCredentialOperation(
    async () =>
      (await readPendingRevokes()).find((pending) => !excluded.has(pending.pairId)) ?? null,
  );
}

export async function clearPendingRevoke(pairId?: string): Promise<void> {
  return enqueueCredentialOperation(async () => {
    const pending = pairId
      ? (await readPendingRevokes()).filter((entry) => entry.pairId !== pairId)
      : [];
    if (pending.length)
      await SecureStore.setItemAsync(PENDING_REVOKE_KEY, JSON.stringify(pending), secureOptions);
    else await SecureStore.deleteItemAsync(PENDING_REVOKE_KEY, secureOptions);
  });
}
