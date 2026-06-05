import { invoke } from "@tauri-apps/api/core";

export interface AgentModelOption {
  id: string;
  label: string;
  provider: string;
  supportsImages?: boolean;
  thinkingLevel?: "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | string | null;
  contextWindow?: number | null;
  isDefault?: boolean;
}

export const agentModelOptions: AgentModelOption[] = [];
export const defaultAgentModelId = "";

export async function loadAgentModelOptions() {
  return normalizeAgentModelOptions(await invoke<AgentModelOption[]>("list_agent_models"));
}

export function normalizeAgentModelOptions(models: AgentModelOption[]) {
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

export function modelSupportsImages(modelId: string, models: AgentModelOption[]) {
  return Boolean(modelOption(modelId, models)?.supportsImages);
}

export function modelThinkingLevel(modelId: string, models: AgentModelOption[]) {
  return modelOption(modelId, models)?.thinkingLevel ?? undefined;
}

function modelOption(modelId: string, models: AgentModelOption[]) {
  const normalizedId = modelId.includes("/") ? modelId.split("/").pop() ?? modelId : modelId;
  return models.find(model => model.id === modelId || model.id === normalizedId);
}
