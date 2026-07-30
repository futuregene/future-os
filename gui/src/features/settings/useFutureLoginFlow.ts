import type { FutureLoginStart } from "../../integrations/agent/providers";
import { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { pollFutureLogin, startFutureLogin } from "../../integrations/agent/providers";
import { usePolling } from "../../lib/usePolling";

export type FutureLoginPhase
  = | "idle"
    | "starting"
    | "waiting"
    | "denied"
    | "expired"
    | "error";

const SLOW_DOWN_STEP_MS = 5000;
// Poll faster than the server's suggested interval for snappier "authorized"
// detection; if the server pushes back with `slow_down` we back off (+5s).
const FAST_POLL_MS = 2000;
// usePolling ticks at this fixed cadence; the real poll spacing is gated by
// `nextPollAtRef`, so a slow_down back-off widens the interval without
// restarting the timer (a restart would fire an immediate extra poll — the
// opposite of what slow_down asks for).
const BASE_TICK_MS = 1000;

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Device-code login state machine, shared by the settings login dialog and the
 * app-wide forced-login gate. Stays `idle` until `begin()` runs — the caller
 * picks the trigger (dialog open vs. a button press in the gate). `cancel()`
 * aborts any in-flight attempt. Polling runs only while `phase === "waiting"`.
 *
 * `onAuthorized` fires once on success; the device-login poll also broadcasts a
 * `future-auth-changed` event so the app-wide gate clears regardless of which
 * surface ran the flow.
 */
export function useFutureLoginFlow(onAuthorized: () => void) {
  const { t } = useTranslation("settings");
  const [phase, setPhase] = useState<FutureLoginPhase>("idle");
  const [start, setStart] = useState<FutureLoginStart | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  // Latest attempt id: a poll response is discarded if a newer attempt (retry /
  // cancel) started while it was in flight (usePolling does not cancel in-flight
  // async). Also gates the per-attempt expiry deadline.
  const attemptRef = useRef(0);
  const deadlineRef = useRef(0);
  // Current poll spacing (grows by 5s on each slow_down) and the epoch-ms gate
  // for the next allowed poll. Kept in refs so back-off never churns the polling
  // effect's deps (which would restart the timer and poll immediately).
  const intervalRef = useRef(FAST_POLL_MS);
  const nextPollAtRef = useRef(0);

  const begin = useCallback(async () => {
    const attempt = attemptRef.current + 1;
    attemptRef.current = attempt;
    setPhase("starting");
    setMessage(null);
    setStart(null);
    try {
      const next = await startFutureLogin();
      if (attempt !== attemptRef.current)
        return;
      setStart(next);
      // Start snappy; respect the server interval only if it asks for slower.
      intervalRef.current = Math.min(Math.max(1, next.interval) * 1000, FAST_POLL_MS);
      nextPollAtRef.current = 0; // first tick polls immediately
      deadlineRef.current = Date.now() + next.expiresIn * 1000;
      setPhase("waiting");
    }
    catch (error) {
      if (attempt !== attemptRef.current)
        return;
      setMessage(errorText(error));
      setPhase("error");
    }
  }, []);

  const cancel = useCallback(() => {
    // Bump the attempt id so any in-flight begin/poll is ignored, and drop back
    // to idle so the polling effect's `enabled` flips off.
    attemptRef.current += 1;
    setPhase("idle");
    setMessage(null);
    setStart(null);
  }, []);

  usePolling(
    async () => {
      const current = start;
      if (!current)
        return;
      const attempt = attemptRef.current;
      if (Date.now() > deadlineRef.current) {
        // Invalidate any in-flight poll so a late "authorized" can't slip past
        // expiry and fire onAuthorized.
        attemptRef.current += 1;
        setPhase("expired");
        setMessage(t("futureLogin.expired"));
        return;
      }
      // Back-off gate: only poll once we're past the reserved slot.
      if (Date.now() < nextPollAtRef.current)
        return;
      // Reserve the next slot up front so a slow in-flight poll doesn't stack.
      nextPollAtRef.current = Date.now() + intervalRef.current;

      let result;
      try {
        result = await pollFutureLogin(current.deviceCode);
      }
      catch (error) {
        if (attempt !== attemptRef.current)
          return;
        setMessage(errorText(error));
        setPhase("error");
        return;
      }
      if (attempt !== attemptRef.current)
        return;

      switch (result.status) {
        case "authorized":
          // Invalidate further polls before handing off to the caller.
          attemptRef.current += 1;
          setPhase("idle");
          setStart(null);
          onAuthorized();
          break;
        case "pending":
          break;
        case "slow_down":
          // RFC 8628: widen the interval by 5s and wait it out — no immediate
          // retry (which is what the gate above enforces).
          intervalRef.current += SLOW_DOWN_STEP_MS;
          nextPollAtRef.current = Date.now() + intervalRef.current;
          break;
        case "denied":
          setMessage(result.message ?? t("futureLogin.denied"));
          setPhase("denied");
          break;
        case "expired":
          setMessage(result.message ?? t("futureLogin.expired"));
          setPhase("expired");
          break;
        default:
          setMessage(result.message ?? t("futureLogin.failed"));
          setPhase("error");
          break;
      }
    },
    BASE_TICK_MS,
    { enabled: phase === "waiting" && start !== null, deps: [phase, start] },
  );

  return { phase, message, start, begin, cancel };
}
