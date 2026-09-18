import type { AgentStatus } from "../../../integrations/agent/agentStatus";
import { useEffect, useState } from "react";
import { getAgentStatus } from "../../../integrations/agent/agentStatus";

export const MINIMUM_AGENT_SPLASH_MS = 1000;
export const AGENT_WAIT_HINT_DELAY_MS = 5000;
export const AGENT_STARTUP_TIMEOUT_MS = 15000;

export interface AgentStartupStatus extends AgentStatus {
  showWait: boolean;
}

const INITIAL_STATUS: AgentStatus = {
  phase: "checking",
  desktopVersion: "",
  agentVersion: null,
};

/**
 * Gate all account/provider work behind an Agent business-RPC handshake.
 * Startup states poll quickly; ready and terminal-looking states keep slower
 * probes so a later crash shows the Agent page and an external recovery can
 * admit the app without a second login.
 */
export function useAgentStatus(): AgentStartupStatus {
  const [status, setStatus] = useState<AgentStatus>(INITIAL_STATUS);
  const [showWait, setShowWait] = useState(false);
  const [splashStartedAt] = useState(() => Date.now());

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    const waitHintTimer = window.setTimeout(setShowWait, AGENT_WAIT_HINT_DELAY_MS, true);
    const startupTimeoutTimer = window.setTimeout(() => {
      setStatus(current => (
        current.phase === "checking" || current.phase === "starting"
          ? { ...current, phase: "startup_timeout" }
          : current
      ));
    }, AGENT_STARTUP_TIMEOUT_MS);

    const probe = async () => {
      let next: AgentStatus;
      try {
        next = await getAgentStatus();
      }
      catch {
        next = { ...INITIAL_STATUS, phase: "unavailable" };
      }
      if (cancelled)
        return;
      const commit = () => {
        if (cancelled)
          return;
        if (
          (next.phase === "checking" || next.phase === "starting")
          && Date.now() - splashStartedAt >= AGENT_STARTUP_TIMEOUT_MS
        ) {
          next = { ...next, phase: "startup_timeout" };
        }
        setStatus(current => (
          current.phase === next.phase
          && current.desktopVersion === next.desktopVersion
          && current.agentVersion === next.agentVersion
            ? current
            : next
        ));
        const delay = next.phase === "checking" || next.phase === "starting"
          ? 600
          : next.phase === "ready"
            ? 5000
            : 2000;
        timer = window.setTimeout(probe, delay);
      };

      // A fast Agent must not make the startup surface flash for a few frames.
      // Delay only successful admission; failures remain visible immediately.
      const remainingSplashMs = MINIMUM_AGENT_SPLASH_MS - (Date.now() - splashStartedAt);
      if (next.phase === "ready" && remainingSplashMs > 0)
        timer = window.setTimeout(commit, remainingSplashMs);
      else
        commit();
    };

    void probe();
    return () => {
      cancelled = true;
      if (timer !== undefined)
        window.clearTimeout(timer);
      window.clearTimeout(waitHintTimer);
      window.clearTimeout(startupTimeoutTimer);
    };
  }, [splashStartedAt]);

  return { ...status, showWait };
}
