import type { Dispatch, SetStateAction } from "react";
import type { AgentModelOption } from "../../../integrations/agent/agentClient";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { loadAgentModelOptions, modelKey, modelOption, resolveInitialModelId } from "../../../integrations/agent/agentClient";
import { listAgentProviders } from "../../../integrations/agent/providers";
import { errorMessage } from "../../../lib/errors";
import { onFutureEvent } from "../../../lib/futureEvents";
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
   * - `all_disabled`: models loaded, but the user has hidden every one.
   */
  readiness?: "ready" | "needs_login" | "no_models" | "all_disabled" | null;
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
  // "Unable to load Future Agent models" surfaces when the gRPC call itself
  // fails (agent unreachable), not a model-data problem.
  if (normalized.includes("unable to load future agent models")) {
    return "agent_unavailable";
  }
  // Genuine model errors: agent returned but data is invalid / request rejected.
  if (
    normalized.includes("invalid model data")
    || normalized.includes("rejected the model list")
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
  // Use the same resolver as "New Chat" so the initial view and the button
  // converge on the identical default regardless of catalog readiness.
  const [selectedModelId, setSelectedModelId] = useState(() => resolveInitialModelId([]));
  // Generation guard: the 10s poll doesn't cancel an in-flight call (see
  // usePolling), and a connect with no timeout can hang across a tick. Without
  // this, a slow tick that fails *after* a newer tick already succeeded would
  // clobber the fresh model list with `[]` and flip the UI to disconnected,
  // resetting the user's model selection. Only the newest call may write.
  const refreshGenRef = useRef(0);

  const refreshAgentModels = useCallback(async () => {
    // Don't flip to "checking" on every poll/retry — that flips the status to
    // disconnected→checking→disconnected each tick and makes the offline notice
    // flash. The initial "checking" comes from the initial state; subsequent
    // refreshes silently keep the last status until a new result lands.
    const generation = ++refreshGenRef.current;
    try {
      const nextModels = await loadAgentModelOptions();
      if (generation !== refreshGenRef.current)
        return;
      // Keep the old reference when the catalog is unchanged: a fresh array
      // identity every 10s tick would flow AppShell → AgentThread → Composer
      // and defeat the composer's memo once per poll.
      setModelOptions(previous =>
        JSON.stringify(previous) === JSON.stringify(nextModels) ? previous : nextModels,
      );
      // Selection reconciliation lives in the visible-set effect below (so it
      // also reacts to models being enabled/disabled, not just catalog changes).
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
      if (generation !== refreshGenRef.current)
        return;
      // Bail out when the result is unchanged: the poll ticks every 10s, and
      // an unconditional set with a fresh object (checkedAt) re-rendered
      // AppShell every tick even while the connection stayed identical.
      setAgentConnection((previous) => {
        if (
          previous.status === "connected"
          && previous.readiness === readiness
          && previous.error == null
          && previous.kind == null
        ) {
          return previous;
        }
        return {
          checkedAt: Date.now(),
          error: null,
          kind: null,
          readiness,
          status: "connected",
        };
      });
    }
    catch (error) {
      // A stale failure must not blank the freshly-loaded catalog nor flip the
      // status — a newer tick has already reported the truth.
      if (generation !== refreshGenRef.current)
        return;
      const message = errorMessage(error);
      // Keep the last good catalog: blanking it made every model id
      // unresolvable, which dropped the composer to the last-used/default
      // model and "off" thinking level until the next successful poll.
      setAgentConnection((previous) => {
        const kind = classifyAgentConnectionError(message);
        if (
          previous.status === "disconnected"
          && previous.error === message
          && previous.kind === kind
        ) {
          return previous;
        }
        return {
          checkedAt: Date.now(),
          error: message,
          kind,
          status: "disconnected",
        };
      });
    }
  }, []);

  usePolling(refreshAgentModels, 10000, { deps: [refreshAgentModels] });

  // The onboarding init flow warms the agent's model cache + registry; refresh
  // immediately so the composer reflects the new models the instant the gate
  // closes (otherwise we'd wait up to the next 10s poll tick).
  useEffect(
    () => onFutureEvent("future-models-synced", () => void refreshAgentModels()),
    [refreshAgentModels],
  );

  const visibleModelOptions = useMemo(
    () => modelOptions.filter(model => !hiddenModels.includes(modelKey(model))),
    [hiddenModels, modelOptions],
  );

  // Reconcile the draft/global selection against the *visible* set (catalog minus
  // the user's hidden models). Runs whenever that set changes, so a selection
  // that was deleted from the catalog or disabled in Settings falls back to the
  // default pick in real time, and an empty set clears it so pickers show their
  // empty state. Per-thread selections persist separately and aren't touched here.
  useEffect(() => {
    setSelectedModelId(current =>
      current && modelOption(current, visibleModelOptions)
        ? current
        : resolveInitialModelId(visibleModelOptions),
    );
  }, [visibleModelOptions]);

  // Surface "loaded but all hidden" as its own readiness so the UI can tell it
  // apart from "nothing loaded" (needs login / empty catalog). Derived rather
  // than baked into the poll so it tracks hidden-model toggles immediately.
  const connectionWithVisibility = useMemo<AgentConnectionState>(() => {
    if (
      agentConnection.status === "connected"
      && agentConnection.readiness === "ready"
      && modelOptions.length > 0
      && visibleModelOptions.length === 0
    ) {
      return { ...agentConnection, readiness: "all_disabled" };
    }
    return agentConnection;
  }, [agentConnection, modelOptions.length, visibleModelOptions.length]);

  return {
    agentConnection: connectionWithVisibility,
    modelOptions,
    refreshAgentModels,
    selectedModelId,
    setSelectedModelId,
    visibleModelOptions,
  };
}
