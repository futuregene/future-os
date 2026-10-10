import type { SkillCatalogData } from "../agent/useSkillRecommendation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { listRemoteAvailableSkills, listRemoteInstalledSkills } from "./remotePeerClient";

/**
 * The catalogue of a paired computer, for the recommendation card.
 *
 * The candidate set has to be *that* machine's skills: recommending one this
 * machine has installed, or offering to install one the other machine already
 * has, would both be wrong in a way the user only discovers after accepting.
 *
 * A failed read is an empty catalogue rather than an error: recommendation is
 * best-effort, and "no candidates" already means "no recommendation", which is
 * the same outcome the caller would reach from a thrown read.
 */
export function useRemoteSkillCatalog(
  desktopId: string,
  enabled: boolean,
): SkillCatalogData & { reload: () => void } {
  const [catalog, setCatalog] = useState<SkillCatalogData>({ catalogue: [], installed: [] });
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    if (!enabled) {
      setCatalog({ catalogue: [], installed: [] });
      return;
    }
    let cancelled = false;
    void (async () => {
      const [available, installed] = await Promise.all([
        listRemoteAvailableSkills(desktopId).catch(() => []),
        listRemoteInstalledSkills(desktopId).catch(() => []),
      ]);
      // A read that finishes after the conversation moved on must not install
      // another computer's catalogue here.
      if (cancelled)
        return;
      setCatalog({
        catalogue: available.map(skill => ({
          id: skill.id,
          description: skill.description,
          descriptionZh: skill.descriptionZh,
          version: skill.latestVersion ?? null,
        })),
        installed: installed.map(skill => ({ id: skill.id })),
      });
    })();
    return () => {
      cancelled = true;
    };
  }, [desktopId, enabled, revision]);

  // Installing a skill on that host changes the candidate set, and the card's
  // caller has no other way to say so.
  const reload = useCallback(() => setRevision(current => current + 1), []);

  // Memoised because the caller feeds this to a hook whose candidate set is
  // derived from it: a new object every render would rebuild that set on every
  // render, including during typing.
  return useMemo(() => ({ ...catalog, reload }), [catalog, reload]);
}
