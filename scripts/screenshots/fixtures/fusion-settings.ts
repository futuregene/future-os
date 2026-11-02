import type { FusionSettings, FusionSettingsPatch } from "@future-os/thread-projection";
import { fusionSettingsIssue } from "@future-os/thread-projection";

export const fusionSettingsModels = [
  { id: "deepseek-flash", label: "DeepSeek Flash", provider: "future", reasoning: true, thinkingLevels: ["default", "low", "medium", "high"], contextWindow: 128000 },
  { id: "glm-5.3-flash", label: "GLM 5.3 Flash", provider: "future", reasoning: true, thinkingLevels: ["default", "low", "medium", "high"], contextWindow: 128000 },
  { id: "qwen3.7-plus", label: "Qwen 3.7 Plus", provider: "future", reasoning: true, thinkingLevels: ["default", "low", "medium", "high"], contextWindow: 128000 },
];
export function initialFusionSettings(): FusionSettings {
  const role = (model: string, thinking_level: string) => ({ model: `future/${model}`, thinking_level, max_output_tokens: 16384, retry_max_output_tokens: 32768 });
  return { enabled: true, revision: 0, configuration: { version: 2, moderator: role("deepseek-flash", "medium"), panel: [role("deepseek-flash", "medium"), role("glm-5.3-flash", "low"), role("qwen3.7-plus", "medium")], step_timeout_secs: 300, run_timeout_secs: 2700 } };
}
export function applyFusionSettings(current: FusionSettings, patch: FusionSettingsPatch): FusionSettings {
  if (patch.expected_revision !== current.revision) throw new Error("Fusion settings changed; reload before saving");
  if (patch.configuration && fusionSettingsIssue(patch.configuration)) throw new Error("Invalid Fusion configuration");
  return { enabled: patch.enabled ?? current.enabled, configuration: patch.configuration ?? current.configuration, revision: current.revision + 1 };
}
