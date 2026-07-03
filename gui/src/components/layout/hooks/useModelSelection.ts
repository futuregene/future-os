import type { AgentModelOption } from "../../../integrations/agent/agentClient";
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { useEffect, useRef, useState } from "react";
import { defaultThinkingLevel, modelThinkingLevel, normalizeThinkingLevel, rememberLastUsedModel } from "../../../integrations/agent/agentClient";
import { updateThreadModel, updateThreadThinkingLevel } from "../../../integrations/storage/threadStore";

interface UseModelSelectionParams {
  activeThread: StoredThread | null;
  selectedModelId: string;
  setSelectedModelId: (modelId: string) => void;
  visibleModelOptions: AgentModelOption[];
  refreshStore: (nextActiveThreadId?: string) => Promise<void>;
}

export interface ModelSelection {
  selectedThinkingLevel: string;
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
  visibleModelOptions,
  refreshStore,
}: UseModelSelectionParams): ModelSelection {
  const [selectedThinkingLevel, setSelectedThinkingLevel] = useState(defaultThinkingLevel);
  const draftThinkingModelRef = useRef("");

  useEffect(() => {
    if (activeThread || draftThinkingModelRef.current === selectedModelId)
      return;

    draftThinkingModelRef.current = selectedModelId;
    setSelectedThinkingLevel(thinkingLevelForModel(selectedModelId, visibleModelOptions));
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

    await updateThreadModel({
      threadId: activeThread.id,
      modelId,
    });
    await updateThreadThinkingLevel({
      threadId: activeThread.id,
      thinkingLevel: nextLevel,
    });
    await refreshStore(activeThread.id);
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
    if (!activeThread)
      return;

    await updateThreadThinkingLevel({
      threadId: activeThread.id,
      thinkingLevel: nextLevel,
    });
    await refreshStore(activeThread.id);
  }

  function syncSelection(modelId: string, thinkingLevel: string) {
    setSelectedModelId(modelId);
    setSelectedThinkingLevel(normalizeThinkingLevel(thinkingLevel));
    draftThinkingModelRef.current = modelId;
  }

  return {
    selectedThinkingLevel,
    changeModel,
    changeDraftModel,
    changeThinkingLevel,
    syncSelection,
  };
}
