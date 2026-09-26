// @vitest-environment jsdom
import type { Mock } from "vitest";
import type { AppSettings } from "../../integrations/storage/appSettings";
import type { RemoteStatus } from "./remoteClient";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RemoteView } from "./RemoteView";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const startRemote = vi.fn<() => Promise<unknown>>();
const stopRemote = vi.fn<() => Promise<unknown>>();
const unpairRemote = vi.fn<() => Promise<unknown>>();

vi.mock("./remoteClient", async importOriginal => ({
  ...await importOriginal<typeof import("./remoteClient")>(),
  startRemote: () => startRemote(),
  stopRemote: () => stopRemote(),
  unpairRemote: () => unpairRemote(),
}));

const openExternalUrl = vi.fn<(url: string) => Promise<void>>();
vi.mock("../../integrations/storage/files", () => ({
  openExternalUrl: (url: string) => openExternalUrl(url),
}));

const startWindowDrag = vi.fn();
vi.mock("../../lib/windowDrag", () => ({
  startWindowDrag: () => startWindowDrag(),
}));

// The titlebar toggle asks the native window for its fullscreen state; the
// Remote view's own behaviour does not depend on it.
vi.mock("../../lib/useIsFullscreen", () => ({
  useIsFullscreen: () => false,
}));

function status(overrides: Partial<RemoteStatus> = {}): RemoteStatus {
  return {
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

let container: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
let refresh: Mock<() => Promise<void>>;

interface Props {
  appSettings?: AppSettings;
  leftPanelExpanded?: boolean;
  onRefreshRemote?: () => Promise<void>;
  remoteStatus?: RemoteStatus | null;
}

async function mount(overrides: Props = {}) {
  const props = {
    appSettings: {} as AppSettings,
    leftPanelExpanded: true,
    onChangeSettings: vi.fn<(patch: Partial<AppSettings>) => void>(),
    onToggleLeftPanel: vi.fn<() => void>(),
    onRefreshRemote: refresh,
    remoteStatus: status(),
    ...overrides,
  };
  await act(async () => {
    root.render(createElement(RemoteView, props));
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
  return [...container.querySelectorAll<HTMLButtonElement>("button")]
    .find(button => (button.textContent ?? "").trim() === text);
}

beforeEach(() => {
  startRemote.mockReset();
  stopRemote.mockReset();
  unpairRemote.mockReset();
  openExternalUrl.mockReset();
  startWindowDrag.mockReset();
  startRemote.mockResolvedValue(undefined);
  stopRemote.mockResolvedValue(undefined);
  unpairRemote.mockResolvedValue(undefined);
  openExternalUrl.mockResolvedValue(undefined);
  refresh = vi.fn<() => Promise<void>>(async () => {});
  (document as unknown as { execCommand: () => boolean }).execCommand = () => true;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe("remoteView pairing lifecycle", () => {
  it("offers pairing when nothing is paired, and reports the preparation state", async () => {
    let release!: () => void;
    startRemote.mockImplementation(() => new Promise<void>((resolve) => {
      release = resolve;
    }));
    await mount();
    expect(container.textContent).toContain("Phone Remote Control");

    await act(async () => {
      buttonByText("Pair & start")?.click();
    });
    expect(startRemote).toHaveBeenCalledTimes(1);
    expect(container.textContent).toContain("Establishing a secure connection. The QR code will appear when it is ready.");
    expect(buttonByText("Pair & start")).toBeUndefined();

    await act(async () => {
      release();
    });
    await flush();
    // The status refresh always runs, even when start succeeded.
    expect(refresh).toHaveBeenCalled();
  });

  it("keeps the internal detail in the console and shows a stable message on failure", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    startRemote.mockRejectedValue(new Error("SPAWN_FAILED: nats missing"));
    try {
      await mount();
      await act(async () => {
        buttonByText("Pair & start")?.click();
      });
      await flush();
      expect(container.textContent).toContain("The action could not be completed. Try again later. (LC999)");
      expect(container.textContent).not.toContain("SPAWN_FAILED");
      expect(consoleError).toHaveBeenCalledWith("remote start failed:", expect.any(Error));
      expect(buttonByText("Pair & start")).toBeTruthy();
    }
    finally {
      consoleError.mockRestore();
    }
  });

  it("shows the pairing code with its countdown and can copy it", async () => {
    vi.useFakeTimers();
    const code = "futureos://remote/pair?code=abc";
    await mount({
      remoteStatus: status({
        pairId: "pair_7f3a",
        pairingCode: code,
        pairingCodeExpiresAt: Math.floor(Date.now() / 1000) + 90,
        phase: "ready",
      }),
    });
    expect(container.textContent).toContain("Pairing code");
    expect(container.textContent).toContain("expires in 1:30");
    expect(container.querySelector("[role='img']")).not.toBeNull();

    // The countdown ticks with the polled clock.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(container.textContent).toContain("expires in 1:29");

    const clipboardWrite = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: clipboardWrite } });
    await act(async () => {
      buttonByText("Copy")?.click();
    });
    await flush();
    expect(clipboardWrite).toHaveBeenCalledWith(code);
    expect(buttonByText("Copied")).toBeTruthy();
  });

  it("clamps the countdown at zero once the code has expired", async () => {
    await mount({
      remoteStatus: status({
        pairId: "pair_1",
        pairingCode: "abc",
        pairingCodeExpiresAt: Math.floor(Date.now() / 1000) - 30,
        phase: "ready",
      }),
    });
    expect(container.textContent).toContain("expires in 0:00");
  });

  it("reports a failed clipboard write and keeps the code on screen", async () => {
    const code = "futureos://remote/pair?code=abc";
    await mount({
      remoteStatus: status({
        pairId: "pair_1",
        pairingCode: code,
        pairingCodeExpiresAt: Math.floor(Date.now() / 1000) + 60,
        phase: "ready",
      }),
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn(async () => Promise.reject(new Error("denied"))) },
    });
    await act(async () => {
      buttonByText("Copy")?.click();
    });
    await flush();
    expect(container.textContent).toContain("Copy failed");
    expect(buttonByText("Copy")).toBeTruthy();
  });

  it("cancels pairing from the code card", async () => {
    await mount({
      remoteStatus: status({
        pairId: "pair_1",
        pairingCode: "abc",
        pairingCodeExpiresAt: Math.floor(Date.now() / 1000) + 60,
        phase: "ready",
      }),
    });
    await act(async () => {
      buttonByText("Cancel")?.click();
    });
    await flush();
    expect(stopRemote).toHaveBeenCalledTimes(1);
    expect(refresh).toHaveBeenCalled();
  });

  it("omits the QR code and the copy action when the code is not a deep link", async () => {
    await mount({
      remoteStatus: status({ pairId: "pair_1", pairingCode: "123456", pairingCodeExpiresAt: null, phase: "ready" }),
    });
    // Still shows the code card and the scan hint, just no QR payload.
    expect(container.textContent).toContain("Scan with FutureOS Mobile");
    expect(container.querySelector("[role='img']")).toBeNull();
    expect(container.textContent).not.toContain("expires in");

    const clipboardWrite = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: clipboardWrite } });
    await act(async () => {
      buttonByText("Copy")?.click();
    });
    await flush();
    // Nothing to copy: the button is inert rather than copying a half payload.
    expect(clipboardWrite).not.toHaveBeenCalled();
    expect(buttonByText("Copy")).toBeTruthy();
  });
});

