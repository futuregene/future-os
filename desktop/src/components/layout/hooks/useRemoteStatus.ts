import type { RemoteStatus } from "../../../features/remote/remoteClient";
import { useCallback, useState } from "react";
import { getRemoteStatus, remoteConnectionPresentation } from "../../../features/remote/remoteClient";
import { usePolling } from "../../../lib/usePolling";

/**
 * Live connection state for the left-nav Remote indicator dot:
 * - `"connected"` — bridge is up and healthy (blue dot).
 * - `"connecting"` — remote access is currently attempting a connection (yellow dot).
 * - `"disconnected"` — remote access has stopped or cannot continue (red dot).
 * - `null` — not yet paired (no dot).
 *
 * The caller currently gates this while Remote is still pre-release; the
 * backend supervisor remains independent of this poll.
 */
export type RemoteIndicator = "connected" | "connecting" | "disconnected" | null;

/**
 * Shared remote bridge status — polled once at the app level, consumed by both
 * the sidebar indicator dot and the Remote page so they always agree.
 *
 * Mirrors `useAgentConnection`'s "silent retry" rule: a failed poll keeps the
 * last known state rather than flashing to disconnected.
 */
export function useRemoteStatus(enabled: boolean): {
  status: RemoteStatus | null;
  indicator: RemoteIndicator;
  refresh: () => Promise<void>;
} {
  const [status, setStatus] = useState<RemoteStatus | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await getRemoteStatus();
      // Skip re-render when nothing changed (same JSON). The 3s app-level poll
      // ticks on a large React tree — AppShell is the root — so a no-op tick
      // must not cause a full subtree render in dev builds.
      setStatus(prev => JSON.stringify(prev) === JSON.stringify(next) ? prev : next);
    }
    catch {
      // Keep the last known status on a failed poll (no flashing).
    }
  }, []);

  usePolling(refresh, 3000, { enabled, deps: [refresh] });

  const indicator: RemoteIndicator = remoteConnectionPresentation(status)?.level ?? null;

  return { status, indicator, refresh };
}
