// @vitest-environment jsdom
// Regression: uninstalling a skill must update the left rail's installed-
// skills badge. The rail listens for the "skills-changed" window event and
// re-reads `list_installed_skills` — previously SkillsView refreshed only its
// own lists and never emitted the event, so the badge kept the stale count.
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emitFutureEvent, onFutureEvent } from "../../lib/futureEvents";

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

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows uninstall progress for one second, then collapses the row before broadcasting", async () => {
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
    vi.useFakeTimers();
    await act(async () => {
      buttonByText(container, "Confirm uninstall").click();
    });

    // The Agent has already removed the skill, but a background invalidation
    // must not remove the row before its exit animation starts.
    await act(async () => {
      emitFutureEvent("skills-changed", undefined);
    });
    await flushAsync();

    const alphaRow = container.querySelector<HTMLElement>("[data-skill-id='alpha']");
    expect(alphaRow).not.toBeNull();
    expect(alphaRow?.dataset.exiting).toBe("false");
    expect(buttonByText(container, "Uninstalling...").querySelector(".animate-spin")).not.toBeNull();

    // A fast backend response still keeps the progress state visible for the
    // requested minimum duration, and does not refresh the rest of the UI yet.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(999);
    });
    expect(alphaRow?.dataset.exiting).toBe("false");
    expect(badgeReads).toEqual([1]);

    // At one second the row starts its fade/collapse transition. The following
    // row remains in document order and moves up as the first row collapses.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(alphaRow?.dataset.exiting).toBe("true");
    expect(container.querySelector("[data-skill-id='beta']")).not.toBeNull();
    expect(badgeReads).toEqual([1]);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    await flushAsync();

    // The rail's re-read saw the post-uninstall list (1 skill left) only after
    // the row exit completed.
    expect(badgeReads).toEqual([1, 1]);
    expect(installed).toEqual(["beta"]);
    expect(container.querySelector("[data-skill-id='alpha']")).toBeNull();

    stopRail();
    await act(async () => {
      root.unmount();
    });
    container.remove();
  });

  it("shows a spinner while an install request is in flight", async () => {
    let finishInstall!: () => void;
    const installed = ["alpha"];
    const installPending = new Promise<void>((resolve) => {
      finishInstall = () => {
        installed.push("gamma");
        resolve();
      };
    });
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_skills")
        return Promise.resolve(installed.map(installedSkill));
      if (command === "list_available_skills")
        return Promise.resolve([availableSkill("alpha"), availableSkill("gamma")]);
      if (command === "install_skill")
        return installPending;
      return Promise.resolve(undefined);
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
    await act(async () => {
      buttonByText(container, "All").click();
    });

    vi.useFakeTimers();
    await act(async () => {
      buttonByText(container, "Install").click();
    });
    const installingButton = buttonByText(container, "Installing...");
    expect(installingButton.disabled).toBe(true);
    expect(installingButton.querySelector(".animate-spin")).not.toBeNull();

    // Even if the backend completes immediately, the in-flight display lasts
    // at least one second and does not briefly return to "Install".
    await act(async () => {
      finishInstall();
      await installPending;
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(999);
    });
    expect(buttonByText(container, "Installing...")).toBe(installingButton);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(container.textContent).not.toContain("Installing...");
    expect(buttonByText(container, "Uninstall")).not.toBeNull();

    await act(async () => {
      root.unmount();
    });
    container.remove();
  });

  it("keeps each operation's label and spinner until a slow Agent request finishes", async () => {
    const installed = ["alpha", "beta"];
    let finishUninstall!: () => void;
    let finishInstall!: () => void;
    const uninstallPending = new Promise<boolean>((resolve) => {
      finishUninstall = () => {
        installed.splice(installed.indexOf("alpha"), 1);
        resolve(true);
      };
    });
    const installPending = new Promise<void>((resolve) => {
      finishInstall = () => {
        installed.push("gamma");
        resolve();
      };
    });
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_skills")
        return Promise.resolve(installed.map(installedSkill));
      if (command === "list_available_skills")
        return Promise.resolve(["alpha", "beta", "gamma"].map(availableSkill));
      if (command === "uninstall_skill")
        return uninstallPending;
      if (command === "install_skill")
        return installPending;
      return Promise.resolve(undefined);
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
    await act(async () => {
      buttonByText(container, "All").click();
    });
    await act(async () => {
      buttonByText(container, "Uninstall").click();
    });
    vi.useFakeTimers();
    await act(async () => {
      buttonByText(container, "Confirm uninstall").click();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_500);
    });
    expect(buttonByText(container, "Uninstalling...").querySelector(".animate-spin")).not.toBeNull();
    expect(container.querySelector("[data-skill-action-id='alpha']")?.classList.contains("opacity-100")).toBe(true);

    await act(async () => {
      finishUninstall();
      await uninstallPending;
    });
    expect(container.querySelector("[data-skill-action-id='alpha']")?.classList.contains("opacity-0")).toBe(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(container.querySelector("[data-skill-action-id='alpha']")?.classList.contains("opacity-100")).toBe(true);
    expect(buttonByText(container, "Install")).not.toBeNull();
    await act(async () => {
      const gammaInstall = container.querySelector<HTMLButtonElement>("[data-skill-action-id='gamma'] button");
      if (!gammaInstall)
        throw new Error("gamma install button missing");
      gammaInstall.click();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_500);
    });
    expect(buttonByText(container, "Installing...").querySelector(".animate-spin")).not.toBeNull();

    await act(async () => {
      finishInstall();
      await installPending;
    });
    expect(container.textContent).not.toContain("Installing...");
    expect(installed).toEqual(["beta", "gamma"]);

    await act(async () => {
      root.unmount();
    });
    container.remove();
  });
});
