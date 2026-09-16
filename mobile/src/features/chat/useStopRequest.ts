import { useCallback, useEffect, useRef, useState } from "react";
import { AppState } from "react-native";

type StopStatus = "idle" | "requesting" | "requested" | "failed";
type Attempt = { id: number; startedAt: number; inFlight: boolean };

/** Keep request acknowledgement distinct from the authoritative streaming state.
 * A failed request must be visible; a successful ACK must not pretend the run
 * has ended. Diagnostics contain no message text or transport credentials. */
export function useStopRequest(
  streaming: boolean,
  sessionId: string,
  abort: () => Promise<void>,
) {
  const [status, setStatus] = useState<StopStatus>("idle");
  const [scope, setScope] = useState({ streaming, sessionId });
  if (scope.streaming !== streaming || scope.sessionId !== sessionId) {
    setScope({ streaming, sessionId });
    setStatus("idle");
  }
  const attemptRef = useRef<Attempt | null>(null);
  const sequenceRef = useRef(0);
  const lagRef = useRef<{ at: number; delay: number }[]>([]);
  const nextTickRef = useRef<number | null>(null);

  useEffect(() => {
    if (!streaming) return;
    let timer: ReturnType<typeof setInterval> | undefined;
    const reset = (state: string | null) => {
      clearInterval(timer);
      lagRef.current = [];
      nextTickRef.current = null;
      if (state !== null && state !== "active") return;
      nextTickRef.current = performance.now() + 250;
      // Sample without React state updates or per-tick logging. Discard time
      // spent backgrounded: OS timer suspension is not a JS rendering stall.
      timer = setInterval(() => {
        const now = performance.now();
        lagRef.current = [
          ...lagRef.current.filter(sample => now - sample.at <= 2000),
          { at: now, delay: Math.max(0, now - nextTickRef.current!) },
        ];
        nextTickRef.current = now + 250;
      }, 250);
    };
    reset(AppState.currentState);
    const subscription = AppState.addEventListener("change", reset);
    return () => {
      clearInterval(timer);
      subscription.remove();
      nextTickRef.current = null;
      lagRef.current = [];
    };
  }, [streaming]);

  const log = useCallback((attempt: Attempt, stage: string) => {
    const now = performance.now();
    const recentJsLagMs = Math.round(Math.max(
      0,
      ...lagRef.current.filter(sample => now - sample.at <= 2000).map(sample => sample.delay),
      nextTickRef.current === null ? 0 : now - nextTickRef.current,
    ));
    // eslint-disable-next-line no-console -- One record per stop phase, not per streaming update.
    console.info("[remote] stop timing", {
      sessionId,
      attempt: attempt.id,
      stage,
      elapsedMs: Math.round(now - attempt.startedAt),
      recentJsLagMs,
    });
  }, [sessionId]);

  useEffect(() => {
    if (streaming) return;
    const attempt = attemptRef.current;
    if (attempt) log(attempt, "streaming_cleared");
    attemptRef.current = null;
  }, [streaming, log]);

  useEffect(() => {
    return () => {
      // Navigation/unmount is NOT confirmation that the run stopped. Invalidate
      // late replies so they cannot update a different conversation's controls.
      const attempt = attemptRef.current;
      if (attempt) log(attempt, "view_detached");
      attemptRef.current = null;
    };
  }, [sessionId, log]);

  const stop = useCallback(async () => {
    if (!streaming || attemptRef.current?.inFlight) return;
    const attempt: Attempt = {
      id: ++sequenceRef.current,
      startedAt: performance.now(),
      inFlight: true,
    };
    attemptRef.current = attempt;
    setStatus("requesting");
    // This is JS callback receipt, not native touch time: their clocks need
    // not share an epoch. Native-to-JS latency still needs a device profiler.
    log(attempt, "press_received");
    try {
      await abort();
      log(attempt, "request_acknowledged");
      if (attemptRef.current === attempt) setStatus("requested");
    } catch {
      log(attempt, "request_failed");
      if (attemptRef.current === attempt) setStatus("failed");
    } finally {
      attempt.inFlight = false;
    }
  }, [streaming, abort, log]);

  return { status, stop };
}
