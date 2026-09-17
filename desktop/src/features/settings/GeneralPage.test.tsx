// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { DEFAULT_APP_SETTINGS } from "../../integrations/storage/appSettings";
import { GeneralPage } from "./GeneralPage";

vi.mock("../../integrations/agent/useSandboxAvailability", () => ({
  useSandboxAvailability: () => ({ available: true, resolved: true }),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

it("offers opt-in title generation without context compaction", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onToggle = vi.fn();
  try {
    await act(async () => root.render(
      <GeneralPage
        approvalTier="off"
        onChangeApprovalTier={() => {}}
        autoUpgradeSkills={false}
        onToggleAutoUpgradeSkills={() => {}}
        bellOnComplete
        onToggleBellOnComplete={() => {}}
        autoTitleFirstTurn={DEFAULT_APP_SETTINGS.autoTitleFirstTurn}
        onToggleAutoTitleFirstTurn={onToggle}
      />,
    ));
    expect(container.querySelectorAll("[role=switch]")).toHaveLength(3);
    expect(container.textContent).not.toContain("Show thinking process");
    expect(DEFAULT_APP_SETTINGS).not.toHaveProperty("showThinking");
    const toggle = container.querySelector<HTMLButtonElement>("[role=switch][aria-label='Generate a title after the first answer']");
    expect(toggle).not.toBeNull();
    expect(toggle!.getAttribute("aria-checked")).toBe("false");
    expect(container.textContent).toContain("Later answers do not trigger it");
    expect(container.textContent).toContain("Conversation context is not compacted");
    await act(async () => toggle!.click());
    expect(onToggle).toHaveBeenCalledExactlyOnceWith(true);
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
