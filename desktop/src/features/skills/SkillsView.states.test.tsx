// @vitest-environment jsdom
import type { AvailableSkill, InstalledSkill } from "../../integrations/skills/skillsClient";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import { invalidateSkillCatalog } from "../../integrations/skills/skillsClient";
import { onFutureEvent } from "../../lib/futureEvents";
import { flushAsync } from "../../test/renderHook";
import { SkillsView } from "./SkillsView";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (command: string, args?: unknown) => invokeMock(command, args),
}));
// LeftPanelTitlebarToggle 鈫?useIsFullscreen reads the Tauri window API, which is
// irrelevant (and absent) under jsdom.
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFullscreen: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
  }),
}));

function installedSkill(id: string, overrides: Partial<InstalledSkill> = {}): InstalledSkill {
  return {
    id,
    name: id,
    description: `desc ${id}`,
    nameZh: null,
    descriptionZh: null,
    version: "1.0.0",
    ...overrides,
  };
}

function availableSkill(id: string, overrides: Partial<AvailableSkill> = {}): AvailableSkill {
  return {
    id,
    name: id,
    description: `desc ${id}`,
    nameZh: `${id}-zh`,
    descriptionZh: `desc-zh ${id}`,
    category: "research",
    categoryZh: "鐮旂┒",
    latestVersion: "2.0.0",
    ...overrides,
  };
}

/** Mutable backend state, so a test reads like the agent's real behaviour. */
interface Backend {
  installed: InstalledSkill[];
  available: AvailableSkill[];
  installedError?: string;
  availableError?: string;
  installFails?: boolean;
  installOmitsId?: boolean;
  uninstallFails?: boolean;
  uninstallKeepsId?: boolean;
  syncFails?: boolean;
  syncReportsFailed?: string[];
  syncRejects?: boolean;
}

function installBackend(backend: Backend) {
  invokeMock.mockImplementation((command: string, args?: { id?: string; version?: string }) => {
    switch (command) {
      case "list_installed_skills":
        return backend.installedError
          ? Promise.reject(new Error(backend.installedError))
          : Promise.resolve([...backend.installed]);
      case "list_available_skills":
        return backend.availableError
          ? Promise.reject(new Error(backend.availableError))
          : Promise.resolve([...backend.available]);
      case "install_skill": {
        if (backend.installFails)
          return Promise.reject(new Error("install refused"));
        if (!backend.installOmitsId && args?.id) {
          const fresh = installedSkill(args.id, { version: args.version ?? "1.0.0" });
          backend.installed = backend.installed.some(skill => skill.id === args.id)
            ? backend.installed.map(skill => (skill.id === args.id ? fresh : skill))
            : [...backend.installed, fresh];
        }
        return Promise.resolve(undefined);
      }
      case "uninstall_skill": {
        if (backend.uninstallFails)
          return Promise.reject(new Error("uninstall refused"));
        if (!backend.uninstallKeepsId)
          backend.installed = backend.installed.filter(skill => skill.id !== args?.id);
        return Promise.resolve(true);
      }
      case "sync_skills": {
        if (backend.syncRejects)
          return Promise.reject(new Error("agent offline"));
        return Promise.resolve({
          installed: [],
          upgraded: backend.installed.map(skill => skill.id),
          skipped: [],
          failed: backend.syncReportsFailed ?? [],
        });
      }
      default:
        return Promise.resolve(undefined);
    }
  });
}

const roots: { root: ReturnType<typeof createRoot>; container: HTMLElement }[] = [];
let toasts: { message: string; tone?: string }[] = [];
let offToast: (() => void) | null = null;

beforeEach(() => {
  invokeMock.mockReset();
  invalidateSkillCatalog();
  toasts = [];
  offToast = onFutureEvent("toast", detail => toasts.push(detail));
  void i18n.changeLanguage("en");
});

