import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeCommand } from "../tauri/invoke";
import {
  bootstrapBuiltinSkills,
  getSkillGuide,
  installSkill,
  invalidateSkillCatalog,
  listAvailableSkills,
  listInstalledSkills,
  loadSkillCatalog,
  refreshSkills,
  uninstallSkill,
} from "./skillsClient";

vi.mock("../tauri/invoke", () => ({
  invokeCommand: vi.fn(),
}));

describe("skillsClient", () => {
  beforeEach(() => {
    vi.mocked(invokeCommand).mockReset();
    vi.mocked(invokeCommand).mockResolvedValue(undefined);
    invalidateSkillCatalog();
  });

  describe("the shared skill catalog", () => {
    /// Sending one message used to fire four of these RPCs (the composer's two
    /// plus the recommender's two), and each one opens the agent's database —
    /// the burst that produced "database is locked" in the agent log.
    it("reads each list once when several readers ask at the same time", async () => {
      vi.mocked(invokeCommand).mockResolvedValue([]);
      await Promise.all([
        listInstalledSkills(),
        listAvailableSkills(),
        listInstalledSkills(),
        listAvailableSkills(),
      ]);
      const calls = vi.mocked(invokeCommand).mock.calls.map(([name]) => name);
      expect(calls.filter(name => name === "list_installed_skills")).toHaveLength(1);
      expect(calls.filter(name => name === "list_available_skills")).toHaveLength(1);
    });

    it("keeps serving the same lists until something changes", async () => {
      vi.mocked(invokeCommand).mockResolvedValue([]);
      await listInstalledSkills();
      await listInstalledSkills();
      // Both lists come back from one read, however many readers ask.
      expect(invokeCommand).toHaveBeenCalledTimes(2);
      const names = vi.mocked(invokeCommand).mock.calls.map(([name]) => name);
      expect(names).toEqual(["list_installed_skills", "list_available_skills"]);
    });

    it("re-reads after an install, so the next read is not stale", async () => {
      vi.mocked(invokeCommand).mockResolvedValue([]);
      await listInstalledSkills();
      await installSkill("future-web", "1.0");
      await listInstalledSkills();
      expect(
        vi.mocked(invokeCommand).mock.calls.filter(([name]) => name === "list_installed_skills"),
      ).toHaveLength(2);
    });

    it("retries after a failure instead of caching the rejection", async () => {
      // The two lists are read together, and either can fail on its own (the
      // catalogue needs the platform). A failure must not be cached, or every
      // later caller inherits it.
      const failing = (name: string) =>
        name === "list_available_skills"
          ? Promise.reject(new Error("agent down"))
          : Promise.resolve([]);
      vi.mocked(invokeCommand).mockImplementation(failing as never);
      await expect(listAvailableSkills()).rejects.toThrow("agent down");

      vi.mocked(invokeCommand).mockResolvedValue([{ id: "s1" }]);
      await expect(listAvailableSkills()).resolves.toEqual([{ id: "s1" }]);
    });

    it("does not report an unobserved rejection when nothing awaits the catalogue", async () => {
      vi.mocked(invokeCommand).mockRejectedValue(new Error("platform unreachable"));
      const { catalogue } = loadSkillCatalog();
      catalogue.catch(() => undefined);
      // Let the rejection land; an unhandled one would fail the test run itself.
      await new Promise(resolve => setTimeout(resolve, 0));
    });
  });

  it("lists installed skills", async () => {
    vi.mocked(invokeCommand).mockResolvedValue([{ id: "s1" }]);
    await expect(listInstalledSkills()).resolves.toEqual([{ id: "s1" }]);
    expect(invokeCommand).toHaveBeenCalledWith("list_installed_skills");
  });

  it("lists available skills", async () => {
    await listAvailableSkills();
    expect(invokeCommand).toHaveBeenCalledWith("list_available_skills");
  });

  it("fetches the skill guide", async () => {
    await getSkillGuide();
    expect(invokeCommand).toHaveBeenCalledWith("get_skill_guide");
  });

  it("installs a skill version", async () => {
    await installSkill("s1", "1.0.0");
    expect(invokeCommand).toHaveBeenCalledWith("install_skill", { id: "s1", version: "1.0.0" });
  });

  it("uninstalls a skill", async () => {
    await uninstallSkill("s1");
    expect(invokeCommand).toHaveBeenCalledWith("uninstall_skill", { id: "s1" });
  });

  it("refreshes skills", async () => {
    await refreshSkills();
    expect(invokeCommand).toHaveBeenCalledWith("refresh_skills");
  });

  it("bootstraps built-in skills", async () => {
    await bootstrapBuiltinSkills();
    expect(invokeCommand).toHaveBeenCalledWith("bootstrap_builtin_skills");
  });
});
