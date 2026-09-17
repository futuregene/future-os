import { useCallback, useMemo, type RefObject } from "react";
import type { RemoteClient } from "./client";
import { requestReadPage } from "./readPages";
import type { AvailableSkill, DesktopSettings, InstalledSkill, ModelsData, RemoteCommand } from "./types";

/** Commands act on one connected desktop. No local preference storage or offline
 * write queue; a result from a replaced connection is never applied to the UI. */
export function useDesktopManagement(clientRef: RefObject<RemoteClient | null>) {
  const request = useCallback(async <T,>(command: RemoteCommand, mutation = false): Promise<T> => {
    const client = clientRef.current;
    if (!client) throw new Error("not_connected");
    const identity = client.accessIdentity;
    const isCurrent = () => clientRef.current === client && client.accessIdentity === identity;
    const result = mutation
      ? await client.request<T>(command, "settings", 60_000)
      : await requestReadPage<T>(client, command, "settings", isCurrent);
    if (!isCurrent()) throw new Error("stale_desktop");
    return result.data;
  }, [clientRef]);

  return useMemo(() => ({
    getDesktopSettings: () => request<DesktopSettings>({ type: "get_desktop_settings" }),
    updateDesktopSettings: (settings: Partial<DesktopSettings>) =>
      request<DesktopSettings>({ type: "update_desktop_settings", settings }, true),
    listSettingsModels: async () => (await request<ModelsData>({ type: "list_settings_models" })).models,
    listInstalledSkills: async () => (await request<{ skills: InstalledSkill[] }>({ type: "list_skills" })).skills,
    listAvailableSkills: async () => (await request<{ skills: AvailableSkill[] }>({ type: "list_available_skills" })).skills,
    installSkill: (skillId: string, version: string) => request<void>({ type: "install_skill", skillId, version }, true),
    uninstallSkill: async (skillId: string) => (await request<{ removed: boolean }>({ type: "uninstall_skill", skillId }, true)).removed,
  }), [request]);
}
