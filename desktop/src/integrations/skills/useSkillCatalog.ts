import type { AvailableSkill, InstalledSkill } from "./skillsClient";
import { useEffect, useState, useSyncExternalStore } from "react";
import { getSkillCatalogRevision, loadSkillCatalog, subscribeSkillCatalog } from "./skillsClient";

interface SkillCatalogSnapshot {
  installed: InstalledSkill[];
  catalogue: AvailableSkill[];
}

/** Shared cached reads, refreshed for local mutations and external changes. */
export function useSkillCatalog(enabled = true) {
  const revision = useSyncExternalStore(subscribeSkillCatalog, getSkillCatalogRevision);
  const [snapshot, setSnapshot] = useState<SkillCatalogSnapshot>({ installed: [], catalogue: [] });
  useEffect(() => {
    if (!enabled)
      return;
    let cancelled = false;
    const { installed, catalogue } = loadSkillCatalog();
    void Promise.all([installed.catch(() => []), catalogue.catch(() => [])])
      .then(([installed, catalogue]) => {
        if (!cancelled)
          setSnapshot({ installed, catalogue });
      });
    return () => {
      cancelled = true;
    };
  }, [enabled, revision]);
  return snapshot;
}
