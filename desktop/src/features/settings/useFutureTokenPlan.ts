import type { FutureTokenPlan } from "../../integrations/agent/providers";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  clearFutureTokenPlanCache,
  getFutureTokenPlan,
  peekFutureTokenPlan,
} from "../../integrations/agent/providers";
import { onFutureEvent } from "../../lib/futureEvents";

export type FutureTokenPlanStatus = "idle" | "loading" | "available" | "unavailable";

/**
 * The signed-in account's subscription and current quota window.
 *
 * Kept local to the account page rather than app-wide like the balance: only
 * this page shows it. It is refetched when the page opens and after a run ends
 * (`agent_end`), because the current window's used credits move with each model
 * call the platform bills.
 *
 * `enabled` is false while signed out or in the community edition (which is not
 * billed by the platform), and then nothing is fetched or shown.
 */
export function useFutureTokenPlan(enabled: boolean) {
  const cached = peekFutureTokenPlan();
  const [tokenPlan, setTokenPlan] = useState<FutureTokenPlan | null>(() => cached);
  const [status, setStatus] = useState<FutureTokenPlanStatus>(() => (cached ? "available" : "idle"));
  // Each request claims a sequence number; a response whose number is no longer
  // the latest (a newer refresh, sign-out, credential change) is dropped instead
  // of overwriting it.
  const requestRef = useRef(0);

  const refresh = useCallback(() => {
    if (!enabled)
      return;
    const request = ++requestRef.current;
    setStatus("loading");
    getFutureTokenPlan(true).then(
      (next) => {
        if (request !== requestRef.current)
          return;
        setTokenPlan(next);
        setStatus("available");
      },
      () => {
        if (request !== requestRef.current)
          return;
        setTokenPlan(null);
        setStatus("unavailable");
      },
    );
  }, [enabled]);

  useEffect(() => {
    if (enabled) {
      refresh();
      return;
    }
    // Signed out or community edition: invalidate any in-flight response and
    // stop showing what a previous session (or the platform edition) had.
    requestRef.current += 1;
    clearFutureTokenPlanCache();
    setTokenPlan(null);
    setStatus("idle");
  }, [enabled, refresh]);

  useEffect(
    () => onFutureEvent("agent_end", () => {
      clearFutureTokenPlanCache();
      refresh();
    }),
    [refresh],
  );

  // A credential change means the subscription belongs to an account that may
  // no longer be the one signed in.
  useEffect(
    () => onFutureEvent("future-auth-changed", () => {
      requestRef.current += 1;
      clearFutureTokenPlanCache();
      setTokenPlan(null);
      setStatus("idle");
    }),
    [],
  );

  return { tokenPlan, status, refresh };
}
