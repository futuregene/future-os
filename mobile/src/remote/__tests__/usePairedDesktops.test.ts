import { createElement, useRef } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { RemoteApiError } from "@future-os/remote-client/connectionState";
import { attemptPendingRevoke, claimPairingCode, refreshCredentials, serverRevoke } from "../pairing";
import { discardPendingContinuation } from "../pendingContinuationStorage";
import { discardPendingPrompt } from "../pendingPromptStorage";
import {
  clearCredentials,
  clearPendingRevoke,
  labelDesktop,
  loadActiveDesktopId,
  loadCredentials,
  loadPairedDesktops,
  loadPendingRevoke,
  renameDesktop,
  saveCredentials,
  savePendingRevoke,
  setActiveDesktop,
} from "../storage";
import type { PairedDesktop, RemoteCredentials } from "../types";
import { usePairedDesktops, type PairedDesktops } from "../usePairedDesktops";

jest.mock("../storage", () => ({
  __esModule: true,
  clearCredentials: jest.fn(async () => {}),
  clearPendingRevoke: jest.fn(async () => {}),
  labelDesktop: jest.fn(async () => {}),
  loadActiveDesktopId: jest.fn(async () => null),
  loadCredentials: jest.fn(async () => null),
  loadPairedDesktops: jest.fn(async () => []),
  loadPendingRevoke: jest.fn(async () => null),
  renameDesktop: jest.fn(async () => {}),
  saveCredentials: jest.fn(async () => {}),
  savePendingRevoke: jest.fn(async () => {}),
  setActiveDesktop: jest.fn(async () => {}),
}));

jest.mock("../pairing", () => ({
  __esModule: true,
  attemptPendingRevoke: jest.fn(async () => {}),
  claimPairingCode: jest.fn(async () => null),
  refreshCredentials: jest.fn(async (credentials: unknown) => credentials),
  serverRevoke: jest.fn(async () => {}),
}));

jest.mock("../pendingPromptStorage", () => ({
  __esModule: true,
  discardPendingPrompt: jest.fn(async () => {}),
}));

jest.mock("../pendingContinuationStorage", () => ({
  __esModule: true,
  discardPendingContinuation: jest.fn(async () => {}),
}));

jest.mock("react-native", () => ({
  __esModule: true,
  AppState: {
    currentState: "active",
    addEventListener: jest.fn(() => ({ remove: jest.fn() })),
  },
  Platform: {
    OS: "ios",
    select: (specifics: Record<string, unknown>) =>
      specifics?.ios ?? specifics?.native ?? specifics?.default,
  },
  TurboModuleRegistry: {
    get: () => null,
    getEnforcing: () => {
      throw new Error("native module not found");
    },
  },
  NativeEventEmitter: class {},
}));

const credentials: RemoteCredentials = {
  pairId: "pair",
  deviceId: "device",
  seed: "seed",
  userJwt: "jwt",
  refreshToken: "refresh",
  natsWsUrl: "wss://nats.test",
  tokenUrl: "https://example.test/auth/token",
  expectedDesktopId: "desktop",
  expectedDesktopPublicKey: "pubkey",
};
const second: RemoteCredentials = {
  ...credentials,
  pairId: "pair-2",
  expectedDesktopId: "desktop-2",
};
const first = { desktopId: "desktop", pairId: "pair" };
const other = { desktopId: "desktop-2", pairId: "pair-2" };

function cast<T>(value: unknown): T {
  return value as T;
}

/**
 * The app's pairing surface.
 *
 * Which desktops are paired, and which one the screens read, is answered once
 * for the whole app rather than per connection — a pairing outlives every
 * socket, and a runtime per desktop is only possible if this is not entangled
 * with any of them.
 */