describe("remoteView connection actions", () => {
  it("starts a stopped but paired device and can disconnect a running one", async () => {
    await mount({ remoteStatus: status({ pairId: "pair_abcd", phase: "stopped" }) });
    expect(container.textContent).toContain("7F3A".replace("7F3A", "ABCD"));
    expect(buttonByText("Connect")).toBeTruthy();

    await act(async () => {
      buttonByText("Connect")?.click();
    });
    await flush();
    expect(startRemote).toHaveBeenCalledTimes(1);

    act(() => root.unmount());
    root = createRoot(container);
    await mount({ remoteStatus: status({ pairId: "pair_abcd", phase: "ready" }) });
    expect(buttonByText("Disconnect")).toBeTruthy();
    await act(async () => {
      buttonByText("Disconnect")?.click();
    });
    await flush();
    expect(stopRemote).toHaveBeenCalledTimes(1);
  });

  it("offers Pair again for a revoked credential and Reconnect while recovering", async () => {
    await mount({ remoteStatus: status({ pairId: "pair_1", phase: "revoked" }) });
    expect(buttonByText("Pair again")).toBeTruthy();
    expect(container.textContent).toContain("Pairing expired");

    act(() => root.unmount());
    root = createRoot(container);
    await mount({
      remoteStatus: status({ pairId: "pair_1", phase: "reconnecting", reason: "network", recovery: { attempt: 1, maxAttempts: 3, nextRetryAt: null, since: 0 } }),
    });
    // A non-terminal state keeps the "Connecting" badge and offers the action
    // that answers the reason (here: check the network).
    expect(container.textContent).toContain("Connecting");
    expect(buttonByText("Reconnect")).toBeTruthy();
  });

  it("sends the user back to sign-in when the account needs authorization", async () => {
    const events: string[] = [];
    const listener = () => events.push("show-onboarding");
    window.addEventListener("futureos:show-onboarding", listener);
    try {
      await mount({ remoteStatus: status({ pairId: "pair_1", phase: "failed", reason: "account_authorization" }) });
      expect(buttonByText("Sign in again")).toBeTruthy();
      await act(async () => {
        buttonByText("Sign in again")?.click();
      });
      expect(events).toEqual(["show-onboarding"]);
      expect(startRemote).not.toHaveBeenCalled();
    }
    finally {
      window.removeEventListener("futureos:show-onboarding", listener);
    }
  });

  it("names each terminal failure with its support code", async () => {
    const cases: Array<[RemoteStatus["reason"], string, string]> = [
      ["network", "Network unavailable. Check your connection and try again later. (NW001)", "Reconnect"],
      ["credential_revoked", "Pairing expired. Pair this device again. (PA001)", "Pair again"],
      ["credential_expired", "Service temporarily unavailable. Try again later. (AU002)", "Reconnect"],
      ["service_authorization", "Service temporarily unavailable. Contact support. (AU001)", "Reconnect"],
      ["protocol", "Service temporarily unavailable. Contact support. (PT001)", "Reconnect"],
      ["generation_unhealthy", "Service temporarily unavailable. Try again later. (RT001)", "Reconnect"],
      ["remote_server", "Service temporarily unavailable. Try again later. (SV001)", "Reconnect"],
      ["local", "This device encountered a problem. Try again later. (LC001)", "Reconnect"],
      ["system_sleep", "Network unavailable. Check your connection and try again later. (PW001)", "Reconnect"],
    ];
    for (const [reason, message, action] of cases) {
      await mount({ remoteStatus: status({ pairId: "pair_1", phase: "failed", reason }) });
      expect(container.textContent, String(reason)).toContain(message);
      expect(buttonByText(action), String(reason)).toBeTruthy();
      act(() => root.unmount());
      root = createRoot(container);
    }
  });

  it("asks the user to reconnect while the desktop is still preparing", async () => {
    await mount({ remoteStatus: status({ agentAvailable: false, pairId: "pair_1", phase: "ready" }) });
    // The device-preparing state is a connecting state for this surface: the
    // badge stays "Connecting" and the action is a reconnect/restart.
    expect(container.textContent).toContain("Connecting");
    expect(buttonByText("Connect")).toBeTruthy();
    expect(buttonByText("Disconnect")).toBeUndefined();
  });

  it("reports a failing disconnect", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    stopRemote.mockRejectedValue(new Error("stop failed"));
    try {
      await mount({ remoteStatus: status({ pairId: "pair_1", phase: "ready" }) });
      await act(async () => {
        buttonByText("Disconnect")?.click();
      });
      await flush();
      expect(container.textContent).toContain("The action could not be completed.");
      expect(consoleError).toHaveBeenCalledWith("remote stop failed:", expect.any(Error));
    }
    finally {
      consoleError.mockRestore();
    }
  });
});

