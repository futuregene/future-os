/** Host-owned settings; identical model identities can have independent roles. */
export interface FusionModelConfig {
  model: string;
  thinking_level: string;
  max_output_tokens: number;
  retry_max_output_tokens: number;
}
export interface FusionConfiguration {
  version: number;
  panel: FusionModelConfig[];
  moderator: FusionModelConfig;
  step_timeout_secs: number;
  run_timeout_secs: number;
}
export interface FusionSettings { enabled: boolean; configuration: FusionConfiguration; revision: number }
export interface FusionSettingsPatch { expected_revision: number; enabled?: boolean; configuration?: FusionConfiguration }
export type FusionSettingsIssue = "moderatorRequired" | "participantCount" | "participantRequired" | "thinkingRequired";
export function fusionSettingsIssue(config: FusionConfiguration): FusionSettingsIssue | null {
  if (!config.moderator.model.trim()) return "moderatorRequired";
  if (config.panel.length < 2 || config.panel.length > 5) return "participantCount";
  if (config.panel.some(model => !model.model.trim())) return "participantRequired";
  if ([config.moderator, ...config.panel].some(model => !model.thinking_level.trim())) return "thinkingRequired";
  return null;
}
export function newFusionParticipant(): FusionModelConfig {
  return { model: "", thinking_level: "default", max_output_tokens: 16384, retry_max_output_tokens: 32768 };
}
