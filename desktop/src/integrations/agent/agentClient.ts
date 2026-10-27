import { Channel } from "@tauri-apps/api/core";
import { invokeCommand } from "../tauri/invoke";

/**
 * The legacy six-level vocabulary: what every client offered before the agent
 * sent a per-model `thinkingLevels` list.
 *
 * It is now only a fallback for an agent that omits the field (an older agent,
 * or an older remote host) — a current agent sends the model's own list, and
 * that list's implicit `default` entry means "send no parameter at all".
 */
export const legacyThinkingLevels = ["off", "minimal", "low", "medium", "high", "xhigh"] as const;

/**
 * A thinking level: any non-empty string the wire understands.
 *
 * `default` is the implicit level ("send no thinking/effort parameter"), `off`
 * explicitly disables thinking, and every other value is the provider's own
 * wire value, sent verbatim. Widened from a fixed union on purpose: the agent
 * publishes arbitrary `models.json` values (e.g. `max`), and a client-side
 * whitelist would drop them.
 */
export type ThinkingLevel = string;

/**
 * The implicit level a conversation starts at when nothing else is chosen: the
 * provider's own default, with no thinking/effort parameter on the wire.
 *
 * For an older agent (no `thinkingLevels`), `modelThinkingLevel` instead
 * returns its historical `medium` — never the literal `default`, which an old
 * agent would not understand.
 */
export const defaultThinkingLevel: ThinkingLevel = "default";

/**
 * Level names the product ships a translation for. This is **not** a wire
 * vocabulary — the custom-provider editor offers these as toggle chips, but a
 * model's list may hold any other token and it is sent verbatim.
 */
export const knownThinkingLevels = ["off", "minimal", "low", "medium", "high", "xhigh", "max"] as const;

