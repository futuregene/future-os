import type { Dispatch, SetStateAction } from "react";
import type { AgentModelOption } from "../../../integrations/agent/agentClient";
import { useCallback, useMemo, useState } from "react";
import { defaultAgentModelId, defaultModelId, loadAgentModelOptions } from "../../../integrations/agent/agentClient";
import { usePolling } from "../../../lib/usePolling";

export interface AgentConnectionState {
  status: "checking" | "connected" | "disconnected";
  error?: string | null;
  kind?: "agent_unavailable" | "model_error" | "unknown" | null;
  checkedAt?: number | null;
}

export interface AgentConnection {
  agentConnection: AgentConnectionState;
  /** All advertised models (used by Settings). */
  modelOptions: AgentModelOption[];
  /** Models minus the user's hidden set (used by pickers). */
  visibleModelOptions: AgentModelOption[];
  selectedModelId: string;
  setSelectedModelId: Dispatch<SetStateAction<string>>;
  refreshAgentModels: () => Promise<void>;
}

function classifyAgentConnectionError(message: string): AgentConnectionState["kind"] {
  const normalized = message.toLowerCase();
  if (normalized.includes("unable to connect to future agent")) {
    return "agent_unavailable";
  }
  if (
    normalized.includes("unable to load future agent models")
    || normalized.includes("model")
    || normalized.includes("list_models")
  ) {
    return "model_error";
  }
  return "unknown";
}

/**
 * Owns the agent connection: loads the model catalog on a 10s poll, tracks
 * connected/disconnected status (classifying the failure for targeted UI
 * hints), and holds the selected model id. `visibleModelOptions` applies the
 * caller's hidden-model set.
 */
export function useAgentConnection(hiddenModels: string[]): AgentConnection {
  const [agentConnection, setAgentConnection] = useState<AgentConnectionState>({ status: "checking" });
  const [modelOptions, setModelOptions] = useState<AgentModelOption[]>([]);
  const [selectedModelId, setSelectedModelId] = useState(defaultAgentModelId);

  const refreshAgentModels = useCallback(async () => {
    setAgentConnection(current => current.status === "connected"
      ? current
      : { ...current, status: "checking" });
    try {
      const nextModels = await loadAgentModelOptions();
      setModelOptions(nextModels);
      setSelectedModelId(current =>
        nextModels.some(model => model.id === current)
          ? current
          : defaultModelId(nextModels),
      );
      setAgentConnection({
        checkedAt: Date.now(),
        error: null,
        kind: null,
        status: "connected",
      });
    }
    catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setModelOptions([]);
      setAgentConnection({
        checkedAt: Date.now(),
        error: message,
        kind: classifyAgentConnectionError(message),
        status: "disconnected",
      });
    }
  }, []);

  usePolling(refreshAgentModels, 10000, { deps: [refreshAgentModels] });

  const visibleModelOptions = useMemo(
    () => modelOptions.filter(model => !hiddenModels.includes(`${model.provider}/${model.id}`)),
    [hiddenModels, modelOptions],
  );

  return {
    agentConnection,
    modelOptions,
    refreshAgentModels,
    selectedModelId,
    setSelectedModelId,
    visibleModelOptions,
  };
}
