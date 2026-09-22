import type { FutureAuthState, ProvidersView } from "../../../integrations/agent/providers";
import { useEffect, useState } from "react";
import { getFutureAuthState, listAgentProviders } from "../../../integrations/agent/providers";

export type AppStartupSnapshot
  = | { phase: "pending" }
    | { phase: "failed" }
    | { phase: "ready"; auth: FutureAuthState; providers: ProvidersView };

const UNAVAILABLE_AUTH: FutureAuthState = {
  status: "unavailable",
  profile: null,
};

/**
 * Capture the account and provider snapshots behind the Agent readiness gate.
 * Neither result is interpreted as the other: an auth verification failure is
 * retained as `unavailable`, while failure to read provider configuration is a
 * startup failure because the app cannot safely choose its first screen.
 */
export function useAppStartup(agentReady: boolean): AppStartupSnapshot {
  const [snapshot, setSnapshot] = useState<AppStartupSnapshot>({ phase: "pending" });

  useEffect(() => {
    if (!agentReady) {
      setSnapshot({ phase: "pending" });
      return;
    }

    let cancelled = false;
    Promise.all([
      getFutureAuthState().catch(() => UNAVAILABLE_AUTH),
      listAgentProviders(),
    ]).then(
      ([auth, providers]) => {
        if (!cancelled)
          setSnapshot({ phase: "ready", auth, providers });
      },
      () => {
        if (!cancelled)
          setSnapshot({ phase: "failed" });
      },
    );

    return () => {
      cancelled = true;
    };
  }, [agentReady]);

  return snapshot;
}