afterEach(() => {
  offToast?.();
  offToast = null;
  for (const { root, container } of roots.splice(0)) {
    act(() => root.unmount());
    container.remove();
  }
  void i18n.changeLanguage("en");
});

async function renderView(backend: Backend, props: { onTrySkill?: (name: string) => void; onStartCoachConversation?: (content: string) => Promise<void> } = {}) {
  installBackend(backend);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  roots.push({ container, root });
  await act(async () => {
    root.render(createElement(SkillsView, {
      leftPanelExpanded: true,
      onToggleLeftPanel: () => {},
      onStartCoachConversation: props.onStartCoachConversation ?? (async () => {}),
      onTrySkill: props.onTrySkill ?? (() => {}),
    }));
  });
  await flushAsync();
  return container;
}

function button(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(item => item.textContent?.trim() === text) as HTMLButtonElement | undefined;
}

function tabButton(container: HTMLElement, label: string) {
  return [...container.querySelectorAll("div.w-64 button")].find(item => item.textContent === label) as HTMLButtonElement;
}

/** Switch tabs without tripping the 1s minimum-busy timer (no timers pending). */
async function showAllTab(container: HTMLElement) {
  await act(async () => {
    tabButton(container, "All").click();
  });
}

/**
 * Drive a mutation to completion. Every install/uninstall/upgrade action keeps
 * its busy state for at least one second (`minimumBusyMs`) before it can settle,
 * so the test takes over the clock after mounting (mounting itself must use real
 * timers, or React's own scheduling would be frozen too).
 */
async function runMutation(start: () => void, extraMs = 0) {
  vi.useFakeTimers();
  await act(async () => {
    start();
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000 + extraMs);
  });
  vi.useRealTimers();
  await flushAsync();
}

describe("skillsView loading, empty and error states", () => {
  it("shows the loading row until both lists resolve", async () => {
    let resolveInstalled!: (value: unknown) => void;
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_skills") {
        return new Promise((resolve) => {
          resolveInstalled = resolve;
        });
      }
      if (command === "list_available_skills")
        return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    roots.push({ container, root });
    await act(async () => {
      root.render(createElement(SkillsView, {
        leftPanelExpanded: false,
        onToggleLeftPanel: () => {},
        onStartCoachConversation: async () => {},
        onTrySkill: () => {},
      }));
    });
    await flushAsync();

    expect(container.textContent).toContain("Loading...");

    await act(async () => {
      resolveInstalled([]);
    });
    await flushAsync();
    // An empty installed list sends a first-time visitor to the catalogue tab.
    expect(tabButton(container, "All").className).toContain("bg-surface");
  });

  it("shows the empty catalogue state when nothing is published", async () => {
    const container = await renderView({ installed: [], available: [] });

    expect(container.textContent).toContain("The skill marketplace is empty");
  });

  it("reports an installed-list failure instead of showing an empty tab, and retries", async () => {
    const backend: Backend = { installed: [], available: [], installedError: "agent unreachable" };
    const container = await renderView(backend);

    expect(container.textContent).toContain("Failed to load installed skills");
    expect(container.textContent).toContain("agent unreachable");
    expect(container.textContent).not.toContain("No skills installed");

    backend.installedError = undefined;
    backend.installed = [installedSkill("alpha")];
    await act(async () => {
      button(container, "Retry")!.click();
    });
    await flushAsync();

    expect(container.textContent).toContain("alpha");
    expect(container.textContent).not.toContain("agent unreachable");
  });

  it("reports a catalogue failure on the All tab while the Installed tab keeps working", async () => {
    const backend: Backend = { installed: [installedSkill("alpha")], available: [], availableError: "platform unreachable" };
    const container = await renderView(backend);
    expect(container.textContent).toContain("alpha");

    await showAllTab(container);
    expect(container.textContent).toContain("Failed to load the skill marketplace");
    expect(container.textContent).toContain("platform unreachable");

    backend.availableError = undefined;
    backend.available = [availableSkill("gamma")];
    await act(async () => {
      button(container, "Retry")!.click();
    });
    await flushAsync();

    expect(container.textContent).toContain("gamma");
    expect(container.textContent).not.toContain("platform unreachable");
  });
});

