import type { AppSettings } from "../../../integrations/storage/appSettings";
import { useEffect, useRef, useState } from "react";
import i18n from "../../../i18n";
import { DEFAULT_APP_SETTINGS, getAppSettings, updateAppSettings } from "../../../integrations/storage/appSettings";
import { errorMessage } from "../../../lib/errors";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { useAsyncResource } from "../../../lib/useAsyncResource";

export interface UseAppSettingsResult {
  appSettings: AppSettings;
  changeSettings: (patch: Partial<AppSettings>) => Promise<void>;
}

/**
 * Owns the persisted app settings: loads them once, mirrors them into local
 * state so `changeSettings` can apply an optimistic update, and reconciles with
 * the server result. Writes are serialized; a failed latest write reports the
 * error and reloads the authoritative state instead of leaving false UI.
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
  // Serialize writes so two rapid switches cannot complete out of order and
  // overwrite a newer setting with an older full-settings response.
  const writeQueueRef = useRef<Promise<void>>(Promise.resolve());
  const mutationGenerationRef = useRef(0);

  useEffect(() => {
    if (dirtyRef.current)
      return;
    setAppSettings(loadedAppSettings);
  }, [loadedAppSettings]);

  async function changeSettings(patch: Partial<AppSettings>) {
    dirtyRef.current = true;
    const generation = ++mutationGenerationRef.current;
    setAppSettings(current => ({ ...current, ...patch }));

    const write = writeQueueRef.current.then(async () => {
      try {
        const next = await updateAppSettings(patch);
        // A newer optimistic edit is already visible; don't replace it with
        // this older request's full snapshot. The newer queued write will
        // reconcile once it completes.
        if (generation === mutationGenerationRef.current)
          setAppSettings(next);
      }
      catch (error) {
        emitFutureEvent("toast", {
          message: i18n.t("layout:settings.updateFailed", { message: errorMessage(error) }),
          tone: "error",
        });
        // Only the latest failed write owns reconciliation. If another write is
        // queued, its eventual full response will provide the authoritative state.
        if (generation === mutationGenerationRef.current) {
          try {
            setAppSettings(await getAppSettings());
          }
          catch {
            // The reload can fail for the same backend outage. Keep the current
            // value but retain the visible error; a later user edit retries.
          }
        }
      }
    });
    writeQueueRef.current = write;
    await write;
  }

  return { appSettings, changeSettings };
}
