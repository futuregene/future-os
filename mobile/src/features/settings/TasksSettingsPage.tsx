import { useCallback, useEffect, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import { useRemoteControls } from "../../remote/RemoteContext";
import type { RemoteTaskDep, RemoteTaskDetail, RemoteTaskRevision, RemoteTaskRow, RemoteTaskRun } from "../../remote/taskTypes";
import { colors, layout, radius, spacing } from "../../theme/tokens";
import { ResourceStatus, SettingsField, SettingsSection, settingsStyles } from "./SettingsPrimitives";
import { useDesktopResource } from "./useDesktopResource";

/** Local draft of the trigger, mirroring the desktop form. */
interface DraftTrigger {
  mode: "manual" | "once" | "interval" | "daily" | "weekly" | "monthly";
  date: string;
  time: string;
  everyMinutes: string;
  days: string;
  day: string;
}

const defaultTrigger: DraftTrigger = {
  mode: "manual",
  date: "",
  time: "09:00",
  everyMinutes: "60",
  days: "mon",
  day: "1",
};

function triggerFrom(detail: RemoteTaskDetail): DraftTrigger {
  if (detail.triggerKind !== "schedule")
    return { ...defaultTrigger };
  const trigger = detail.trigger ?? {};
  const mode = String(trigger.mode ?? "daily") as DraftTrigger["mode"];
  return {
    mode,
    date: String(trigger.date ?? ""),
    time: String(trigger.time ?? "09:00"),
    everyMinutes: String(trigger.every_minutes ?? 60),
    days: Array.isArray(trigger.days) ? (trigger.days as string[]).join(",") : "mon",
    day: String(trigger.day ?? 1),
  };
}

function triggerPayload(draft: DraftTrigger): { triggerKind: string; trigger: Record<string, unknown> } {
  if (draft.mode === "manual")
    return { triggerKind: "manual", trigger: {} };
  const time = draft.time.trim() || "09:00";
  switch (draft.mode) {
    case "once":
      return { triggerKind: "schedule", trigger: { mode: "once", date: draft.date.trim(), time } };
    case "interval":
      return { triggerKind: "schedule", trigger: { mode: "interval", every_minutes: Number(draft.everyMinutes) || 1 } };
    case "weekly": {
      const days = draft.days.split(",").map(day => day.trim()).filter(Boolean);
      return { triggerKind: "schedule", trigger: { mode: "weekly", days, time } };
    }
    case "monthly":
      return { triggerKind: "schedule", trigger: { mode: "monthly", day: Number(draft.day) || 1, time } };
    default:
      return { triggerKind: "schedule", trigger: { mode: "daily", time } };
  }
}

function summarize(t: (key: string, options?: Record<string, unknown>) => string, task: RemoteTaskRow): string {
  if (task.triggerKind !== "schedule")
    return t("tasks.trigger.manual");
  const trigger = task.trigger ?? {};
  switch (String(trigger.mode ?? "")) {
    case "once":
      return `${trigger.date} ${trigger.time}`;
    case "interval":
      return t("tasks.trigger.every", { minutes: Number(trigger.every_minutes ?? 0) });
    case "weekly":
      return `${(Array.isArray(trigger.days) ? trigger.days : []).join(", ")} ${trigger.time}`;
    case "monthly":
      return t("tasks.trigger.monthly", { day: Number(trigger.day ?? 1), time: String(trigger.time ?? "") });
    default:
      return `${t("tasks.trigger.daily")} ${trigger.time}`;
  }
}

/** Task management on the paired desktop (list, editor, runs, versions). */
export function TasksSettingsPage({ desktopOnline }: { desktopOnline: boolean }) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const tasks = useDesktopResource(remote.listTasks, 0, desktopOnline);
  const [openId, setOpenId] = useState<string | null>(null);
  const [detail, setDetail] = useState<RemoteTaskDetail | null>(null);
  const [runs, setRuns] = useState<RemoteTaskRun[]>([]);
  const [deps, setDeps] = useState<RemoteTaskDep[]>([]);
  const [revisions, setRevisions] = useState<RemoteTaskRevision[]>([]);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const active = useRef(true);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);

  const loadDetail = useCallback(async (taskId: string) => {
    const [nextDetail, nextRuns, nextDeps, nextRevisions] = await Promise.all([
      remote.getTask(taskId),
      remote.listTaskRuns(taskId),
      remote.listTaskDeps(taskId),
      remote.listTaskRevisions(taskId),
    ]);
    if (!active.current) return;
    setDetail(nextDetail);
    setRuns(nextRuns);
    setDeps(nextDeps);
    setRevisions(nextRevisions);
  }, [remote]);

  const open = useCallback((taskId: string) => {
    setOpenId(taskId);
    setDetail(null);
    void loadDetail(taskId).catch(() => {
      if (active.current) setFailed(true);
    });
  }, [loadDetail]);

  const mutate = async (operation: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    setFailed(false);
    try {
      await operation();
      await tasks.reload();
      if (openId) await loadDetail(openId);
    }
    catch {
      if (active.current) setFailed(true);
    }
    finally {
      if (active.current) setBusy(false);
    }
  };

  if (openId && detail) {
    return (
      <TaskEditor
        key={`${detail.id}:${detail.promptVersion}`}
        detail={detail}
        busy={busy}
        deps={deps}
        failed={failed}
        revisions={revisions}
        runs={runs}
        onBack={() => setOpenId(null)}
        onMutate={mutate}
      />
    );
  }

  return (
    <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
      <SettingsSection title={t("tasks.title")}>
        {tasks.loading || tasks.failed ? <ResourceStatus loading={tasks.loading} failed={tasks.failed} onReload={() => void tasks.reload()} /> : null}
        {(tasks.data ?? []).length === 0 && !tasks.loading
          ? <Text style={settingsStyles.description}>{t("tasks.empty")}</Text>
          : (tasks.data ?? []).map(task => (
              <Pressable accessibilityRole="button" key={task.id} onPress={() => open(task.id)} style={settingsStyles.card}>
                <View style={settingsStyles.row}>
                  <Text style={settingsStyles.label}>{task.name}</Text>
                  <Text style={settingsStyles.description}>
                    {task.latestRun ? t(`tasks.status.${task.latestRun.status}`) : ""}
                  </Text>
                </View>
                <Text style={settingsStyles.description}>
                  {summarize(t, task)}
                  {task.nextDueAt ? ` · ${new Date(task.nextDueAt).toLocaleString()}` : ""}
                </Text>
              </Pressable>
            ))}
      </SettingsSection>
      <Text style={settingsStyles.description}>{t("tasks.phoneHint")}</Text>
    </ScrollView>
  );
}

