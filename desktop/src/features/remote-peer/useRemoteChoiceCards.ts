import { useCallback, useEffect, useState } from "react";
import { useChoiceCardsEnabled } from "../markdown/ChoiceCardsContext";
import { hostDesktopSettings, useHostCatalogueInvalidation } from "./remoteHostCatalog";

/**
 * Whether *that* machine draws `futureos-choice` cards.
 *
 * The switch belongs to the machine the conversation lives on, because it also
 * decides whether that machine's agent is taught the fence at all. So a card is
 * drawn here on that machine's answer, not on this one's switch: a reader who
 * turned cards on for a headless host expects to see them in its conversations,
 * and one who turned them off does not get a card this machine happens to like.
 *
 * Until the answer arrives the value is this machine's own switch
 * (`useChoiceCardsEnabled`): the host settings are one cached read per
 * connection, so this is the first frame of a remote conversation rather than a
 * standing state — and inheriting means a machine that agrees with itself draws
 * nothing twice, instead of flashing a card in when the read lands.
 *
 * Anything that leaves the host's answer unknown — a failure, a dropped
 * connection — keeps that inherited value rather than forcing "off": an
 * unreachable host is not a host that said no.
 */
export function useRemoteChoiceCards(desktopId: string): boolean {
  const local = useChoiceCardsEnabled();
  const [host, setHost] = useState<boolean | null>(null);

  const read = useCallback(() => {
    if (!desktopId) {
      setHost(null);
      return;
    }
    void hostDesktopSettings(desktopId)
      .then(settings => setHost(settings.choiceCards))
      .catch(() => setHost(null));
  }, [desktopId]);

  useEffect(() => {
    // A conversation on another machine must not keep this one's answer.
    setHost(null);
    read();
  }, [read]);

  // The host announced a settings change of its own; the cached read behind it
  // was dropped before this runs.
  useHostCatalogueInvalidation(desktopId, (domain) => {
    if (domain === "settings")
      read();
  });

  return host ?? local;
}