export interface AgentModelOption {
  id: string;
  label: string;
  provider: string;
  supportsImages?: boolean;
  /**
   * Deprecated capability flag. Nothing branches on it anymore: whether a model
   * has thinking controls is `thinkingLevels` alone.
   */
  reasoning?: boolean | null;
  /** Historical single default level; superseded by `defaultThinkingLevel`. */
  thinkingLevel?: ThinkingLevel | string | null;
  /**
   * The model's effective levels, in display order. A current agent sends a
   * non-empty list that always starts with the implicit `default`; every other
   * entry is a wire value sent verbatim (`off` = explicitly disable). Absent on
   * an older agent, where the client falls back to {@link legacyThinkingLevels}.
   */
  thinkingLevels?: string[] | null;
  /**
   * The level a conversation starts at on this model, already clamped by the
   * agent — so seeding the picker from it cannot land on a level the model would
   * snap away. `"default"` for a current agent; absent on an older one, where
   * the desktop's historical `medium` stands.
   */
  defaultThinkingLevel?: ThinkingLevel | string | null;
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

/**
 * Stable terminal categories the backend reports on a reply that ended before a
 * clean `agent_end`, mirroring `agent_bridge::stream::termination_error`.
 */
export type ResponseTerminationKind
  = | "upstream_disconnected"
    | "response_timeout"
    | "output_limit"
    | "model_content_filter"
    | "model_response_error"
    | "model_paused"
    | "provider_cancelled"
    | "response_unconfirmed"
    /**
     * NOT a failure: the Agent accepted the prompt and queued it behind the run
     * already executing. The run is alive and its reply still arrives.
     */
    | "run_queued";

interface AgentPromptResponse {
  content: string;
  /** False when the agent stream ended before a clean `agent_end`. */
  complete?: boolean;
  /**
   * Stable terminal category when `complete` is false. Not necessarily a failure:
   * `run_queued` means the run is alive behind another one.
   */
  terminationKind?: ResponseTerminationKind;
  /** The stable Agent session id bound to this conversation. */
  sessionId?: string;
}

export const defaultAgentModelId = "";

export interface AgentPromptInput {
  message: string;
  modelContext?: string;
  threadId: string;
  sessionId?: string | null;
  runId?: string | null;
  modelId?: string | null;
  attachments?: AttachmentInput[];
  thinkingLevel?: string | null;
  onAccepted?: () => void;
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
  onAccepted,
}: AgentPromptInput) {
  const acceptance = onAccepted ? new Channel<null>(() => onAccepted()) : undefined;
  const response = await invokeCommand<AgentPromptResponse>("agent_prompt", {
    ...(acceptance ? { onAccepted: acceptance } : {}),
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
    ...(response.terminationKind ? { terminationKind: response.terminationKind } : {}),
    sessionId: response.sessionId,
  };
}

export async function loadAgentModelOptions() {
  return normalizeAgentModelOptions(await invokeCommand<AgentModelOption[]>("list_agent_models"));
}

export interface CompactContextResult {
  operationId: string;
  accepted: true;
}

/** Run standalone semantic compaction for an existing conversation. */
export function compactThreadContext(threadId: string) {
  return invokeCommand<CompactContextResult>("compact_thread_context", { threadId });
}

/** Result of an explicit Future model sync (agent warms its cache + registry). */
export interface SyncFutureModelsResult {
  synced: boolean;
  modelCount: number;
  revision: number;
}

/**
 * Fetch the Future provider's models inside the agent, warming its cache and
 * rebuilding its registry. Used by onboarding and the Settings manual action;
 * the backend scheduler calls the same execution path directly.
 */
export async function syncFutureModels() {
  return invokeCommand<SyncFutureModelsResult>("sync_future_models");
}

/** Result of a background Future catalogue refresh request. */
export interface RequestFutureModelsRefreshResult {
  /**
   * Whether the agent started a refresh; `false` means the backoff window was
   * still open, a refresh was already running, or no Future credential exists.
   */
  requested: boolean;
}

/**
 * Ask the agent to refresh the Future catalogue **in the background**. The call
 * returns once the fetch is scheduled; the refreshed list reaches the UI later
 * through the agent's `provider-config-changed` event (which the desktop
 * republishes as `providers-changed`), so nothing needs to await it.
 */
export async function requestFutureModelsRefresh() {
  return invokeCommand<RequestFutureModelsRefreshResult>("request_future_models_refresh");
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
  const lastOption = lastUsed ? modelOption(lastUsed, models) : undefined;
  if (lastOption)
    return modelKey(lastOption);
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
 * level (if this model still offers it) → the model's own starting level.
 */
export function resolveInitialThinkingLevel(modelId: string, models: AgentModelOption[]): ThinkingLevel {
  const allowed = modelThinkingLevels(modelId, models);
  const lastUsed = readLastUsedThinkingLevel();
  if (lastUsed && allowed.includes(lastUsed))
    return lastUsed;
  return modelThinkingLevel(modelId, models);
}

/**
 * Display label for a model id, or `undefined` when there's no match and no id
 * to fall back on. The integration layer stays i18n-free: call sites supply a
 * localized fallback (e.g. `modelLabel(...) ?? t("common:modelFallback")`).
 */
export function modelLabel(modelId: string, models: AgentModelOption[]): string | undefined {
  return modelOption(modelId, models)?.label ?? (modelId || undefined);
}

/**
 * The thinking levels a model offers, in display order, exactly as the agent
 * published them.
 *
 * A current agent sends the model's effective list: non-empty, starting with
 * the implicit `default`, every other entry a wire value sent verbatim. The
 * list is passed through untouched — no client-side vocabulary — so a model
 * declaring `max` (or any other token) shows it, and `default` alone means the
 * model has no thinking controls to choose from.
 *
 * No list — an agent older than the field, or an older remote host — keeps the
 * legacy six, which is what every client offered before this field existed.
 */
export function modelThinkingLevels(modelId: string, models: AgentModelOption[]): readonly ThinkingLevel[] {
  const declared = modelOption(modelId, models)?.thinkingLevels;
  if (declared && declared.length > 0)
    return declared;
  return legacyThinkingLevels;
}

/**
 * The level a conversation starts at on this model.
 *
 * The agent sends the level already clamped to the model's list, so it is
 * preferred whenever it is offered. A current agent sends `default`, whose
 * position in the list is the implicit first entry. An older agent (no list)
 * keeps the desktop's historical `medium` — the literal `default` is never sent
 * to it, because it predates the value.
 */
export function modelThinkingLevel(modelId: string, models: AgentModelOption[]): ThinkingLevel {
  const option = modelOption(modelId, models);
  const allowed = modelThinkingLevels(modelId, models);
  const starting = option?.defaultThinkingLevel;
  if (starting && allowed.includes(starting))
    return starting;
  // A declared list is a current agent; its first entry is the implicit default.
  if (option?.thinkingLevels && option.thinkingLevels.length > 0)
    return allowed[0]!;
  return "medium";
}

/**
 * A level to show for a model, replacing any value the model does not offer.
 *
 * Display-only: a stored level from another model (a thread saved before this
 * model narrowed its list, a task written on a different machine) is not shown as
 * the current effort, because it is not one this model can run at. The
 * replacement is the model's own starting level — the agent's clamp of the
 * implicit default — rather than a client-side "nearest level" rule.
 */
export function resolveThinkingLevel(level: string | null | undefined, modelId: string, models: AgentModelOption[]): ThinkingLevel {
  const allowed = modelThinkingLevels(modelId, models);
  if (level && allowed.includes(level))
    return level;
  return modelThinkingLevel(modelId, models);
}

/**
 * Coerce a level from outside this module (agent state, a stored thread, an
 * emitted event) into a safe wire value.
 *
 * Any non-empty string is a legal level now, so only emptiness falls back to
 * the implicit default. Whether the model offers the value is a separate
 * question — `resolveThinkingLevel` answers that.
 */
export function normalizeThinkingLevel(level?: string | null): ThinkingLevel {
  const trimmed = level?.trim();
  return trimmed || defaultThinkingLevel;
}

export function modelOption(modelId: string, models: AgentModelOption[]) {
  // Prefer an exact provider-qualified match so we resolve the right provider
  // when several expose the same model id.
  const exact = models.find(model => modelKey(model) === modelId);
  if (exact)
    return exact;
  // Legacy bare ids may themselves contain slashes. Only a unique exact id
  // can be upgraded safely; stripping a prefix or choosing the first provider
  // can change the endpoint/credentials behind the user's selection.
  const legacyMatches = models.filter(model => model.id === modelId);
  return legacyMatches.length === 1 ? legacyMatches[0] : undefined;
}
