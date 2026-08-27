// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useAgentDoneBell } from "./useAgentDoneBell";

const playDoneBell = vi.fn();
vi.mock("../../../lib/doneBell", () => ({
  playDoneBell: (...args: unknown[]) => playDoneBell(...args),
}));

const requestUserAttention = vi.fn();
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ requestUserAttention }),
  UserAttentionType: { Critical: 1, Informational: 2 },
}));

function emitAgentEnd() {
  window.dispatchEvent(new CustomEvent("futureos:agent_end", { detail: undefined }));
}

describe("useAgentDoneBell", () => {
  beforeEach(() => {
    playDoneBell.mockReset();
    requestUserAttention.mockReset();
  });

  it("bells and requests attention when a run finishes", () => {
    const { unmount } = renderHook(() => useAgentDoneBell(true));
    act(() => {
      emitAgentEnd();
    });
    expect(playDoneBell).toHaveBeenCalledTimes(1);
    // UserAttentionType.Critical compiles to 1.
    expect(requestUserAttention).toHaveBeenCalledWith(1);
    unmount();
  });

  it("stays silent when bellOnComplete is off", () => {
    const { unmount } = renderHook(() => useAgentDoneBell(false));
    act(() => {
      emitAgentEnd();
    });
    expect(playDoneBell).not.toHaveBeenCalled();
    expect(requestUserAttention).not.toHaveBeenCalled();
    unmount();
  });

  it("stops listening after unmount", () => {
    const { unmount } = renderHook(() => useAgentDoneBell(true));
    unmount();
    act(() => {
      emitAgentEnd();
    });
    expect(playDoneBell).not.toHaveBeenCalled();
  });
});
