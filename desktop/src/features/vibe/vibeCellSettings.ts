import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { modelLabel, modelSupportsThinking, modelThinkingLevel, resolveThinkingLevel } from "../../integrations/agent/agentClient";
import { updateCachedAgentState, useCachedAgentState } from "../../integrations/agent/agentStateCache";
import { updateThreadModel, updateThreadThinkingLevel } from "../../integrations/storage/threads";

/**
 * A cell's *settings*: read-only on screen, editable in the menu.
 *
 * Why the split: from a sofa, four dropdowns in the middle of a dictated sentence
 * are noise — but "which model is this cell running, and how much is it allowed to
 * do" is exactly what a reader watching several conversations wants at a glance. So
 * the values stay visible on every layer, and only the *change* moves into the menu,
 * where `←`/`→` already means "adjust this value".
 *
 * Not a component file on purpose (`VibeSettings.tsx` is the line): these are the
 * reads and writes, and a hook that also lived beside a component would be one file
 * fast refresh cannot treat as pure presentation.
 */
export interface VibeCellSettings {
  modelId: string;
  /** The model's display name, or the id when the catalogue does not know it. */
  modelName: string;
  thinkingLevel: string;
  approvalTier: ApprovalTier;
  /** Whether this model has a thinking level at all. */
  supportsThinking: boolean;
}

/**
 * One conversation's settings, as the panel shows and changes them.
 *
 * The per-thread values come from the agent's own cached state (what that session is
 * actually running on) and are written **through the thread's id** — never through
 * the shell's "active thread", which in the panel is whichever conversation the main
 * window happened to have open. That distinction is the whole reason this hook exists
 * instead of reusing `useModelSelection`'s `changeModel`.
 */
export function useVibeCellSettings(thread: StoredThread | null, fallback: {
  approvalTier: ApprovalTier;
  modelId: string;
  modelOptions: AgentModelOption[];
  thinkingLevel: string;
}): VibeCellSettings {
  const state = useCachedAgentState(thread?.id);
  const modelId = state?.model ?? fallback.modelId;
  const supportsThinking = modelSupportsThinking(modelId, fallback.modelOptions);
  return {
    // The approval tier is an application setting, not a conversation's: the menu says
    // so where it is changed, or a reader will expect per-cell tiers that do not exist.
    approvalTier: fallback.approvalTier,
    modelId,
    modelName: modelLabel(modelId, fallback.modelOptions) ?? modelId,
    supportsThinking,
    thinkingLevel: resolveThinkingLevel(state?.thinkingLevel ?? fallback.thinkingLevel, modelId, fallback.modelOptions),
  };
}

/** Persist a conversation's model, and its thinking level to that model's default. */
export async function setVibeCellModel(
  threadId: string,
  modelId: string,
  modelOptions: AgentModelOption[],
): Promise<void> {
  const thinkingLevel = modelThinkingLevel(modelId, modelOptions);
  await updateThreadModel({ modelId, threadId });
  await updateThreadThinkingLevel({ thinkingLevel, threadId });
  // The cache is what the panel reads, so the change shows on the next render
  // instead of after the store's next refresh.
  updateCachedAgentState(threadId, { model: modelId, thinkingLevel });
}

/** Persist one conversation's thinking level. */
export async function setVibeCellThinkingLevel(threadId: string, thinkingLevel: string): Promise<void> {
  await updateThreadThinkingLevel({ thinkingLevel, threadId });
  updateCachedAgentState(threadId, { thinkingLevel });
}