describe("skillsView installed rows", () => {
  it("renders the catalogue's localized text, category badge and version", async () => {
    const backend: Backend = {
      installed: [installedSkill("alpha")],
      available: [availableSkill("alpha")],
    };
    const container = await renderView(backend);

    // English UI: the catalogue's canonical text wins over the agent's.
    expect(container.textContent).toContain("alpha");
    expect(container.textContent).toContain("v1.0.0");
    expect(container.textContent).toContain("research");
    expect(container.textContent).not.toContain("alpha-zh");
  });

  it("uses the Chinese catalogue text when the UI language is Chinese", async () => {
    await act(async () => {
      await i18n.changeLanguage("zh");
    });
    const backend: Backend = {
      installed: [installedSkill("alpha")],
      available: [availableSkill("alpha")],
    };
    const container = await renderView(backend);

    expect(container.textContent).toContain("鐮旂┒");
  });

  it("falls back to the agent's own fields when the catalogue has no entry", async () => {
    const backend: Backend = {
      installed: [installedSkill("local-only", { description: "", descriptionZh: "鏈湴璇存槑", version: null })],
      available: [],
    };
    const container = await renderView(backend);

    expect(container.textContent).toContain("local-only");
    expect(container.textContent).not.toContain("v1.0");
    // No description and no catalogue entry 鈬?neither paragraph nor category.
    expect(container.querySelector("[data-skill-id='local-only'] p")).toBeNull();
  });

  it("handles a skill with a null version and null descriptions", async () => {
    const backend: Backend = {
      installed: [installedSkill("bare", { description: "", version: null })],
      available: [],
    };
    const container = await renderView(backend);

    expect(container.querySelector("[data-skill-id='bare']")!.textContent).toContain("bare");
    expect(container.querySelectorAll("[data-skill-id='bare'] span.font-medium")).toHaveLength(1);
  });

  it("shows the installed-tab empty state after the user returns to it", async () => {
    // A first visit with nothing installed lands on the catalogue tab; coming
    // back to Installed must still explain the empty tab rather than looking
    // like a failed load.
    const backend: Backend = { installed: [], available: [availableSkill("gamma")] };
    const container = await renderView(backend);

    await act(async () => {
      tabButton(container, "Installed").click();
    });

    expect(container.textContent).toContain("No skills installed yet");
    expect(container.textContent).toContain("Browse the skill marketplace");
  });

  it("offers Try it to the caller", async () => {
    const onTrySkill = vi.fn();
    const backend: Backend = { installed: [installedSkill("alpha")], available: [availableSkill("alpha")] };
    const container = await renderView(backend, { onTrySkill });

    act(() => button(container, "Try it")!.click());

    expect(onTrySkill).toHaveBeenCalledExactlyOnceWith("alpha");
  });

  it("shows an upgrade button for a skill with a newer catalogue version", async () => {
    const backend: Backend = {
      installed: [installedSkill("alpha", { version: "1.0.0" })],
      available: [availableSkill("alpha", { latestVersion: "2.0.0", upgradeAvailable: true })],
    };
    const container = await renderView(backend);

    const upgrade = button(container, "Upgrade")!;
    expect(upgrade.title).toContain("2.0.0");
    expect(button(container, "Upgrade all (1)")).toBeTruthy();

    await runMutation(() => upgrade.click());

    expect(backend.installed.find(skill => skill.id === "alpha")!.version).toBe("2.0.0");
  });

  it("hides the upgrade button for an up-to-date skill", async () => {
    const backend: Backend = {
      installed: [installedSkill("alpha", { version: "2.0.0" })],
      available: [availableSkill("alpha", { latestVersion: "2.0.0", upgradeAvailable: false })],
    };
    const container = await renderView(backend);

    expect(button(container, "Upgrade")).toBeUndefined();
    expect(button(container, "Upgrade all")!.disabled).toBe(true);
  });
});

