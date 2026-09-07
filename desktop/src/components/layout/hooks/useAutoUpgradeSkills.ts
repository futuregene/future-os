import { useEffect, useRef } from "react";
import { computeSkillUpgrades } from "../../../features/skills/autoUpgrade";
import {
  installSkill,
  listAvailableSkills,
  listInstalledSkills,
} from "../../../integrations/skills/skillsClient";
import { emitFutureEvent } from "../../../lib/futureEvents";

/**
 * Silently upgrade when enabled. Each effect owns its cancellation signal;
 * replacement effects wait for an in-flight install rather than being skipped.
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
        const [installed, available] = await Promise.all([
          listInstalledSkills(),
          listAvailableSkills(),
        ]);
        if (signal.aborted)
          return;

        const upgrades = computeSkillUpgrades(installed, available);
        let upgradedCount = 0;
        for (const upgrade of upgrades) {
          if (signal.aborted)
            break;
          try {
            await installSkill(upgrade.id, upgrade.version);
            upgradedCount += 1;
          }
          catch (error) {
            console.warn(`[skills] auto-upgrade failed for ${upgrade.id}`, error);
          }
        }
        if (!signal.aborted && upgradedCount > 0)
          emitFutureEvent("skills-changed", undefined);
      }
      catch (error) {
        console.warn("[skills] auto-upgrade skipped", error);
      }
    });

    return () => controller.abort();
  }, [enabled]);
}
