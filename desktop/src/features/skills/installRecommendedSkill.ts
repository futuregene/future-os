import { installSkill, listAvailableSkills, refreshSkills } from "../../integrations/skills/skillsClient";
import { emitFutureEvent } from "../../lib/futureEvents";

/**
 * Install a recommended skill at its latest catalogue version, then tell the
 * agent to re-discover so the new `/name` resolves on the message that follows.
 *
 * Shared by both composers that can show a recommendation card (the
 * new-conversation and the in-conversation one) so the install policy lives in
 * one place. Resolves `false` on any failure — the caller keeps the card up so
 * the user can retry or dismiss, and never sends a slash command for a skill
 * that is not installed.
 */
export async function installRecommendedSkill(skillId: string): Promise<boolean> {
  try {
    // A skill's catalogue id equals its install directory name and its SKILL.md
    // `name`, so the id doubles as the slash-command name.
    const catalogue = await listAvailableSkills();
    const version = catalogue.find(entry => entry.id === skillId)?.latestVersion;
    if (!version)
      return false;
    await installSkill(skillId, version);
    await refreshSkills();
    // Announce the change in the renderer as well. The catalogue cache is
    // dropped by the calls above, but the left rail's count is not cache
    // driven: it re-reads `list_installed_skills` only when "skills-changed"
    // arrives, and its own install from the Skills page emits this event. The
    // backend does publish an invalidation too, but that arrives while the
    // catalog cache may still hold the pre-install list, so the badge kept the
    // old count. Emitting here — after both mutations settled — is the same
    // contract the Skills page already relies on.
    emitFutureEvent("skills-changed", undefined);
    return true;
  }
  catch {
    return false;
  }
}
