import { ensureFreshCredentials } from "../src/pairing";
import type { RemoteCredentials } from "../src/types";
const token = (expiry: number) => `header.${Buffer.from(JSON.stringify({ exp: expiry })).toString("base64url")}.signature`;
const credentials = (expiry: number): RemoteCredentials => ({
  pairId: "pair_policy", deviceId: "controller_policy", seed: "unused", userJwt: token(expiry), refreshToken: "unused",
  natsWsUrl: "wss://relay.example", tokenUrl: "https://relay.example/token", expectedDesktopId: "host_policy", expectedDesktopPublicKey: "unused",
});

test("a corrupt JWT is refused before contacting the host refresh service", async () => {
  const refresh = jest.fn();
  await expect(ensureFreshCredentials({ ...credentials(0), userJwt: "bad" }, undefined, refresh)).rejects.toThrow("invalid_jwt");
  expect(refresh).not.toHaveBeenCalled();
});

test("a fresh credential keeps its identity without calling the host", async () => {
  const current = credentials(Math.floor(Date.now() / 1000) + 3600);
  const refresh = jest.fn();
  await expect(ensureFreshCredentials(current, undefined, refresh)).resolves.toBe(current);
  expect(refresh).not.toHaveBeenCalled();
});

test("near-expiry refresh forwards the same owner cancellation signal and preserves host refusal", async () => {
  const current = credentials(Math.floor(Date.now() / 1000) + 30);
  const owner = new AbortController();
  const refusal = new Error("fixture_refresh_refused");
  const refresh = jest.fn().mockRejectedValue(refusal);
  await expect(ensureFreshCredentials(current, owner.signal, refresh)).rejects.toBe(refusal);
  expect(refresh).toHaveBeenCalledWith(current, owner.signal);
});
