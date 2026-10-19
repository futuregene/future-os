// @vitest-environment jsdom
import type { RemoteStatus } from "./remoteClient";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RemoteHostClientPage } from "./RemoteHostClientPage";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const disconnectRemoteDevice = vi.fn<(pairId: string) => Promise<unknown>>();
const unpairRemoteDevice = vi.fn<(pairId: string) => Promise<unknown>>();
const takeoverRemote = vi.fn<(pairId: string) => Promise<unknown>>();

vi.mock("./remoteClient", async importOriginal => ({
  ...await importOriginal<typeof import("./remoteClient")>(),
  disconnectRemoteDevice: (pairId: string) => disconnectRemoteDevice(pairId),
  unpairRemoteDevice: (pairId: string) => unpairRemoteDevice(pairId),
  takeoverRemote: (pairId: string) => takeoverRemote(pairId),
}));

function status(devices: RemoteStatus["devices"]): RemoteStatus {
  return {
    client: null,
    agentAvailable: true,
    desktopId: "desk_1",
    desktopPublicKey: "pk",
    natsUrl: "nats://example",
    pairId: devices?.[0]?.pairId ?? "",
    pairingCode: null,
    pairingCodeExpiresAt: null,
    phase: "ready",
    reason: null,
    recovery: null,
    warningCode: null,
    webLanUrl: null,
    webUrl: null,
    devices,
  };
}

const phone = {
  pairId: "pair_alpha",
  phase: "ready" as const,
  control: "active" as const,
  connected: true,
  client: { kind: "mobile", name: "Pixel 9" },
  pairingCode: null,
  pairingCodeExpiresAt: null,
  desktopId: "desk_1",
};
const imac = {
  pairId: "pair_beta",
  phase: "ready" as const,
  control: "standby" as const,
  connected: true,
  client: { kind: "desktop", name: "studio-imac" },
  pairingCode: null,
  pairingCodeExpiresAt: null,
  desktopId: "desk_1",
};

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let refresh: ReturnType<typeof vi.fn<() => Promise<void>>>;
let onBack: ReturnType<typeof vi.fn<() => void>>;

async function mount(remoteStatus: RemoteStatus | null, pairId = "pair_alpha") {
  await act(async () => {
    root.render(createElement(RemoteHostClientPage, {
      onBack,
      onRefreshRemote: refresh,
      pairId,
      remoteStatus,
    }));
  });
  await flush();
}

async function flush(times = 3) {
  await act(async () => {
    for (let index = 0; index < times; index += 1)
      await Promise.resolve();
  });
}

function buttonByText(text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.textContent ?? "").trim() === text);
}

beforeEach(() => {
  disconnectRemoteDevice.mockReset().mockResolvedValue(undefined);
  unpairRemoteDevice.mockReset().mockResolvedValue(undefined);
  takeoverRemote.mockReset().mockResolvedValue(undefined);
  refresh = vi.fn<() => Promise<void>>(async () => {});
  onBack = vi.fn<() => void>();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe("one client's page", () => {
  it("says who the client is, without inventing a name it never gave", async () => {
    await mount(status([phone]));
    expect(container.textContent).toContain("Pixel 9 (a phone)");
    expect(container.textContent).toContain("ALPHA");
    expect(container.textContent).toContain("In control");

    act(() => root.unmount());
    root = createRoot(container);
    await mount(status([{ ...phone, client: null }]), "pair_alpha");
    expect(container.textContent).toContain("A connected device");
  });

  it("disconnects a connected client but keeps it paired", async () => {
    await mount(status([phone]));
    await act(async () => buttonByText("Disconnect")!.click());
    await flush();

    expect(disconnectRemoteDevice).toHaveBeenCalledWith("pair_alpha");
    expect(unpairRemoteDevice).not.toHaveBeenCalled();
    expect(refresh).toHaveBeenCalled();
  });

  it("does not offer disconnect for a client that has no live bridge", async () => {
    await mount(status([{ ...phone, connected: false, phase: "stopped", control: "standby" }]));
    expect(buttonByText("Disconnect")).toBeUndefined();
    expect(container.textContent).toContain("Offline");
  });

  it("removes the client only after the confirmation, then goes back", async () => {
    await mount(status([phone, imac]));
    await act(async () => buttonByText("Remove")!.click());
    expect(container.textContent).toContain("Remove this client?");
    expect(container.textContent).toContain("Pixel 9 (a phone) will be unpaired");

    // Cancelling leaves the pairing alone.
    const cancel = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").trim() === "Cancel");
    await act(async () => cancel!.click());
    expect(unpairRemoteDevice).not.toHaveBeenCalled();

    await act(async () => buttonByText("Remove")!.click());
    // The shared confirm dialog's destructive button is "Delete".
    await act(async () => buttonByText("Delete")!.click());
    await flush();

    expect(unpairRemoteDevice).toHaveBeenCalledWith("pair_alpha");
    expect(disconnectRemoteDevice).not.toHaveBeenCalled();
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("hands control over from a standby client", async () => {
    await mount(status([phone, imac]), "pair_beta");
    await act(async () => buttonByText("Let this control")!.click());
    await flush();
    expect(takeoverRemote).toHaveBeenCalledWith("pair_beta");
  });

  it("reports an action that failed instead of pretending it worked", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    disconnectRemoteDevice.mockRejectedValue(new Error("device_offline"));
    try {
      await mount(status([phone]));
      await act(async () => buttonByText("Disconnect")!.click());
      await flush();
      expect(container.textContent).toContain("The action could not be completed.");
      expect(consoleError).toHaveBeenCalledWith("remote disconnect device failed:", expect.any(Error));
    }
    finally {
      consoleError.mockRestore();
    }
  });

  it("says the client is gone when it disappeared from the list", async () => {
    await mount(status([phone]), "pair_missing");
    expect(container.textContent).toContain("This client is no longer in the list.");
    expect(buttonByText("Disconnect")).toBeUndefined();
  });
});
