import type { FutureAuthState, ProvidersView } from "../../../integrations/agent/providers";
import { useEffect, useRef } from "react";
import { getFutureAuthState, listAgentProviders } from "../../../integrations/agent/providers";
import { useAsyncResource } from "../../../lib/useAsyncResource";

export type AppStartupSnapshot
  = | { phase: "pending" }
    | { phase: "failed"; reason: "providersFailed" | "providersTimeout" }
    | { phase: "ready"; auth: FutureAuthState; providers: ProvidersView };

export const STARTUP_AUTH_TIMEOUT_MS = 5000;
export const STARTUP_PROVIDERS_TIMEOUT_MS = 15000;
const UNAVAILABLE_AUTH: FutureAuthState = { status: "unavailable", profile: null };
const PENDING: AppStartupSnapshot = { phase: "pending" };

class StartupTimeout extends Error {}

/** Bound the wait and release timers/listeners when this startup attempt ends. */
function bounded<T>(work: Promise<T>, ms: number, signal: AbortSignal): Promise<T> {
  if (signal.aborted)
    return Promise.reject(signal.reason);
  return new Promise<T>((resolve, reject) => {
    let timer: number;
    const abort = () => {
      window.clearTimeout(timer);
      reject(signal.reason);
    };
    const cleanup = () => {
      window.clearTimeout(timer);
      signal.removeEventListener("abort", abort);
    };
    timer = window.setTimeout(() => {
      cleanup();
      reject(new StartupTimeout("Startup settings did not respond"));
    }, ms);
    signal.addEventListener("abort", abort, { once: true });
    work.then(
      (value) => {
        cleanup();
        resolve(value);
      },
      (error) => {
        cleanup();
        reject(error);
      },
    );
  });
}

/**
 * Account verification may degrade offline; provider configuration must be
 * known before onboarding can be chosen. Both waits end, and retry is explicit.
 */
export function useAppStartup(agentReady: boolean, attempt = 0): AppStartupSnapshot {
  const lifetimeRef = useRef<AbortController | null>(null);
  // Install before the resource's effect. StrictMode replays get a fresh
  // controller; an old attempt can neither retain timers nor publish results.
  useEffect(() => {
    const controller = new AbortController();
    lifetimeRef.current = controller;
    return () => controller.abort();
  }, [agentReady, attempt]);

  const resource = useAsyncResource<AppStartupSnapshot>(async () => {
    if (!agentReady)
      return PENDING;
    const signal = lifetimeRef.current!.signal;
    try {
      const [auth, providers] = await Promise.all([
        bounded(getFutureAuthState(), STARTUP_AUTH_TIMEOUT_MS, signal).catch(() => UNAVAILABLE_AUTH),
        bounded(listAgentProviders(), STARTUP_PROVIDERS_TIMEOUT_MS, signal),
      ]);
      return { phase: "ready", auth, providers };
    }
    catch (error) {
      return { phase: "failed", reason: error instanceof StartupTimeout ? "providersTimeout" : "providersFailed" };
    }
  }, [agentReady, attempt], PENDING);

  return !agentReady || resource.loading ? PENDING : resource.data;
}