describe("skillsView install and uninstall errors", () => {
  it("toasts a rejected install and clears the busy state", async () => {
    const backend: Backend = {
      installed: [],
      available: [availableSkill("gamma")],
      installFails: true,
    };
    const container = await renderView(backend);
    await showAllTab(container);

    await runMutation(() => button(container, "Install")!.click());

    expect(toasts).toHaveLength(1);
    expect(toasts[0]!.tone).toBe("error");
    expect(toasts[0]!.message).toContain("install refused");
    // The row returns to its installable state so the user can retry.
    expect(button(container, "Install")).toBeTruthy();
  });

  it("toasts when the agent omits the skill from the post-install snapshot", async () => {
    const backend: Backend = {
      installed: [],
      available: [availableSkill("gamma")],
      installOmitsId: true,
    };
    const container = await renderView(backend);
    await showAllTab(container);

    await runMutation(() => button(container, "Install")!.click());

    expect(toasts[0]!.message).toContain("was not returned by the Agent");
    expect(button(container, "Install")).toBeTruthy();
  });

  it("toasts a rejected uninstall", async () => {
    const backend: Backend = {
      installed: [installedSkill("alpha")],
      available: [availableSkill("alpha")],
      uninstallFails: true,
    };
    const container = await renderView(backend);

    act(() => button(container, "Uninstall")!.click());
    await runMutation(() => button(container, "Confirm uninstall")!.click());

    expect(toasts[0]!.message).toContain("uninstall refused");
    expect(container.textContent).toContain("alpha");
  });

  it("toasts when the agent still reports an uninstalled skill", async () => {
    const backend: Backend = {
      installed: [installedSkill("alpha")],
      available: [availableSkill("alpha")],
      uninstallKeepsId: true,
    };
    const container = await renderView(backend);

    act(() => button(container, "Uninstall")!.click());
    await runMutation(() => button(container, "Confirm uninstall")!.click());

    expect(toasts[0]!.message).toContain("still returned by the Agent");
  });

  it("can cancel the uninstall confirmation", async () => {
    const backend: Backend = { installed: [installedSkill("alpha")], available: [availableSkill("alpha")] };
    const container = await renderView(backend);

    act(() => button(container, "Uninstall")!.click());
    expect(button(container, "Confirm uninstall")).toBeTruthy();

    act(() => button(container, "Cancel")!.click());

    expect(button(container, "Uninstall")).toBeTruthy();
    expect(button(container, "Confirm uninstall")).toBeUndefined();
  });
});

describe("skillsView upgrade all", () => {
  async function upgradeReady() {
    const backend: Backend = {
      installed: [installedSkill("alpha", { version: "1.0.0" }), installedSkill("beta", { version: "1.0.0" })],
      available: [
        availableSkill("alpha", { latestVersion: "2.0.0", upgradeAvailable: true }),
        availableSkill("beta", { latestVersion: "2.0.0", upgradeAvailable: true }),
      ],
    };
    const container = await renderView(backend);
    return { backend, container };
  }

  it("upgrades every outdated skill and refreshes both lists", async () => {
    const { backend, container } = await upgradeReady();
    backend.installed = backend.installed.map(skill => ({ ...skill, version: "2.0.0" }));
    backend.available = backend.available.map(skill => ({ ...skill, upgradeAvailable: false }));

    await runMutation(() => button(container, "Upgrade all (2)")!.click());

    expect(invokeMock).toHaveBeenCalledWith("sync_skills", undefined);
    expect(toasts).toEqual([]);
    // The refreshed view no longer offers an upgrade.
    expect(button(container, "Upgrade all")).toBeTruthy();
    expect(button(container, "Upgrade all")!.disabled).toBe(true);
  });

  it("reports the skills the agent failed to upgrade", async () => {
    const { backend, container } = await upgradeReady();
    backend.syncReportsFailed = ["beta: signature mismatch"];

    await runMutation(() => button(container, "Upgrade all (2)")!.click());

    expect(toasts).toHaveLength(1);
    expect(toasts[0]!.tone).toBe("error");
    expect(toasts[0]!.message).toContain("beta: signature mismatch");
  });

  it("reports a rejected sync", async () => {
    const { backend, container } = await upgradeReady();
    backend.syncRejects = true;

    await runMutation(() => button(container, "Upgrade all (2)")!.click());

    expect(toasts[0]!.message).toContain("agent offline");
    // The per-skill busy state is released, so the user can try again.
    expect(button(container, "Upgrade all (2)")!.disabled).toBe(false);
  });
});

