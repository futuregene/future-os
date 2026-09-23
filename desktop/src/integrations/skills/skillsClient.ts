import { invokeCommand } from "../tauri/invoke";

/** A skill the agent currently loads (source of the "Installed" tab). */
export interface InstalledSkill {
  id: string;
  name: string;
  description: string;
  nameZh: string | null;
  descriptionZh: string | null;
  version: string | null;
}

/** A skill from the platform catalogue (source of the "All" tab). */
export interface AvailableSkill {
  id: string;
  name: string;
  description: string;
  nameZh: string;
  descriptionZh: string;
  category: string;
  categoryZh: string;
  latestVersion: string | null;
  upgradeAvailable?: boolean;
}

/** Installed skills, as reconciled by the Agent SkillManager. */
export function listInstalledSkills(): Promise<InstalledSkill[]> {
  return invokeCommand<InstalledSkill[]>("list_installed_skills");
}

/** The platform skill catalogue. Requires the platform to be reachable. */
export function listAvailableSkills(): Promise<AvailableSkill[]> {
  return invokeCommand<AvailableSkill[]>("list_available_skills");
}

/** A zh/en text pair from the platform guide config. */
export interface LocalizedText {
  zh: string;
  en: string;
}

/** The platform skill-guide config (`GET /client/v1/guide`). */
export interface SkillGuide {
  links: { help: string };
  skills: {
    /** The onboarding banner's first-message prompt, per UI language. */
    coachPrompt: LocalizedText;
    /** The skill manual link the coach prompt references, per UI language. */
    manual: LocalizedText;
  };
}

/** The platform skill-guide config. Unauthenticated, like the catalogue. */
export function getSkillGuide(): Promise<SkillGuide> {
  return invokeCommand<SkillGuide>("get_skill_guide");
}

/** Download + unpack a skill version into the app scope. */
export function installSkill(id: string, version: string): Promise<void> {
  return invokeCommand<void>("install_skill", { id, version });
}

export interface SkillSyncResult {
  installed: string[];
  upgraded: string[];
  skipped: string[];
  failed: string[];
}

/** Upgrade managed installs and add unseen builtins on this host. */
export function syncSkills(): Promise<SkillSyncResult> {
  return invokeCommand<SkillSyncResult>("sync_skills");
}

/** Remove a skill from every scope it's installed in. */
export function uninstallSkill(id: string): Promise<boolean> {
  return invokeCommand<boolean>("uninstall_skill", { id });
}

/** Tell the agent to drop its skills cache and re-discover immediately. */
export function refreshSkills(): Promise<void> {
  return invokeCommand<void>("refresh_skills");
}

/** One skill candidate offered to the recommender, and the recommendation shape. */
export interface SkillCandidate {
  name: string;
  description: string;
}

/**
 * Recommend at most one uninstalled skill for the user's first-turn text via
 * the agent's Jev recommender. The caller decides when to invoke (new session,
 * first message, length cap, login/balance) and supplies the candidate set
 * (catalogue − installed). Returns null on refusal, timeout, error, or when the
 * feature is unavailable server-side — recommendation is best-effort, so every
 * failure collapses to "no recommendation" and the caller submits normally.
 */
export function suggestSkill(
  query: string,
  candidates: SkillCandidate[],
): Promise<SkillCandidate | null> {
  return invokeCommand<SkillCandidate | null>("suggest_skill", { query, candidates });
}

/** Force-run the built-in skill bootstrap (installs platform skills via CLI). */
export function bootstrapBuiltinSkills(): Promise<void> {
  return invokeCommand<void>("bootstrap_builtin_skills");
}
