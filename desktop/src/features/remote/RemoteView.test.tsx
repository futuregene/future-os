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
const unpairRemoteDevice = vi.fn<(pairId: string) => Promise<unknown>>();
const takeoverRemote = vi.fn<(pairId: string) => Promise<unknown>>();
const getRemoteStatus = vi.fn<() => Promise<RemoteStatus>>();

vi.mock("./remoteClient", async importOriginal => ({
  ...await importOriginal<typeof import("./remoteClient")>(),
  startRemote: () => startRemote(),
  stopRemote: () => stopRemote(),
  addRemoteDevice: () => addRemoteDevice(),
  unpairRemoteDevice: (pairId: string) => unpairRemoteDevice(pairId),
  takeoverRemote: (pairId: string) => takeoverRemote(pairId),
  // Closing the invitation window re-reads the device list before discarding
  // anything, so the harness has to answer as the backend would.
  getRemoteStatus: () => getRemoteStatus(),
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
  onRefreshRemote?: () => Promise<void>;
  remoteStatus?: RemoteStatus | null;
}

async function mount(overrides: Props = {}) {
  const props = {
    onRefreshRemote: refresh,
    remoteStatus: status(),
    ...overrides,
  };
  getRemoteStatus.mockResolvedValue(props.remoteStatus ?? status());
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

/** Whether this row carries a button with that exact label. */
function hasButton(row: ParentNode, text: string): boolean {
  return [...row.querySelectorAll<HTMLButtonElement>("button")]
    .some(button => (button.textContent ?? "").trim() === text);
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

/** The unpair confirm, if it is up: distinguished by the question it asks. */
function confirmDialog(): HTMLElement | null {
  return [...container.querySelectorAll<HTMLElement>("[role='dialog']")]
    .find(dialog => (dialog.textContent ?? "").includes("Unpair this device?")) ?? null;
}

async function toggleHost(): Promise<void> {
  await act(async () => {
    hostSwitch().click();
  });
  await flush();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
  startRemote.mockReset();
  stopRemote.mockReset();
  addRemoteDevice.mockReset();
  unpairRemoteDevice.mockReset();
  takeoverRemote.mockReset();
  getRemoteStatus.mockReset();
  openExternalUrl.mockReset();
  startWindowDrag.mockReset();
  startRemote.mockResolvedValue(undefined);
  stopRemote.mockResolvedValue(undefined);
  takeoverRemote.mockResolvedValue(undefined);
  // The backend answers with the connection it just created; the window speaks
  // for that one, so the mock has to name it.
  addRemoteDevice.mockResolvedValue({ pairId: "pair_gamma" });
  unpairRemoteDevice.mockResolvedValue(undefined);
  getRemoteStatus.mockResolvedValue(status());
  openExternalUrl.mockResolvedValue(undefined);
  // Freeze the clock before minting the invitation: even a short render can
  // cross a real second boundary and start the countdown at 1:59, not 2:00.
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

  /**
   * The page has to say what this machine is called.
   *
   * The rows below name connections (`pair_…`), not computers, so a user
   * holding a phone that paired with this one had nothing on either screen to
   * match them by — the id the phone recorded was on this machine only, and
   * never rendered.
   */
  it("shows this machine's own id while the switch is on", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", desktopId: "desktop_ABC123", devices: [control] }),
    });

    const id = container.querySelector("[data-testid='local-device-id']");
    expect(id?.textContent?.trim()).toBe("desktop_ABC123");

    const clipboardWrite = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: clipboardWrite } });
    const copy = [...id!.parentElement!.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").trim() === "Copy")!;
    await act(async () => copy.click());
    await flush();

    expect(clipboardWrite).toHaveBeenCalledWith("desktop_ABC123");
    expect(copy.textContent?.trim()).toBe("Copied");
  });

  it("hides the id while the switch is off", async () => {
    await mount({ remoteStatus: status({ pairId: "pair_alpha", phase: "stopped", desktopId: "desktop_ABC123" }) });
    // Off means this machine accepts nothing, so there is no id to hand out.
    expect(container.querySelector("[data-testid='local-device-id']")).toBeNull();
  });
});

