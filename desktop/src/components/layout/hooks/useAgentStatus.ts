import type { AgentStatus } from "../../../integrations/agent/agentStatus";
import { useEffect, useState } from "react";
import { getAgentStatus, subscribeShutdownProgress } from "../../../integrations/agent/agentStatus";

export const MINIMUM_AGENT_SPLASH_MS = 1000;
export const AGENT_WAIT_HINT_DELAY_MS = 5000;
export const AGENT_STARTUP_TIMEOUT_MS = 15000;

export interface AgentStartupStatus extends AgentStatus {
  showWait: boolean;
  admitted: boolean;
}

const INITIAL_STATUS: AgentStatus = {
  phase: "checking",
  desktopVersion: "",
  agentVersion: null,
};

/**
 * Gate all account/provider work behind an Agent business-RPC handshake.
 * Startup states poll quickly; ready and terminal-looking states keep slower
 * probes. Once admitted, AppShell stays mounted during recovery so drafts,
 * remote sessions and subscription cursors survive local Agent failures.
 */
export function useAgentStatus(): AgentStartupStatus {
  const [status, setStatus] = useState<AgentStatus>(INITIAL_STATUS);
  const [admitted, setAdmitted] = useState(false);
  const [showWait, setShowWait] = useState(false);
  const [splashStartedAt] = useState(() => Date.now());

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    const unlisten = subscribeShutdownProgress((shutdown) => {
      if (!cancelled) {
        setStatus(current => (
          current.shutdown?.state !== undefined && current.shutdown.state !== "running" && shutdown.state === "running"
            ? current
            : { ...current, phase: "stopping", shutdown, error: shutdown.errors.join("; ") || undefined }
        ));
      }
    });
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
        if (next.phase === "ready")
          setAdmitted(true);
        setStatus(current => (
          current.shutdown && (!next.shutdown || (current.shutdown.state !== "running" && next.shutdown.state === "running"))
            ? current
            : current.phase === next.phase
              && current.desktopVersion === next.desktopVersion
              && current.agentVersion === next.agentVersion
              && current.error === next.error
              && JSON.stringify(current.shutdown) === JSON.stringify(next.shutdown)
              ? current
              : next
        ));
        const delay = next.phase === "checking" || next.phase === "starting" || next.phase === "recovering" || next.phase === "stopping"
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
      void unlisten.then(stop => stop());
      if (timer !== undefined)
        window.clearTimeout(timer);
      window.clearTimeout(waitHintTimer);
      window.clearTimeout(startupTimeoutTimer);
    };
  }, [splashStartedAt]);

  return { ...status, showWait, admitted };
}
