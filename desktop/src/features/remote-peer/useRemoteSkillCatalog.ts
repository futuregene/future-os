import type { SkillCatalogData } from "../agent/useSkillRecommendation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { forgetHostCatalogue, hostSkillCatalogue, useHostCatalogueInvalidation } from "./remoteHostCatalog";

/** The empty catalogue: what a host that cannot answer is reported as. */
const NOTHING: SkillCatalogData = { catalogue: [], installed: [] };

/**
 * The catalogue of a paired computer, for the recommendation card.
 *
 * The candidate set has to be *that* machine's skills: recommending one this
 * machine has installed, or offering to install one the other machine already
 * has, would both be wrong in a way the user only discovers after accepting.
 *
 * Both lists describe the host rather than a conversation, so they are read once
 * per connection and dropped when the host says its skills moved — the phone
 * caches its available-skills read per connection for the same reason. Reading
 * them on every conversation opened cost two commands before the page that draws
 * the transcript, and paid them again for every other conversation.
 *
 * A failed read is an empty catalogue rather than an error: recommendation is
 * best-effort, and "no candidates" already means "no recommendation", which is
 * the same outcome the caller would reach from a thrown read.
 */
export function useRemoteSkillCatalog(
  desktopId: string,
  enabled: boolean,
): SkillCatalogData & { reload: () => void } {
  const [catalog, setCatalog] = useState<SkillCatalogData>(NOTHING);

  const read = useCallback(() => {
    if (!desktopId || !enabled) {
      setCatalog(NOTHING);
      return;
    }
    void hostSkillCatalogue(desktopId)
      .then(({ available, installed }) => {
        setCatalog({
          catalogue: available.map(skill => ({
            id: skill.id,
            description: skill.description,
            descriptionZh: skill.descriptionZh,
            version: skill.latestVersion ?? null,
          })),
          installed: installed.map(skill => ({ id: skill.id })),
        });
      })
      // A read that failed leaves the last one standing: the card's own empty
      // state is the honest answer, and a rejection here would be an unhandled
      // one (there is nothing on screen that a failed *candidate* read breaks).
      .catch(() => undefined);
  }, [desktopId, enabled]);

  useEffect(() => {
    read();
  }, [read]);

  // The host's skills moved, and the cached read behind this one was dropped
  // before we got here.
  useHostCatalogueInvalidation(desktopId, (domain) => {
    if (domain === "skills")
      read();
  });

  // Installing a skill on that host changes the candidate set, and the card's
  // caller has no other way to say so. The cached read goes with it, so this is
  // a real request rather than the answer that was there a moment ago.
  const reload = useCallback(() => {
    if (desktopId)
      forgetHostCatalogue(desktopId, "skills");
    read();
  }, [desktopId, read]);

  // Memoised because the caller feeds this to a hook whose candidate set is
  // derived from it: a new object every render would rebuild that set on every
  // render, including during typing.
  return useMemo(() => ({ ...catalog, reload }), [catalog, reload]);
}