describe("off hides everything the switch enables", () => {
  it("renders no list and no add button for a null status", async () => {
    await mount({ remoteStatus: null });
    expect(hostSwitch().getAttribute("aria-checked")).toBe("false");
    expect(buttonByText("Add a device")).toBeUndefined();
    expect(container.textContent).not.toContain("Paired devices");
  });

  it("hides the client list and adding while off", async () => {
    await mount({
      remoteStatus: status({
        pairId: "pair_alpha",
        phase: "stopped",
        devices: [control],
      }),
    });

    // The switch is the whole off state: no client list, no add control, no
    // footnote explaining what a closed switch would do.
    expect(hostSwitch().getAttribute("aria-checked")).toBe("false");
    expect(container.textContent).not.toContain("Pixel 9");
    expect(container.textContent).not.toContain("Paired devices");
    expect(buttonByText("Add a device")).toBeUndefined();
    expect(container.textContent).not.toContain("Download mobile app");
    expect(addDialog()).toBeNull();
  });
});

describe("the list", () => {
  it("carries each connection's actions in its own row", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, standby] }),
    });

    expect(hostSwitch().getAttribute("aria-checked")).toBe("true");
    expect(container.textContent).toContain("Paired devices");
    expect(container.textContent).toContain("Pixel 9 (a phone)");
    expect(container.textContent).toContain("studio-imac (a computer)");
    expect(container.textContent).toContain("In control");
    expect(container.textContent).toContain("Standby");

    const rowOf = (name: string) =>
      [...container.querySelectorAll<HTMLLIElement>("li")]
        .find(row => (row.textContent ?? "").includes(name))!;

    // Unpair is on every row: it is the only way back out of a paired
    // connection, and a standby device is still a connection.
    expect(hasButton(rowOf("Pixel 9"), "Unpair")).toBe(true);
    expect(hasButton(rowOf("studio-imac"), "Unpair")).toBe(true);
    // The standby device can take control; the one already driving this machine
    // has nothing to hand over, and no button that would do nothing.
    expect(hasButton(rowOf("studio-imac"), "Let this control")).toBe(true);
    expect(hasButton(rowOf("Pixel 9"), "Let this control")).toBe(false);
    // The row is a record, not a door: its name is not a control, and the page
    // it used to open left none of its buttons behind as loose controls.
    expect([...container.querySelectorAll("li button")]
      .some(button => (button.textContent ?? "").includes("studio-imac"))).toBe(false);
    expect(buttonByText("Disconnect")).toBeUndefined();
    expect(buttonByText("Remove")).toBeUndefined();
    // The list is the whole body; the switch above it is the only preference,
    // so it is the only switch on the section.
    expect(container.querySelectorAll("button[role='switch']").length).toBe(1);
  });

  it("stops calling a device in control once its client has left", async () => {
    // The bridge keeps serving a pairing whose client was disconnected, so the
    // row has to read the client half: driving a machine that is not there is
    // exactly the lie the row used to tell.
    await mount({
      remoteStatus: status({
        pairId: "pair_alpha",
        phase: "ready",
        devices: [{ ...control, present: false }],
      }),
    });

    expect(container.textContent).toContain("Offline");
    expect(container.textContent).not.toContain("In control");
  });

  it("offers no takeover to a standby device that is not there", async () => {
    await mount({
      remoteStatus: status({
        pairId: "pair_alpha",
        phase: "ready",
        devices: [{ ...standby, present: false }],
      }),
    });

    const row = [...container.querySelectorAll<HTMLLIElement>("li")]
      .find(item => (item.textContent ?? "").includes("studio-imac"))!;
    expect(hasButton(row, "Let this control")).toBe(false);
    // Unpair stays: a device that left is still a pairing to remove.
    expect(hasButton(row, "Unpair")).toBe(true);
  });

  it("hands control to a standby device from its own row", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, standby] }),
    });

    const row = [...container.querySelectorAll<HTMLLIElement>("li")]
      .find(item => (item.textContent ?? "").includes("studio-imac"))!;
    const takeover = [...row.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").trim() === "Let this control")!;
    await act(async () => takeover.click());
    await flush();

    expect(takeoverRemote).toHaveBeenCalledWith("pair_beta");
  });

  it("unpairs a standby device, not just the one in control", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, standby] }),
    });

    const row = [...container.querySelectorAll<HTMLLIElement>("li")]
      .find(item => (item.textContent ?? "").includes("studio-imac"))!;
    const unpair = [...row.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").trim() === "Unpair")!;
    await act(async () => unpair.click());
    await flush();
    // The confirm names the device it would cut off, and only then unpairs it.
    expect(container.textContent).toContain("studio-imac (a computer) will no longer connect to this computer.");
    await act(async () => buttonByText("Delete")!.click());
    await flush();

    expect(unpairRemoteDevice).toHaveBeenCalledWith("pair_beta");
  });

  it("says so when nothing is connected yet, and offers to add one", async () => {
    await mount({ remoteStatus: status({ phase: "enabled" }) });
    expect(container.textContent).toContain("No devices are paired yet.");
    expect(buttonByText("Add a device")).toBeTruthy();
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
    expect(container.textContent).not.toContain("No devices are paired yet.");
  });

  it("draws no connection status line of its own", async () => {
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

    // The rows already say what each connection is doing; a headline above them
    // repeating "connecting" was noise.
    expect(container.textContent).not.toContain("Connecting");
    expect(container.textContent).not.toContain("Connecting, check the computer's network");
    expect(container.textContent).toContain("Offline");
    expect(buttonByText("Add a device")).toBeTruthy();
  });

  it("never lists an invitation nobody has scanned", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });

    // An unclaimed invitation is not a connection: it belongs to the add
    // window, and a row for it read as "a device is waiting".
    expect(container.textContent).not.toContain("Waiting to pair");
    expect(container.textContent).toContain("Pixel 9 (a phone)");
  });
});

