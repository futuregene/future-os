import { useCallback, useEffect, useRef, useState } from "react";
import { invokeCommand } from "../../../integrations/tauri/invoke";

/** Mirrors the backend `UpdateStatus` (serde camelCase). */
export interface UpdateStatus {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  platformSupported: boolean;
  downloadUrl: string | null;
}

const CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000; // 24 hours

/**
 * Silently checks for app updates on mount and every 24 hours thereafter.
 * The last successful check result is cached for the app's lifetime so the
 * UpdatePage can display it immediately without a redundant round-trip.
 *
 * - `hasUpdate` — true when a new version exists and the user hasn't visited
 *   the update tab yet (drives the red-dot indicators).
 * - `cachedStatus` — the full result from the most recent successful check.
 * - `markSeen` — clears the red-dot flag (user acknowledged the update).
 */
export function useUpdateChecker(): {
  hasUpdate: boolean;
  cachedStatus: UpdateStatus | null;
  markSeen: () => void;
} {
  const [hasUpdate, setHasUpdate] = useState(false);
  const [cachedStatus, setCachedStatus] = useState<UpdateStatus | null>(null);
  const seenRef = useRef(false);

  const check = useCallback(async () => {
    try {
      const status = await invokeCommand<UpdateStatus>("check_app_update");
      setCachedStatus(status);
      if (status.hasUpdate && !seenRef.current) {
        setHasUpdate(true);
      }
    }
    catch {
      // Silent — network errors or non-release builds shouldn't surface UI noise.
    }
  }, []);

  useEffect(() => {
    void check();
    const timer = setInterval(() => void check(), CHECK_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [check]);

  const markSeen = useCallback(() => {
    seenRef.current = true;
    setHasUpdate(false);
  }, []);

  return { hasUpdate, cachedStatus, markSeen };
}
