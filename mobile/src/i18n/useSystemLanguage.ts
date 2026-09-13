import { useEffect, useState } from "react";
import { AppState } from "react-native";
import { useLocales } from "expo-localization";
import { initializeLanguagePreference, syncPreferredLanguage } from "./preferences";

/** Track the OS language without overriding a saved manual selection. */
export function useSystemLanguage(): boolean {
  const locales = useLocales();
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let mounted = true;
    void initializeLanguagePreference().then(() => {
      if (mounted) setReady(true);
    });
    return () => { mounted = false; };
  }, []);

  useEffect(() => {
    if (ready) syncPreferredLanguage();
  }, [locales, ready]);

  useEffect(() => {
    // Android can change language while we're in Settings without restarting
    // the app. Read fresh locales on resume even if no locale event arrived.
    const subscription = AppState.addEventListener("change", state => {
      if (state === "active") syncPreferredLanguage();
    });
    return () => subscription.remove();
  }, []);
  return ready;
}
