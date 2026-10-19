// @vitest-environment jsdom
import type { Mock } from "vitest";
import type { RemoteStatus } from "./remoteClient";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RemoteHostSection } from "./RemoteView";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const startRemote = vi.fn<() => Promise<unknown>>();
const stopRemote = vi.fn<() => Promise<unknown>>();
const addRemoteDevice = vi.fn<() => Promise<unknown>>();

vi.mock("./remoteClient", async importOriginal => ({
  ...await importOriginal<typeof import("./remoteClient")>(),
  startRemote: () => startRemote(),
  stopRemote: () => stopRemote(),
  addRemoteDevice: () => addRemoteDevice(),
}));

const openExternalUrl = vi.fn<(url: string) => Promise<void>>();
vi.mock("../../integrations/storage/files", () => ({
  openExternalUrl: (url: string) => openExternalUrl(url),
}));

const startWindowDrag = vi.fn();
vi.mock("../../lib/windowDrag", () => ({
  startWindowDrag: () => startWindowDrag(),
}));

function status(overrides: Partial<RemoteStatus> = {}): RemoteStatus {
  return {
    client: null,
    agentAvailable: true,
    desktopId: "desk_1",
    desktopPublicKey: "pk",
    natsUrl: "nats://example",
    pairId: "",
    pairingCode: null,
    pairingCodeExpiresAt: null,
    phase: "stopped",
    reason: null,
    recovery: null,
    warningCode: null,
    webLanUrl: null,
    webUrl: null,
    ...overrides,
  };
}

const control = {
  pairId: "pair_alpha",
  phase: "ready" as const,
  control: "active" as const,
  connected: true,
  client: { kind: "mobile", name: "Pixel 9" },
  pairingCode: null,
  pairingCodeExpiresAt: null,
  desktopId: "desk_1",
};
const standby = {
  pairId: "pair_beta",
  phase: "ready" as const,
  control: "standby" as const,
  connected: true,
  client: { kind: "desktop", name: "studio-imac" },
  pairingCode: null,
  pairingCodeExpiresAt: null,
  desktopId: "desk_1",
};
const invited = {
  pairId: "pair_gamma",
  phase: "ready" as const,
  control: "standby" as const,
  connected: false,
  client: null,
  pairingCode: "futureos://remote/pair?code=xyz",
  pairingCodeExpiresAt: Math.floor(Date.now() / 1000) + 120,
  desktopId: "desk_1",
};

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let refresh: Mock<() => Promise<void>>;

interface Props {
  onOpenDevice?: (pairId: string) => void;
  onOpenSettings?: () => void;
  onRefreshRemote?: () => Promise<void>;
  remoteStatus?: RemoteStatus | null;
}

async function mount(overrides: Props = {}) {
  const props = {
    onOpenDevice: vi.fn<(pairId: string) => void>(),
    onOpenSettings: vi.fn<() => void>(),
    onRefreshRemote: refresh,
    remoteStatus: status(),
    ...overrides,
  };
  await act(async () => {
    root.render(createElement(RemoteHostSection, props));
  });
  await flush();
  return props;
}

async function flush(times = 3) {
  await act(async () => {
    for (let index = 0; index < times; index += 1)
      await Promise.resolve();
  });
}

function buttonByText(text: string): HTMLButtonElement | undefined {
  // Dialog buttons live under the overlay; `querySelectorAll` on the container
  // still finds them because React renders in place, not in a portal.
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.textContent ?? "").trim() === text);
}

/** The on/off switch: the only `role=switch` this section owns. */
function hostSwitch(): HTMLButtonElement {
  return container.querySelectorAll<HTMLButtonElement>("button[role='switch']")[0]!;
}

/** The floating add-client window, if it is open. */
function addDialog(): HTMLElement | null {
  return container.querySelector("[role='dialog']");
}

async function toggleHost(): Promise<void> {
  await act(async () => {
    hostSwitch().click();
  });
  await flush();
}

