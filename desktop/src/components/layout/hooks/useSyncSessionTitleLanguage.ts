import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { getLanguage } from "../../../i18n";
import { sessionTitleSettings } from "../../../integrations/agent/sessionTitleSettings";
import { errorMessage } from "../../../lib/errors";
import { emitFutureEvent } from "../../../lib/futureEvents";

/** Sync on startup as well as language changes, without enabling automatic titles. */
export function useSyncSessionTitleLanguage() {
  const { i18n, t } = useTranslation("settings");
  useEffect(() => {
    void sessionTitleSettings({ uiLanguage: getLanguage() }).catch((error) => {
      emitFutureEvent("toast", {
        tone: "error",
        message: t("autoSessionTitle.error", { message: errorMessage(error) }),
      });
    });
  }, [i18n.language, t]);
}
