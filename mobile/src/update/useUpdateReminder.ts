import { useEffect } from "react";
import { AppState } from "react-native";
import { useTranslation } from "react-i18next";
import { promptUpgrade } from "./prompt";
import { checkForUpdate } from "./update";

const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1_000;
const RETRY_INTERVAL_MS = 60 * 1_000;

/** Check on launch/resume and during long foreground sessions, without nagging. */
export function useUpdateReminder(): void {
  const { t } = useTranslation();
  useEffect(() => {
    let active = true;
    let checking = false;
    let nextCheckAt = 0;
    const prompted = new Set<string>();
    const check = async () => {
      if (!active || AppState.currentState !== "active" || checking || Date.now() < nextCheckAt) return;
      checking = true;
      nextCheckAt = Date.now() + RETRY_INTERVAL_MS;
      try {
        const status = await checkForUpdate();
        if (!active) return;
        // Don't present a dialog behind another app. Retry on the next resume.
        if (AppState.currentState !== "active") {
          nextCheckAt = 0;
          return;
        }
        nextCheckAt = Date.now() + CHECK_INTERVAL_MS;
        if (status.hasUpdate && !prompted.has(status.latestVersion)) {
          prompted.add(status.latestVersion);
          promptUpgrade(status, t);
        }
      } catch {
        // Offline launch isn't the last chance: retry while active or on resume.
      } finally {
        checking = false;
      }
    };
    void check();
    const subscription = AppState.addEventListener("change", state => {
      if (state === "active") void check();
    });
    const timer = setInterval(() => void check(), RETRY_INTERVAL_MS);
    return () => {
      active = false;
      subscription.remove();
      clearInterval(timer);
    };
  }, [t]);
}
