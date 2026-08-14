import type { RemoteStatus } from "../../../features/remote/remoteClient";
import { useCallback, useState } from "react";
import { getRemoteStatus } from "../../../features/remote/remoteClient";
import { usePolling } from "../../../lib/usePolling";

/**
 * Live connection state for the left-nav Remote indicator dot:
 * - `"connected"` — bridge is up and healthy (blue dot).
 * - `"reconnecting"` — bridge is reconnecting a failed generation (yellow dot).
 * - `"error"` — bridge reports a problem, e.g. network/revoked (yellow dot).
 * - `null` — not connected / not running (no dot).
 *
 * Remote control is a dev-only feature, so the caller gates `enabled` to
 * non-release builds; when disabled the poll never runs.
 */
export type RemoteIndicator = "connected" | "reconnecting" | "error" | null;

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

  const indicator: RemoteIndicator = status?.reconnecting
    ? "reconnecting"
    : status?.errorCode
      ? "error"
      : status?.running && status?.connected
        ? "connected"
        : null;

  return { status, indicator, refresh };
}
