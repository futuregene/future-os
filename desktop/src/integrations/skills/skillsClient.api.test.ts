import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  bootstrapBuiltinSkills,
  getSkillGuide,
  invalidateSkillCatalog,
  listAvailableSkills,
  listInstalledSkills,
  loadSkillCatalog,
  recordSkillReco,
  refreshSkills,
  skillRecoToday,
  suggestSkill,
  syncSkills,
  uninstallSkill,
} from "./skillsClient";

const invokeMock = vi.fn<(cmd: string, args?: unknown) => Promise<unknown>>();

vi.mock("../tauri/invoke", () => ({
  invokeCommand: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}));

function commands() {
  return invokeMock.mock.calls.map(([cmd]) => cmd);
}

beforeEach(() => {
  invokeMock.mockReset();
  invalidateSkillCatalog();
  invokeMock.mockResolvedValue([]);
});

describe("skill catalogue sharing", () => {
  it("serves both lists from one pair of RPCs and reuses them", async () => {
    const installed = [{ id: "a", name: "A", description: "", nameZh: null, descriptionZh: null, version: "1.0.0" }];
    const catalogue = [{ id: "a", name: "A", description: "", nameZh: "A", descriptionZh: "", category: "c", categoryZh: "c", latestVersion: "1.0.0" }];
    invokeMock.mockImplementation((cmd: string) => Promise.resolve(cmd === "list_installed_skills" ? installed : catalogue));

    await expect(listInstalledSkills()).resolves.toBe(installed);
    await expect(listAvailableSkills()).resolves.toBe(catalogue);
    // One RPC each, shared by every reader in the same window.
    expect(commands()).toEqual(["list_installed_skills", "list_available_skills"]);
    expect(loadSkillCatalog().installed).toBe(loadSkillCatalog().installed);
  });

  it("dedupes concurrent readers onto the same in-flight requests", () => {
    const first = loadSkillCatalog();
    const second = loadSkillCatalog();

    expect(first.installed).toBe(second.installed);
    expect(commands()).toEqual(["list_installed_skills", "list_available_skills"]);
  });

  it("forgets both lists after a failed catalogue read, so the next caller retries", async () => {
    invokeMock.mockImplementation((cmd: string) => (cmd === "list_available_skills"
      ? Promise.reject(new Error("platform unreachable"))
      : Promise.resolve([])));

    const failing = loadSkillCatalog();
    await expect(failing.catalogue).rejects.toThrow("platform unreachable");
    await Promise.resolve();

    // A failed read must not poison later callers with the same stale rejection.
    invokeMock.mockImplementation((cmd: string) => Promise.resolve(cmd === "list_available_skills" ? [{ id: "b" }] : []));
    await expect(listAvailableSkills()).resolves.toEqual([{ id: "b" }]);
    expect(commands().filter(cmd => cmd === "list_available_skills")).toHaveLength(2);
  });

  it("keeps a new catalogue when a superseded failure settles later", async () => {
    // The first catalogue read fails; by then a mutation has already replaced
    // the cache, so `forget` must not clear the newer entry.
    let rejectCatalogue!: (reason: unknown) => void;
    invokeMock.mockImplementation((cmd: string) => (cmd === "list_available_skills"
      ? new Promise((_resolve, reject) => {
          rejectCatalogue = reject;
        })
      : Promise.resolve([])));

    const stale = loadSkillCatalog();
    invalidateSkillCatalog();
    invokeMock.mockImplementation((cmd: string) => Promise.resolve(cmd === "list_available_skills" ? [{ id: "fresh" }] : []));
    const fresh = loadSkillCatalog();

    rejectCatalogue(new Error("late failure"));
    await expect(stale.catalogue).rejects.toThrow("late failure");
    await Promise.resolve();

    await expect(fresh.catalogue).resolves.toEqual([{ id: "fresh" }]);
  });
});

describe("skill mutations invalidate the cached catalogue", () => {
  /** After a mutation settles, the next read must hit the agent again. */
  async function expectInvalidates(mutate: () => Promise<unknown>) {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue([]);
    await listInstalledSkills();
    expect(commands().filter(cmd => cmd === "list_installed_skills")).toHaveLength(1);

    invokeMock.mockResolvedValue({ installed: [], upgraded: [], skipped: [], failed: [] });
    await mutate();
    invokeMock.mockResolvedValue([]);
    await listInstalledSkills();

    expect(commands().filter(cmd => cmd === "list_installed_skills")).toHaveLength(2);
  }

  it("installSkill clears the cache", async () => {
    const { installSkill } = await import("./skillsClient");
    await expectInvalidates(() => installSkill("a", "1.0.0"));
    expect(commands()).toContain("install_skill");
  });

  it("uninstallSkill clears the cache", async () => {
    await expectInvalidates(() => uninstallSkill("a"));
    expect(commands()).toContain("uninstall_skill");
  });

  it("refreshSkills clears the cache", async () => {
    await expectInvalidates(() => refreshSkills());
    expect(commands()).toContain("refresh_skills");
  });

  it("syncSkills clears the cache", async () => {
    await expectInvalidates(() => syncSkills());
    expect(commands()).toContain("sync_skills");
  });

  it("still clears the cache when a mutation rejects", async () => {
    invokeMock.mockResolvedValue([]);
    await listInstalledSkills();

    invokeMock.mockRejectedValue(new Error("install failed"));
    await expect(uninstallSkill("a")).rejects.toThrow("install failed");

    invokeMock.mockResolvedValue([]);
    await listInstalledSkills();
    expect(commands().filter(cmd => cmd === "list_installed_skills")).toHaveLength(2);
  });
});

describe("skill RPC payloads", () => {
  it("forwards the recommend query and candidates verbatim", async () => {
    const candidate = { name: "make-ppt", description: "Build a deck" };
    invokeMock.mockResolvedValue(null);

    await expect(suggestSkill("做一份 PPT", [candidate])).resolves.toBeNull();
    expect(invokeMock).toHaveBeenLastCalledWith("suggest_skill", {
      query: "做一份 PPT",
      candidates: [candidate],
    });

    invokeMock.mockResolvedValue(candidate);
    await expect(suggestSkill("deck", [candidate])).resolves.toBe(candidate);
  });

  it("reads and records the daily recommendation state", async () => {
    const today = { count: 2, skillIds: ["a"], messageHashes: ["h1"] };
    invokeMock.mockResolvedValue(today);
    await expect(skillRecoToday()).resolves.toBe(today);
    expect(invokeMock).toHaveBeenLastCalledWith("skill_reco_today", undefined);

    invokeMock.mockResolvedValue(undefined);
    await expect(recordSkillReco("a", "h1")).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenLastCalledWith("record_skill_reco", { skillId: "a", messageHash: "h1" });
  });

  it("reads the guide config and bootstraps the builtin skills", async () => {
    const guide = { links: { help: "https://help.example.com" }, skills: { coachPrompt: { zh: "中", en: "en" }, manual: { zh: "中", en: "en" } } };
    invokeMock.mockResolvedValue(guide);
    await expect(getSkillGuide()).resolves.toBe(guide);

    invokeMock.mockResolvedValue(undefined);
    await expect(bootstrapBuiltinSkills()).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenLastCalledWith("bootstrap_builtin_skills", undefined);
  });
});
