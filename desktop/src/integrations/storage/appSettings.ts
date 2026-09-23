import { invokeCommand } from "../tauri/invoke";

/** Approval tier: fully open (default), ask everything, or use the available OS sandbox. */
export type ApprovalTier = "off" | "manual" | "sandbox";

export interface AppSettings {
  approvalTier: ApprovalTier;
  hiddenModels: string[];
  /**
   * Silently upgrade installed skills to their latest version on app open (and
   * immediately when toggled on). Off by default.
   */
  autoUpgradeSkills: boolean;
  /**
   * Auto-connect the single paired remote device on app launch (the user can
   * still disconnect by hand). Off by default. The Remote feature is dev-only,
   * so this only takes effect on non-release builds.
   */
  autoConnectRemote: boolean;
  /**
   * The user closed the skill-onboarding banner on the new-conversation
   * screen. Off by default (the banner shows until dismissed).
   */
  skillGuideDismissed: boolean;
  /**
   * The user acknowledged the Skills nav-entry intro bubble (去看看 / 知道了
   * / click-outside). Off by default; once set, the bubble and its blue dot
   * never show again (until app data is wiped).
   */
  skillIntroDismissed: boolean;
  /**
   * Play a completion bell and request window attention when an agent run
   * finishes. On by default.
   */
  bellOnComplete: boolean;
  /** Generate and save a title after the first answer, without compacting context. On by default. */
  autoTitleFirstTurn: boolean;
  /** UI language mirrored for backend title generation while the webview is suspended. */
  titleLanguage: "en" | "zh";
  /** Community-edition UI hides billing surfaces and treats Future like a normal builtin provider. */
  communityEdition: boolean;
  /**
   * Recommend at most one uninstalled skill when a message is sent (PRD v1.6
   * §3, any turn — not just a conversation's first). **On by default**; the
   * Settings toggle opts out.
   */
  skillRecommend: boolean;
}

/** Fallback used before the persisted settings load. */
export const DEFAULT_APP_SETTINGS: AppSettings = {
  approvalTier: "off",
  hiddenModels: [],
  autoUpgradeSkills: false,
  autoConnectRemote: false,
  skillGuideDismissed: false,
  skillIntroDismissed: false,
  bellOnComplete: true,
  autoTitleFirstTurn: true,
  titleLanguage: "en",
  communityEdition: false,
  skillRecommend: true,
};

export async function getAppSettings() {
  return invokeCommand<AppSettings>("get_app_settings");
}

export async function updateAppSettings(input: {
  approvalTier?: ApprovalTier;
  hiddenModels?: string[];
  autoUpgradeSkills?: boolean;
  autoConnectRemote?: boolean;
  skillGuideDismissed?: boolean;
  skillIntroDismissed?: boolean;
  bellOnComplete?: boolean;
  autoTitleFirstTurn?: boolean;
  titleLanguage?: "en" | "zh";
  communityEdition?: boolean;
  skillRecommend?: boolean;
}) {
  return invokeCommand<AppSettings>("update_app_settings", { input });
}
