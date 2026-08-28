// @vitest-environment jsdom
// Regression: uninstalling a skill must update the left rail's installed-
// skills badge. The rail listens for the "skills-changed" window event and
// re-reads `list_installed_skills` — previously SkillsView refreshed only its
// own lists and never emitted the event, so the badge kept the stale count.
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { onFutureEvent } from "../../lib/futureEvents";

import { flushAsync } from "../../test/renderHook";
import { SkillsView } from "./SkillsView";
import "../../test/i18nTestSetup";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) => invokeMock(command, args),
}));

// SkillsView renders LeftPanelTitlebarToggle → useIsFullscreen, which touches
// the Tauri window API — unavailable (and irrelevant) under jsdom.
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
  }),
}));

function installedSkill(id: string) {
  return {
    id,
    name: id,
    description: `desc ${id}`,
    nameZh: id,
    descriptionZh: `desc-zh ${id}`,
    version: "1.0.0",
  };
}

function availableSkill(id: string) {
  return {
    id,
    name: id,
    description: `desc ${id}`,
    nameZh: id,
    descriptionZh: `desc-zh ${id}`,
    category: "cat",
    categoryZh: "cat-zh",
    latestVersion: "1.0.0",
  };
}

function buttonByText(container: HTMLElement, text: string): HTMLButtonElement {
  const matches = [...container.querySelectorAll("button")]
    .filter(button => button.textContent?.trim() === text);
  if (matches.length === 0)
    throw new Error(`no button with text "${text}"`);
  return matches[0]!;
}

describe("skillsView skills-changed broadcast", () => {
  beforeEach(() => {
    invokeMock.mockReset();
  });

  it("emits skills-changed after uninstall so rail-badge listeners re-read", async () => {
    // Mock backend state: two installed skills; uninstall removes by id.
    const installed = ["alpha", "beta"];
    invokeMock.mockImplementation((command: string, args?: { id?: string }) => {
      if (command === "list_installed_skills")
        return Promise.resolve(installed.map(installedSkill));
      if (command === "list_available_skills")
        return Promise.resolve(installed.map(availableSkill));
      if (command === "uninstall_skill") {
        const index = installed.indexOf(args?.id ?? "");
        if (index >= 0)
          installed.splice(index, 1);
        return Promise.resolve(index >= 0);
      }
      return Promise.resolve(undefined);
    });

    // Mirror ActivityRail: on "skills-changed", re-read the installed list and
    // remember what the badge would show.
    const badgeReads: number[] = [];
    const stopRail = onFutureEvent("skills-changed", () => {
      void invokeMock("list_installed_skills").then((skills: Array<{ id: string }>) => {
        badgeReads.push(skills.length);
      });
    });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);
    await act(async () => {
      root.render(createElement(SkillsView, {
        leftPanelExpanded: true,
        onToggleLeftPanel: () => {},
        onStartCoachConversation: async () => {},
        onTrySkill: () => {},
      }));
    });
    await flushAsync();
    expect(container.textContent).toContain("alpha");

    // Uninstall "alpha" through the real buttons (uninstall → confirm).
    await act(async () => {
      buttonByText(container, "Uninstall").click();
    });
    await act(async () => {
      buttonByText(container, "Confirm uninstall").click();
    });
    await flushAsync();

    // The rail's re-read saw the post-uninstall list (1 skill left).
    expect(badgeReads).toEqual([1]);
    expect(installed).toEqual(["beta"]);

    stopRail();
    await act(async () => {
      root.unmount();
    });
    container.remove();
  });
});
