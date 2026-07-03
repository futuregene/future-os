import type { Dispatch, SetStateAction } from "react";
import type { AgentModelOption } from "../../../integrations/agent/agentClient";
import { useCallback, useMemo, useState } from "react";
import { defaultAgentModelId, loadAgentModelOptions, modelOption, resolveInitialModelId } from "../../../integrations/agent/agentClient";
import { listAgentProviders } from "../../../integrations/agent/providers";
import { usePolling } from "../../../lib/usePolling";

export interface AgentConnectionState {
  status: "checking" | "connected" | "disconnected";
  error?: string | null;
  kind?: "agent_unavailable" | "model_error" | "unknown" | null;
  /**
   * When connected, why there are (or aren't) usable models:
   * - `ready`: models available.
   * - `needs_login`: no FutureGene login and no custom provider → no credentials.
   * - `no_models`: credentials exist, but the model list is still empty.
   */
  readiness?: "ready" | "needs_login" | "no_models" | null;
  checkedAt?: number | null;
}

export interface UseAgentConnectionResult {
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
export function useAgentConnection(hiddenModels: string[]): UseAgentConnectionResult {
  const [agentConnection, setAgentConnection] = useState<AgentConnectionState>({ status: "checking" });
  const [modelOptions, setModelOptions] = useState<AgentModelOption[]>([]);
  const [selectedModelId, setSelectedModelId] = useState(defaultAgentModelId);

  const refreshAgentModels = useCallback(async () => {
    // Don't flip to "checking" on every poll/retry — that flips the status to
    // disconnected→checking→disconnected each tick and makes the offline notice
    // flash. The initial "checking" comes from the initial state; subsequent
    // refreshes silently keep the last status until a new result lands.
    try {
      const nextModels = await loadAgentModelOptions();
      setModelOptions(nextModels);
      setSelectedModelId(current =>
        current && modelOption(current, nextModels)
          ? current
          : resolveInitialModelId(nextModels),
      );
      // Agent is reachable. If there are no models, find out whether that's
      // because nothing is configured (needs login / a provider) or because the
      // configured providers simply expose none — so the UI can say which.
      let readiness: AgentConnectionState["readiness"] = "ready";
      if (nextModels.length === 0) {
        readiness = "no_models";
        try {
          const providers = await listAgentProviders();
          const hasCredentials
            = providers.builtin.some(provider => provider.hasApiKey)
              || providers.custom.length > 0;
          readiness = hasCredentials ? "no_models" : "needs_login";
        }
        catch {
          // Can't tell — leave as a generic "no models" rather than guessing.
        }
      }
      setAgentConnection({
        checkedAt: Date.now(),
        error: null,
        kind: null,
        readiness,
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
