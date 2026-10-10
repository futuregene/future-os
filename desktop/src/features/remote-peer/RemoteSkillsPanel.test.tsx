// @vitest-environment jsdom
import type { RemotePeer } from "./remotePeerClient";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, expect, it, vi } from "vitest";
import { RemoteSkillsPanel } from "./RemoteSkillsPanel";

/**
 * Managing the skills on another computer.
 *
 * Installing and removing act on *that* machine, so what is asserted is which
 * command goes where with which id — and that the lists are re-read afterwards,
 * because installing moves a row between them and only the host knows which
 * version it ended up with.
 */

const installed = vi.fn<(desktopId: string) => Promise<unknown>>();
const catalogue = vi.fn<(desktopId: string) => Promise<unknown>>();
const install = vi.fn<(desktopId: string, skillId: string, version: string) => Promise<unknown>>();
const remove = vi.fn<(desktopId: string, skillId: string) => Promise<unknown>>();
vi.mock("./remotePeerClient", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./remotePeerClient")>();
  return {
    ...actual,
    installRemoteSkill: (...args: Parameters<typeof install>) => install(...args),
    listRemoteAvailableSkills: (...args: Parameters<typeof catalogue>) => catalogue(...args),
    listRemoteInstalledSkills: (...args: Parameters<typeof installed>) => installed(...args),
    uninstallRemoteSkill: (...args: Parameters<typeof remove>) => remove(...args),
  };
});

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const peer: RemotePeer = {
  agentAvailable: true,
  bridgeInstanceId: "bridge_1",
  connected: true,
  desktopId: "desktop_a",
  error: null,
  features: [],
  icon: "laptop",
  name: "Studio iMac",
  pairId: "pair_1",
};

async function mount(props: { available?: boolean } = {}) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<RemoteSkillsPanel available={props.available ?? true} peer={peer} />);
  });
  await settle();
  return {
    container,
    button: (label: string) => [...container.querySelectorAll("button")]
      .find(node => node.textContent === label),
    buttons: (label: string) => [...container.querySelectorAll("button")]
      .filter(node => node.textContent === label),
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

async function settle(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await Promise.resolve();
  });
}

beforeEach(() => {
  document.body.innerHTML = "";
  installed.mockReset().mockResolvedValue([]);
  catalogue.mockReset().mockResolvedValue([]);
  install.mockReset().mockResolvedValue(undefined);
  remove.mockReset().mockResolvedValue(true);
});

it("shows what is installed and what could be, and says so when both are empty", async () => {
  const view = await mount();
  expect(view.text()).toContain("Nothing installed on that computer yet.");
  await view.unmount();
});

/** A catalogue row that is already installed is not offered again. */
it("offers an install only for what the host does not already have", async () => {
  installed.mockResolvedValue([{ id: "alpha", name: "Alpha", description: "already here" }]);
  catalogue.mockResolvedValue([
    { id: "alpha", name: "Alpha", description: "already here" },
    { id: "beta", name: "Beta", description: "not here yet", latestVersion: "2.1.0" },
  ]);

  const view = await mount();

  expect(view.buttons("Install")).toHaveLength(1);
  const ids = [...view.container.querySelectorAll("button")]
    .map(node => node.textContent);
  expect(ids).toContain("Remove");
  await view.unmount();
});

it("installs the catalogue's own latest version", async () => {
  catalogue.mockResolvedValue([
    { id: "beta", name: "Beta", description: "not here yet", latestVersion: "2.1.0" },
  ]);
  const view = await mount();

  await act(async () => view.button("Install")!.click());
  await settle();

  expect(install).toHaveBeenCalledWith("desktop_a", "beta", "2.1.0");
  // Re-read: installing moves the row between the two lists, and the version the
  // host settled on is its answer to give.
  expect(installed.mock.calls.length).toBeGreaterThan(1);
  await view.unmount();
});

it("removes an installed skill by its id", async () => {
  installed.mockResolvedValue([{ id: "alpha", name: "Alpha", description: "here" }]);
  const view = await mount();

  await act(async () => view.button("Remove")!.click());
  await settle();

  expect(remove).toHaveBeenCalledWith("desktop_a", "alpha");
  await view.unmount();
});

/** A host with no catalogue still has its installed list, so one must not blank the other. */
it("keeps the installed list when the catalogue fails", async () => {
  installed.mockResolvedValue([{ id: "alpha", name: "Alpha", description: "here" }]);
  catalogue.mockRejectedValue(new Error("agent_unavailable"));

  const view = await mount();

  expect(view.text()).toContain("Alpha");
  expect(view.text()).not.toContain("agent_unavailable");
  await view.unmount();
});

it("surfaces a failed read and a failed change", async () => {
  installed.mockRejectedValue(new Error("peer_not_connected"));
  const view = await mount();
  expect(view.text()).toContain("peer_not_connected");
  await view.unmount();

  installed.mockReset().mockResolvedValue([{ id: "alpha", name: "Alpha", description: "here" }]);
  remove.mockRejectedValue(new Error("skill_in_use"));
  const second = await mount();
  await act(async () => second.button("Remove")!.click());
  await settle();
  expect(second.text()).toContain("skill_in_use");
  await second.unmount();
});

/** Nothing is offered while the host cannot be reached: the click could not land. */
it("disables the controls when the host is not reachable", async () => {
  installed.mockResolvedValue([{ id: "alpha", name: "Alpha", description: "here" }]);
  catalogue.mockResolvedValue([{ id: "beta", name: "Beta", description: "new", latestVersion: "1" }]);

  const view = await mount({ available: false });

  expect(view.button("Remove")!.disabled).toBe(true);
  expect(view.button("Install")!.disabled).toBe(true);
  await view.unmount();
});

/** A second change waits: the host applies one at a time. */
it("holds the other controls while a change is in flight", async () => {
  installed.mockResolvedValue([{ id: "alpha", name: "Alpha", description: "here" }]);
  catalogue.mockResolvedValue([{ id: "beta", name: "Beta", description: "new", latestVersion: "1" }]);
  let release: (() => void) | null = null;
  install.mockImplementation(() => new Promise((resolve) => {
    release = () => resolve(undefined);
  }));

  const view = await mount();
  await act(async () => view.button("Install")!.click());
  expect(view.button("Remove")!.disabled).toBe(true);

  await act(async () => {
    release?.();
  });
  await settle();
  await view.unmount();
});

/**
 * A translated skill shows its translation for the app's language.
 *
 * jsdom's language is en-US, so the English side is what a mount exercises; the
 * Chinese side and the fallback directions are asserted on the pure function in
 * the client tests, where the language can be chosen.
 */
it("shows a skill in the app's language", async () => {
  installed.mockResolvedValue([
    { id: "both", name: "Both", description: "English text", nameZh: "两者", descriptionZh: "中文说明" },
  ]);
  const view = await mount();

  expect(view.text()).toContain("Both");
  expect(view.text()).toContain("English text");
  expect(view.text()).not.toContain("中文说明");
  await view.unmount();
});
