// @vitest-environment jsdom
/**
 * The local half of the done-bell: this window's own `agent_end`.
 *
 * In its own file because the future-event bus is process-wide — the hook
 * subscribes to it, and every hook a sibling test left mounted would ring
 * alongside this one, so an absolute call count is only meaningful when this is
 * the only subscriber alive.
 */
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useRemoteDoneBell } from "./useRemoteDoneBell";

const playDoneBell = vi.fn();
vi.mock("../../../lib/doneBell", () => ({
  playDoneBell: () => playDoneBell(),
}));

const requestUserAttention = vi.fn();
vi.mock("../../../lib/windowAttention", () => ({
  requestUserAttention: () => requestUserAttention(),
}));

// The hook also subscribes to the host's push channel through the Tauri event
// bridge; without this stub the real `listen` rejects (there is no Tauri host
// in jsdom) and the rejection surfaces as an unhandled error in the run.
vi.mock("@tauri-apps/api/window");
vi.mock("@tauri-apps/api/event", () => ({
  listen: async () => () => {},
}));

beforeEach(() => {
  playDoneBell.mockReset();
  requestUserAttention.mockReset();
});

describe("useRemoteDoneBell (local agent_end)", () => {
  it("rings the bell and asks for attention when this window's run ends", async () => {
    const hook = renderHook(() => useRemoteDoneBell(true));
    await flushAsync();

    act(() => emitFutureEvent("agent_end", undefined));

    expect(playDoneBell).toHaveBeenCalledTimes(1);
    expect(requestUserAttention).toHaveBeenCalledTimes(1);
    hook.unmount();
  });

  it("stays silent while the bell setting is off", async () => {
    const hook = renderHook(() => useRemoteDoneBell(false));
    await flushAsync();

    act(() => emitFutureEvent("agent_end", undefined));

    expect(playDoneBell).not.toHaveBeenCalled();
    expect(requestUserAttention).not.toHaveBeenCalled();
    hook.unmount();
  });

  it("stops ringing once the hook is unmounted", async () => {
    const hook = renderHook(() => useRemoteDoneBell(true));
    await flushAsync();
    hook.unmount();

    act(() => emitFutureEvent("agent_end", undefined));

    expect(playDoneBell).not.toHaveBeenCalled();
  });
});
