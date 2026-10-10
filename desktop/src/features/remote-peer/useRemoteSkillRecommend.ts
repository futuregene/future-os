import { useEffect, useState } from "react";
import { requestRemotePeer } from "./remotePeerClient";

/**
 * Whether that computer wants its conversations to get skill recommendations.
 *
 * A read rather than this machine's toggle: the setting belongs to the machine
 * being written to, and a user who turned recommendation off *there* should not
 * be offered cards because it is still on here.
 *
 * Anything unclear — an unreachable host, a failure, a host that predates the
 * setting — is "off": a card the user did not ask for is worse than one they
 * never see, and the recommender is best-effort either way.
 */
export function useRemoteSkillRecommend(desktopId: string): boolean {
  const [enabled, setEnabled] = useState(false);

  useEffect(() => {
    if (!desktopId) {
      setEnabled(false);
      return;
    }
    let cancelled = false;
    void requestRemotePeer<Record<string, unknown>>(
      desktopId,
      { type: "get_desktop_settings" },
      "list",
    )
      .then((settings) => {
        if (!cancelled)
          setEnabled(settings?.skillRecommend === true);
      })
      .catch(() => {
        if (!cancelled)
          setEnabled(false);
      });
    return () => {
      cancelled = true;
    };
  }, [desktopId]);

  return enabled;
}
