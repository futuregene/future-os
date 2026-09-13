import * as SecureStore from "expo-secure-store";
import {
  clearCredentials,
  clearPendingRevoke,
  loadCredentials,
  loadPairedDesktops,
  loadDeviceId,
  loadLastModel,
  loadLastThinking,
  loadPendingRevoke,
  saveCredentials,
  saveDeviceId,
  saveLastModel,
  saveLastThinking,
  savePendingRevoke,
} from "../storage";
import type { RemoteCredentials } from "../types";

jest.mock("expo-secure-store", () => ({
  __esModule: true,
  WHEN_UNLOCKED_THIS_DEVICE_ONLY: "when-unlocked",
  getItemAsync: jest.fn(),
  setItemAsync: jest.fn(),
  deleteItemAsync: jest.fn(),
}));

const mockedStore = SecureStore as jest.Mocked<typeof SecureStore>;
let values: Map<string, string>;
beforeEach(() => {
  jest.resetAllMocks();
  values = new Map();
  mockedStore.getItemAsync.mockImplementation(async (key) => values.get(key) ?? null);
  mockedStore.setItemAsync.mockImplementation(async (key, value) => {
    values.set(key, value);
  });
  mockedStore.deleteItemAsync.mockImplementation(async (key) => {
    values.delete(key);
  });
});

const credentials: RemoteCredentials = {
  pairId: "pair_1",
  deviceId: "dev_1",
  seed: "seed",
  userJwt: "jwt",
  refreshToken: "refresh",
  natsWsUrl: "wss://nats.example",
  tokenUrl: "https://example/auth/token",
  expectedDesktopId: "desktop_1",
  expectedDesktopPublicKey: "Ukey",
};

const other: RemoteCredentials = { ...credentials, pairId: "pair_2", expectedDesktopId: "desktop_2", seed: "other-seed" };
const registryKey = "futureos.remote.desktops.v3";

function seedLegacy(slot?: "a" | "b") {
  const fields: Record<string, string> = {
    "pair-id": credentials.pairId, seed: credentials.seed, "user-jwt": credentials.userJwt,
    "refresh-token": credentials.refreshToken, "nats-ws-url": credentials.natsWsUrl,
    "token-url": credentials.tokenUrl, "desktop-id": credentials.expectedDesktopId,
    "desktop-public-key": credentials.expectedDesktopPublicKey,
  };
  for (const [key, value] of Object.entries(fields))
    values.set(`futureos.remote.${key}.v1${slot ? `.${slot}` : ""}`, value);
  values.set("futureos.remote.device-id.v1", credentials.deviceId);
  if (slot) values.set("futureos.remote.credential-commit.v2", slot);
}

describe("credential storage", () => {
  test("empty installation has no selected desktop", async () => {
    expect(await loadCredentials()).toBeNull();
    expect(await loadPairedDesktops()).toEqual([]);
  });

  test.each([undefined, "a", "b"] as const)("migrates legacy slot %s without re-pairing", async (slot) => {
    seedLegacy(slot);
    expect(await loadCredentials()).toEqual(credentials);
    expect(await loadPairedDesktops()).toEqual([{ desktopId: "desktop_1", pairId: "pair_1" }]);
    await saveCredentials(other);
    expect(await loadCredentials("desktop_1")).toEqual(credentials);
    expect(values.get("futureos.remote.credential-commit.v2")).toBe("cleared");
  });

  test("partial legacy bundle and missing device identity self-heal", async () => {
    seedLegacy();
    values.delete("futureos.remote.device-id.v1");
    expect(await loadCredentials()).toBeNull();
    expect(await loadPairedDesktops()).toEqual([]);
  });

  test("a rejected credential operation does not poison the queue", async () => {
    mockedStore.setItemAsync.mockRejectedValueOnce(new Error("storage full"));
    await expect(saveCredentials(credentials)).rejects.toThrow("storage full");
    await saveCredentials(credentials);
    expect(await loadCredentials()).toEqual(credentials);
  });

  test("adding, selecting and refreshing desktops preserves the other bundles", async () => {
    await saveCredentials(credentials);
    await saveCredentials(other);
    expect(await loadCredentials()).toEqual(other);
    const refreshed = { ...credentials, userJwt: "new-jwt" };
    await saveCredentials(refreshed);
    expect(await loadCredentials()).toEqual(refreshed);
    expect(await loadCredentials("desktop_2")).toEqual(other);
    expect(await loadPairedDesktops()).toHaveLength(2);
    expect(values.get(registryKey)).not.toContain("seed");
  });

  test("re-pairing the same desktop replaces its entry, not another desktop", async () => {
    await saveCredentials(credentials);
    await saveCredentials(other);
    await saveCredentials({ ...credentials, pairId: "replacement" });
    await clearCredentials(credentials.pairId); // late old-pair revoke
    expect(await loadPairedDesktops()).toHaveLength(2);
    expect((await loadCredentials())?.pairId).toBe("replacement");
    expect(await loadCredentials("desktop_2")).toEqual(other);
  });

  test("clear removes only the selected desktop and keeps the installation identity", async () => {
    await saveCredentials(credentials);
    await saveCredentials(other);
    await clearCredentials();
    expect(await loadCredentials()).toBeNull();
    expect(await loadCredentials("desktop_1")).toEqual(credentials);
    expect(await loadDeviceId()).toBe(credentials.deviceId);
    expect(await loadPairedDesktops()).toHaveLength(1);
  });

  test("clearing an inactive desktop keeps the active selection", async () => {
    await saveCredentials(credentials);
    await saveCredentials(other);
    await clearCredentials(credentials.pairId);
    expect(await loadCredentials()).toEqual(other);
    expect(await loadCredentials("desktop_1")).toBeNull();
  });

  test("rejects another installation's credentials", async () => {
    await saveCredentials(credentials);
    await expect(saveCredentials({ ...other, deviceId: "wrong" })).rejects.toThrow("credential_device_mismatch");
    expect(await loadCredentials()).toEqual(credentials);
  });

  test("corrupt registry is reported without overwriting it", async () => {
    values.set(registryKey, "not-json");
    await expect(saveCredentials(credentials)).rejects.toThrow();
    expect(values.get(registryKey)).toBe("not-json");
  });

  test("serializes clear behind an in-flight credential save", async () => {
    await loadCredentials(); // finish migration before blocking the bundle write
    let finishSeedWrite: (() => void) | undefined;
    mockedStore.setItemAsync.mockImplementation(async (key, value) => {
      if (key.endsWith(".seed.a")) await new Promise<void>((resolve) => { finishSeedWrite = resolve; });
      values.set(key, value);
    });
    const save = saveCredentials(credentials);
    const clear = clearCredentials();
    for (let tick = 0; tick < 30; tick++) await Promise.resolve();
    expect(finishSeedWrite).toBeDefined();
    finishSeedWrite!();
    await save;
    await clear;
    expect(await loadCredentials()).toBeNull();
    expect(await loadPairedDesktops()).toEqual([]);
  });
});

