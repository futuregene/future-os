import { useCallback, useMemo, type RefObject } from "react";
import type { RemoteClient } from "./client";
import { requestReadPage } from "./readPages";
import type {
  RemoteTaskDep,
  RemoteTaskDetail,
  RemoteTaskRevision,
  RemoteTaskRow,
  RemoteTaskRun,
} from "./taskTypes";
import type {
  AvailableSkill,
  BuiltinProviderUpdate,
  CustomProviderUpsert,
  DesktopSettings,
  InstalledSkill,
  ModelsData,
  ProvidersView,
  RemoteCommand,
} from "./types";

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
    /**
     * At most one skill that fits `query`, or null when the desktop's agent
     * declines, times out, has no key, or the request fails. Best-effort by
     * contract: every failure is "no recommendation", never an error the caller
     * has to handle (see the desktop's `agent_bridge::suggest_skill`).
     */
    suggestSkill: async (query: string, candidates: { name: string; description: string }[]) =>
      (await request<{ skill: { name: string; description: string } | null }>(
        { type: "suggest_skill", query, candidates },
      )).skill,
    /** Today's recommendation state, for the phone's own daily budget. */
    skillRecoToday: async () =>
      (await request<{ today: { count: number; skillIds: string[]; messageHashes: string[] } }>(
        { type: "skill_reco_today" },
      )).today,
    /** Record one recommendation the phone has shown; only shown ones count. */
    recordSkillReco: (skillId: string, messageHash: string) =>
      request<void>({ type: "record_skill_reco", skillId, messageHash }, true),
    listProviders: () => request<ProvidersView>({ type: "list_providers" }),
    // One atomic built-in write: the key (set or cleared) and the Base URL
    // override are applied together, exactly like the desktop dialog.
    updateBuiltinProvider: (provider: BuiltinProviderUpdate) =>
      request<ProvidersView>({ type: "update_builtin_provider", provider }, true),
    upsertCustomProvider: (provider: CustomProviderUpsert) =>
      request<ProvidersView>({ type: "upsert_custom_provider", provider }, true),
    deleteCustomProvider: (providerId: string) =>
      request<ProvidersView>({ type: "delete_custom_provider", providerId }, true),
    // Tasks: the phone manages them and reads run ledgers; execution stays on
    // the paired desktop's tick loop.
    listTasks: async () => (await request<{ tasks: RemoteTaskRow[] }>({ type: "list_tasks" })).tasks,
    getTask: (taskId: string) => request<RemoteTaskDetail>({ type: "get_task", taskId }),
    createTask: (task: Record<string, unknown>) =>
      request<RemoteTaskDetail>({ type: "create_task", task }, true),
    updateTask: (taskId: string, task: Record<string, unknown>) =>
      request<RemoteTaskDetail>({ type: "update_task", taskId, task }, true),
    deleteTask: (taskId: string) => request<void>({ type: "delete_task", taskId }, true),
    setTaskEnabled: (taskId: string, enabled: boolean) =>
      request<RemoteTaskDetail>({ type: "set_task_enabled", taskId, enabled }, true),
    runTask: (taskId: string) => request<RemoteTaskDetail>({ type: "run_task", taskId }, true),
    listTaskRuns: async (taskId: string, limit = 20) =>
      (await request<{ runs: RemoteTaskRun[] }>({ type: "list_task_runs", taskId, limit })).runs,
    listTaskDeps: async (taskId: string) =>
      (await request<{ deps: RemoteTaskDep[] }>({ type: "list_task_deps", taskId })).deps,
    setTaskDep: (taskId: string, upstreamTaskId: string, on: string) =>
      request<void>({ type: "set_task_dep", taskId, upstreamTaskId, on }, true),
    removeTaskDep: (taskId: string, upstreamTaskId: string) =>
      request<void>({ type: "remove_task_dep", taskId, upstreamTaskId }, true),
    listTaskRevisions: async (taskId: string) =>
      (await request<{ revisions: RemoteTaskRevision[] }>({ type: "list_task_revisions", taskId })).revisions,
    applyTaskRevision: (taskId: string, revisionId: string) =>
      request<RemoteTaskDetail>({ type: "apply_task_revision", taskId, revisionId }, true),
  }), [request]);
}