describe("usePairedDesktops", () => {
  let pairing: { current: PairedDesktops };
  let renderer: ReactTestRenderer | null;

  function Harness(): null {
    const networkAvailableRef = useRef<boolean | null>(true);
    pairing.current = usePairedDesktops({ networkAvailableRef });
    return null;
  }

  function render(): void {
    act(() => { renderer = create(createElement(Harness)); });
  }

  async function drain(times = 40): Promise<void> {
    for (let i = 0; i < times; i += 1) await Promise.resolve();
  }

  async function flush(): Promise<void> {
    await act(async () => { await drain(); });
  }

  beforeEach(() => {
    jest.clearAllMocks();
    pairing = { current: undefined as unknown as PairedDesktops };
    renderer = null;
    cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([]);
    cast<jest.Mock>(loadCredentials).mockResolvedValue(null);
    cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue(null);
    cast<jest.Mock>(loadPendingRevoke).mockResolvedValue(null);
    cast<jest.Mock>(claimPairingCode).mockResolvedValue(null);
    // `clearAllMocks` clears calls, not implementations: the revoke tests below
    // pin a failure with `mockRejectedValue`, which would otherwise stay the
    // implementation the next test drains against.
    cast<jest.Mock>(attemptPendingRevoke).mockResolvedValue(undefined);
    cast<jest.Mock>(serverRevoke).mockResolvedValue(undefined);
  });

  afterEach(async () => {
    if (renderer) {
      await act(async () => { renderer!.unmount(); await drain(); });
      renderer = null;
    }
  });

  describe("the stored registry", () => {
    test("reads the paired desktops and stops booting", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first, other] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop-2");
      render();
      await flush();
      expect(pairing.current.booting).toBe(false);
      expect(pairing.current.desktops).toEqual([first, other]);
      expect(pairing.current.activeDesktopId).toBe("desktop-2");
    });

    test("mounts with no desktops when nothing is paired", async () => {
      render();
      await flush();
      expect(pairing.current.booting).toBe(false);
      expect(pairing.current.desktops).toEqual([]);
      expect(pairing.current.activeDesktopId).toBeNull();
    });

    test("a dangling active pointer leaves the picker rather than a broken pairing", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop-gone");
      render();
      await flush();
      expect(pairing.current.desktops).toEqual([first]);
      expect(pairing.current.activeDesktopId).toBeNull();
      expect(pairing.current.error).toBeNull();
    });

    test("a registry read failure is reported without dropping the boot", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockRejectedValue(new Error("invalid_desktop_registry"));
      render();
      await flush();
      expect(pairing.current.booting).toBe(false);
      expect(pairing.current.error).toBe("invalid_desktop_registry");
    });
  });

  describe("pending revokes", () => {
    test("a queued server revoke is attempted and cleared on mount", async () => {
      cast<jest.Mock>(loadPendingRevoke).mockResolvedValue({
        pairId: "pair",
        deviceId: "device",
        seed: "seed",
        refreshToken: "refresh",
        tokenUrl: "https://example.test/auth/token",
      });
      render();
      await flush();
      expect(attemptPendingRevoke).toHaveBeenCalled();
      expect(clearPendingRevoke).toHaveBeenCalled();
    });

    test("a pending-revoke failure is swallowed", async () => {
      cast<jest.Mock>(loadPendingRevoke).mockResolvedValue({ pairId: "pair" });
      cast<jest.Mock>(attemptPendingRevoke).mockRejectedValue(new Error("offline"));
      render();
      await flush();
      expect(pairing.current.error).toBeNull();
    });

    test("a 4xx refusal is terminal and is not re-sent", async () => {
      jest.useFakeTimers();
      try {
        cast<jest.Mock>(loadPendingRevoke).mockResolvedValue({ pairId: "pair" });
        cast<jest.Mock>(attemptPendingRevoke).mockRejectedValue(
          new RemoteApiError("forbidden", "PA002", 403),
        );
        render();
        await flush();
        expect(attemptPendingRevoke).toHaveBeenCalledTimes(1);
        // The drainer runs again on its interval. A 4xx the server will keep
        // refusing must not be re-sent forever; a transport error must be,
        // which the next test pins.
        await act(async () => { jest.advanceTimersByTime(30_000); await drain(); });
        expect(attemptPendingRevoke).toHaveBeenCalledTimes(1);
      } finally {
        jest.useRealTimers();
      }
    });

    test("a transport failure is retried on the next drain", async () => {
      jest.useFakeTimers();
      try {
        cast<jest.Mock>(loadPendingRevoke).mockResolvedValue({ pairId: "pair" });
        cast<jest.Mock>(attemptPendingRevoke).mockRejectedValue(new Error("offline"));
        render();
        await flush();
        expect(attemptPendingRevoke).toHaveBeenCalledTimes(1);
        await act(async () => { jest.advanceTimersByTime(30_000); await drain(); });
        expect(attemptPendingRevoke).toHaveBeenCalledTimes(2);
      } finally {
        jest.useRealTimers();
      }
    });
  });

  describe("pairing", () => {
    test("a claim persists the credentials and opens the new desktop", async () => {
      render();
      await flush();
      cast<jest.Mock>(claimPairingCode).mockResolvedValue(second);
      await act(async () => { await pairing.current.pair("code"); });
      expect(claimPairingCode).toHaveBeenCalledWith("code");
      expect(saveCredentials).toHaveBeenCalledWith(second);
      expect(pairing.current.activeDesktopId).toBe("desktop-2");
    });

    test("a failed claim throws so the pairing screen can explain it", async () => {
      render();
      await flush();
      cast<jest.Mock>(claimPairingCode).mockRejectedValue(new Error("bad code"));
      await act(async () => {
        await expect(pairing.current.pair("code")).rejects.toThrow("bad code");
      });
      expect(saveCredentials).not.toHaveBeenCalled();
    });

    test("adding a desktop does not unpair the previous desktop", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop");
      render();
      await flush();
      cast<jest.Mock>(claimPairingCode).mockResolvedValue(second);
      await act(async () => { await pairing.current.pair("second-qr"); });
      expect(clearCredentials).not.toHaveBeenCalled();
      expect(serverRevoke).not.toHaveBeenCalled();
    });
  });

  describe("selection", () => {
    test("selecting another desktop only moves the pointer", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first, other] as PairedDesktop[]);
      render();
      await flush();
      await act(async () => { await pairing.current.selectDesktop("desktop-2"); });
      expect(setActiveDesktop).toHaveBeenCalledWith("desktop-2");
      expect(pairing.current.activeDesktopId).toBe("desktop-2");
      // A selection is not a pairing operation: nothing is claimed or revoked,
      // and no credential is rewritten.
      expect(claimPairingCode).not.toHaveBeenCalled();
      expect(serverRevoke).not.toHaveBeenCalled();
      expect(saveCredentials).not.toHaveBeenCalled();
      expect(clearCredentials).not.toHaveBeenCalled();
    });

    test("a desktop removed by another reader leaves the selection empty", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first, other] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop-2");
      render();
      await flush();
      expect(pairing.current.activeDesktopId).toBe("desktop-2");
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first] as PairedDesktop[]);
      await act(async () => { await pairing.current.removeDesktop("desktop-2"); });
      expect(pairing.current.activeDesktopId).toBeNull();
    });
  });

  describe("reconnect generations", () => {
    test("reconnecting one desktop does not disturb the others", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first, other] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop");
      render();
      await flush();
      act(() => pairing.current.reconnect("desktop-2"));
      expect(pairing.current.generations).toEqual({ "desktop-2": 1 });
      act(() => pairing.current.reconnect("desktop-2"));
      expect(pairing.current.generations).toEqual({ "desktop-2": 2 });
      expect(pairing.current.generations.desktop).toBeUndefined();
    });

    test("reconnecting with no desktop named rebuilds the open one", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first, other] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop");
      render();
      await flush();
      act(() => pairing.current.reconnect());
      expect(pairing.current.generations).toEqual({ desktop: 1 });
    });
  });

  describe("removal", () => {
    test("unpairing the open desktop revokes it server-side and clears it locally", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop");
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      render();
      await flush();
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([]);
      await act(async () => { await pairing.current.unpair(); });
      expect(savePendingRevoke).toHaveBeenCalledWith(credentials);
      expect(clearCredentials).toHaveBeenCalledWith(credentials.pairId);
      expect(discardPendingPrompt).toHaveBeenCalledWith(credentials.pairId);
      expect(discardPendingContinuation).toHaveBeenCalledWith(credentials.pairId);
      expect(serverRevoke).toHaveBeenCalledWith(credentials);
      expect(pairing.current.desktops).toEqual([]);
    });

    test("a refused server revoke stays queued for a later drain", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop");
      cast<jest.Mock>(loadCredentials).mockResolvedValue(credentials);
      cast<jest.Mock>(serverRevoke).mockRejectedValue(new Error("offline"));
      render();
      await flush();
      await act(async () => { await pairing.current.unpair(); });
      expect(savePendingRevoke).toHaveBeenCalledWith(
        expect.objectContaining({
          pairId: "pair",
          deviceId: "device",
          seed: "seed",
          refreshToken: "refresh",
          tokenUrl: "https://example.test/auth/token",
        }),
      );
      expect(clearPendingRevoke).not.toHaveBeenCalled();
    });

    test("removing another desktop leaves the open pairing alone", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first, other] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop");
      render();
      await flush();
      cast<jest.Mock>(loadCredentials).mockImplementation(async (id?: string) =>
        id === "desktop-2" ? second : null);
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first] as PairedDesktop[]);
      await act(async () => { await pairing.current.removeDesktop("desktop-2"); });
      expect(savePendingRevoke).toHaveBeenCalledWith(second);
      expect(clearCredentials).toHaveBeenCalledWith(second.pairId);
      expect(discardPendingPrompt).toHaveBeenCalledWith(second.pairId);
      expect(pairing.current.activeDesktopId).toBe("desktop");
      expect(pairing.current.desktops).toEqual([first]);
    });

    // The desktop ended the pairing itself: the phone's copy goes, but asking the
    // platform to revoke a credential the desktop has already killed would only
    // leave a retry behind.
    test("forgetting a desktop the desktop unpaired revokes nothing", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first, other] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop");
      render();
      await flush();
      cast<jest.Mock>(loadCredentials).mockImplementation(async (id?: string) =>
        id === "desktop-2" ? second : null);
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first] as PairedDesktop[]);

      await act(async () => { await pairing.current.forgetDesktop("desktop-2"); });

      expect(clearCredentials).toHaveBeenCalledWith(second.pairId);
      expect(discardPendingPrompt).toHaveBeenCalledWith(second.pairId);
      expect(discardPendingContinuation).toHaveBeenCalledWith(second.pairId);
      expect(savePendingRevoke).not.toHaveBeenCalled();
      expect(serverRevoke).not.toHaveBeenCalled();
      expect(pairing.current.desktops).toEqual([first]);
      expect(pairing.current.activeDesktopId).toBe("desktop");
    });

    // The check a failed connection runs: the platform's own answer about this
    // pairing, with nothing stored and no revoke implied.
    test("verifying a pairing only asks the platform to renew its credentials", async () => {
      render();
      await flush();
      await act(async () => { await pairing.current.verifyPairing(credentials); });
      expect(refreshCredentials).toHaveBeenCalledWith(credentials);
      expect(saveCredentials).not.toHaveBeenCalled();
      expect(clearCredentials).not.toHaveBeenCalled();
    });

    test("removing a desktop that is not paired locally changes nothing", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first] as PairedDesktop[]);
      cast<jest.Mock>(loadActiveDesktopId).mockResolvedValue("desktop");
      render();
      await flush();
      cast<jest.Mock>(loadCredentials).mockResolvedValue(null);
      await act(async () => { await pairing.current.removeDesktop("desktop-9"); });
      expect(savePendingRevoke).not.toHaveBeenCalled();
      expect(clearCredentials).not.toHaveBeenCalled();
    });
  });

  describe("naming", () => {
    test("renaming a desktop persists it and re-reads the list", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first] as PairedDesktop[]);
      render();
      await flush();
      const renamed = [{ desktopId: "desktop", pairId: "pair", name: "Lab" }] as PairedDesktop[];
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue(renamed);
      await act(async () => { await pairing.current.renameDesktop("desktop", "Lab"); });
      expect(renameDesktop).toHaveBeenCalledWith("desktop", "Lab");
      expect(pairing.current.desktops).toEqual(renamed);
    });

    test("naming a desktop with a glyph stores it locally", async () => {
      cast<jest.Mock>(loadPairedDesktops).mockResolvedValue([first] as PairedDesktop[]);
      render();
      await flush();
      await act(async () => { await pairing.current.labelDesktop("desktop", { icon: "star" }); });
      expect(labelDesktop).toHaveBeenCalledWith("desktop", { icon: "star" });
    });
  });
});
