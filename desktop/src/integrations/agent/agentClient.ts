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
  /** Curated positioning blurb (Chinese) from the Future platform catalog; absent elsewhere. */
  description?: string | null;
  /** English counterpart of `description`; shown when the UI language is not Chinese. */
  descriptionEn?: string | null;
  /** Future-platform recommendation flag (drives the onboarding model picker). */
  recommended?: boolean;
}

/** Attachment wire shape accepted by the agent_prompt Tauri command. */
export interface AttachmentInput {
  path: string;
  kind: "image" | "file";
  name: string;
  /** Cached-thumbnail path persisted in entry metadata for message reload. */
  thumbnail?: string;
}

interface AgentPromptResponse {
  content: string;
  /** False when the agent stream ended before a clean `agent_end`. */
  complete?: boolean;
  /** The agent session id — persisted on the thread for subsequent prompts. */
  sessionId?: string;
  /**
   * True when the thread's previous agent session was gone (data lost or cwd
   * drift) and a fresh empty session replaced it — prior agent-side context
   * is unavailable even though the GUI still shows the history.
   */
  sessionRecreated?: boolean;
}

export const defaultAgentModelId = "";
export const defaultThinkingLevel: ThinkingLevel = "off";

export interface AgentPromptInput {
  message: string;
  modelContext?: string;
  threadId: string;
  sessionId?: string | null;
  runId?: string | null;
  modelId?: string | null;
  attachments?: AttachmentInput[];
  thinkingLevel?: string | null;
}

export async function sendPromptToFutureAgent({
  message,
  modelContext,
  threadId,
  sessionId,
  runId,
  modelId,
  attachments,
  thinkingLevel,
}: AgentPromptInput) {
  const response = await invokeCommand<AgentPromptResponse>("agent_prompt", {
    request: {
      attachments: attachments ?? [],
      message,
      modelContext: modelContext ?? "",
      sessionId: sessionId ?? null,
      threadId,
      runId: runId ?? null,
      modelId: modelId ?? null,
      thinkingLevel: thinkingLevel ?? null,
    },
  });
  return {
    content: response.content,
    complete: response.complete !== false,
    sessionId: response.sessionId,
    sessionRecreated: response.sessionRecreated === true,
  };
}

export async function loadAgentModelOptions() {
  return normalizeAgentModelOptions(await invokeCommand<AgentModelOption[]>("list_agent_models"));
}

/** Result of the post-login Future model sync (agent warms its cache + registry). */
export interface SyncFutureModelsResult {
  synced: boolean;
  modelCount: number;
}

/**
 * Post-login init: synchronously fetch the Future provider's models inside the
 * agent (warming its cache and rebuilding its registry) so the model list is
 * complete before the onboarding gate closes. Blocks on the platform fetch —
 * only call from the onboarding init flow.
 */
export async function syncFutureModels() {
  return invokeCommand<SyncFutureModelsResult>("sync_future_models");
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

/**
 * Pick the catalog blurb that matches the UI language. The Future platform
 * supplies a Chinese `description` and an English `descriptionEn`; return the
 * variant matching `language` (everything except "en" is treated as Chinese,
 * mirroring the `i18n.language !== "en"` convention used elsewhere), falling
 * back to the other variant when the preferred one is missing so a model row is
 * never left blank. Trimmed; `null` when neither variant is present.
 *
 * Pure on purpose: callers pass `i18n.language` in rather than this helper
 * importing i18n, so it stays testable and side-effect free.
 */
export function localizedModelDescription(
  model: Pick<AgentModelOption, "description" | "descriptionEn">,
  language: string | undefined,
): string | null {
  const preferred = language === "en" ? model.descriptionEn : model.description;
  const fallback = language === "en" ? model.description : model.descriptionEn;
  return preferred?.trim() || fallback?.trim() || null;
}

/** Built-in Future provider id (display name "Future"). */
const FUTURE_PROVIDER_ID = "future";
/** localStorage key holding the last user-picked model, as `provider/id`. */
const LAST_USED_MODEL_STORAGE_KEY = "futureos:last-used-model";
/** localStorage key holding the last user-picked thinking level. */
const LAST_USED_THINKING_LEVEL_STORAGE_KEY = "futureos:last-used-thinking-level";

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

export function readLastUsedModel(): string | null {
  try {
    return window.localStorage.getItem(LAST_USED_MODEL_STORAGE_KEY);
  }
  catch {
    return null;
  }
}

/**
 * Pick the model to select when there is no valid in-session choice yet.
 * Priority: the last user-picked model → Future's deepseek-v4-pro → the
 * first Future model → the first model in the catalog.
 *
 * When the catalog is empty (models haven't loaded yet), returns the
 * last-used model or the empty fallback — harmless because reconciliation
 * will correct it once the catalog arrives.
 */
export function resolveInitialModelId(models: AgentModelOption[]): string {
  const lastUsed = readLastUsedModel();
  // Last-used model is valid in the current catalog? Use it.
  if (lastUsed && modelOption(lastUsed, models))
    return lastUsed;
  // Catalog not loaded yet — return last-used as best-effort seed (if any).
  if (models.length === 0)
    return lastUsed || defaultAgentModelId;
  // Full catalog available — apply provider-aware priority.
  const futureModels = models.filter(model => model.provider === FUTURE_PROVIDER_ID);
  const dsv4 = futureModels.find(model => model.id === "deepseek-v4-pro");
  if (dsv4)
    return modelKey(dsv4);
  if (futureModels.length > 0)
    return modelKey(futureModels[0]!);
  return models[0] ? modelKey(models[0]) : defaultAgentModelId;
}

/** Persist the last thinking level the user picked in the composer. */
export function rememberLastUsedThinkingLevel(level: string): void {
  if (!level)
    return;
  try {
    window.localStorage.setItem(LAST_USED_THINKING_LEVEL_STORAGE_KEY, level);
  }
  catch {
    // localStorage may be unavailable (private mode / disabled) — best effort.
  }
}

export function readLastUsedThinkingLevel(): string | null {
  try {
    return window.localStorage.getItem(LAST_USED_THINKING_LEVEL_STORAGE_KEY);
  }
  catch {
    return null;
  }
}

/**
 * Pick the thinking level for a fresh draft. Priority: the last user-picked
 * level (if still a valid level) → the model's own default thinking level.
 */
export function resolveInitialThinkingLevel(modelId: string, models: AgentModelOption[]): ThinkingLevel {
  const lastUsed = readLastUsedThinkingLevel();
  if (lastUsed && thinkingLevels.includes(lastUsed as ThinkingLevel))
    return lastUsed as ThinkingLevel;
  return normalizeThinkingLevel(modelThinkingLevel(modelId, models));
}

/**
 * Display label for a model id, or `undefined` when there's no match and no id
 * to fall back on. The integration layer stays i18n-free: call sites supply a
 * localized fallback (e.g. `modelLabel(...) ?? t("common:modelFallback")`).
 */
export function modelLabel(modelId: string, models: AgentModelOption[]): string | undefined {
  return modelOption(modelId, models)?.label ?? (modelId || undefined);
}

export function modelThinkingLevel(modelId: string, models: AgentModelOption[]) {
  // Well-known models get their preferred default, overriding whatever
  // the agent's list_models returns (which currently hardcodes "high").
  if (modelId === "deepseek-v4-pro" || modelId.endsWith("/deepseek-v4-pro"))
    return "high";
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
