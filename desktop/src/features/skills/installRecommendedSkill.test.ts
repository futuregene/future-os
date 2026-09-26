import { beforeEach, describe, expect, it, vi } from "vitest";
import { installRecommendedSkill } from "./installRecommendedSkill";

const mocks = vi.hoisted(() => ({
  installSkill: vi.fn(),
  listAvailableSkills: vi.fn(),
  refreshSkills: vi.fn(),
}));

vi.mock("../../integrations/skills/skillsClient", () => ({
  installSkill: mocks.installSkill,
  listAvailableSkills: mocks.listAvailableSkills,
  refreshSkills: mocks.refreshSkills,
}));

function entry(id: string, latestVersion: string | null) {
  return {
    id,
    name: id,
    description: "",
    nameZh: id,
    descriptionZh: "",
    category: "c",
    categoryZh: "c",
    latestVersion,
  };
}

beforeEach(() => {
  mocks.installSkill.mockReset();
  mocks.installSkill.mockResolvedValue(undefined);
  mocks.listAvailableSkills.mockReset();
  mocks.refreshSkills.mockReset();
  mocks.refreshSkills.mockResolvedValue(undefined);
});

describe("installRecommendedSkill", () => {
  it("installs the catalogue version and then re-discovers the skills", async () => {
    mocks.listAvailableSkills.mockResolvedValue([entry("other", "9.9.9"), entry("make-ppt", "2.1.0")]);

    await expect(installRecommendedSkill("make-ppt")).resolves.toBe(true);

    expect(mocks.installSkill).toHaveBeenCalledExactlyOnceWith("make-ppt", "2.1.0");
    expect(mocks.refreshSkills).toHaveBeenCalledTimes(1);
    // Order matters: the version must come from the catalogue and the refresh
    // must follow the install, or the next slash command resolves to nothing.
    expect(mocks.installSkill.mock.invocationCallOrder[0]!)
      .toBeLessThan(mocks.refreshSkills.mock.invocationCallOrder[0]!);
  });

  it("resolves false without installing when the skill is not in the catalogue", async () => {
    mocks.listAvailableSkills.mockResolvedValue([entry("other", "1.0.0")]);

    await expect(installRecommendedSkill("make-ppt")).resolves.toBe(false);

    expect(mocks.installSkill).not.toHaveBeenCalled();
    expect(mocks.refreshSkills).not.toHaveBeenCalled();
  });

  it("resolves false for a catalogue entry without a published version", async () => {
    mocks.listAvailableSkills.mockResolvedValue([entry("make-ppt", null)]);

    await expect(installRecommendedSkill("make-ppt")).resolves.toBe(false);
    expect(mocks.installSkill).not.toHaveBeenCalled();
  });

  it("takes the first match when the catalogue repeats an id", async () => {
    mocks.listAvailableSkills.mockResolvedValue([entry("make-ppt", "1.0.0"), entry("make-ppt", "2.0.0")]);

    await expect(installRecommendedSkill("make-ppt")).resolves.toBe(true);
    expect(mocks.installSkill).toHaveBeenCalledExactlyOnceWith("make-ppt", "1.0.0");
  });

  it.each([
    ["the catalogue read", () => mocks.listAvailableSkills.mockRejectedValue(new Error("platform unreachable"))],
    ["the install", () => mocks.installSkill.mockRejectedValue(new Error("download failed"))],
    ["the refresh", () => mocks.refreshSkills.mockRejectedValue(new Error("agent offline"))],
  ] as const)("resolves false when %s fails", async (_label, arrange) => {
    mocks.listAvailableSkills.mockResolvedValue([entry("make-ppt", "1.0.0")]);
    arrange();

    await expect(installRecommendedSkill("make-ppt")).resolves.toBe(false);
  });

  it("resolves false for a string rejection too", async () => {
    mocks.listAvailableSkills.mockRejectedValue("boom");

    await expect(installRecommendedSkill("make-ppt")).resolves.toBe(false);
  });

  it("forwards a CJK skill id unchanged", async () => {
    mocks.listAvailableSkills.mockResolvedValue([entry("做ppt", "3.0.0")]);

    await expect(installRecommendedSkill("做ppt")).resolves.toBe(true);
    expect(mocks.installSkill).toHaveBeenCalledExactlyOnceWith("做ppt", "3.0.0");
  });
});
