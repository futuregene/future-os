import type { AppSettings } from "../../../integrations/storage/appSettings";
import { useEffect, useRef, useState } from "react";
import { DEFAULT_APP_SETTINGS, getAppSettings, updateAppSettings } from "../../../integrations/storage/appSettings";
import { useAsyncResource } from "../../../lib/useAsyncResource";

export interface UseAppSettingsResult {
  appSettings: AppSettings;
  changeSettings: (patch: Partial<AppSettings>) => Promise<void>;
}

/**
 * Owns the persisted app settings: loads them once, mirrors them into local
 * state so `changeSettings` can apply an optimistic update, and reconciles with
 * the server result (keeping the optimistic value on failure).
 */
export function useAppSettings(): UseAppSettingsResult {
  const { data: loadedAppSettings } = useAsyncResource<AppSettings>(
    getAppSettings,
    [],
    DEFAULT_APP_SETTINGS,
  );
  const [appSettings, setAppSettings] = useState<AppSettings>(DEFAULT_APP_SETTINGS);
  // Once the user has edited a setting, the initial async load's snapshot is
  // stale — applying it would clobber the optimistic value (the backend already
  // holds the new one). Stop mirroring the load after the first change.
  const dirtyRef = useRef(false);

  useEffect(() => {
    if (dirtyRef.current)
      return;
    setAppSettings(loadedAppSettings);
  }, [loadedAppSettings]);

  async function changeSettings(patch: Partial<AppSettings>) {
    dirtyRef.current = true;
    setAppSettings(current => ({ ...current, ...patch }));
    try {
      const next = await updateAppSettings(patch);
      setAppSettings(next);
    }
    catch {
      // Keep the optimistic value; a later load will reconcile.
    }
  }

  return { appSettings, changeSettings };
}
