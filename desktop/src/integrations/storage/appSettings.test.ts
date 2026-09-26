import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_APP_SETTINGS, getAppSettings, updateAppSettings } from "./appSettings";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();

vi.mock("../tauri/invoke", () => ({
  invokeCommand: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

beforeEach(() => {
  invokeMock.mockReset();
});

describe("app settings RPC", () => {
  it("reads the persisted settings through the documented command", async () => {
    invokeMock.mockResolvedValue(DEFAULT_APP_SETTINGS);

    await expect(getAppSettings()).resolves.toBe(DEFAULT_APP_SETTINGS);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith("get_app_settings", undefined);
  });

  it("passes a settings patch through as `input`, preserving absent keys", async () => {
    const next = { ...DEFAULT_APP_SETTINGS, communityEdition: true };
    invokeMock.mockResolvedValue(next);

    await expect(updateAppSettings({ communityEdition: true })).resolves.toBe(next);
    expect(invokeMock).toHaveBeenCalledExactlyOnceWith("update_app_settings", {
      input: { communityEdition: true },
    });
  });

  it("round-trips every settings field the UI can write without renaming any of them", async () => {
    const patch = {
      approvalTier: "manual" as const,
      hiddenModels: ["future/gpt-5", "deepseek/deepseek-chat"],
      autoUpgradeSkills: true,
      autoConnectRemote: true,
      skillGuideDismissed: true,
      skillIntroDismissed: true,
      bellOnComplete: false,
      autoTitleFirstTurn: false,
      titleLanguage: "zh" as const,
      communityEdition: true,
      skillRecommend: false,
    };
    invokeMock.mockResolvedValue({ ...DEFAULT_APP_SETTINGS, ...patch });

    await updateAppSettings(patch);

    expect(invokeMock).toHaveBeenCalledExactlyOnceWith("update_app_settings", { input: patch });
  });

  it("surfaces a read failure instead of falling back to defaults silently", async () => {
    invokeMock.mockRejectedValue(new Error("settings file is corrupt"));

    await expect(getAppSettings()).rejects.toThrow("settings file is corrupt");
  });

  it("propagates a rejected write (the caller reloads the authoritative settings)", async () => {
    invokeMock.mockRejectedValue("write protected");

    await expect(updateAppSettings({ bellOnComplete: true })).rejects.toBe("write protected");
  });
});
