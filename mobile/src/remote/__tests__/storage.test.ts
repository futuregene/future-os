import * as SecureStore from "expo-secure-store";
import {
  clearCredentials,
  clearPendingRevoke,
  loadCredentials,
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

describe("credential storage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  test("loadCredentials returns null when every field is absent", async () => {
    mockedStore.getItemAsync.mockResolvedValue(null);
    expect(await loadCredentials()).toBeNull();
  });

  test("loadCredentials clears and returns null when only some fields exist", async () => {
    // First field present, the rest absent — a torn write must self-heal.
    mockedStore.getItemAsync.mockImplementation(async key =>
      key === "futureos.remote.pair-id.v1" ? "pair_1" : null,
    );
    expect(await loadCredentials()).toBeNull();
    expect(mockedStore.deleteItemAsync).toHaveBeenCalled();
  });

  test("loadCredentials rebuilds a full credential set", async () => {
    mockedStore.getItemAsync.mockImplementation(async key => {
      const byKey: Record<string, string> = {
        "futureos.remote.pair-id.v1": "pair_1",
        "futureos.remote.device-id.v1": "dev_1",
        "futureos.remote.seed.v1": "seed",
        "futureos.remote.user-jwt.v1": "jwt",
        "futureos.remote.refresh-token.v1": "refresh",
        "futureos.remote.nats-ws-url.v1": "wss://nats.example",
        "futureos.remote.token-url.v1": "https://example/auth/token",
        "futureos.remote.desktop-id.v1": "desktop_1",
        "futureos.remote.desktop-public-key.v1": "Ukey",
      };
      return byKey[key] ?? null;
    });
    expect(await loadCredentials()).toEqual(credentials);
  });

  test("saveCredentials writes every field", async () => {
    await saveCredentials(credentials);
    expect(mockedStore.setItemAsync).toHaveBeenCalledTimes(9);
    expect(mockedStore.setItemAsync).toHaveBeenCalledWith(
      "futureos.remote.seed.v1",
      "seed",
      expect.anything(),
    );
  });

  test("clearCredentials deletes every credential field but keeps the device id", async () => {
    await clearCredentials();
    expect(mockedStore.deleteItemAsync).toHaveBeenCalledTimes(8);
  });

  test("serializes clear behind an in-flight credential save", async () => {
    let finishSeedWrite: (() => void) | undefined;
    mockedStore.setItemAsync.mockImplementation(async key => {
      if (key !== "futureos.remote.seed.v1") return;
      await new Promise<void>(resolve => {
        finishSeedWrite = resolve;
      });
    });

    const save = saveCredentials(credentials);
    const clear = clearCredentials();
    await Promise.resolve();
    expect(mockedStore.deleteItemAsync).not.toHaveBeenCalled();

    finishSeedWrite?.();
    await save;
    await clear;
    expect(mockedStore.deleteItemAsync).toHaveBeenCalledTimes(8);
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
      JSON.stringify(revoke),
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

  test("loadPendingRevoke clears a corrupt entry and returns null", async () => {
    mockedStore.getItemAsync.mockResolvedValue("not json{");
    expect(await loadPendingRevoke()).toBeNull();
    expect(mockedStore.deleteItemAsync).toHaveBeenCalledWith(
      "futureos.remote.pending-revoke.v1",
      expect.anything(),
    );
  });

  test("clearPendingRevoke deletes the slot", async () => {
    await clearPendingRevoke();
    expect(mockedStore.deleteItemAsync).toHaveBeenCalledWith(
      "futureos.remote.pending-revoke.v1",
      expect.anything(),
    );
  });
});
