import * as SecureStore from "expo-secure-store";
import type { RemoteCredentials } from "./types";

const CREDENTIAL_KEYS: { [Key in keyof RemoteCredentials]: string } = {
  pairId: "futureos.remote.pair-id.v1",
  deviceId: "futureos.remote.credential-device-id.v1",
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

const secureOptions: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
};

export async function loadCredentials(): Promise<RemoteCredentials | null> {
  const entries = await Promise.all(
    Object.entries(CREDENTIAL_KEYS).map(async ([field, key]) => [
      field,
      await SecureStore.getItemAsync(key, secureOptions),
    ]),
  );
  if (entries.every(([, value]) => value == null)) return null;
  if (entries.some(([, value]) => value == null)) {
    await clearCredentials();
    return null;
  }
  return Object.fromEntries(entries) as unknown as RemoteCredentials;
}

export async function saveCredentials(credentials: RemoteCredentials): Promise<void> {
  await Promise.all(
    (Object.keys(CREDENTIAL_KEYS) as (keyof RemoteCredentials)[]).map(field =>
      SecureStore.setItemAsync(CREDENTIAL_KEYS[field], credentials[field], secureOptions),
    ),
  );
}

export async function clearCredentials(): Promise<void> {
  await Promise.all(
    Object.values(CREDENTIAL_KEYS).map(key => SecureStore.deleteItemAsync(key, secureOptions)),
  );
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
