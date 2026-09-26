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
import { loadDeviceId, saveDeviceId } from "../storage";

jest.mock("expo-device", () => ({ __esModule: true, modelName: "iPhone 15" }));
jest.mock("react-native", () => ({ __esModule: true, Platform: { OS: "ios" } }));
jest.mock("../storage", () => ({
  __esModule: true,
  loadDeviceId: jest.fn(),
  saveDeviceId: jest.fn(),
}));

const mockedLoadDeviceId = loadDeviceId as jest.Mock;
const mockedSaveDeviceId = saveDeviceId as jest.Mock;

const CLAIM_URL = "https://future-os.cn/client/v1/remote/pair/claim";

function base64Url(value: unknown): string {
  return globalThis
    .btoa(JSON.stringify(value))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

function pairingCode(exp = 1_800_000_000, claimUrl = CLAIM_URL): string {
  return base64Url({ v: 2, nonce: "nonce_1", claim_url: claimUrl, exp });
}

function invitation(claimUrl = CLAIM_URL): string {
  return `futureos://remote/pair?v=2&code=${pairingCode(undefined, claimUrl)}&desktopId=desktop_1&desktopKey=UABC&secureKey=${"A".repeat(43)}&secret=${"A".repeat(43)}`;
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
      tokenUrl: "https://future-os.cn/client/v1/remote/auth/token",
      expectedDesktopId: "desktop_1",
      expectedDesktopPublicKey: "UABC",
    });
    expect(credentials.seed).not.toBe("");
    expect(credentials.secureBundle).toBeTruthy();
    const bundle = JSON.parse(credentials.secureBundle!);
    expect(bundle.secret).toBe("A".repeat(43));
    expect(JSON.stringify(lastFetchBody())).not.toContain(bundle.secret);
    expect(JSON.stringify(lastFetchBody())).not.toContain(bundle.identity.privateKey);
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

  test("an invitation without the v2 transport keys is refused before any request", async () => {
    // A v1 code carries no secureKey/secret, so the Noise channel this client
    // only speaks cannot be established. Failing here names the problem
    // instead of surfacing an unexplained handshake error later.
    const legacy = "futureos://remote/pair?code=abc_123&desktopId=desktop_1&desktopKey=UABC";
    await expect(claimPairingCode(legacy)).rejects.toThrow("pairing_identity_mismatch");
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  test("an already-registered device id is reused instead of rotated", async () => {
    // Rotating the device id on every claim would leave the desktop with a
    // stale peer entry for the same phone.
    mockedLoadDeviceId.mockResolvedValue("dev_existing");
    const credentials = await claimPairingCode(invitation());
    expect(credentials.deviceId).toBe("dev_existing");
    expect(mockedSaveDeviceId).not.toHaveBeenCalled();
    expect(lastFetchBody().device_id).toBe("dev_existing");
  });

  test("an error body that is not JSON still reports the HTTP status", async () => {
    // A proxy or a captive portal can answer with HTML; the phone must still
    // say what the server said (the status), not that JSON parsing failed.
    globalThis.fetch = jest.fn().mockResolvedValue({
      ok: false,
      status: 502,
      json: async () => { throw new Error("not json"); },
    } as unknown as Response);
    await expect(claimPairingCode(invitation())).rejects.toThrow("HTTP 502");
  });

  test("rejects an invitation whose embedded code does not decode", async () => {
    const bad = invitation().replace(pairingCode(), "!!!");
    await expect(claimPairingCode(bad)).rejects.toThrow("invalid_pairing_code");
  });

  test.each(["future-os.cn", "test.future-os.cn"])("uses the QR's %s platform for claim, refresh and revoke", async host => {
    const platform = `https://${host}/client/v1/remote`;
    const credentials = await claimPairingCode(invitation(`${platform}/pair/claim`));
    expect(globalThis.fetch).toHaveBeenLastCalledWith(`${platform}/pair/claim`, expect.anything());
    expect(credentials.tokenUrl).toBe(`${platform}/auth/token`);
    await refreshCredentials(credentials);
    expect(globalThis.fetch).toHaveBeenLastCalledWith(`${platform}/auth/token`, expect.anything());
    await serverRevoke(credentials);
    expect(globalThis.fetch).toHaveBeenLastCalledWith(`${platform}/pair/revoke`, expect.anything());
  });

  test("rejects a claim URL from an untrusted host before making a request", async () => {
    await expect(claimPairingCode(invitation("https://example.com/client/v1/remote/pair/claim")))
      .rejects.toThrow("unexpected_pairing_host");
    expect(globalThis.fetch).not.toHaveBeenCalled();
    expect(mockedSaveDeviceId).not.toHaveBeenCalled();
  });

  test("rejects a non-wss NATS endpoint", async () => {
    globalThis.fetch = jest.fn().mockResolvedValue(jsonResponse({ nats_ws_url: "ws://nats.example" }));
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
    globalThis.fetch = jest
      .fn()
      .mockResolvedValue(jsonResponse({ user_jwt: jwt(), nats_ws_url: "ws://nats.example" }));
    await expect(refreshCredentials(makeCredentials())).rejects.toThrow("nats_ws_not_tls");
  });
});

describe("ensureFreshCredentials", () => {
  beforeEach(() => {
    jest.clearAllMocks();
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
