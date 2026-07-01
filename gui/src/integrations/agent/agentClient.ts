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

export function defaultModelId(models: AgentModelOption[]) {
  return models.find(model => model.isDefault)?.id ?? models[0]?.id ?? defaultAgentModelId;
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

function modelOption(modelId: string, models: AgentModelOption[]) {
  const normalizedId = modelId.includes("/") ? modelId.split("/").pop() ?? modelId : modelId;
  return models.find(model => model.id === modelId || model.id === normalizedId);
}
