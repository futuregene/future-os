import { useEffect, useRef } from "react";
import { computeSkillUpgrades } from "../../../features/skills/autoUpgrade";
import {
  installSkill,
  listAvailableSkills,
  listInstalledSkills,
} from "../../../integrations/skills/skillsClient";
import { emitFutureEvent } from "../../../lib/futureEvents";

/**
 * Silently upgrades installed skills to their latest catalogue version. Runs
 * whenever `enabled` becomes true — on app open if the setting is already on,
 * and immediately when the user toggles it on. Fully silent: successes and
 * failures only reach the console, never a toast. A run is skipped while a
 * previous one is still in flight, and if the catalogue can't be reached (e.g.
 * the platform is offline) it simply does nothing this launch.
 */
export function useAutoUpgradeSkills(enabled: boolean): void {
  const runningRef = useRef(false);

  useEffect(() => {
    if (!enabled || runningRef.current)
      return;

    let cancelled = false;
    runningRef.current = true;

    void (async () => {
      try {
        const [installed, available] = await Promise.all([
          listInstalledSkills(),
          listAvailableSkills(),
        ]);
        if (cancelled)
          return;

        const upgrades = computeSkillUpgrades(installed, available);
        let upgradedCount = 0;
        for (const upgrade of upgrades) {
          if (cancelled)
            break;
          try {
            // Overwrite-install the newer version. Sequential to avoid parallel
            // writes into the shared skills directory.
            await installSkill(upgrade.id, upgrade.version);
            upgradedCount += 1;
          }
          catch (error) {
            console.warn(`[skills] auto-upgrade failed for ${upgrade.id}`, error);
          }
        }

        // Let an open Skills view refresh so it reflects the new versions.
        if (!cancelled && upgradedCount > 0)
          emitFutureEvent("skills-changed", undefined);
      }
      catch (error) {
        // Catalogue or installed-list fetch failed — stay silent.
        console.warn("[skills] auto-upgrade skipped", error);
      }
      finally {
        runningRef.current = false;
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [enabled]);
}
