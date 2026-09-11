// @vitest-environment jsdom
import { listen } from "@tauri-apps/api/event";
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { UpdatePage } from "./UpdatePage";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("../../integrations/tauri/invoke", () => ({ invokeCommand: vi.fn() }));
vi.mock("../../integrations/tauri/useBuildInfo", () => ({ useBuildInfo: () => ({ data: { version: "1.0" } }) }));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const status = { currentVersion: "1.0", latestVersion: "2.0", hasUpdate: true, platformSupported: true, canInstallInApp: true, downloadUrl: null };

beforeEach(() => vi.clearAllMocks());

it.each([false, true])("handles install under StrictMode including subscription failure=%s", async (failed) => {
  const unlisten = vi.fn();
  if (failed)
    vi.mocked(listen).mockRejectedValueOnce(new Error("subscription unavailable"));
  else
    vi.mocked(listen).mockResolvedValueOnce(unlisten);
  vi.mocked(invokeCommand).mockResolvedValue(undefined);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => root.render(<StrictMode><UpdatePage cachedStatus={status} /></StrictMode>));
    const button = [...container.querySelectorAll("button")].find(item => item.textContent === "Download and install")!;
    expect(button).toBeTruthy();
    await act(async () => button.click());
    if (failed) {
      expect(container.textContent).toContain("subscription unavailable");
      expect(invokeCommand).not.toHaveBeenCalled();
      expect([...container.querySelectorAll("button")].find(item => item.textContent === "Download and install")?.disabled).toBe(false);
    }
    else {
      expect(invokeCommand).toHaveBeenCalledWith("install_app_update");
      expect(unlisten).toHaveBeenCalledTimes(1);
    }
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
