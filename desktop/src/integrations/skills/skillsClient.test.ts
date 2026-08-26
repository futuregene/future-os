import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeCommand } from "../tauri/invoke";
import {
  bootstrapBuiltinSkills,
  getSkillGuide,
  installSkill,
  listAvailableSkills,
  listInstalledSkills,
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
