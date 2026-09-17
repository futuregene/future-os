// @vitest-environment jsdom
import type { AppSettings } from "../../../integrations/storage/appSettings";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { setLanguage } from "../../../i18n";
import { DEFAULT_APP_SETTINGS, getAppSettings, updateAppSettings } from "../../../integrations/storage/appSettings";
import { useAppSettings } from "./useAppSettings";

vi.mock("../../../integrations/storage/appSettings", async importOriginal => ({
  ...await importOriginal<typeof import("../../../integrations/storage/appSettings")>(),
  getAppSettings: vi.fn(),
  updateAppSettings: vi.fn(),
}));
vi.mock("../../../integrations/agent/useSandboxAvailability", () => ({
  useSandboxAvailability: () => ({ available: true, resolved: true }),
  shouldPersistSandboxFallback: () => false,
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it("mirrors the UI language for automatic titles and preserves the stored opt-in", async () => {
  setLanguage("en");
  let stored: AppSettings = { ...DEFAULT_APP_SETTINGS, autoTitleFirstTurn: true };
  vi.mocked(getAppSettings).mockImplementation(async () => ({ ...stored }));
  vi.mocked(updateAppSettings).mockImplementation(async (patch) => {
    stored = { ...stored, ...patch };
    return { ...stored };
  });
  let settings: ReturnType<typeof useAppSettings>;
  function Host() {
    settings = useAppSettings();
    return null;
  }
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Host />));
    expect(updateAppSettings).toHaveBeenLastCalledWith({ titleLanguage: "en" });
    expect(settings!.appSettings.autoTitleFirstTurn).toBe(true);

    await act(async () => setLanguage("zh"));
    expect(updateAppSettings).toHaveBeenLastCalledWith({ titleLanguage: "zh" });
    expect(settings!.appSettings.titleLanguage).toBe("zh");
    expect(settings!.appSettings.autoTitleFirstTurn).toBe(true);

    await act(async () => settings!.changeSettings({ autoTitleFirstTurn: false }));
    expect(updateAppSettings).toHaveBeenLastCalledWith({ autoTitleFirstTurn: false });
    expect(stored.autoTitleFirstTurn).toBe(false);
    expect(stored.titleLanguage).toBe("zh");
  }
  finally {
    await act(async () => root.unmount());
    vi.mocked(updateAppSettings).mockClear();
    setLanguage("en");
    expect(updateAppSettings).not.toHaveBeenCalled();
  }
});
