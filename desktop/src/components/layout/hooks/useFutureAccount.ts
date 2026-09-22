import type { FutureAuthState, FutureAuthStatus } from "../../../integrations/agent/providers";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  clearFutureBalanceCache,
  clearFutureProfileCache,
  getFutureAuthState,
  getFutureBalance,
  peekFutureBalance,
  peekFutureProfile,
  storeFutureBalance,
} from "../../../integrations/agent/providers";
import { onFutureEvent } from "../../../lib/futureEvents";
import { useTauriEvent } from "../../../lib/useTauriEvent";

export type FutureSessionStatus = FutureAuthStatus | "checking";
export type FutureBalanceStatus = "idle" | "loading" | "available" | "unavailable";

export interface FutureAccount {
  status: FutureSessionStatus;
  balanceStatus: FutureBalanceStatus;
  balance: number | null;
  email: string | null;
  refreshAuth: () => void;
  refreshBalance: () => void;
}

/**
 * App-wide FutureOS account state. The platform profile check is authoritative:
 * a configured key is not treated as a valid login until that check succeeds.
 * Request generations prevent a response from an old credential restoring stale
 * email or balance after logout, environment change, or reauthentication.
 */
export function useFutureAccount(initialAuth?: FutureAuthState): FutureAccount {
  const [status, setStatus] = useState<FutureSessionStatus>(initialAuth?.status ?? "checking");
  const [balanceStatus, setBalanceStatus] = useState<FutureBalanceStatus>(
    () => peekFutureBalance() ? "available" : "idle",
  );
  const [balance, setBalance] = useState<number | null>(() => peekFutureBalance()?.credits ?? null);
  const [email, setEmail] = useState<string | null>(() => initialAuth?.profile?.email ?? peekFutureProfile()?.email ?? null);
  const generationRef = useRef(0);

  const refreshAuth = useCallback(() => {
    const generation = generationRef.current;
    getFutureAuthState().then(
      (next) => {
        if (generation !== generationRef.current)
          return;
        setStatus(next.status);
        if (next.status === "authenticated" && next.profile) {
          setEmail(next.profile.email);
        }
        else if (next.status === "signed_out" || next.status === "invalid") {
          // Invalidate any profile/balance work started with this credential.
          generationRef.current += 1;
          clearFutureProfileCache();
          clearFutureBalanceCache();
          setEmail(null);
          setBalance(null);
          setBalanceStatus("idle");
        }
        else if (next.status === "unavailable") {
          setBalanceStatus(current => current === "available" ? current : "unavailable");
        }
        // On a temporary verification failure, retain last-known account data.
      },
      () => {
        if (generation === generationRef.current) {
          setStatus("unavailable");
          setBalanceStatus(current => current === "available" ? current : "unavailable");
        }
      },
    );
  }, []);

  const refreshBalance = useCallback(() => {
    if (status !== "authenticated")
      return;
    const generation = generationRef.current;
    setBalanceStatus("loading");
    getFutureBalance(true).then(
      (next) => {
        if (generation !== generationRef.current)
          return;
        setBalance(next.credits);
        setBalanceStatus("available");
      },
      () => {
        if (generation !== generationRef.current)
          return;
        setBalance(null);
        setBalanceStatus("unavailable");
        // A balance failure may be the first observation of a revoked key.
        refreshAuth();
      },
    );
  }, [refreshAuth, status]);

  useEffect(() => {
    refreshAuth();
  }, [refreshAuth]);

  useEffect(() => {
    if (status === "authenticated")
      refreshBalance();
  }, [status, refreshBalance]);

  useTauriEvent<{ credits: number }>("scheduler-future-balance", (next) => {
    storeFutureBalance(next);
    setBalance(next.credits);
    setBalanceStatus("available");
  });

  useTauriEvent("scheduler-future-auth-invalid", refreshAuth);

  useEffect(
    () => onFutureEvent("agent_end", () => {
      clearFutureBalanceCache();
      refreshBalance();
    }),
    [refreshBalance],
  );

  useEffect(
    () => onFutureEvent("future-auth-changed", () => {
      generationRef.current += 1;
      clearFutureBalanceCache();
      clearFutureProfileCache();
      setBalance(null);
      setEmail(null);
      setBalanceStatus("idle");
      setStatus("checking");
      queueMicrotask(refreshAuth);
    }),
    [refreshAuth],
  );

  return { status, balanceStatus, balance, email, refreshAuth, refreshBalance };
}