beforeEach(() => {
  startRemote.mockReset();
  stopRemote.mockReset();
  addRemoteDevice.mockReset();
  openExternalUrl.mockReset();
  startWindowDrag.mockReset();
  startRemote.mockResolvedValue(undefined);
  stopRemote.mockResolvedValue(undefined);
  addRemoteDevice.mockResolvedValue(undefined);
  openExternalUrl.mockResolvedValue(undefined);
  // The invitation is minted for "two minutes from now", so its expiry has to
  // be anchored to the test run, not to module import: a slow full-suite load
  // can put more than a second between the two, and the countdown then starts
  // at 1:59 instead of 2:00.
  invited.pairingCodeExpiresAt = Math.floor(Date.now() / 1000) + 120;
  refresh = vi.fn<() => Promise<void>>(async () => {});
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe("the switch", () => {
  it("starts a stopped but paired machine and stops a running one", async () => {
    await mount({ remoteStatus: status({ pairId: "pair_abcd", phase: "stopped" }) });
    expect(hostSwitch().getAttribute("aria-checked")).toBe("false");

    await toggleHost();
    expect(startRemote).toHaveBeenCalledTimes(1);

    act(() => root.unmount());
    root = createRoot(container);
    await mount({ remoteStatus: status({ pairId: "pair_abcd", phase: "ready" }) });
    expect(hostSwitch().getAttribute("aria-checked")).toBe("true");

    await toggleHost();
    expect(stopRemote).toHaveBeenCalledTimes(1);
  });

  it("says the bridge is starting, and does not claim to be on before it is", async () => {
    let release!: () => void;
    startRemote.mockImplementation(() => new Promise<void>((resolve) => {
      release = resolve;
    }));
    await mount();

    await act(async () => {
      hostSwitch().click();
    });
    expect(startRemote).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("Establishing a secure connection. The QR code will appear when it is ready.");
    expect(hostSwitch().getAttribute("aria-checked")).toBe("false");

    await act(async () => release());
    await flush();
    expect(refresh).toHaveBeenCalled();
  });

  it("keeps internal detail out of the UI when starting fails", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    startRemote.mockRejectedValue(new Error("SPAWN_FAILED: nats missing"));
    try {
      await mount();
      await act(async () => hostSwitch().click());
      await flush();
      expect(container.textContent).toContain("The action could not be completed. Try again later. (LC999)");
      expect(container.textContent).not.toContain("SPAWN_FAILED");
      expect(consoleError).toHaveBeenCalledWith("remote start failed:", expect.any(Error));
      expect(hostSwitch().getAttribute("aria-checked")).toBe("false");
    }
    finally {
      consoleError.mockRestore();
    }
  });

  it("reports a failing stop", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    stopRemote.mockRejectedValue(new Error("stop failed"));
    try {
      await mount({ remoteStatus: status({ pairId: "pair_1", phase: "ready" }) });
      await toggleHost();
      expect(container.textContent).toContain("The action could not be completed.");
      expect(consoleError).toHaveBeenCalledWith("remote stop failed:", expect.any(Error));
    }
    finally {
      consoleError.mockRestore();
    }
  });

  it("sends the user to sign-in instead of starting a bridge the platform will refuse", async () => {
    const events: string[] = [];
    const listener = () => events.push("show-onboarding");
    window.addEventListener("futureos:show-onboarding", listener);
    try {
      await mount({ remoteStatus: status({ pairId: "pair_1", phase: "failed", reason: "account_authorization" }) });
      expect(container.textContent).toContain("Your FutureOS sign-in has expired. Sign in again.");

      await toggleHost();
      expect(events).toEqual(["show-onboarding"]);
      expect(startRemote).not.toHaveBeenCalled();
    }
    finally {
      window.removeEventListener("futureos:show-onboarding", listener);
    }
  });

  it("names each terminal failure with its support code, and stays off", async () => {
    const cases: Array<[RemoteStatus["reason"], string]> = [
      ["network", "Network unavailable. Check your connection and try again later. (NW001)"],
      ["credential_revoked", "Pairing expired. Pair this device again. (PA001)"],
      ["credential_expired", "Service temporarily unavailable. Try again later. (AU002)"],
      ["service_authorization", "Service temporarily unavailable. Contact support. (AU001)"],
      ["protocol", "Service temporarily unavailable. Contact support. (PT001)"],
      ["generation_unhealthy", "Service temporarily unavailable. Try again later. (RT001)"],
      ["remote_server", "Service temporarily unavailable. Try again later. (SV001)"],
      ["local", "This device encountered a problem. Try again later. (LC001)"],
      ["system_sleep", "Network unavailable. Check your connection and try again later. (PW001)"],
    ];
    for (const [reason, message] of cases) {
      await mount({ remoteStatus: status({ pairId: "pair_1", phase: "failed", reason }) });
      expect(container.textContent, String(reason)).toContain(message);
      expect(hostSwitch().getAttribute("aria-checked"), String(reason)).toBe("false");
      act(() => root.unmount());
      root = createRoot(container);
    }
  });

  it("warns about a failed web listener without claiming the link is down", async () => {
    await mount({ remoteStatus: status({ pairId: "pair_1", phase: "ready", warningCode: "web_bind" }) });
    expect(container.textContent).toContain("Some features are temporarily unavailable. Try again later. (LC002)");
  });
});