describe("device / model / thinking preferences", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  test("device id round-trips", async () => {
    mockedStore.getItemAsync.mockResolvedValue("dev_1");
    expect(await loadDeviceId()).toBe("dev_1");
    await saveDeviceId("dev_2");
    expect(mockedStore.setItemAsync).toHaveBeenCalledWith(
      "futureos.remote.device-id.v1",
      "dev_2",
      expect.anything(),
    );
  });

  test("last model round-trips", async () => {
    mockedStore.getItemAsync.mockResolvedValue("openai/gpt-5");
    expect(await loadLastModel()).toBe("openai/gpt-5");
    await saveLastModel("anthropic/claude");
    expect(mockedStore.setItemAsync).toHaveBeenCalledWith(
      "futureos.remote.last-model.v1",
      "anthropic/claude",
      expect.anything(),
    );
  });

  test("last thinking level round-trips", async () => {
    mockedStore.getItemAsync.mockResolvedValue("high");
    expect(await loadLastThinking()).toBe("high");
    await saveLastThinking("low");
    expect(mockedStore.setItemAsync).toHaveBeenCalledWith(
      "futureos.remote.last-thinking.v1",
      "low",
      expect.anything(),
    );
  });
});

describe("pending revoke queue", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  const revoke = {
    pairId: "pair_1",
    deviceId: "dev_1",
    seed: "seed",
    refreshToken: "refresh",
    tokenUrl: "https://example/auth/token",
  };

  test("savePendingRevoke serializes the minimal payload", async () => {
    await savePendingRevoke(revoke);
    expect(mockedStore.setItemAsync).toHaveBeenCalledWith(
      "futureos.remote.pending-revoke.v1",
      JSON.stringify([revoke]),
      expect.anything(),
    );
  });

  test("loadPendingRevoke returns null when absent", async () => {
    mockedStore.getItemAsync.mockResolvedValue(null);
    expect(await loadPendingRevoke()).toBeNull();
  });

  test("loadPendingRevoke parses a queued revoke", async () => {
    mockedStore.getItemAsync.mockResolvedValue(JSON.stringify(revoke));
    expect(await loadPendingRevoke()).toEqual(revoke);
  });

  test("loadPendingRevoke reports corrupt storage without deleting it", async () => {
    mockedStore.getItemAsync.mockResolvedValue("not json{");
    await expect(loadPendingRevoke()).rejects.toThrow();
    expect(mockedStore.deleteItemAsync).not.toHaveBeenCalled();
  });

  test("clearPendingRevoke deletes the slot", async () => {
    await clearPendingRevoke();
    expect(mockedStore.deleteItemAsync).toHaveBeenCalledWith(
      "futureos.remote.pending-revoke.v1",
      expect.anything(),
    );
  });
});

describe("crash-consistent credentials and revoke ownership", () => {
  test("a failed replacement preserves the previously committed bundle", async () => {
    await saveCredentials(credentials);
    mockedStore.setItemAsync.mockImplementation(async (key, value) => {
      if (key.endsWith(".seed.b")) throw new Error("disk full");
      values.set(key, value);
    });
    await expect(
      saveCredentials({ ...credentials, seed: "new-seed", pairId: "new-pair" }),
    ).rejects.toThrow("disk full");
    expect(await loadCredentials()).toEqual(credentials);
  });
  test("a failed commit marker leaves the old JWT readable", async () => {
    await saveCredentials(credentials);
    mockedStore.setItemAsync.mockImplementation(async (key, value) => {
      if (key === registryKey) throw new Error("commit failed");
      values.set(key, value);
    });
    await expect(saveCredentials({ ...credentials, userJwt: "new-jwt" })).rejects.toThrow();
    expect(await loadCredentials()).toEqual(credentials);
  });
  test("clearing one revoke cannot discard a later pair's compensation", async () => {
    await savePendingRevoke(credentials);
    await savePendingRevoke({ ...credentials, pairId: "pair_2" });
    await clearPendingRevoke(credentials.pairId);
    expect((await loadPendingRevoke())?.pairId).toBe("pair_2");
    expect(values.get("futureos.remote.pending-revoke.v1")).not.toContain("userJwt");
  });
});
