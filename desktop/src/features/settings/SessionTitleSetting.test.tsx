// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { setLanguage } from "../../i18n";
import { sessionTitleSettings } from "../../integrations/agent/sessionTitleSettings";
import { SessionTitleSetting } from "./SessionTitleSetting";

vi.mock("../../integrations/agent/sessionTitleSettings", () => ({ sessionTitleSettings: vi.fn() }));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  vi.resetAllMocks();
  setLanguage("en");
});

it("defaults off and saves the interface language when enabled", async () => {
  let enabled = false;
  vi.mocked(sessionTitleSettings).mockImplementation(async (input) => {
    enabled = input?.autoSessionTitle ?? enabled;
    return { autoSessionTitle: enabled, uiLanguage: "zh" };
  });
  setLanguage("zh");
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<SessionTitleSetting />));
    const toggle = container.querySelector<HTMLButtonElement>("[role=switch]")!;
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(container.textContent).toContain("自动生成会话标题");
    await act(async () => toggle.click());
    expect(sessionTitleSettings).toHaveBeenCalledWith({ autoSessionTitle: true, uiLanguage: "zh" });
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    await act(async () => toggle.click());
    expect(sessionTitleSettings).toHaveBeenCalledWith({ autoSessionTitle: false, uiLanguage: "zh" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
  }
  finally {
    act(() => root.unmount());
  }
});

it("does not display a failed save as enabled", async () => {
  vi.mocked(sessionTitleSettings)
    .mockResolvedValueOnce({ autoSessionTitle: false, uiLanguage: "en" })
    .mockRejectedValueOnce(new Error("write failed"));
  const container = document.createElement("div");
  const root = createRoot(container);
  try {
    await act(async () => root.render(<SessionTitleSetting />));
    const toggle = container.querySelector<HTMLButtonElement>("[role=switch]")!;
    await act(async () => toggle.click());
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    expect(container.textContent).toContain("write failed");
    expect(toggle.disabled).toBe(false);
  }
  finally {
    act(() => root.unmount());
  }
});
