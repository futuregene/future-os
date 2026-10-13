import type { RemotePeerEvent } from "../../../features/remote-peer/remotePeerClient";
import { useEffect, useRef } from "react";
import { playDoneBell } from "../../../lib/doneBell";
import { onFutureEvent } from "../../../lib/futureEvents";
import { useTauriEvent } from "../../../lib/useTauriEvent";
import { requestUserAttention } from "../../../lib/windowAttention";

/**
 * Global "remote run done" alert: when any paired host finishes or fails a
 * run, play the same WebAudio bell and request window attention that local
 * runs use. Gated by the same `bellOnComplete` setting.
 *
 * Termination events (`run_finished`, `run_failed`, `agent_end`) can all arrive
 * for the same run, so each `(desktopId, sessionId, runId)` is remembered and
 * only the first one rings.
 *
 * Pushes from before this window was listening (e.g. after a reconnect) must
 * not ring: only events that start a new run after we started watching trigger
 * the bell. This is enforced with a small in-memory set of "runs known to be
 * in flight" — a run is only "done" if it was first seen running.
 */
export function useRemoteDoneBell(enabled: boolean) {
  const acknowledgedRef = useRef<Set<string>>(new Set());
  const flyingRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    acknowledgedRef.current.clear();
    flyingRef.current.clear();
  }, []);

  useEffect(() => {
    if (!enabled)
      return;
    return onFutureEvent("agent_end", () => {
      playDoneBell();
      void requestUserAttention();
    });
  }, [enabled]);

  useTauriEvent<RemotePeerEvent>("remote-peer-event", (event) => {
    if (!enabled)
      return;
    const { desktopId, kind, payload } = event;
    if (kind !== "event" || typeof payload !== "object" || payload === null)
      return;

    const record = payload as Record<string, unknown>;
    const type = typeof record.type === "string" ? record.type : "";
    const sessionId = typeof record.sessionId === "string" ? record.sessionId : "";
    const runId = typeof record.runId === "string" ? record.runId : "";
    if (!sessionId)
      return;

    const runKey = `${desktopId}::${sessionId}::${runId || "_"}`;

    // Track in-flight runs so we only ring for runs that started after we did.
    const isRunStart = type === "agent_start" || type === "run_started";
    if (isRunStart && runId) {
      flyingRef.current.add(runKey);
      return;
    }

    const isRunEnd = type === "run_finished" || type === "run_failed" || type === "agent_end";
    if (!isRunEnd)
      return;

    // Only ring for runs that were seen in flight; this suppresses the
    // flood of historical end events after a reconnect. Runs without a runId
    // are still rung once per session, because some terminal events (legacy
    // agent_end) do not carry one.
    if (runId && !flyingRef.current.has(runKey))
      return;

    if (acknowledgedRef.current.has(runKey))
      return;
    acknowledgedRef.current.add(runKey);

    playDoneBell();
    void requestUserAttention();
  });
}
