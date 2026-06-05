export interface AgentModelOption {
  id: string;
  label: string;
  provider: string;
  supportsImages?: boolean;
  thinkingLevel?: "off" | "xhigh";
}

export const agentModelOptions = [
  {
    id: "deepseek-v4-flash",
    label: "DeepSeek V4 Flash",
    provider: "deepseek",
    thinkingLevel: "off",
  },
  {
    id: "deepseek-v4-pro",
    label: "DeepSeek V4 Pro",
    provider: "deepseek",
    thinkingLevel: "off",
  },
  {
    id: "qwen3.6-plus",
    label: "Qwen 3.6 Plus",
    provider: "dashscope-coding",
    supportsImages: true,
    thinkingLevel: "xhigh",
  },
] satisfies AgentModelOption[];

export const defaultAgentModelId = agentModelOptions[0].id;

export function modelLabel(modelId: string) {
  return agentModelOptions.find(model => model.id === modelId)?.label ?? modelId;
}

export function modelSupportsImages(modelId: string) {
  return Boolean(modelOption(modelId)?.supportsImages);
}

export function modelThinkingLevel(modelId: string) {
  return modelOption(modelId)?.thinkingLevel;
}

function modelOption(modelId: string) {
  const normalizedId = modelId.includes("/") ? modelId.split("/").pop() ?? modelId : modelId;
  return agentModelOptions.find(model => model.id === normalizedId);
}
