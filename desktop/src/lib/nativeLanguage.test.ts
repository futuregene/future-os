// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeCommand } from "../integrations/tauri/invoke";

vi.mock("../integrations/tauri/invoke", () => ({ invokeCommand: vi.fn() }));

/**
 * The window's language is the only source the native quit confirmation has:
 * Rust raises that dialog outside any invoke, so the resolved tag has to reach
 * the backend before the user can quit. The module pushes on import and on every
 * change; outside Tauri the push throws or rejects and must stay silent.
 */
describe("native language registration", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.mocked(invokeCommand).mockReset();
    vi.mocked(invokeCommand).mockResolvedValue(undefined);
    localStorage.removeItem("future.language");
  });

  it("pushes the resolved language at start and again when the user switches", async () => {
    await import("./nativeLanguage");
    expect(invokeCommand).toHaveBeenCalledWith("set_ui_language", { language: "en" });

    const i18n = (await import("../i18n")).default;
    await i18n.changeLanguage("zh");
    expect(invokeCommand).toHaveBeenLastCalledWith("set_ui_language", { language: "zh" });
  });

  it("takes an explicit tag for the push it performs", async () => {
    const { pushNativeLanguage } = await import("./nativeLanguage");
    vi.mocked(invokeCommand).mockClear();
    pushNativeLanguage("zh");
    expect(invokeCommand).toHaveBeenCalledWith("set_ui_language", { language: "zh" });
  });

  it("stays silent when the backend rejects the push", async () => {
    vi.mocked(invokeCommand).mockRejectedValueOnce(new Error("no tauri"));
    await expect(import("./nativeLanguage")).resolves.toBeTruthy();
    // An unhandled rejection would fail this file at the runner level.
    await new Promise(resolve => setTimeout(resolve, 0));
  });

  it("stays silent when invoking outside Tauri throws synchronously", async () => {
    vi.mocked(invokeCommand).mockImplementationOnce(() => {
      throw new Error("window.__TAURI_INTERNALS__ is undefined");
    });
    await expect(import("./nativeLanguage")).resolves.toBeTruthy();
  });
});
