// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { setLanguage } from "../../../i18n";
import { sessionTitleSettings } from "../../../integrations/agent/sessionTitleSettings";
import { useSyncSessionTitleLanguage } from "./useSyncSessionTitleLanguage";

vi.mock("../../../integrations/agent/sessionTitleSettings", () => ({
  sessionTitleSettings: vi.fn().mockResolvedValue({ autoSessionTitle: false, uiLanguage: "en" }),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it("syncs startup and changed UI language without enabling automatic titles", async () => {
  function Harness() {
    useSyncSessionTitleLanguage();
    return null;
  }
  setLanguage("en");
  const root = createRoot(document.createElement("div"));
  try {
    await act(async () => root.render(<Harness />));
    expect(sessionTitleSettings).toHaveBeenLastCalledWith({ uiLanguage: "en" });
    await act(async () => setLanguage("zh"));
    expect(sessionTitleSettings).toHaveBeenLastCalledWith({ uiLanguage: "zh" });
    for (const [patch] of vi.mocked(sessionTitleSettings).mock.calls)
      expect(patch).not.toHaveProperty("autoSessionTitle");
  }
  finally {
    act(() => root.unmount());
    setLanguage("en");
  }
});
