import * as Device from "expo-device";
import { createUser, fromSeed } from "nkeys.js";
import {
  attemptPendingRevoke,
  claimPairingCode,
  ensureFreshCredentials,
  refreshCredentials,
  serverRevoke,
} from "../pairing";
import type { RemoteCredentials } from "../types";
import { isExpectedClaimUrl, natsWsUrlScheme } from "../../config/environment";
import { loadDeviceId, saveDeviceId } from "../storage";

jest.mock("expo-device", () => ({ __esModule: true, modelName: "iPhone 15" }));
jest.mock("react-native", () => ({ __esModule: true, Platform: { OS: "ios" } }));
jest.mock("../../config/environment", () => ({
  __esModule: true,
  isExpectedClaimUrl: jest.fn(),
  natsWsUrlScheme: jest.fn(),
}));
jest.mock("../storage", () => ({
  __esModule: true,
  loadDeviceId: jest.fn(),
  saveDeviceId: jest.fn(),
}));

const mockedIsExpectedClaimUrl = isExpectedClaimUrl as jest.Mock;
const mockedNatsWsUrlScheme = natsWsUrlScheme as jest.Mock;
const mockedLoadDeviceId = loadDeviceId as jest.Mock;
const mockedSaveDeviceId = saveDeviceId as jest.Mock;

const CLAIM_URL = "https://example.com/client/v1/remote/pair/claim";

function base64Url(value: unknown): string {
  return globalThis
    .btoa(JSON.stringify(value))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

function pairingCode(exp = 1_800_000_000): string {
  return base64Url({ v: 2, nonce: "nonce_1", claim_url: CLAIM_URL, exp });
}

function invitation(): string {
  return `futureos://remote/pair?code=${pairingCode()}&desktopId=desktop_1&desktopKey=UABC`;
}

function jwt(exp = 1_800_000_000): string {
  return `header.${base64Url({ exp })}.signature`;
}

function makeCredentials(): RemoteCredentials {
  const keyPair = createUser();
  return {
    pairId: "pair_1",
    deviceId: "dev_1",
    seed: new TextDecoder().decode(keyPair.getSeed()),
    userJwt: jwt(),
    refreshToken: "refresh_1",
    natsWsUrl: "wss://nats.example",
    tokenUrl: "https://example.com/auth/token",
    expectedDesktopId: "desktop_1",
    expectedDesktopPublicKey: "UABC",
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  } as unknown as Response;
}

function lastFetchBody(): Record<string, unknown> {
  const call = (globalThis.fetch as jest.Mock).mock.calls.at(-1)!;
  return JSON.parse(call[1].body as string) as Record<string, unknown>;
}

describe("claimPairingCode", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockedIsExpectedClaimUrl.mockReturnValue(true);
    mockedNatsWsUrlScheme.mockReturnValue("wss");
    mockedLoadDeviceId.mockResolvedValue(null);
    globalThis.fetch = jest.fn().mockResolvedValue(
      jsonResponse({
        pair_id: "pair_1",
        user_jwt: jwt(),
        refresh_token: "refresh_1",
        nats_ws_url: "wss://nats.example",
      }),
    );
  });

  test("claims a valid code and returns the full credential set", async () => {
    const credentials = await claimPairingCode(invitation());
    expect(credentials).toMatchObject({
      pairId: "pair_1",
      deviceId: expect.stringMatching(/^dev_[0-9a-f]{32}$/),
      refreshToken: "refresh_1",
      natsWsUrl: "wss://nats.example",
      tokenUrl: "https://example.com/client/v1/remote/auth/token",
      expectedDesktopId: "desktop_1",
      expectedDesktopPublicKey: "UABC",
    });
    expect(credentials.seed).not.toBe("");
    const body = lastFetchBody();
    expect(body).toMatchObject({
      nonce: "nonce_1",
      device_id: credentials.deviceId,
      device_name: "iPhone 15",
    });
    expect(mockedSaveDeviceId).toHaveBeenCalledWith(credentials.deviceId);
  });

  test("falls back to a platform device name when modelName is absent", async () => {
    (Device as { modelName: string | null }).modelName = null;
    const credentials = await claimPairingCode(invitation());
    expect(lastFetchBody().device_name).toBe("ios device");
    expect(credentials.deviceId).toBeTruthy();
  });

  test("rejects a non-invitation payload", async () => {
    await expect(claimPairingCode("not-an-invitation")).rejects.toThrow("invalid_pairing_code");
  });

  test("rejects an invitation whose embedded code does not decode", async () => {
    const bad = "futureos://remote/pair?code=!!!&desktopId=desktop_1&desktopKey=UABC";
    await expect(claimPairingCode(bad)).rejects.toThrow("invalid_pairing_code");
  });

  test("rejects a claim URL from an unexpected host", async () => {
    mockedIsExpectedClaimUrl.mockReturnValue(false);
    await expect(claimPairingCode(invitation())).rejects.toThrow("unexpected_pairing_host");
  });

  test("rejects a non-wss NATS endpoint", async () => {
    mockedNatsWsUrlScheme.mockReturnValue("ws");
    await expect(claimPairingCode(invitation())).rejects.toThrow("nats_ws_not_tls");
  });

  test("rejects a JWT with no readable expiry", async () => {
    globalThis.fetch = jest.fn().mockResolvedValue(
      jsonResponse({
        pair_id: "pair_1",
        user_jwt: "header.bogus.signature",
        refresh_token: "refresh_1",
        nats_ws_url: "wss://nats.example",
      }),
    );
    await expect(claimPairingCode(invitation())).rejects.toThrow("invalid_jwt");
  });
});

