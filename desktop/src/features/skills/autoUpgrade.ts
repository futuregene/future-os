import type { AvailableSkill, InstalledSkill } from "../../integrations/skills/skillsClient";

/** A skill that has a newer catalogue version than the one installed. */
export interface SkillUpgrade {
  id: string;
  /** The catalogue's latest version to install (overwrite). */
  version: string;
}

/**
 * The Agent marks upgrades using its managed-install and version rules.
 * This only selects rows for the manual button's count and busy state.
 */
export function computeSkillUpgrades(
  installed: InstalledSkill[],
  available: AvailableSkill[],
): SkillUpgrade[] {
  const availableById = new Map<string, AvailableSkill>();
  for (const skill of available) {
    if (!availableById.has(skill.id))
      availableById.set(skill.id, skill);
  }

  const upgrades: SkillUpgrade[] = [];
  for (const skill of installed) {
    const catalogue = availableById.get(skill.id);
    if (catalogue?.upgradeAvailable && catalogue.latestVersion)
      upgrades.push({ id: skill.id, version: catalogue.latestVersion });
  }
  return upgrades;
}
