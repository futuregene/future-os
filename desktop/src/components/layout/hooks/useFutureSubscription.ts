import { useEffect, useState } from "react";
import { getFutureTokenPlan, peekFutureTokenPlan } from "../../../integrations/agent/providers";
import { onFutureEvent } from "../../../lib/futureEvents";

/**
 * Whether the signed-in account has a live token-plan subscription.
 *
 * `null` means unknown — signed out, the community edition, or the platform
 * could not be asked — and callers must not gate on it: a failed read must not
 * look like a missing subscription and lock models the account can actually
 * call.
 *
 * One read per app session. It reuses the cached snapshot the account page
 * shows instead of fetching again, and drops that cache when the credential
 * changes or a run ends — the plan's window usage moves with every model call,
 * and a subscription can lapse between sessions.
 */
export function useFutureSubscription(enabled: boolean): boolean | null {
  const [active, setActive] = useState<boolean | null>(() => peekFutureTokenPlan()?.active ?? null);

  useEffect(() => {
    if (!enabled) {
      setActive(null);
      return;
    }
    let cancelled = false;
    getFutureTokenPlan().then(
      (plan) => {
        if (!cancelled)
          setActive(plan.active);
      },
      () => {
        if (!cancelled)
          setActive(null);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [enabled]);

  // A credential change means the subscription belongs to an account that may
  // no longer be the one signed in; until the next read, nothing is known.
  useEffect(
    () => onFutureEvent("future-auth-changed", () => setActive(null)),
    [],
  );

  useEffect(() => {
    if (!enabled)
      return;
    let cancelled = false;
    const unsubscribe = onFutureEvent("agent_end", () => {
      // Forced: the just-finished run may have been paid from a window that is
      // now used up, and a subscription bought while the app was open must stop
      // greying the picker without a restart.
      getFutureTokenPlan(true).then(
        (plan) => {
          if (!cancelled)
            setActive(plan.active);
        },
        () => {
          if (!cancelled)
            setActive(null);
        },
      );
    });
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [enabled]);

  return active;
}
