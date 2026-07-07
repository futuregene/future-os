import i18n from "../../i18n";
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
  /**
   * False when the agent stream ended before a clean `agent_end` — the content
   *  is a truncated prefix and the caller should finalize the run as failed
   *  rather than completed (RUN-05). Older backends omit it; treat missing as
   *  complete so nothing regresses.
   */
  complete?: boolean;
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
  return { content: response.content, complete: response.complete !== false };
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

/** Built-in FutureGene provider id (display name "FutureGene"). */
const FUTURE_PROVIDER_ID = "future";
/** localStorage key holding the last user-picked model, as `provider/id`. */
const LAST_USED_MODEL_STORAGE_KEY = "futureos:last-used-model";

/** Persist the last model the user picked in the composer, for the next launch. */
export function rememberLastUsedModel(modelId: string): void {
  if (!modelId)
    return;
  try {
    window.localStorage.setItem(LAST_USED_MODEL_STORAGE_KEY, modelId);
  }
  catch {
    // localStorage may be unavailable (private mode / disabled) — best effort.
  }
}

function readLastUsedModel(): string | null {
  try {
    return window.localStorage.getItem(LAST_USED_MODEL_STORAGE_KEY);
  }
  catch {
    return null;
  }
}

/**
 * Pick the model to select when there is no valid in-session choice yet.
 * Priority: the last user-picked model (if it still exists) → the first
 * FutureGene model → the first model in the catalog.
 */
export function resolveInitialModelId(models: AgentModelOption[]): string {
  const lastUsed = readLastUsedModel();
  if (lastUsed && modelOption(lastUsed, models))
    return lastUsed;
  const future = models.find(model => model.provider === FUTURE_PROVIDER_ID);
  if (future)
    return modelKey(future);
  return models[0] ? modelKey(models[0]) : defaultAgentModelId;
}

export function modelLabel(modelId: string, models: AgentModelOption[]) {
  return modelOption(modelId, models)?.label ?? (modelId || i18n.t("common:modelFallback"));
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
