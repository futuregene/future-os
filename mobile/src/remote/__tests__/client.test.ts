import { RemoteClient as SharedClient } from "@future-os/remote-client/client";
import { VERSION } from "../../version.generated";
import { RemoteClient } from "../client";
import { deviceVersion } from "../pairing";
import type { RemoteCredentials } from "../types";

/**
 * The phone's own subclass of the shared client.
 *
 * It exists to supply this platform's identity to `Client` — the device name,
 * kind, build version, id generator, credential refresh and WSS transport. Two
 * of those are only read once a connection reaches `secure_ready`, so nothing
 * else in the suite ever asks for them; these tests pin the wiring instead of
 * leaving the constructor untested until a real desktop disagrees with it.
 */
describe("RemoteClient", () => {
  const credentials: RemoteCredentials = {
    pairId: "pair_1",
    deviceId: "dev_1",
    seed: "S",
    userJwt: "jwt",
    refreshToken: "refresh",
    natsWsUrl: "wss://future-os.cn:443",
    tokenUrl: "https://future-os.cn/client/v1/remote/token",
    expectedDesktopId: "desktop_1",
    expectedDesktopPublicKey: "UABC",
  };
  const callbacks = { onEvent: jest.fn(), onState: jest.fn() } as never;

  test("is the shared client, constructed with this phone's credentials", () => {
    const client = new RemoteClient(credentials, callbacks);
    expect(client).toBeInstanceOf(SharedClient);
    // Nothing is attempted on construction: the runtime decides when to connect.
    expect(client.connectionState).toBeDefined();
  });

  test("declares itself a phone to the desktop it connects to", () => {
    // The kind travels on the `secure_ready` handshake, which only runs on a
    // live connection: the host the subclass builds is the whole of its input,
    // so that is what is pinned here. A desktop that hears nothing cannot say
    // "a phone" about what is connected to it.
    const client = new RemoteClient(credentials, callbacks);
    const host = (client as unknown as { host: { deviceKind(): string } }).host;
    expect(host.deviceKind()).toBe("mobile");
  });

  test("reports the build that is installed, not one inferred at runtime", () => {
    // The desktop has no other way to learn this: a disconnection leaves no
    // version behind on its end.
    expect(deviceVersion()).toBe(VERSION);
    expect(deviceVersion()).toMatch(/^\d+\.\d+\.\d+/);
  });
});