describe("adding a connection", () => {
  it("does not open a QR code until a device is asked for", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control] }),
    });

    // Turning the remote desktop on creates no invitation, so there is no
    // window to show and no code in the page.
    expect(addDialog()).toBeNull();
    expect(container.textContent).not.toContain("Pairing code");
    expect(addRemoteDevice).not.toHaveBeenCalled();

    await act(async () => buttonByText("Add a device")!.click());
    await flush();
    expect(addRemoteDevice).toHaveBeenCalledTimes(1);

    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });
    await act(async () => buttonByText("Add a device")!.click());
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
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });
    await act(async () => buttonByText("Add a device")!.click());
    await flush();

    expect(addDialog()!.textContent).toContain("expires in 2:00");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(addDialog()!.textContent).toContain("expires in 1:59");

    const clipboardWrite = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: clipboardWrite } });
    // The dialog's own Copy: this machine's id carries a Copy button too, so
    // "the Copy button" is only unambiguous inside the window.
    const copy = [...addDialog()!.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").trim() === "Copy")!;
    await act(async () => copy.click());
    await flush();
    expect(clipboardWrite).toHaveBeenCalledWith(invited.pairingCode);
    expect(addDialog()!.textContent).toContain("Copied");
  });

  it("closes the window without stopping the remote desktop", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });
    await act(async () => buttonByText("Add a device")!.click());
    await flush();
    expect(addDialog()).not.toBeNull();

    await act(async () => buttonByText("Cancel")!.click());
    await flush();

    expect(addDialog()).toBeNull();
    // Cancel is about this one connection: the desktop stays on, and the
    // invitation that was never scanned goes away instead of lingering.
    expect(stopRemote).not.toHaveBeenCalled();
    expect(unpairRemoteDevice).toHaveBeenCalledWith("pair_gamma");
    expect(container.textContent).not.toContain("Waiting to pair");
  });

  /**
   * Every invitation nobody claimed goes with the window, not only the last one.
   *
   * Each pending invitation occupies a row of the platform's per-desktop cap
   * whether or not its code is on screen. An invitation minted by an earlier
   * press and then displaced (a second press replaced the one this window spoke
   * for, or the window closed before the refresh landed) stayed behind as a row
   * the user could neither see nor use — enough of them filled the cap and made
   * "add a device" fail over devices that did not exist.
   */
  it("discards every unclaimed invitation when the window closes", async () => {
    const second = {
      ...invited,
      pairId: "pair_delta",
      pairingCode: "futureos://remote/pair?code=delta",
    };
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited, second] }),
    });
    // The list the window closed against still knows about both.
    getRemoteStatus.mockResolvedValue(
      status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited, second] }),
    );
    await act(async () => buttonByText("Add a device")!.click());
    await flush();

    await act(async () => buttonByText("Cancel")!.click());
    await flush();

    expect(unpairRemoteDevice).toHaveBeenCalledWith("pair_gamma");
    expect(unpairRemoteDevice).toHaveBeenCalledWith("pair_delta");
    // A device that claimed its code is a connection, not an invitation: it must
    // not be discarded by closing this window.
    expect(unpairRemoteDevice).not.toHaveBeenCalledWith("pair_alpha");
  });

  it("closes itself once the scanned code becomes a device", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });
    await act(async () => buttonByText("Add a device")!.click());
    await flush();
    expect(addDialog()!.textContent).toContain("futureos://remote/pair?code=xyz");

    // The phone scanned it: the backend stops advertising the one-shot code and
    // the device turns into a row. The window carries the invitation, not the
    // connection, so it leaves with the code instead of sitting on the line
    // that promises a QR "when it is ready" — nothing more is coming.
    await mount({
      remoteStatus: status({
        pairId: "pair_alpha",
        phase: "ready",
        devices: [control, {
          ...invited,
          pairingCode: null,
          connected: true,
          client: { kind: "mobile", name: "Pixel 10" },
        }],
      }),
    });

    expect(addDialog()).toBeNull();
    expect(container.textContent).not.toContain("Establishing a secure connection");
    expect(container.textContent).toContain("Pixel 10 (a phone)");
    // Closing the window is not unpairing a device that now exists.
    expect(unpairRemoteDevice).not.toHaveBeenCalled();
  });

  it("says the code lapsed instead of waiting for a QR that never comes", async () => {
    invited.pairingCodeExpiresAt = Math.floor(Date.now() / 1000) + 5;
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });
    await act(async () => buttonByText("Add a device")!.click());
    await flush();
    expect(addDialog()).not.toBeNull();

    // Nobody scanned it. The status drops the code at expiry — the same "no
    // code" a claimed invitation produces — so the window that says which one
    // happened has to be the view, and it says so instead of showing an
    // invitation that will never appear.
    vi.setSystemTime(new Date(Date.now() + 10_000));
    await mount({
      remoteStatus: status({
        pairId: "pair_alpha",
        phase: "ready",
        devices: [control, { ...invited, pairingCode: null }],
      }),
    });

    expect(addDialog()).toBeNull();
    expect(container.textContent).toContain("That pairing code expired. Add the device again.");
    expect(container.textContent).not.toContain("Establishing a secure connection");
  });

  it("says the cap is reached instead of a generic failure", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await mount({
        remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control] }),
      });
      // What `remote_add_device` actually sends: the AppError serializes as a
      // display string, and the command translates the typed cap code into the
      // one token the UI matches on. The old test used a Debug-formatted error,
      // a shape that never crosses the wire — it passed while the real refusal
      // reached the user as "try again later (LC999)".
      addRemoteDevice.mockRejectedValue(new Error("pairing_limit_reached"));
      await act(async () => buttonByText("Add a device")!.click());
      await flush();
      expect(container.textContent).toContain("This computer already has the maximum number of devices. Unpair one first.");
      expect(addDialog()).toBeNull();

      // The refusal's own words name the same state, so a backend that reports
      // the platform's sentence instead of the code still lands on this
      // sentence rather than on the generic one.
      addRemoteDevice.mockRejectedValue(
        new Error("This desktop already holds 5 devices. Unpair one of them before adding another."),
      );
      await act(async () => buttonByText("Add a device")!.click());
      await flush();
      expect(container.textContent).toContain("This computer already has the maximum number of devices. Unpair one first.");
      expect(container.textContent).not.toContain("LC999");
    }
    finally {
      consoleError.mockRestore();
    }
  });

  /// The bridge is started *after* the code is minted, so an add can fail on a
  /// machine whose remote desktop is already on — the broker was not reachable
  /// when the invitation's bridge went up. That is a different next step
  /// (check the network, retry) from "something went wrong", and saying so is
  /// the whole point: the raw message names NATS, which no user can act on.
  it("tells a broker that could not be reached apart from an unknown failure", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await mount({
        remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control] }),
      });
      addRemoteDevice.mockRejectedValue(new Error("Failed to connect to NATS: IO error: Connection refused"));
      await act(async () => buttonByText("Add a device")!.click());
      await flush();
      expect(container.textContent).toContain("Network unavailable. Check your connection and try again later. (NW001)");
      expect(container.textContent).not.toContain("LC999");
      expect(container.textContent).not.toContain("Failed to connect to NATS");
      expect(addDialog()).toBeNull();
    }
    finally {
      consoleError.mockRestore();
    }
  });

  /// The platform can answer with a bare gateway error (`502` from the mint
  /// call, no JSON body). That is the service being briefly unavailable — the
  /// same request succeeds seconds later — so it must not be dressed up as a
  /// fault on this device (`LC999`), which sends the user looking locally.
  it("names a platform gateway error as the service being unavailable", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await mount({
        remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control] }),
      });
      addRemoteDevice.mockRejectedValue(new Error("Failed to create pairing code (HTTP 502)"));
      await act(async () => buttonByText("Add a device")!.click());
      await flush();
      expect(container.textContent).toContain("Service temporarily unavailable. Try again later. (SV001)");
      expect(container.textContent).not.toContain("LC999");
      expect(addDialog()).toBeNull();
    }
    finally {
      consoleError.mockRestore();
    }
  });

  it("asks for a sign-in again when the account is what refused the invitation", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await mount({
        remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control] }),
      });
      addRemoteDevice.mockRejectedValue(new Error("Not signed in to FutureOS."));
      await act(async () => buttonByText("Add a device")!.click());
      await flush();
      expect(container.textContent).toContain("Your FutureOS sign-in has expired. Sign in again.");
      expect(addDialog()).toBeNull();
    }
    finally {
      consoleError.mockRestore();
    }
  });
});