describe("skillsView filters", () => {
  const backend: Backend = {
    installed: [installedSkill("alpha"), installedSkill("beta")],
    available: [
      availableSkill("alpha", { category: "research", categoryZh: "鐮旂┒" }),
      availableSkill("beta", { category: "writing", categoryZh: "鍐欎綔" }),
      availableSkill("gamma", { category: "research", categoryZh: "鐮旂┒" }),
    ],
  };

  it("filters the installed list by keyword and reports the count", async () => {
    const container = await renderView(backend);
    expect(container.textContent).toContain("2 / 2");

    const search = container.querySelector<HTMLInputElement>("input[role=textbox], input:not([type])")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, "beta");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });

    expect(container.textContent).toContain("1 / 2");
    expect(container.textContent).toContain("beta");
    expect(button(container, "Clear")!).toBeTruthy();

    // A query that matches nothing shows the filter-specific empty state.
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, "zzzz");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(container.textContent).toContain("No matching skills");

    await act(async () => {
      button(container, "Clear")!.click();
    });
    expect(container.textContent).toContain("2 / 2");
    expect(button(container, "Clear")).toBeUndefined();
  });

  it("filters by category, including the catalogue-mapped category of an installed skill", async () => {
    const container = await renderView(backend);
    const select = container.querySelector<HTMLSelectElement>("select")!;

    expect([...select.options].map(option => option.textContent)).toEqual(["All categories", "research", "writing"]);

    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!.call(select, "writing");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });

    expect(container.textContent).toContain("beta");
    expect(container.textContent).toContain("1 / 2");
  });

  it("labels the category options in Chinese when the UI language is Chinese", async () => {
    await act(async () => {
      await i18n.changeLanguage("zh");
    });
    const container = await renderView(backend);
    const select = container.querySelector<HTMLSelectElement>("select")!;

    expect([...select.options].map(option => option.textContent)).toContain("鐮旂┒");
  });

  it("keeps filters independent per tab", async () => {
    const container = await renderView(backend);

    const search = container.querySelector<HTMLInputElement>("input:not([type])")!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(search, "alpha");
      search.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await showAllTab(container);

    // The All tab starts unfiltered: the Installed tab's query must not leak.
    expect(container.textContent).toContain("3 / 3");
  });
});