function TaskEditor({ detail, busy, deps, failed, revisions, runs, onBack, onMutate }: {
  detail: RemoteTaskDetail;
  busy: boolean;
  deps: RemoteTaskDep[];
  failed: boolean;
  revisions: RemoteTaskRevision[];
  runs: RemoteTaskRun[];
  onBack(): void;
  onMutate(operation: () => Promise<unknown>): Promise<void>;
}) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  // The draft is initial state only: the parent keys this component by the
  // loaded revision, so a save (or a reload) remounts it with fresh values
  // instead of an effect writing state on every incoming snapshot.
  const [prompt, setPrompt] = useState(detail.prompt);
  const [trigger, setTrigger] = useState<DraftTrigger>(() => triggerFrom(detail));

  const save = () => {
    const payload = triggerPayload(trigger);
    void onMutate(() => remote.updateTask(detail.id, {
      name: detail.name,
      prompt,
      cwd: detail.cwd,
      modelId: detail.modelId,
      thinkingLevel: detail.thinkingLevel,
      sessionPolicy: detail.sessionPolicy,
      reflection: detail.reflection,
      depJoin: detail.depJoin,
      enabled: detail.enabled,
      ...payload,
    }));
  };

  return (
    <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
      {failed ? <ResourceStatus loading={false} failed onReload={() => void onMutate(async () => undefined)} /> : null}
      <SettingsSection title={detail.name}>
        <Button label={t("tasks.runNow")} disabled={busy} onPress={() => void onMutate(() => remote.runTask(detail.id))} />
        <Button label={detail.enabled ? t("tasks.disable") : t("tasks.enable")} disabled={busy} onPress={() => void onMutate(() => remote.setTaskEnabled(detail.id, !detail.enabled))} />
        <Button label={t("common.back")} onPress={onBack} />
      </SettingsSection>

      <SettingsSection title={t("tasks.form.prompt")}>
        <TextInput multiline style={settingsStyles.input} value={prompt} onChangeText={setPrompt} />
        <Button label={t("tasks.form.save")} disabled={busy || prompt === detail.prompt} onPress={save} />
      </SettingsSection>

      <SettingsSection title={t("tasks.form.trigger")}>
        <View style={settingsStyles.actions}>
          {(["manual", "once", "interval", "daily", "weekly", "monthly"] as const).map(mode => (
            <Pressable
              accessibilityRole="radio"
              accessibilityState={{ selected: trigger.mode === mode }}
              disabled={busy}
              key={mode}
              onPress={() => setTrigger(current => ({ ...current, mode }))}
              style={[styles.choice, trigger.mode === mode && styles.choiceSelected]}
            >
              <Text style={settingsStyles.label}>{t(`tasks.triggerMode.${mode}`)}</Text>
            </Pressable>
          ))}
        </View>
        {trigger.mode === "once"
          ? (
              <SettingsField label={t("tasks.form.date")}>
                <TextInput placeholder="2026-12-24" style={settingsStyles.input} value={trigger.date} onChangeText={date => setTrigger(current => ({ ...current, date }))} />
              </SettingsField>
            )
          : null}
        {trigger.mode !== "manual" && trigger.mode !== "interval"
          ? (
              <SettingsField label={t("tasks.form.time")}>
                <TextInput style={settingsStyles.input} value={trigger.time} onChangeText={time => setTrigger(current => ({ ...current, time }))} />
              </SettingsField>
            )
          : null}
        {trigger.mode === "interval"
          ? (
              <SettingsField label={t("tasks.form.everyMinutes")}>
                <TextInput keyboardType="number-pad" style={settingsStyles.input} value={trigger.everyMinutes} onChangeText={everyMinutes => setTrigger(current => ({ ...current, everyMinutes }))} />
              </SettingsField>
            )
          : null}
        {trigger.mode === "weekly"
          ? (
              <SettingsField label={t("tasks.form.days")} hint="mon,tue,wed">
                <TextInput autoCapitalize="none" style={settingsStyles.input} value={trigger.days} onChangeText={days => setTrigger(current => ({ ...current, days }))} />
              </SettingsField>
            )
          : null}
        {trigger.mode === "monthly"
          ? (
              <SettingsField label={t("tasks.form.day")} hint={t("tasks.form.shortMonthHint")}>
                <TextInput keyboardType="number-pad" style={settingsStyles.input} value={trigger.day} onChangeText={day => setTrigger(current => ({ ...current, day }))} />
              </SettingsField>
            )
          : null}
        <Button label={t("tasks.form.save")} disabled={busy} onPress={save} />
        <Text style={settingsStyles.description}>{t("tasks.form.fullPermissionWarning")}</Text>
      </SettingsSection>

      {deps.length > 0
        ? (
            <SettingsSection title={t("tasks.deps")}>
              {deps.map(dep => (
                <Text key={dep.upstreamTaskId} style={settingsStyles.description}>
                  {dep.upstreamName} · {t(`tasks.on.${dep.on}`)} · {dep.satisfied ? t("tasks.depsReady") : t("tasks.depsWaiting")}
                </Text>
              ))}
            </SettingsSection>
          )
        : null}

      {runs.length > 0
        ? (
            <SettingsSection title={t("tasks.runs")}>
              {runs.map(run => (
                <Text key={run.id} style={settingsStyles.description}>
                  {t(`tasks.kind.${run.kind}`)} · {t(`tasks.status.${run.status}`)} · {run.resultSummary ?? run.errorMessage ?? ""}
                </Text>
              ))}
            </SettingsSection>
          )
        : null}

      {revisions.length > 0
        ? (
            <SettingsSection title={t("tasks.revisions")}>
              {revisions.map(revision => (
                <View key={revision.id} style={settingsStyles.card}>
                  <Text style={settingsStyles.label}>{`v${revision.version} · ${t(`tasks.source.${revision.source}`)}`}</Text>
                  <Text style={settingsStyles.description}>{revision.reason ?? revision.promptPreview}</Text>
                  <Button label={t("tasks.apply")} disabled={busy} onPress={() => void onMutate(() => remote.applyTaskRevision(detail.id, revision.id))} />
                </View>
              ))}
            </SettingsSection>
          )
        : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  choice: { minHeight: layout.touchTarget, justifyContent: "center", padding: spacing.sm, borderWidth: 1, borderColor: colors.line, borderRadius: radius.md, backgroundColor: colors.surface },
  choiceSelected: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
});
