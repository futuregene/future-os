// @vitest-environment jsdom
import type { AppSettings } from "../../../integrations/storage/appSettings";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { setLanguage } from "../../../i18n";
import { DEFAULT_APP_SETTINGS, getAppSettings, updateAppSettings } from "../../../integrations/storage/appSettings";
import { useTauriEvent } from "../../../lib/useTauriEvent";
import { useAppSettings } from "./useAppSettings";

vi.mock("../../../lib/useTauriEvent", () => ({ useTauriEvent: vi.fn() }));

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

it("mirrors the UI language for automatic titles and preserves an explicit opt-out despite the enabled default", async () => {
  setLanguage("en");
  expect(DEFAULT_APP_SETTINGS.autoTitleFirstTurn).toBe(true);
  let stored: AppSettings = { ...DEFAULT_APP_SETTINGS, autoTitleFirstTurn: false };
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
    expect(settings!.appSettings.autoTitleFirstTurn).toBe(false);

    await act(async () => setLanguage("zh"));
    expect(updateAppSettings).toHaveBeenLastCalledWith({ titleLanguage: "zh" });
    expect(settings!.appSettings.titleLanguage).toBe("zh");
    expect(settings!.appSettings.autoTitleFirstTurn).toBe(false);

    await act(async () => settings!.changeSettings({ autoTitleFirstTurn: true }));
    expect(updateAppSettings).toHaveBeenLastCalledWith({ autoTitleFirstTurn: true });
    expect(stored.autoTitleFirstTurn).toBe(true);
    expect(stored.titleLanguage).toBe("zh");

    // A phone edit invalidates desktop state without another local write.
    stored = { ...stored, autoUpgradeSkills: false, autoConnectRemote: true, hiddenModels: ["test/hidden"] };
    const notify = vi.mocked(useTauriEvent).mock.calls.find(([name]) => name === "app_settings_changed")![1];
    await act(async () => notify(undefined));
    expect(settings!.appSettings).toEqual(stored);

    // Foreground resume heals missed notifications too.
    stored = { ...stored, autoTitleFirstTurn: false };
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(settings!.appSettings.autoTitleFirstTurn).toBe(false);
  }
  finally {
    await act(async () => root.unmount());
    vi.mocked(updateAppSettings).mockClear();
    setLanguage("en");
    expect(updateAppSettings).not.toHaveBeenCalled();
  }
});