describe("off hides everything the switch enables", () => {
  it("renders no list, no add button and no settings for a null status", async () => {
    await mount({ remoteStatus: null });
    expect(hostSwitch().getAttribute("aria-checked")).toBe("false");
    expect(buttonByText("Add")).toBeUndefined();
    expect(buttonByText("Settings")).toBeUndefined();
    expect(container.textContent).not.toContain("Paired clients");
  });

  it("hides the client list, adding and settings while off", async () => {
    await mount({
      remoteStatus: status({
        pairId: "pair_alpha",
        phase: "stopped",
        devices: [control],
      }),
    });

    // The switch is the whole off state: no client list, no add control, no
    // settings control, no footnote explaining what a closed switch would do.
    expect(hostSwitch().getAttribute("aria-checked")).toBe("false");
    expect(container.textContent).not.toContain("Pixel 9");
    expect(container.textContent).not.toContain("Paired clients");
    expect(buttonByText("Add")).toBeUndefined();
    expect(buttonByText("Settings")).toBeUndefined();
    expect(container.textContent).not.toContain("Download mobile app");
    expect(addDialog()).toBeNull();
  });
});

describe("the list", () => {
  it("lists the paired clients and nothing else", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, standby] }),
    });

    expect(hostSwitch().getAttribute("aria-checked")).toBe("true");
    expect(container.textContent).toContain("Paired clients");
    expect(container.textContent).toContain("Pixel 9 (a phone)");
    expect(container.textContent).toContain("studio-imac (a computer)");
    expect(container.textContent).toContain("In control");
    expect(container.textContent).toContain("Standby");

    // A list, not a control panel: disconnect/delete live on a client's page.
    expect(buttonByText("Disconnect")).toBeUndefined();
    expect(buttonByText("Remove")).toBeUndefined();
    expect(buttonByText("Unpair")).toBeUndefined();
    // The list is the whole body; the settings live behind their own control.
    expect(container.textContent).not.toContain("Auto-connect on launch");
  });

  it("opens a client's page when its row is clicked", async () => {
    const props = await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, standby] }),
    });

    const row = [...container.querySelectorAll<HTMLButtonElement>("li button")]
      .find(button => (button.textContent ?? "").includes("studio-imac"))!;
    await act(async () => row.click());

    expect(props.onOpenDevice).toHaveBeenCalledWith("pair_beta");
  });

  it("opens this direction's settings from its own control", async () => {
    const props = await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control] }),
    });

    await act(async () => buttonByText("Settings")!.click());
    expect(props.onOpenSettings).toHaveBeenCalledTimes(1);
  });

  it("says so when nothing is paired yet", async () => {
    await mount({ remoteStatus: status({ phase: "ready" }) });
    expect(container.textContent).toContain("No clients are paired yet.");
    expect(buttonByText("Add")).toBeTruthy();
  });

  it("still lists the one paired client an older backend describes", async () => {
    // The per-device rows arrived later; the top-level fields still describe the
    // one device, and showing an empty list would claim it was unpaired.
    await mount({
      remoteStatus: status({
        pairId: "custom-id",
        phase: "ready",
        client: { kind: "mobile", name: "Pixel 8" },
      }),
    });
    expect(container.textContent).toContain("Pixel 8 (a phone)");
    expect(container.textContent).toContain("CUSTOM-ID");
    expect(container.textContent).not.toContain("No clients are paired yet.");
  });

  it("keeps the connection hint above the list while reconnecting", async () => {
    await mount({
      remoteStatus: status({
        pairId: "pair_alpha",
        phase: "reconnecting",
        reason: "network",
        devices: [
          { ...control, connected: false, control: "standby" },
          { ...standby, connected: false },
        ],
      }),
    });

    expect(container.textContent).toContain("Connecting");
    expect(container.textContent).toContain("Offline");
    expect(container.textContent).not.toContain("In control");
    expect(buttonByText("Add")).toBeTruthy();
  });
});

