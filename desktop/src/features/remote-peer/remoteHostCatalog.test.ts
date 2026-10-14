// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";
import {
  forgetHostCatalogue,
  hostDesktopSettings,
  hostModels,
  hostSkillCatalogue,
  useHostCatalogueInvalidation,
  warmHostCatalogue,
} from "./remoteHostCatalog";

/**
 * The host's own catalogue: read once per connection, dropped when it moves.
 *
 * These reads describe a *machine*, and reading them per conversation opened put
 * four commands ahead of the page that draws the transcript — half of them paid
 * again for every other conversation on the same host. What is asserted here is
 * the lifetime that keeps them off that path.
 */

const request = vi.fn();

/** What a healthy host answers, per command. */
async function healthyAnswer(_desktopId: unknown, command: unknown) {
  const type = (command as { type?: string }).type;
  if (type === "list_models")
    return { models: [{ id: "gpt-5", provider: "openai" }] };
  if (type === "get_desktop_settings")
    return { skillRecommend: true };
  if (type === "list_available_skills")
    return { skills: [{ id: "pdf-tools", name: "PDF tools", description: "Read PDFs" }] };
  if (type === "list_skills")
    return { skills: [{ id: "installed", name: "Installed", description: "d" }] };
  return {};
}

vi.mock("./remotePeerClient", () => ({
  getRemoteDesktopSettings: async (desktopId: string) => {
    const raw = await request(desktopId, { type: "get_desktop_settings" }) as { skillRecommend?: boolean };
    return { skillRecommend: raw?.skillRecommend === true };
  },
  listRemoteAvailableSkills: (desktopId: string) =>
    request(desktopId, { type: "list_available_skills" }).then(
      (raw: { skills?: { id: string; name: string; description: string }[] }) =>
        (raw?.skills ?? []).map(skill => ({ ...skill, latestVersion: "1.0.0" })),
    ),
  listRemoteInstalledSkills: (desktopId: string) =>
    request(desktopId, { type: "list_skills" }).then(
      (raw: { skills?: unknown[] }) => (raw?.skills ?? []) as never,
    ),
  listRemoteModels: (desktopId: string) =>
    request(desktopId, { type: "list_models" }).then(
      (raw: { models?: unknown[] }) => (raw?.models ?? []) as never,
    ),
}));

let listeners: Array<(event: { payload: unknown }) => void> = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.push(handler);
    return () => {
      listeners = listeners.filter(item => item !== handler);
    };
  },
}));

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** The command types asked of one host, in order. */
function asked(desktopId = "desktop_a"): string[] {
  return request.mock.calls
    .filter(([id]) => id === desktopId)
    .map(([, command]) => (command as { type: string }).type);
}

/** A `_global` announcement, as the host publishes one to its paired clients. */
function announce(type: string, desktopId = "desktop_a"): void {
  act(() => {
    listeners.forEach(listener => listener({
      payload: { desktopId, kind: "event", payload: { sessionId: "_global", type } },
    }));
  });
}

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i += 1) await flushAsync();
}

beforeEach(() => {
  listeners = [];
  // Reset, not clear: a test that makes a host refuse must not leave the next
  // one with a host that refuses everything.
  request.mockReset().mockImplementation(healthyAnswer);
  forgetHostCatalogue("desktop_a");
  forgetHostCatalogue("desktop_b");
});

describe("the cached reads", () => {
  it("asks a host once however many times it is read", async () => {
    const [first, second] = await Promise.all([hostModels("desktop_a"), hostModels("desktop_a")]);
    expect(first).toEqual([{ id: "gpt-5", provider: "openai" }]);
    // The second ask is the same promise, not a second command.
    expect(second).toBe(first);
    await hostModels("desktop_a");
    expect(asked()).toEqual(["list_models"]);

    // And per host: another machine is its own read.
    await hostModels("desktop_b");
    expect(asked("desktop_b")).toEqual(["list_models"]);
  });

  it("reads a host's own settings and skills once each", async () => {
    await hostDesktopSettings("desktop_a");
    await hostDesktopSettings("desktop_a");
    const catalogue = await hostSkillCatalogue("desktop_a");
    expect(catalogue.available[0]).toMatchObject({ id: "pdf-tools", latestVersion: "1.0.0" });
    expect(catalogue.installed).toEqual([{ id: "installed", name: "Installed", description: "d" }]);
    expect(asked()).toEqual(["get_desktop_settings", "list_available_skills", "list_skills"]);
  });

  /**
   * A failure is not a catalogue.
   *
   * A refused read that stayed cached would serve "this host has no models" for
   * the rest of the window, and every later ask — the picker opening, the card
   * being evaluated — would be answered from it.
   */
  it("does not remember a failure", async () => {
    request.mockRejectedValueOnce(new Error("peer_not_connected"));
    await expect(hostModels("desktop_a")).rejects.toThrow("peer_not_connected");

    request.mockResolvedValueOnce({ models: [{ id: "gpt-5" }] });
    await expect(hostModels("desktop_a")).resolves.toEqual([{ id: "gpt-5" }]);
    expect(asked()).toEqual(["list_models", "list_models"]);
  });

  it("drops one domain without touching the others", async () => {
    await hostModels("desktop_a");
    await hostDesktopSettings("desktop_a");
    forgetHostCatalogue("desktop_a", "models");
    await hostModels("desktop_a");
    await hostDesktopSettings("desktop_a");
    expect(asked()).toEqual(["list_models", "get_desktop_settings", "list_models"]);

    // And all of it, when the connection that answered is gone.
    forgetHostCatalogue("desktop_a");
    await hostModels("desktop_a");
    expect(asked()).toEqual([
      "list_models",
      "get_desktop_settings",
      "list_models",
      "list_models",
    ]);
  });
});