describe("refreshCredentials", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockedNatsWsUrlScheme.mockReturnValue("wss");
  });

  test("rotates the JWT and NATS endpoint", async () => {
    globalThis.fetch = jest
      .fn()
      .mockResolvedValue(jsonResponse({ user_jwt: jwt(), nats_ws_url: "wss://nats-2.example" }));
    const credentials = makeCredentials();
    const refreshed = await refreshCredentials(credentials);
    expect(refreshed).toMatchObject({
      ...credentials,
      natsWsUrl: "wss://nats-2.example",
    });
    const body = lastFetchBody();
    expect(body).toMatchObject({
      pair_id: "pair_1",
      device_id: "dev_1",
      role: "client",
      refresh_token: "refresh_1",
    });
    expect(body.public_key).toBe(
      fromSeed(new TextEncoder().encode(credentials.seed)).getPublicKey(),
    );
  });

  test("rejects a non-wss refreshed endpoint", async () => {
    mockedNatsWsUrlScheme.mockReturnValue("ws");
    globalThis.fetch = jest
      .fn()
      .mockResolvedValue(jsonResponse({ user_jwt: jwt(), nats_ws_url: "ws://nats.example" }));
    await expect(refreshCredentials(makeCredentials())).rejects.toThrow("nats_ws_not_tls");
  });
});

describe("ensureFreshCredentials", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockedNatsWsUrlScheme.mockReturnValue("wss");
  });

  test("fails loudly on a corrupt JWT instead of looping a refresh", async () => {
    const credentials = { ...makeCredentials(), userJwt: "header.bogus.signature" };
    await expect(ensureFreshCredentials(credentials)).rejects.toThrow("invalid_jwt");
  });

  test("refreshes when the token is within 60s of expiry", async () => {
    const exp = Math.floor(Date.now() / 1000) + 30; // 30s left
    const credentials = { ...makeCredentials(), userJwt: jwt(exp) };
    globalThis.fetch = jest
      .fn()
      .mockResolvedValue(jsonResponse({ user_jwt: jwt(), nats_ws_url: "wss://nats.example" }));
    const result = await ensureFreshCredentials(credentials);
    expect(result.userJwt).not.toBe(credentials.userJwt);
  });

  test("returns the credential untouched when still fresh", async () => {
    const credentials = makeCredentials(); // exp far in the future
    await expect(ensureFreshCredentials(credentials)).resolves.toBe(credentials);
  });
});

describe("serverRevoke", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  test("succeeds on a 2xx response", async () => {
    globalThis.fetch = jest.fn().mockResolvedValue(jsonResponse({}, 200));
    await expect(serverRevoke(makeCredentials())).resolves.toBeUndefined();
  });

  test("treats 401/404 as terminal success so the retry queue drains", async () => {
    globalThis.fetch = jest.fn().mockResolvedValue(jsonResponse({}, 401));
    await expect(serverRevoke(makeCredentials())).resolves.toBeUndefined();
    globalThis.fetch = jest.fn().mockResolvedValue(jsonResponse({}, 404));
    await expect(serverRevoke(makeCredentials())).resolves.toBeUndefined();
  });

  test("rethrows the server error on a non-terminal failure", async () => {
    globalThis.fetch = jest.fn().mockResolvedValue(jsonResponse({ message: "boom" }, 500));
    await expect(serverRevoke(makeCredentials())).rejects.toThrow("boom");
  });
});

describe("attemptPendingRevoke", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  test("fires the queued revoke and resolves", async () => {
    globalThis.fetch = jest.fn().mockResolvedValue(jsonResponse({}, 200));
    const credentials = makeCredentials();
    const pending = {
      pairId: credentials.pairId,
      deviceId: credentials.deviceId,
      seed: credentials.seed,
      refreshToken: credentials.refreshToken,
      tokenUrl: credentials.tokenUrl,
    };
    await expect(attemptPendingRevoke(pending)).resolves.toBeUndefined();
    expect(globalThis.fetch).toHaveBeenCalledWith(
      "https://example.com/pair/revoke",
      expect.objectContaining({ method: "POST" }),
    );
  });
});
