import { useCallback, useEffect, useState } from "react";
import {
  clearFutureBalanceCache,
  clearFutureProfileCache,
  FUTURE_PROVIDER_ID,
  getFutureBalance,
  getFutureProfile,
  listAgentProviders,
  peekFutureBalance,
  peekFutureProfile,
  storeFutureBalance,
} from "../../../integrations/agent/providers";
import { onFutureEvent } from "../../../lib/futureEvents";
import { useAsyncResource } from "../../../lib/useAsyncResource";
import { useTauriEvent } from "../../../lib/useTauriEvent";

export interface FutureAccount {
  /** Credit balance, truncated to an integer-ish value; null when signed out or on error. */
  balance: number | null;
  /** Signed-in email; null when signed out or on error. */
  email: string | null;
}

/**
 * Single source of truth for the signed-in FutureOS account: credit balance and
 * email. The backend scheduler refreshes the balance hourly; this hook still
 * refreshes immediately on mount, conversation completion (`agent_end`), and
 * auth transitions (`future-auth-changed`).
 */
export function useFutureAccount(): FutureAccount {
  const { data: providers, reload: reloadProviders } = useAsyncResource(listAgentProviders, [], null);
  const loggedIn = Boolean(
    providers?.builtin.some(p => p.id === FUTURE_PROVIDER_ID && p.hasApiKey),
  );

  // Seed from the in-memory cache so reopening a view (e.g. the Settings dialog)
  // shows the last-known value instantly instead of flashing null → value.
  const [balance, setBalance] = useState<number | null>(() => peekFutureBalance()?.credits ?? null);
  const [email, setEmail] = useState<string | null>(() => peekFutureProfile()?.email ?? null);

  const refreshBalance = useCallback(() => {
    if (!loggedIn) {
      setBalance(null);
      return;
    }
    getFutureBalance(true).then(
      b => setBalance(b.credits),
      () => setBalance(null),
    );
  }, [loggedIn]);

  const refreshEmail = useCallback(() => {
    if (!loggedIn) {
      setEmail(null);
      return;
    }
    getFutureProfile(true).then(
      p => setEmail(p.email),
      () => setEmail(null),
    );
  }, [loggedIn]);

  // Fetch on mount and on every login/logout transition.
  useEffect(() => {
    refreshBalance();
    refreshEmail();
  }, [refreshBalance, refreshEmail]);

  useTauriEvent<{ credits: number }>("scheduler-future-balance", (next) => {
    storeFutureBalance(next);
    setBalance(next.credits);
  });

  // A finished conversation likely spent credits — refresh promptly.
  useEffect(
    () => onFutureEvent("agent_end", () => {
      clearFutureBalanceCache();
      refreshBalance();
    }),
    [refreshBalance],
  );

  // Auth change: drop caches + local state, then reload providers so `loggedIn`
  // recomputes (which re-triggers the fetch effects above).
  useEffect(
    () => onFutureEvent("future-auth-changed", () => {
      clearFutureBalanceCache();
      clearFutureProfileCache();
      setBalance(null);
      setEmail(null);
      reloadProviders();
    }),
    [reloadProviders],
  );

  return { balance, email };
}
