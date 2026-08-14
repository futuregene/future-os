import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { promptUpgrade } from "./prompt";
import { checkForUpdate } from "./update";

/** Auto-check for updates once per app launch, prompting when one is found. */
export function useUpdateReminder(): void {
  const { t } = useTranslation();
  useEffect(() => {
    let active = true;
    void checkForUpdate()
      .then(status => {
        if (active && status.hasUpdate) promptUpgrade(status, t);
      })
      .catch(() => {
        // Silent — a network failure on launch shouldn't surface UI noise.
      });
    return () => {
      active = false;
    };
  }, [t]);
}
