import { useCallback, useEffect, useRef, useState } from "react";
import { clearFutureBalanceCache, FUTURE_PROVIDER_ID, getFutureBalance, listAgentProviders } from "../../../integrations/agent/providers";
import { onFutureEvent } from "../../../lib/futureEvents";
import { useAsyncResource } from "../../../lib/useAsyncResource";

const POLL_INTERVAL_MS = 3_600_000; // 1 hour

/**
 * Fetch the FutureOS credit balance. Returns `null` when signed out or on
 * error; `number` (credits) otherwise. Polls every hour; also refreshes on
 * `agent_end` (conversation finished) and on `future-auth-changed` events.
 */
export function useFutureBalance(): number | null {
  const { data: providers } = useAsyncResource(listAgentProviders, [], null);
  const loggedIn = Boolean(
    providers?.builtin.some(
      p => p.id === FUTURE_PROVIDER_ID && p.hasApiKey,
    ),
  );

  const [balance, setBalance] = useState<number | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const refresh = useCallback(() => {
    if (!loggedIn) {
      setBalance(null);
      return;
    }
    getFutureBalance(true).then(
      b => setBalance(b.credits),
      () => setBalance(null),
    );
  }, [loggedIn]);

  // Fetch on login/logout transitions.
  useEffect(() => {
    refresh();
  }, [refresh]);

  // 1-hour polling + agent_end listener.
  useEffect(() => {
    if (loggedIn) {
      intervalRef.current = setInterval(refresh, POLL_INTERVAL_MS);
    }
    return () => {
      if (intervalRef.current)
        clearInterval(intervalRef.current);
    };
  }, [loggedIn, refresh]);

  // Refresh after a conversation finishes (agent_end custom event).
  useEffect(() => onFutureEvent("agent_end", () => {
    clearFutureBalanceCache();
    refresh();
  }), [refresh]);

  // Clear on logout / key-update.
  useEffect(() => onFutureEvent("future-auth-changed", () => {
    clearFutureBalanceCache();
    setBalance(null);
  }), []);

  return balance;
}
