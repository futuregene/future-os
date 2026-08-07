import { useCallback, useEffect, useRef, useState } from "react";
import {
  clearFutureBalanceCache,
  clearFutureProfileCache,
  FUTURE_PROVIDER_ID,
  getFutureBalance,
  getFutureProfile,
  listAgentProviders,
  peekFutureBalance,
  peekFutureProfile,
} from "../../../integrations/agent/providers";
import { onFutureEvent } from "../../../lib/futureEvents";
import { useAsyncResource } from "../../../lib/useAsyncResource";

const POLL_INTERVAL_MS = 3_600_000; // 1 hour

export interface FutureAccount {
  /** Credit balance, truncated to an integer-ish value; null when signed out or on error. */
  balance: number | null;
  /** Signed-in email; null when signed out or on error. */
  email: string | null;
}

/**
 * Single source of truth for the signed-in FutureOS account: credit balance and
 * email. Polls the balance hourly, refreshes it when a conversation finishes
 * (`agent_end`), and reloads everything on auth transitions (`future-auth-changed`)
 * — including a provider reload so the logged-in flag recomputes immediately
 * after sign-in/out instead of waiting for the next poll.
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
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

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

  // Hourly balance polling while signed in.
  useEffect(() => {
    if (!loggedIn)
      return;
    intervalRef.current = setInterval(refreshBalance, POLL_INTERVAL_MS);
    return () => {
      if (intervalRef.current)
        clearInterval(intervalRef.current);
    };
  }, [loggedIn, refreshBalance]);

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