describe("remoteView unpair", () => {
  it("confirms before unpairing, and reports a failure", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      await mount({ remoteStatus: status({ pairId: "pair_bee", phase: "stopped" }) });
      await act(async () => {
        buttonByText("Unpair")?.click();
      });
      expect(container.textContent).toContain("Unpair this device?");
      expect(container.textContent).toContain("Paired as BEE");

      // Cancelling leaves the pairing alone.
      const cancel = [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find(button => (button.textContent ?? "").trim() === "Cancel");
      await act(async () => {
        cancel?.click();
      });
      expect(container.textContent).not.toContain("Unpair this device?");
      expect(unpairRemote).not.toHaveBeenCalled();

      await act(async () => {
        buttonByText("Unpair")?.click();
      });
      expect(container.textContent).toContain("Unpair this device?");

      unpairRemote.mockRejectedValue(new Error("unpair failed"));
      const confirm = [...container.querySelectorAll<HTMLButtonElement>("button")]
        .find(button => (button.textContent ?? "").trim() === "Delete");
      await act(async () => {
        confirm?.click();
      });
      await flush();
      expect(unpairRemote).toHaveBeenCalledTimes(1);
      expect(consoleError).toHaveBeenCalledWith("remote unpair failed:", expect.any(Error));
      // The dialog closes and the generic message takes over.
      expect(container.textContent).not.toContain("Unpair this device?");
    }
    finally {
      consoleError.mockRestore();
    }
  });

  it("leaves an unnamed pair id as it is", async () => {
    await mount({ remoteStatus: status({ pairId: "custom-id", phase: "stopped" }) });
    expect(container.textContent).toContain("CUSTOM-ID");
  });
});

