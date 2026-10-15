import { useCallback, useEffect, useState } from "react";
import { hostDesktopSettings, useHostCatalogueInvalidation } from "./remoteHostCatalog";

/**
 * Whether that computer wants its conversations to get skill recommendations.
 *
 * A read rather than this machine's toggle: the setting belongs to the machine
 * being written to, and a user who turned recommendation off *there* should not
 * be offered cards because it is still on here.
 *
 * The setting describes the *host*, so it is read once per connection and dropped
 * when the host says its own settings moved — the same cadence the phone reads it
 * with (`useDesktopRuntime`'s `refreshSettings` as a runtime connects, then
 * `app_settings_changed`). Reading it on every conversation opened put one
 * command ahead of the page that draws the transcript, and paid it again for
 * every other conversation on the same machine.
 *
 * Anything unclear — an unreachable host, a failure, a host that predates the
 * setting — is "off": a card the user did not ask for is worse than one they
 * never see, and the recommender is best-effort either way.
 */
export function useRemoteSkillRecommend(desktopId: string): boolean {
  const [enabled, setEnabled] = useState(false);

  const read = useCallback(() => {
    if (!desktopId) {
      setEnabled(false);
      return;
    }
    void hostDesktopSettings(desktopId)
      .then(settings => setEnabled(settings.skillRecommend))
      .catch(() => setEnabled(false));
  }, [desktopId]);

  useEffect(() => {
    read();
    // A conversation on another machine must not keep this one's answer.
    return () => setEnabled(false);
  }, [read]);

  // The host announced a settings change of its own, and the cached read behind
  // it was dropped before this runs.
  useHostCatalogueInvalidation(desktopId, (domain) => {
    if (domain === "settings")
      read();
  });

  return enabled;
}