describe("adding a client", () => {
  it("opens a floating window with the QR code and the pairing code", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control] }),
    });
    expect(addDialog()).toBeNull();

    // The invitation only exists after the backend mints it; the refresh the
    // section performs is what brings it into the status, so mount with it.
    await act(async () => buttonByText("Add")!.click());
    await flush();
    expect(addRemoteDevice).toHaveBeenCalledTimes(1);

    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });
    await act(async () => buttonByText("Add")!.click());
    await flush();

    const dialog = addDialog()!;
    expect(dialog).not.toBeNull();
    expect(dialog.querySelector("[role='img']")).not.toBeNull();
    // The code itself, not only the QR: the other computer needs to type it.
    expect(dialog.textContent).toContain("futureos://remote/pair?code=xyz");
    expect(dialog.textContent).toContain("Pairing code");
    expect(dialog.textContent).toContain("expires in 2:00");
  });

  it("counts the invitation down and can copy the code", async () => {
    vi.useFakeTimers();
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });
    await act(async () => buttonByText("Add")!.click());
    await flush();

    expect(addDialog()!.textContent).toContain("expires in 2:00");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(addDialog()!.textContent).toContain("expires in 1:59");

    const clipboardWrite = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: clipboardWrite } });
    await act(async () => buttonByText("Copy")!.click());
    await flush();
    expect(clipboardWrite).toHaveBeenCalledWith(invited.pairingCode);
    expect(addDialog()!.textContent).toContain("Copied");
  });

  it("closes the window but keeps the waiting client in the list", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });
    await act(async () => buttonByText("Add")!.click());
    await flush();
    expect(addDialog()).not.toBeNull();

    await act(async () => buttonByText("Done")!.click());
    expect(addDialog()).toBeNull();
    // The unclaimed invitation is still a row: it can be reopened or removed.
    expect(container.textContent).toContain("Waiting to pair");
  });

  it("says the cap is reached instead of a generic failure", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await mount({
        remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control] }),
      });
      addRemoteDevice.mockRejectedValue(new Error("Remote { code: Some(\"pairing_limit_reached\") }"));
      await act(async () => buttonByText("Add")!.click());
      await flush();
      expect(container.textContent).toContain("This computer already has the maximum number of devices. Unpair one first.");
      expect(addDialog()).toBeNull();
    }
    finally {
      consoleError.mockRestore();
    }
  });
});