describe("remoteView chrome", () => {
  it("links to the mobile download and drags the window from the header", async () => {
    await mount();
    await act(async () => {
      buttonByText("Download mobile app")?.click();
    });
    await flush();
    expect(openExternalUrl).toHaveBeenCalledWith("https://future-os.cn/#download-mobile");

    await act(async () => {
      container.querySelector("header")?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(startWindowDrag).toHaveBeenCalledTimes(1);
  });

  it("warns about a failed web listener without claiming the link is down", async () => {
    await mount({ remoteStatus: status({ pairId: "pair_1", phase: "ready", warningCode: "web_bind" }) });
    expect(container.textContent).toContain("Some features are temporarily unavailable. Try again later. (LC002)");
  });

  it("toggles the left panel when it is collapsed", async () => {
    const props = await mount({ leftPanelExpanded: false });
    const toggle = container.querySelector<HTMLButtonElement>("header button")!;
    await act(async () => {
      toggle.click();
    });
    expect(props.onToggleLeftPanel).toHaveBeenCalled();

    // With the left panel expanded the affordance is hidden entirely.
    act(() => root.unmount());
    root = createRoot(container);
    await mount({ leftPanelExpanded: true });
    expect(container.querySelector("header button")).toBeNull();
  });

  it("renders nothing pairing-specific for a null status", async () => {
    await mount({ remoteStatus: null });
    expect(container.textContent).toContain("Phone Remote Control");
    expect(buttonByText("Pair & start")).toBeTruthy();
    expect(buttonByText("Unpair")).toBeUndefined();
  });
});
