import type { AgentSessionUsage } from "../../integrations/agent/agentStateCache";
import { useEffect, useState } from "react";
import { requestRemotePeer } from "./remotePeerClient";

/**
 * One conversation's token usage and amount, as its host reports it.
 *
 * Read from the host's own session state (`get_state`), which is where this
 * app's header figure comes from too — the same source, so a remote
 * conversation's account book and the host's own cannot disagree.
 *
 * Fetched on demand rather than with the transcript: the figure is only ever
 * looked at when the dialog opens, and a state read on every streaming push
 * would be a request per delta.
 */
export function useRemoteUsage(
  desktopId: string,
  sessionId: string,
  enabled: boolean,
): { usage: AgentSessionUsage | undefined; loading: boolean; error: string | null } {
  const [usage, setUsage] = useState<AgentSessionUsage | undefined>(undefined);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled || !desktopId || !sessionId) {
      setUsage(undefined);
      setError(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    void requestRemotePeer<Record<string, unknown>>(
      desktopId,
      { type: "get_state", sessionId },
      sessionId,
    ).then((state) => {
      // Absent on an older host, and on a session with nothing recorded yet: the
      // dialog then says so rather than showing a fabricated zero.
      if (!cancelled)
        setUsage(parseRemoteUsage(state?.usage));
    }).catch((err: unknown) => {
      if (!cancelled)
        setError(err instanceof Error ? err.message : String(err));
    }).finally(() => {
      if (!cancelled)
        setLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, [desktopId, sessionId, enabled]);

  return { error, loading, usage };
}

/**
 * The usage record from a host's state payload.
 *
 * Field-by-field and total, like this app's own parser: a missing or malformed
 * field reads as zero rather than as `NaN`, and `undefined` (no `usage` at all)
 * stays `undefined` so the dialog can say "not reported".
 */
function parseRemoteUsage(value: unknown): AgentSessionUsage | undefined {
  if (!value || typeof value !== "object")
    return undefined;
  const record = value as Record<string, unknown>;
  const count = (key: string) => {
    const field = record[key];
    return typeof field === "number" && Number.isFinite(field) ? field : 0;
  };
  return {
    inputTokens: count("inputTokens"),
    outputTokens: count("outputTokens"),
    cacheReadTokens: count("cacheReadTokens"),
    cacheWriteTokens: count("cacheWriteTokens"),
    costCny: count("costCny"),
    costInputCny: count("costInputCny"),
    costOutputCny: count("costOutputCny"),
    costCacheReadCny: count("costCacheReadCny"),
    costCacheWriteCny: count("costCacheWriteCny"),
  };
}
