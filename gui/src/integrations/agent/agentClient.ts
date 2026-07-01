import { invokeCommand } from "../tauri/invoke";

export const thinkingLevels = ["off", "minimal", "low", "medium", "high", "xhigh"] as const;
export type ThinkingLevel = typeof thinkingLevels[number];

export interface AgentModelOption {
  id: string;
  label: string;
  provider: string;
  supportsImages?: boolean;
  thinkingLevel?: ThinkingLevel | string | null;
  contextWindow?: number | null;
  isDefault?: boolean;
}

interface AgentPromptResponse {
  content: string;
}

export const defaultAgentModelId = "";
export const defaultThinkingLevel: ThinkingLevel = "off";

export async function sendPromptToFutureAgent(
  message: string,
  threadId: string,
  sessionId?: string | null,
  runId?: string | null,
  modelId?: string | null,
  imagePaths?: string[],
  thinkingLevel?: string | null,
) {
  const response = await invokeCommand<AgentPromptResponse>("agent_prompt", {
    imagePaths: imagePaths ?? [],
    message,
    sessionId: sessionId ?? null,
    threadId,
    runId: runId ?? null,
    modelId: modelId ?? null,
    thinkingLevel: thinkingLevel ?? null,
  });
  return response.content;
}

export async function loadAgentModelOptions() {
  return normalizeAgentModelOptions(await invokeCommand<AgentModelOption[]>("list_agent_models"));
}

function normalizeAgentModelOptions(models: AgentModelOption[]) {
  const seen = new Set<string>();
  return models
    .filter(model => model.id.trim().length > 0)
    .filter((model) => {
      const key = `${model.provider}/${model.id}`;
      if (seen.has(key))
        return false;
      seen.add(key);
      return true;
    });
}

/**
 * Provider-qualified model identifier (`provider/id`) — the canonical id passed
 * around the GUI and down to the agent. A bare `id` is ambiguous: two providers
 * can expose the same model id, and the agent resolves a bare id to the first
 * match, which may be the wrong provider (wrong base URL / API key). The agent's
 * `resolve()` handles the `provider/model` form exactly.
 */
export function modelKey(model: Pick<AgentModelOption, "id" | "provider">) {
  return model.provider ? `${model.provider}/${model.id}` : model.id;
}

export function defaultModelId(models: AgentModelOption[]) {
  const preferred = models.find(model => model.isDefault) ?? models[0];
  return preferred ? modelKey(preferred) : defaultAgentModelId;
}

export function modelLabel(modelId: string, models: AgentModelOption[]) {
  return modelOption(modelId, models)?.label ?? (modelId || "Model");
}

export function modelThinkingLevel(modelId: string, models: AgentModelOption[]) {
  return modelOption(modelId, models)?.thinkingLevel ?? undefined;
}

export function normalizeThinkingLevel(level?: string | null): ThinkingLevel {
  return thinkingLevels.includes(level as ThinkingLevel) ? level as ThinkingLevel : defaultThinkingLevel;
}

export function modelOption(modelId: string, models: AgentModelOption[]) {
  // Prefer an exact provider-qualified match so we resolve the right provider
  // when several expose the same model id.
  const exact = models.find(model => modelKey(model) === modelId);
  if (exact)
    return exact;
  // Fall back to a bare-id match for legacy selections / threads persisted
  // before ids were provider-qualified (ambiguous, so first match wins).
  const bareId = modelId.includes("/") ? modelId.split("/").pop() ?? modelId : modelId;
  return models.find(model => model.id === bareId);
}