describe("warming a connection", () => {
  it("reads a host's models and settings, and its skills when it wants recommendations", async () => {
    warmHostCatalogue("desktop_a");
    await settle();
    expect(asked()).toEqual([
      "list_models",
      "get_desktop_settings",
      "list_available_skills",
      "list_skills",
    ]);
  });

  /** A host that wants no recommendations is not asked for a candidate set. */
  it("leaves a host's skills alone when it wants no recommendations", async () => {
    request.mockImplementation(async (_desktopId: unknown, command: unknown) => {
      const type = (command as { type?: string }).type;
      if (type === "list_models")
        return { models: [] };
      if (type === "get_desktop_settings")
        return { skillRecommend: false };
      return {};
    });
    warmHostCatalogue("desktop_a");
    await settle();
    expect(asked()).toEqual(["list_models", "get_desktop_settings"]);
  });

  /**
   * A background fill cannot fail the connection it runs behind: a host that
   * never answers costs one refused request and nothing else.
   */
  it("swallows a host that cannot answer anything", async () => {
    request.mockRejectedValue(new Error("peer_not_connected"));
    warmHostCatalogue("desktop_a");
    await settle();
    // Every read was attempted, nothing was remembered, and nothing threw.
    expect(asked()).toEqual(["list_models", "get_desktop_settings"]);
    await expect(hostModels("desktop_a")).rejects.toThrow("peer_not_connected");
  });
});

describe("the host's own announcements", () => {
  it("drops the domain an announcement names, and says which it was", async () => {
    const moved: string[] = [];
    const hook = renderHook(() => useHostCatalogueInvalidation("desktop_a", domain => void moved.push(domain)));
    await settle();

    await hostModels("desktop_a");
    await hostDesktopSettings("desktop_a");
    await hostSkillCatalogue("desktop_a");
    expect(asked()).toEqual([
      "list_models",
      "get_desktop_settings",
      "list_available_skills",
      "list_skills",
    ]);

    announce("model_visibility_changed");
    announce("provider_config_changed");
    announce("app_settings_changed");
    announce("skills_changed");
    await settle();

    expect(moved).toEqual(["models", "models", "settings", "skills"]);
    // Every read behind an announcement is a real one — the domain it named was
    // dropped — and only that one: the others are still the answers they were.
    await hostModels("desktop_a");
    await hostDesktopSettings("desktop_a");
    await hostSkillCatalogue("desktop_a");
    expect(asked()).toEqual([
      "list_models",
      "get_desktop_settings",
      "list_available_skills",
      "list_skills",
      "list_models",
      "get_desktop_settings",
      "list_available_skills",
      "list_skills",
    ]);
    hook.unmount();
  });

  it("ignores an event that is not about a catalogue, and another host's", async () => {
    const moved: string[] = [];
    const hook = renderHook(() => useHostCatalogueInvalidation("desktop_a", domain => void moved.push(domain)));
    await settle();

    announce("text_chunk");
    announce("model_visibility_changed", "desktop_b");
    // A presence tick is not a session event, and carries no type to drop.
    act(() => {
      listeners.forEach(listener => listener({ payload: { desktopId: "desktop_a", kind: "presence", payload: {} } }));
    });
    await settle();

    expect(moved).toEqual([]);
    hook.unmount();
  });

  it("follows nothing while no host is open", async () => {
    const moved: string[] = [];
    const hook = renderHook(() => useHostCatalogueInvalidation(null, domain => void moved.push(domain)));
    await settle();

    announce("skills_changed");
    await settle();

    expect(moved).toEqual([]);
    hook.unmount();
  });
});
