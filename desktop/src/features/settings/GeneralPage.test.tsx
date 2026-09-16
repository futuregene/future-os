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

it("offers an opt-in first-answer summary switch", async () => {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onToggle = vi.fn();
  try {
    await act(async () => root.render(
      <GeneralPage
        approvalTier="off"
        onChangeApprovalTier={() => {}}
        showThinking={false}
        onToggleShowThinking={() => {}}
        autoUpgradeSkills={false}
        onToggleAutoUpgradeSkills={() => {}}
        bellOnComplete
        onToggleBellOnComplete={() => {}}
        autoCompactFirstTurn={DEFAULT_APP_SETTINGS.autoCompactFirstTurn}
        onToggleAutoCompactFirstTurn={onToggle}
      />,
    ));
    const toggle = container.querySelector<HTMLButtonElement>("[role=switch][aria-label='Summarize after the first answer']");
    expect(toggle).not.toBeNull();
    expect(toggle!.getAttribute("aria-checked")).toBe("false");
    expect(container.textContent).toContain("Later answers do not trigger it");
    await act(async () => toggle!.click());
    expect(onToggle).toHaveBeenCalledExactlyOnceWith(true);
  }
  finally {
    act(() => root.unmount());
    container.remove();
  }
});
