import type { AgentModelOption } from "../../../integrations/agent/agentClient";
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { useEffect, useRef, useState } from "react";
import i18n from "../../../i18n";
import { defaultThinkingLevel, modelOption, modelThinkingLevel, normalizeThinkingLevel, rememberLastUsedModel, rememberLastUsedThinkingLevel, resolveInitialModelId, resolveInitialThinkingLevel } from "../../../integrations/agent/agentClient";
import { updateCachedAgentState, useCachedAgentState } from "../../../integrations/agent/agentStateCache";
import { updateThreadModel, updateThreadThinkingLevel } from "../../../integrations/storage/threadStore";
import { errorMessage } from "../../../lib/errors";
import { emitFutureEvent } from "../../../lib/futureEvents";

interface UseModelSelectionParams {
  activeThread: StoredThread | null;
  selectedModelId: string;
  setSelectedModelId: (modelId: string) => void;
  modelOptions: AgentModelOption[];
  visibleModelOptions: AgentModelOption[];
  refreshStore: (nextActiveThreadId?: string) => Promise<void>;
}

export interface ModelSelection {
  selectedThinkingLevel: string;
  /**
   * Why the composer's model picker is empty: nothing loaded vs. everything the
   * user disabled. Only meaningful when the visible set is empty.
   */
  modelsEmptyReason: "no_models" | "all_disabled" | undefined;
  /** Active thread's model, falling back to the default pick when unavailable. */
  activeThreadModelId: string;
  /** Active thread's thinking level, or the draft selection when no thread. */
  activeThinkingLevel: string;
  /** Active-thread model change: persists model + its default thinking level. */
  changeModel: (modelId: string) => Promise<void>;
  /** Draft (no thread yet) model change: updates the local selection only. */
  changeDraftModel: (modelId: string) => void;
  /** Thinking-level change: persists it when a thread is active. */
  changeThinkingLevel: (thinkingLevel: string) => Promise<void>;
  /** Prime the selection to a just-created thread's model + thinking level. */
  syncSelection: (modelId: string, thinkingLevel: string) => void;
}

function thinkingLevelForModel(modelId: string, modelOptions: AgentModelOption[]) {
  return normalizeThinkingLevel(modelThinkingLevel(modelId, modelOptions));
}

/**
 * Owns the model + thinking-level selection. When no thread is active the
 * selection is a draft (used by the new-conversation flow); the draft thinking
 * level follows the selected model's default. When a thread is active, changes
 * persist to it and refresh the store.
 */
export function useModelSelection({
  activeThread,
  selectedModelId,
  setSelectedModelId,
  modelOptions,
  visibleModelOptions,
  refreshStore,
}: UseModelSelectionParams): ModelSelection {
  const [selectedThinkingLevel, setSelectedThinkingLevel] = useState(defaultThinkingLevel);
  const draftThinkingModelRef = useRef("");

  // Why the composer's model picker is empty: nothing loaded vs. everything the
  // user disabled. Only meaningful when the visible set is empty.
  const modelsEmptyReason: "no_models" | "all_disabled" | undefined
    = visibleModelOptions.length > 0
      ? undefined
      : modelOptions.length > 0
        ? "all_disabled"
        : "no_models";
  // Agent state is authoritative for model/thinking; DB values are fallback.
  // Reactive read: a background prefetch re-renders us the moment it lands, so
  // switching to an old thread doesn't briefly show the model default.
  const agentState = useCachedAgentState(activeThread?.id);
  const rawThreadModelId = agentState?.model ?? selectedModelId;
  const activeThreadModelId = modelOption(rawThreadModelId, visibleModelOptions)
    ? rawThreadModelId
    : resolveInitialModelId(visibleModelOptions);
  const activeThinkingLevel = activeThread
    ? normalizeThinkingLevel(agentState?.thinkingLevel ?? modelThinkingLevel(activeThreadModelId, visibleModelOptions))
    : selectedThinkingLevel;

  useEffect(() => {
    if (activeThread || draftThinkingModelRef.current === selectedModelId)
      return;

    draftThinkingModelRef.current = selectedModelId;
    // Restore the last user-picked level on the initial draft resolution; an
    // explicit model switch (changeDraftModel) sets the ref first, so this
    // effect early-returns there and the new model's default wins instead.
    setSelectedThinkingLevel(resolveInitialThinkingLevel(selectedModelId, visibleModelOptions));
  }, [activeThread, selectedModelId, visibleModelOptions]);

  async function changeModel(modelId: string) {
    setSelectedModelId(modelId);
    rememberLastUsedModel(modelId);
    // Follow the new model's default thinking level (same as the draft flow), so
    // switching models can't leave a thread on a level the model doesn't fit.
    const nextLevel = thinkingLevelForModel(modelId, visibleModelOptions);
    setSelectedThinkingLevel(nextLevel);
    draftThinkingModelRef.current = modelId;
    if (!activeThread)
      return;

    try {
      await updateThreadModel({
        threadId: activeThread.id,
        modelId,
      });
      await updateThreadThinkingLevel({
        threadId: activeThread.id,
        thinkingLevel: nextLevel,
      });
      updateCachedAgentState(activeThread.id, { model: modelId, thinkingLevel: nextLevel });
      await refreshStore(activeThread.id);
    }
    catch (error) {
      await reconcileAfterFailure(error, activeThread.id);
    }
  }

  function changeDraftModel(modelId: string) {
    setSelectedModelId(modelId);
    rememberLastUsedModel(modelId);
    setSelectedThinkingLevel(thinkingLevelForModel(modelId, visibleModelOptions));
    draftThinkingModelRef.current = modelId;
  }

  async function changeThinkingLevel(thinkingLevel: string) {
    const nextLevel = normalizeThinkingLevel(thinkingLevel);
    setSelectedThinkingLevel(nextLevel);
    rememberLastUsedThinkingLevel(nextLevel);
    if (!activeThread)
      return;

    updateCachedAgentState(activeThread.id, { thinkingLevel: nextLevel });
    try {
      await updateThreadThinkingLevel({
        threadId: activeThread.id,
        thinkingLevel: nextLevel,
      });
      await refreshStore(activeThread.id);
    }
    catch (error) {
      await reconcileAfterFailure(error, activeThread.id);
    }
  }

  // A persist failed after the optimistic local update: toast, then reload the
  // store so the derived active-thread model/level revert to the stored value
  // (and so the rejection never bubbles up as an unhandled promise rejection).
  async function reconcileAfterFailure(error: unknown, threadId: string) {
    emitFutureEvent("toast", {
      message: i18n.t("layout:model.updateFailed", { message: errorMessage(error) }),
      tone: "error",
    });
    await refreshStore(threadId).catch(() => {});
  }

  function syncSelection(modelId: string, thinkingLevel: string) {
    setSelectedModelId(modelId);
    setSelectedThinkingLevel(normalizeThinkingLevel(thinkingLevel));
    draftThinkingModelRef.current = modelId;
  }

  return {
    selectedThinkingLevel,
    modelsEmptyReason,
    activeThreadModelId,
    activeThinkingLevel,
    changeModel,
    changeDraftModel,
    changeThinkingLevel,
    syncSelection,
  };
}
