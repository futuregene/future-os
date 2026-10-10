import { useEffect, useState } from "react";
import { fetchRemoteToolTarget } from "./remoteToolTarget";

/**
 * The target for one tool call, if there is one to fetch.
 *
 * A hook rather than a fetch inside the component so the request is not repeated
 * for a row that re-renders: `fetchRemoteToolTarget` caches, and this only holds
 * the resolved string.
 *
 * Nothing is requested when the row has no call id or no run: either would be a
 * request the host cannot answer, and a transcript full of those is worse than
 * one that shows less.
 */
export function useRemoteToolTarget(input: {
  desktopId: string;
  enabled: boolean;
  runId: string;
  sessionId: string;
  toolCallId: string;
}): string | null {
  const { desktopId, enabled, runId, sessionId, toolCallId } = input;
  const [target, setTarget] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled || !desktopId || !sessionId || !runId || !toolCallId) {
      setTarget(null);
      return;
    }
    let cancelled = false;
    void fetchRemoteToolTarget({ desktopId, runId, sessionId, toolCallId }).then((value) => {
      // A row that scrolled away or belongs to another conversation must not
      // write its value into this one.
      if (!cancelled)
        setTarget(value);
    });
    return () => {
      cancelled = true;
    };
  }, [desktopId, enabled, runId, sessionId, toolCallId]);

  return target;
}
