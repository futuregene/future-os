import { useEffect, useState } from "react";
import { clearFutureBalanceCache, FUTURE_PROVIDER_ID, getFutureBalance, listAgentProviders } from "../../../integrations/agent/providers";
import { onFutureEvent } from "../../../lib/futureEvents";
import { useAsyncResource } from "../../../lib/useAsyncResource";

/**
 * Fetch the FutureOS credit balance. Returns `null` when signed out or on
 * error; `number` (credits) otherwise. Re-fetches on login; clears on
 * logout / key-update.
 */
export function useFutureBalance(): number | null {
  const { data: providers } = useAsyncResource(listAgentProviders, [], null);
  const loggedIn = Boolean(
    providers?.builtin.some(
      p => p.id === FUTURE_PROVIDER_ID && p.hasApiKey,
    ),
  );

  const [balance, setBalance] = useState<number | null>(null);

  useEffect(() => {
    if (!loggedIn) {
      setBalance(null);
      return;
    }
    getFutureBalance().then(
      b => setBalance(b.credits),
      () => setBalance(null),
    );
  }, [loggedIn]);

  // Clear the balance cache and local state on logout / key-update.
  useEffect(() => onFutureEvent("future-auth-changed", () => {
    clearFutureBalanceCache();
    setBalance(null);
  }), []);

  return balance;
}
