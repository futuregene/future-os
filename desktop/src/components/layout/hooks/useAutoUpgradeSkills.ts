import { useEffect, useRef } from "react";
import { syncSkills } from "../../../integrations/skills/skillsClient";
import { emitFutureEvent } from "../../../lib/futureEvents";

/**
 * Silently upgrade when enabled. Each effect owns its cancellation signal;
 * replacement effects wait for an in-flight sync rather than being skipped.
 * This also handles StrictMode's setup/cleanup/setup and rapid setting toggles.
 */
export function useAutoUpgradeSkills(enabled: boolean): void {
  const pendingRef = useRef<Promise<void>>(Promise.resolve());

  useEffect(() => {
    if (!enabled)
      return;

    const controller = new AbortController();
    const { signal } = controller;
    pendingRef.current = pendingRef.current.then(async () => {
      if (signal.aborted)
        return;
      try {
        const result = await syncSkills();
        if (!signal.aborted && (result.installed.length > 0 || result.upgraded.length > 0))
          emitFutureEvent("skills-changed", undefined);
        for (const failure of result.failed)
          console.warn("[skills] auto-sync failed", failure);
      }
      catch (error) {
        console.warn("[skills] auto-upgrade skipped", error);
      }
    });

    return () => controller.abort();
  }, [enabled]);
}
