import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  addRemoteDevice,
  getRemotePairingStatus,
  getRemoteStatus,
  openUrl,
  startRemote,
  stopRemote,
  takeoverRemote,
  unpairRemote,
  unpairRemoteDevice,
} from "./remoteClient";

const invokeCommand = vi.fn<(command: string, args?: unknown) => Promise<unknown>>();

vi.mock("../../integrations/tauri/invoke", () => ({
  invokeCommand: (command: string, args?: unknown) => invokeCommand(command, args),
}));

beforeEach(() => {
  invokeCommand.mockReset();
  invokeCommand.mockResolvedValue({ phase: "stopped" });
});

describe("remote control commands", () => {
  it("routes each lifecycle action to its own command", async () => {
    await startRemote({});
    await stopRemote();
    await getRemoteStatus();
    await getRemotePairingStatus();
    await unpairRemote();
    expect(invokeCommand.mock.calls.map(([command]) => command)).toEqual([
      "remote_start",
      "remote_stop",
      "remote_status",
      "remote_pairing_status",
      "remote_unpair",
    ]);
  });

  it("returns whatever the backend answered", async () => {
    invokeCommand.mockResolvedValueOnce({ pairId: "pair_1", phase: "ready" });
    await expect(getRemoteStatus()).resolves.toMatchObject({ pairId: "pair_1", phase: "ready" });

    invokeCommand.mockResolvedValueOnce({ pairId: null, paired: false });
    await expect(getRemotePairingStatus()).resolves.toEqual({ pairId: null, paired: false });
  });

  it("passes the start input through as a named field", async () => {
    await startRemote({});
    expect(invokeCommand).toHaveBeenCalledWith("remote_start", { input: {} });
  });

  it("opens a URL through the backend's own handler", async () => {
    await openUrl("https://future-os.cn/#download-mobile");
    expect(invokeCommand).toHaveBeenCalledWith("open_url", { url: "https://future-os.cn/#download-mobile" });
  });

  it("addresses one paired device by its own id", async () => {
    // Pairing a second device must not disturb the first, so both single-device
    // commands carry the pair id rather than a shell-wide "the device".
    await addRemoteDevice();
    await unpairRemoteDevice("pair_2");
    await takeoverRemote("pair_3");
    expect(invokeCommand.mock.calls.slice(-3)).toEqual([
      ["remote_add_device", undefined],
      ["remote_unpair_device", { pairId: "pair_2" }],
      ["remote_takeover", { pairId: "pair_3" }],
    ]);
  });

  it("propagates a backend failure to the caller", async () => {
    invokeCommand.mockRejectedValue(new Error("remote_stop failed: not running"));
    await expect(stopRemote()).rejects.toThrow("remote_stop failed: not running");
  });
});