describe("skillsView catalogue tab", () => {
  it("installs a catalogue-only skill and marks it installed afterwards", async () => {
    const backend: Backend = { installed: [], available: [availableSkill("gamma")] };
    const container = await renderView(backend);
    await showAllTab(container);

    await runMutation(() => button(container, "Install")!.click());

    expect(backend.installed.map(skill => skill.id)).toEqual(["gamma"]);
    expect(toasts).toEqual([]);
  });

  it("disables the install button for a catalogue entry with no published version", async () => {
    const backend: Backend = { installed: [], available: [availableSkill("gamma", { latestVersion: null })] };
    const container = await renderView(backend);
    await showAllTab(container);

    const install = button(container, "No version")!;
    expect(install.disabled).toBe(true);
  });

  it("offers uninstall and upgrade for an installed catalogue entry", async () => {
    const backend: Backend = {
      installed: [installedSkill("alpha", { version: "1.0.0" })],
      available: [availableSkill("alpha", { latestVersion: "2.0.0", upgradeAvailable: true })],
    };
    const container = await renderView(backend);
    await showAllTab(container);

    expect(button(container, "Uninstall")).toBeTruthy();
    expect(button(container, "Upgrade")).toBeTruthy();
    expect(container.querySelector("[data-skill-action-id='alpha']")).not.toBeNull();
  });

  it("upgrades an installed skill from the catalogue tab", async () => {
    const backend: Backend = {
      installed: [installedSkill("alpha", { version: "1.0.0" })],
      available: [availableSkill("alpha", { latestVersion: "2.0.0", upgradeAvailable: true })],
    };
    const container = await renderView(backend);
    await showAllTab(container);

    await runMutation(() => button(container, "Upgrade")!.click());

    expect(backend.installed.find(skill => skill.id === "alpha")!.version).toBe("2.0.0");
    expect(toasts).toEqual([]);
  });

  it("marks a row as exiting while its uninstall animation runs", async () => {
    const backend: Backend = { installed: [installedSkill("alpha"), installedSkill("beta")], available: [] };
    const container = await renderView(backend);

    act(() => button(container, "Uninstall")!.click());
    vi.useFakeTimers();
    await act(async () => {
      button(container, "Confirm uninstall")!.click();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });

    // The agent has already removed the skill, but the row stays mounted until
    // its exit transition ends (it must not vanish mid-animation).
    expect(container.querySelector("[data-skill-id='alpha']")!.getAttribute("data-exiting")).toBe("true");
    expect(container.querySelector("[data-skill-id='beta']")!).not.toBeNull();

    // After the exit window the row is gone from the list.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    vi.useRealTimers();
    await flushAsync();
    expect(container.querySelector("[data-skill-id='alpha']")).toBeNull();
    expect(container.querySelector("[data-skill-id='beta']")).not.toBeNull();
  });

  it("drops a superseded refresh's results instead of overwriting the newer list", async () => {
    const backend: Backend = { installed: [installedSkill("alpha")], available: [availableSkill("alpha")] };
    installBackend(backend);
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    roots.push({ container, root });
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

    // Refresh #1: a slow read (e.g. the platform catalogue hanging).
    let resolveSlowInstalled!: (value: unknown) => void;
    invalidateSkillCatalog();
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_skills") {
        return new Promise((resolve) => {
          resolveSlowInstalled = resolve;
        });
      }
      if (command === "list_available_skills")
        return Promise.resolve([availableSkill("alpha")]);
      return Promise.resolve(undefined);
    });
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:skills-changed", { detail: undefined }));
    });

    // Refresh #2 starts while #1 is still in flight and finishes first.
    invalidateSkillCatalog();
    invokeMock.mockImplementation((command: string) => {
      if (command === "list_installed_skills")
        return Promise.resolve([installedSkill("beta")]);
      if (command === "list_available_skills")
        return Promise.resolve([availableSkill("beta")]);
      return Promise.resolve(undefined);
    });
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:skills-changed", { detail: undefined }));
    });
    await flushAsync();
    expect(container.textContent).toContain("beta");

    // The slow refresh lands last: its stale list must be discarded.
    await act(async () => {
      resolveSlowInstalled([installedSkill("stale")]);
    });
    await flushAsync();

    expect(container.textContent).toContain("beta");
    expect(container.textContent).not.toContain("stale");
  });

  it("reloads when the app broadcasts a skills change", async () => {
    const backend: Backend = { installed: [], available: [availableSkill("gamma")] };
    const container = await renderView(backend);
    await showAllTab(container);
    expect(container.textContent).toContain("gamma");

    backend.available = [availableSkill("delta")];
    // The broadcaster is always a mutation path, which drops the shared
    // catalogue cache before the view re-reads.
    invalidateSkillCatalog();
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:skills-changed", { detail: undefined }));
    });
    await flushAsync();

    expect(container.textContent).toContain("delta");
  });
});

