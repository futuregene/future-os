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

/**
 * The two lists, shared by every reader in this renderer.
 *
 * Three screens need them — the composer's mention completion, the skills page,
 * and the recommendation hook — and each read is an RPC that opens the agent's
 * database on the other side. Sending one message used to fire four of them
 * (two as the composer mounted, two from the recommender), and the burst
 * competed with session persistence for the agent's write lock: the agent
 * logged `Session persistence failed: database is locked` immediately after a
 * run of `list_installed_skills` / `list_available_skills`.
 *
 * Readers in the same tick share one in-flight request, and the resolved lists
 * are reused until a mutation clears them (see `invalidateSkillCatalog`, called
 * by every function below that changes what is installed). The agent stays the
 * single source of truth: this holds no state of its own, only the answer to a
 * question that cannot have changed in between.
 */
let catalog: { installed: Promise<InstalledSkill[]>; catalogue: Promise<AvailableSkill[]> } | null
  = null;

/** Installed skills, as reconciled by the Agent SkillManager. */
export function listInstalledSkills(): Promise<InstalledSkill[]> {
  return loadSkillCatalog().installed;
}

/** The platform skill catalogue. Requires the platform to be reachable. */
export function listAvailableSkills(): Promise<AvailableSkill[]> {
  return loadSkillCatalog().catalogue;
}

/**
 * Both lists, fetched together and reused until invalidated.
 *
 * The catalogue can fail on its own (it needs the platform); that rejection is
 * passed through, because each caller tolerates it differently, and it clears
 * the cache so the next call retries rather than handing every later caller the
 * same failure.
 */
export function loadSkillCatalog(): {
  installed: Promise<InstalledSkill[]>;
  catalogue: Promise<AvailableSkill[]>;
} {
  if (catalog)
    return catalog;
  const entry = {
    installed: invokeCommand<InstalledSkill[]>("list_installed_skills"),
    catalogue: invokeCommand<AvailableSkill[]>("list_available_skills"),
  };
  // Attaching a handler (here, clearing the cache) also marks the rejection as
  // observed, so a caller that never awaits the catalogue cannot crash the app.
  entry.installed.catch(() => forget(entry));
  entry.catalogue.catch(() => forget(entry));
  catalog = entry;
  return entry;
}

/** Drop the cached lists. Called by every mutation, so no caller can forget. */
export function invalidateSkillCatalog(): void {
  catalog = null;
}

/**
 * Clear the cache once a mutation settles.
 *
 * Clearing *before* the call would leave a window in which a read started
 * mid-install is cached and then outlives the install; clearing on settle means
 * a stale list can never survive the mutation that changed it.
 */
function invalidateAfter<T>(call: Promise<T>): Promise<T> {
  return call.finally(invalidateSkillCatalog);
}

function forget(entry: { installed: unknown; catalogue: unknown }): void {
  if (catalog === entry)
    catalog = null;
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
  return invalidateAfter(invokeCommand<void>("install_skill", { id, version }));
}

export interface SkillSyncResult {
  installed: string[];
  upgraded: string[];
  skipped: string[];
  failed: string[];
}

/** Upgrade managed installs and add unseen builtins on this host. */
export function syncSkills(): Promise<SkillSyncResult> {
  return invalidateAfter(invokeCommand<SkillSyncResult>("sync_skills"));
}

/** Remove a skill from every scope it's installed in. */
export function uninstallSkill(id: string): Promise<boolean> {
  return invalidateAfter(invokeCommand<boolean>("uninstall_skill", { id }));
}

/** Tell the agent to drop its skills cache and re-discover immediately. */
export function refreshSkills(): Promise<void> {
  return invalidateAfter(invokeCommand<void>("refresh_skills"));
}

/** One skill candidate offered to the recommender, and the recommendation shape. */
export interface SkillCandidate {
  name: string;
  description: string;
}

/**
 * Recommend at most one uninstalled skill for the user's text via the agent's
 * Jev recommender. The caller decides when to invoke (length cap, login/balance,
 * daily budget) and supplies the candidate set (catalogue − installed). Returns
 * null on refusal, timeout, error, or when the feature is unavailable
 * server-side — recommendation is best-effort, so every failure collapses to
 * "no recommendation" and the caller submits normally.
 */
export function suggestSkill(
  query: string,
  candidates: SkillCandidate[],
): Promise<SkillCandidate | null> {
  return invokeCommand<SkillCandidate | null>("suggest_skill", { query, candidates });
}

/**
 * Today's recommendation state. `count` is the number of cards actually shown
 * (the daily budget counts recommendations, not calls); `skillIds` and
 * `messageHashes` are the duplicate checks.
 */
export interface SkillRecoToday {
  count: number;
  skillIds: string[];
  messageHashes: string[];
}

/** Today's recommendation state, for the daily budget and duplicate checks. */
export function skillRecoToday(): Promise<SkillRecoToday> {
  return invokeCommand<SkillRecoToday>("skill_reco_today");
}

/**
 * Record one recommendation that was actually shown. Only shown
 * recommendations consume the daily budget — a call that recommended nothing
 * must not call this.
 */
export function recordSkillReco(skillId: string, messageHash: string): Promise<void> {
  return invokeCommand<void>("record_skill_reco", { skillId, messageHash });
}

/** Force-run the built-in skill bootstrap (installs platform skills via CLI). */
export function bootstrapBuiltinSkills(): Promise<void> {
  return invokeCommand<void>("bootstrap_builtin_skills");
}