/**
 * What the page does when the backend refuses.
 *
 * Every control here is one `invoke` away from a host that can be gone, busy or
 * mid-restart, and the window that owns the control has to survive that: the
 * page keeps working, the failure is said in words a user can act on, and no
 * internal detail (a NATS URL, a Rust variant) reaches the screen. These
 * assertions are the negative half of the flows above — which is exactly the
 * half a happy-path test cannot reach.
 */
describe("a backend that refuses", () => {
  /** The floating add-client window's own Copy button. */
  function dialogCopy(): HTMLButtonElement {
    return [...addDialog()!.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").trim() === "Copy")!;
  }

  it("says the invitation could not be thrown away, and keeps the page usable", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await mount({
        remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
      });
      await act(async () => buttonByText("Add a device")!.click());
      await flush();
      expect(addDialog()).not.toBeNull();

      // The window is already closed as far as the reader is concerned: it is
      // the *unpair* that fails, after the status read succeeded.
      getRemoteStatus.mockResolvedValue(
        status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
      );
      unpairRemoteDevice.mockRejectedValue(new Error("unpair failed"));
      await act(async () => buttonByText("Cancel")!.click());
      await flush();

      expect(addDialog()).toBeNull();
      expect(consoleError).toHaveBeenCalledWith("remote discard invitation failed:", expect.any(Error));
      expect(container.textContent).toContain("The action could not be completed. Try again later. (LC999)");
      // Still on: throwing away an invitation is not stopping the desktop, and a
      // refused discard is not either.
      expect(hostSwitch().getAttribute("aria-checked")).toBe("true");
      expect(stopRemote).not.toHaveBeenCalled();
      expect(refresh).toHaveBeenCalled();
    }
    finally {
      consoleError.mockRestore();
    }
  });

  it("closes the window even when the device list cannot be re-read", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await mount({
        remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
      });
      await act(async () => buttonByText("Add a device")!.click());
      await flush();

      // Reading the list is what decides *which* invitations to discard; losing
      // that read must not leave the window open over a code nobody can claim.
      getRemoteStatus.mockRejectedValue(new Error("status unavailable"));
      await act(async () => buttonByText("Cancel")!.click());
      await flush();

      expect(consoleError).toHaveBeenCalledWith(
        "remote discard invitation: could not read the device list:",
        expect.any(Error),
      );
      expect(addDialog()).toBeNull();
      // Nothing was discarded on a guess: unpairing is destructive, and the
      // list that would have named the pair ids never arrived.
      expect(unpairRemoteDevice).not.toHaveBeenCalled();
      expect(refresh).toHaveBeenCalled();
    }
    finally {
      consoleError.mockRestore();
    }
  });

  it("reports a refused takeover and leaves the row's device as it was", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await mount({
        remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, standby] }),
      });
      takeoverRemote.mockRejectedValue(new Error("takeover refused"));

      const row = [...container.querySelectorAll<HTMLLIElement>("li")]
        .find(item => (item.textContent ?? "").includes("studio-imac"))!;
      const takeover = [...row.querySelectorAll<HTMLButtonElement>("button")]
        .find(button => (button.textContent ?? "").trim() === "Let this control")!;
      await act(async () => takeover.click());
      await flush();

      expect(consoleError).toHaveBeenCalledWith("remote takeover failed:", expect.any(Error));
      expect(container.textContent).toContain("The action could not be completed. Try again later. (LC999)");
      expect(container.textContent).not.toContain("takeover refused");
      // The page is not stuck in its busy state, so the row can be tried again.
      expect(takeover.disabled).toBe(false);
      expect(refresh).toHaveBeenCalled();
    }
    finally {
      consoleError.mockRestore();
    }
  });

  it("reports a refused unpair and keeps the connection listed", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await mount({
        remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, standby] }),
      });
      const row = [...container.querySelectorAll<HTMLLIElement>("li")]
        .find(item => (item.textContent ?? "").includes("studio-imac"))!;
      const unpair = [...row.querySelectorAll<HTMLButtonElement>("button")]
        .find(button => (button.textContent ?? "").trim() === "Unpair")!;
      await act(async () => unpair.click());
      await flush();

      unpairRemoteDevice.mockRejectedValue(new Error("unpair refused"));
      await act(async () => buttonByText("Delete")!.click());
      await flush();

      expect(consoleError).toHaveBeenCalledWith("remote unpair device failed:", expect.any(Error));
      expect(container.textContent).toContain("The action could not be completed. Try again later. (LC999)");
      // The confirm does not lie about having removed it: the row is still here.
      expect(container.textContent).toContain("studio-imac (a computer)");
      expect(confirmDialog()).toBeNull();
    }
    finally {
      consoleError.mockRestore();
    }
  });

  it("says the pairing code could not be copied instead of claiming it was", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });
    await act(async () => buttonByText("Add a device")!.click());
    await flush();

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn(async () => { throw new Error("clipboard denied"); }) },
    });
    await act(async () => dialogCopy().click());
    await flush();

    expect(container.textContent).toContain("Copy failed — copy the code manually.");
    expect(dialogCopy().textContent?.trim()).toBe("Copy");
  });

  it("says this machine's id could not be copied instead of claiming it was", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", desktopId: "desktop_ABC123", devices: [control] }),
    });
    const idRow = container.querySelector("[data-testid='local-device-id']")!.parentElement!;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn(async () => { throw new Error("clipboard denied"); }) },
    });

    const copy = [...idRow.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").trim() === "Copy")!;
    await act(async () => copy.click());
    await flush();

    expect(container.textContent).toContain("Copy failed — copy the code manually.");
    // The label stays "Copy": the button says what it will do, and the failure is
    // said in the banner rather than by pretending a copy happened.
    expect(copy.isConnected).toBe(true);
    expect(copy.textContent?.trim()).toBe("Copy");
  });

  it("sends the user to sign-in from the banner that explains the refusal", async () => {
    const events: string[] = [];
    const listener = () => events.push("show-onboarding");
    window.addEventListener("futureos:show-onboarding", listener);
    try {
      await mount({ remoteStatus: status({ pairId: "pair_1", phase: "failed", reason: "account_authorization" }) });
      // The banner is the second place the action lives; the switch is the
      // first, and a reader who has already read the sentence uses this one.
      const signIn = buttonByText("Sign in again")!;
      expect(signIn).toBeTruthy();
      await act(async () => signIn.click());
      expect(events).toEqual(["show-onboarding"]);
      // The banner is not a start button in disguise.
      expect(startRemote).not.toHaveBeenCalled();
    }
    finally {
      window.removeEventListener("futureos:show-onboarding", listener);
    }
  });

  it("closes the invitation window from its own backdrop", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    });
    await act(async () => buttonByText("Add a device")!.click());
    await flush();
    getRemoteStatus.mockResolvedValue(
      status({ pairId: "pair_alpha", phase: "ready", devices: [control, invited] }),
    );

    // The dimmed area is a real button with the shared label; picking the one
    // that is not inside the dialog frame is the backdrop itself.
    const backdrop = [...container.querySelectorAll<HTMLButtonElement>("button[aria-label='Close']")]
      .find(button => !button.querySelector("section"))!;
    await act(async () => backdrop.click());
    await flush();

    expect(addDialog()).toBeNull();
    expect(unpairRemoteDevice).toHaveBeenCalledWith("pair_gamma");
    expect(stopRemote).not.toHaveBeenCalled();
  });

  it("backs out of an unpair confirm without cutting the connection", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_alpha", phase: "ready", devices: [control, standby] }),
    });
    const row = [...container.querySelectorAll<HTMLLIElement>("li")]
      .find(item => (item.textContent ?? "").includes("studio-imac"))!;
    const unpair = [...row.querySelectorAll<HTMLButtonElement>("button")]
      .find(button => (button.textContent ?? "").trim() === "Unpair")!;
    await act(async () => unpair.click());
    await flush();
    expect(confirmDialog()).not.toBeNull();

    // Escape is the dialog framework's own dismissal; it goes through this
    // window's `onClose` rather than through the Delete button.
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await flush();

    expect(confirmDialog()).toBeNull();
    expect(unpairRemoteDevice).not.toHaveBeenCalled();
    expect(container.textContent).toContain("studio-imac (a computer)");
  });
});
