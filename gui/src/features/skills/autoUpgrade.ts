import type { AvailableSkill, InstalledSkill } from "../../integrations/skills/skillsClient";
import { isUpgradeAvailable } from "./skillVersion";

/** A skill that has a newer catalogue version than the one installed. */
export interface SkillUpgrade {
  id: string;
  /** The catalogue's latest version to install (overwrite). */
  version: string;
}

/**
 * Given the installed skills and the platform catalogue, return the set of
 * skills that can be upgraded — those whose catalogue `latestVersion` is
 * strictly newer than the installed version. Pure and side-effect free so both
 * the manual "Upgrade all" button and the silent auto-upgrade share one notion
 * of "what's upgradable".
 */
export function computeSkillUpgrades(
  installed: InstalledSkill[],
  available: AvailableSkill[],
): SkillUpgrade[] {
  const latestById = new Map<string, string | null>();
  for (const skill of available) {
    if (!latestById.has(skill.id))
      latestById.set(skill.id, skill.latestVersion);
  }

  const upgrades: SkillUpgrade[] = [];
  for (const skill of installed) {
    const latest = latestById.get(skill.id) ?? null;
    if (latest && isUpgradeAvailable(skill.version, latest))
      upgrades.push({ id: skill.id, version: latest });
  }
  return upgrades;
}