/**
 * Both tabs render an uninstall control only for an id that the *same commit's*
 * `displayedInstalled` holds — the Installed tab maps `filteredInstalled` rows
 * (a subset of it), the All tab gates on `installedIds` (derived from it) — and
 * a held row is re-inserted into it, so `runUninstall`'s `if (!skill) return;`
 * guard cannot be entered. These two tests pin that invariant at both call
 * sites; the line itself is waived as `unreachable-by-construction` in
 * `docs/testing/desktop-settings.md`.
 */
describe("skillsView uninstall control reachability", () => {
  it("keeps a held row's uninstall control inert after a background reload drops the skill", async () => {
    const backend: Backend = { installed: [installedSkill("alpha"), installedSkill("beta")], available: [] };
    const container = await renderView(backend);
    const heldRow = () => container.querySelector<HTMLElement>("[data-skill-id='alpha']");

    // Start alpha's uninstall, then let a background reload land while the
    // agent call is still inside its 1 s minimum-busy window.
    act(() => button(container, "Uninstall")!.click());
    vi.useFakeTimers();
    await act(async () => {
      button(container, "Confirm uninstall")!.click();
    });
    backend.installed = [installedSkill("beta")];
    invalidateSkillCatalog();
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:skills-changed", { detail: undefined }));
    });
    await flushAsync();

    // The backend has dropped alpha, yet the row is held by the in-flight
    // operation: it is re-inserted at the index the handler captured. Held in
    // the list the handler closes over is exactly what keeps `find` succeeding.
    expect(heldRow()).not.toBeNull();
    expect([...container.querySelectorAll("[data-skill-id]")].map(node => node.getAttribute("data-skill-id")))
      .toEqual(["alpha", "beta"]);

    // Its uninstall controls are disabled for as long as the row is held, and
    // React's refusal is real: stripping the attribute and clicking anyway does
    // not reach the handler (`shouldPreventMouseEvent` reads the prop, not the
    // attribute), so the confirmation stays open.
    const cancel = button(heldRow()!, "Cancel")!;
    expect(cancel.disabled).toBe(true);
    // The confirm control now reads "Uninstalling..." for the same operation.
    expect(button(heldRow()!, "Uninstalling...")!.disabled).toBe(true);
    cancel.removeAttribute("disabled");
    act(() => cancel.click());
    expect(button(heldRow()!, "Cancel")).toBeTruthy();
    expect(button(heldRow()!, "Uninstalling...")).toBeTruthy();

    // The exit animation holds it a little longer, with the same refusal.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(heldRow()!.getAttribute("data-exiting")).toBe("true");
    expect(button(heldRow()!, "Uninstalling...")!.disabled).toBe(true);

    // Only when the exit window closes does the row — and its control — leave
    // the list, in the same commit that drops the id from it.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    vi.useRealTimers();
    await flushAsync();
    expect(heldRow()).toBeNull();
    expect(button(container, "Uninstall")).toBeTruthy();
  });

  it("drops the catalogue uninstall control in the same commit as the installed list", async () => {
    const backend: Backend = { installed: [installedSkill("alpha")], available: [availableSkill("alpha")] };
    const container = await renderView(backend);
    await showAllTab(container);
    expect(button(container, "Uninstall")).toBeTruthy();

    // The catalogue still lists alpha; the installed list no longer does.
    backend.installed = [];
    invalidateSkillCatalog();
    await act(async () => {
      window.dispatchEvent(new CustomEvent("futureos:skills-changed", { detail: undefined }));
    });
    await flushAsync();

    expect(container.querySelector("[data-skill-action-id='alpha']")).not.toBeNull();
    expect(button(container, "Uninstall")).toBeUndefined();
    expect(button(container, "Install")).toBeTruthy();
  });
});
