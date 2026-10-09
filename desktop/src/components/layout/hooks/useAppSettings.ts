import type { AppSettings } from "../../../integrations/storage/appSettings";
import { useCallback, useEffect, useRef, useState } from "react";
import i18n, { getLanguage } from "../../../i18n";
import { automaticApprovalAvailable, effectiveApprovalTier, shouldPersistAutomaticApprovalFallback } from "../../../integrations/agent/automaticApproval";
import {
  shouldPersistSandboxFallback,
  useSandboxAvailability,
} from "../../../integrations/agent/useSandboxAvailability";
import { DEFAULT_APP_SETTINGS, getAppSettings, updateAppSettings } from "../../../integrations/storage/appSettings";
import { errorMessage } from "../../../lib/errors";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { useAsyncResource } from "../../../lib/useAsyncResource";
import { useTauriEvent } from "../../../lib/useTauriEvent";

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
export function useAppSettings(futureSessionStatus = "checking"): UseAppSettingsResult {
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
  const sandboxFallbackRef = useRef(false);
  const autoFallbackRef = useRef(false);
  const sandboxAvailability = useSandboxAvailability();

  useEffect(() => {
    if (dirtyRef.current)
      return;
    setAppSettings(loadedAppSettings);
  }, [loadedAppSettings]);

  const changeSettings = useCallback(async (patch: Partial<AppSettings>) => {
    if (patch.approvalTier === "auto" && !automaticApprovalAvailable(futureSessionStatus))
      patch = { ...patch, approvalTier: "sandbox" };
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
  }, [futureSessionStatus]);

  const reloadSettings = useCallback(() => {
    dirtyRef.current = true;
    // Queue the read behind local writes; never replace an optimistic edit
    // with a phone notification's older snapshot.
    const read = writeQueueRef.current.then(async () => {
      const generation = mutationGenerationRef.current;
      try {
        const next = await getAppSettings();
        if (generation === mutationGenerationRef.current)
          setAppSettings(next);
      }
      catch {
        // Keep the current view; focus/reconnect or the next edit retries.
      }
    });
    writeQueueRef.current = read;
  }, []);
  useTauriEvent("app_settings_changed", reloadSettings);
  useEffect(() => {
    window.addEventListener("focus", reloadSettings);
    return () => window.removeEventListener("focus", reloadSettings);
  }, [reloadSettings]);

  // Title generation runs in the backend, including while the webview is
  // suspended. Mirror the same UI language used by the manual title dialog.
  useEffect(() => {
    const syncTitleLanguage = () => {
      void changeSettings({ titleLanguage: getLanguage() });
    };
    syncTitleLanguage();
    i18n.on("languageChanged", syncTitleLanguage);
    return () => {
      i18n.off("languageChanged", syncTitleLanguage);
    };
  }, [changeSettings]);

  const sandboxFallbackRequired = shouldPersistSandboxFallback(
    sandboxAvailability,
    appSettings.approvalTier,
  );

  const autoFallbackRequired = shouldPersistAutomaticApprovalFallback(appSettings.approvalTier, futureSessionStatus);
  useEffect(() => {
    if (!autoFallbackRequired) {
      autoFallbackRef.current = false;
      return;
    }
    if (autoFallbackRef.current)
      return;
    autoFallbackRef.current = true;
    void changeSettings({ approvalTier: "sandbox" });
  }, [autoFallbackRequired, changeSettings]);

  useEffect(() => {
    if (!sandboxFallbackRequired) {
      sandboxFallbackRef.current = false;
      return;
    }
    if (sandboxFallbackRef.current)
      return;
    sandboxFallbackRef.current = true;
    emitFutureEvent("toast", {
      message: i18n.t("settings:approvalTier.fallbackNotice", {
        code: sandboxAvailability.code ?? "probe_failed",
      }),
      tone: "info",
    });
    void changeSettings({ approvalTier: "manual" }).finally(() => {
      sandboxFallbackRef.current = false;
    });
  }, [
    appSettings.approvalTier,
    changeSettings,
    sandboxAvailability.code,
    sandboxFallbackRequired,
  ]);

  return {
    appSettings: { ...appSettings, approvalTier: effectiveApprovalTier(appSettings.approvalTier, futureSessionStatus) },
    changeSettings,
  };
}
