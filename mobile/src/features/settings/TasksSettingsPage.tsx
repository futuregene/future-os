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

/** "Every N minutes", promoting exact hours and days (mirrors the desktop). */
function intervalLabel(
  t: (key: string, options?: Record<string, unknown>) => string,
  minutes: number,
): string {
  if (minutes > 0 && minutes % (24 * 60) === 0)
    return t("tasks.trigger.everyDays", { days: minutes / (24 * 60) });
  if (minutes > 0 && minutes % 60 === 0)
    return t("tasks.trigger.everyHours", { hours: minutes / 60 });
  return t("tasks.trigger.every", { minutes });
}

/**
 * Date + time at minute precision, localized. Matches the desktop's
 * `formatDateTime` so both clients describe the same instant the same way.
 */
function formatWhen(value: string | number, locale: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime()))
    return String(value);
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function summarize(
  t: (key: string, options?: Record<string, unknown>) => string,
  task: RemoteTaskRow,
  locale: string,
): string {
  if (task.triggerKind !== "schedule")
    return t("tasks.trigger.manual");
  const trigger = task.trigger ?? {};
  const time = String(trigger.time ?? "");
  switch (String(trigger.mode ?? "")) {
    case "once": {
      const date = String(trigger.date ?? "");
      if (!/^\d{4}-\d{2}-\d{2}$/.test(date))
        return `${date} ${time}`.trim();
      return formatWhen(`${date}T${time || "00:00"}:00`, locale);
    }
    case "interval":
      return intervalLabel(t, Number(trigger.every_minutes ?? 0));
    case "weekly": {
      const days = (Array.isArray(trigger.days) ? trigger.days : []) as string[];
      return `${days.map(day => t(`tasks.weekday.${day}`)).join(", ")} ${time}`;
    }
    case "monthly":
      return t("tasks.trigger.monthly", {
        day: Number(trigger.day ?? 1),
        time,
      });
    default:
      return `${t("tasks.trigger.daily")} ${time}`;
  }
}

/** Task management on the paired desktop (list, editor, runs, versions). */
export function TasksSettingsPage({ desktopOnline }: { desktopOnline: boolean }) {
  const { t, i18n } = useTranslation();
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
  // Re-entrancy guard for writes. State updates land after the render that set
  // them, so two presses in one tick would both pass a `busy` check; the ref is
  // set synchronously and drops the second one (same shape as the desktop's
  // `writing` guard).
  const writing = useRef(false);
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

  /**
   * Clear the failure and re-read the task it belongs to. Every failure on this
   * page is a task read or write (the list read has its own banner through
   * `tasks.failed`), so the retry re-opens that task rather than guessing.
   */
  const retryTask = useCallback((taskId: string) => {
    setFailed(false);
    void loadDetail(taskId).catch(() => {
      if (active.current) setFailed(true);
    });
  }, [loadDetail]);

  const mutate = async (operation: () => Promise<unknown>) => {
    if (writing.current) return;
    writing.current = true;
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
      writing.current = false;
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
        onRetry={() => retryTask(detail.id)}
      />
    );
  }

  return (
    <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
      <SettingsSection title={t("tasks.title")}>
        {tasks.loading || tasks.failed ? <ResourceStatus loading={tasks.loading} failed={tasks.failed} onReload={() => void tasks.reload()} /> : null}
        {/* A task that could not be read leaves the list on screen, so the
            failure belongs here; a mutation failure renders in the editor. */}
        {failed && openId ? <ResourceStatus loading={false} failed onReload={() => retryTask(openId)} /> : null}
        {(tasks.data ?? []).length === 0 && !tasks.loading
          ? <Text style={settingsStyles.description}>{t("tasks.empty")}</Text>
          : (
              <View style={styles.stack}>
                {(tasks.data ?? []).map(task => (
                  <Pressable accessibilityRole="button" key={task.id} onPress={() => open(task.id)} style={styles.taskCard}>
                  <View style={styles.cardHeader}>
                    <Text style={styles.cardTitle} numberOfLines={1}>{task.name}</Text>
                    <Text style={settingsStyles.description}>
                      {task.latestRun ? t(`tasks.status.${task.latestRun.status}`) : ""}
                    </Text>
                  </View>
                    <Text style={settingsStyles.description}>
                      {summarize(t, task, i18n.language)}
                      {task.nextDueAt ? ` · ${formatWhen(task.nextDueAt, i18n.language)}` : ""}
                    </Text>
                  </Pressable>
                ))}
              </View>
            )}
      </SettingsSection>
      <Text style={settingsStyles.description}>{t("tasks.phoneHint")}</Text>
    </ScrollView>
  );
}

function TaskEditor({ detail, busy, deps, failed, revisions, runs, onBack, onMutate, onRetry }: {
  detail: RemoteTaskDetail;
  busy: boolean;
  deps: RemoteTaskDep[];
  failed: boolean;
  revisions: RemoteTaskRevision[];
  runs: RemoteTaskRun[];
  onBack(): void;
  onMutate(operation: () => Promise<unknown>): Promise<void>;
  onRetry(): void;
}) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  // The draft is initial state only: the parent keys this component by the
  // loaded revision, so a save (or a reload) remounts it with fresh values
  // instead of an effect writing state on every incoming snapshot.
  const [prompt, setPrompt] = useState(detail.prompt);
  const [trigger, setTrigger] = useState<DraftTrigger>(() => triggerFrom(detail));
  const [conversationMode, setConversationMode] = useState(detail.conversationMode ?? "workspace");
  const [cwd, setCwd] = useState(detail.cwd);

  const save = () => {
    const payload = triggerPayload(trigger);
    void onMutate(() => remote.updateTask(detail.id, {
      name: detail.name,
      prompt,
      cwd,
      modelId: detail.modelId,
      thinkingLevel: detail.thinkingLevel,
      sessionPolicy: detail.sessionPolicy,
      conversationMode,
      reflection: detail.reflection,
      depJoin: detail.depJoin,
      enabled: detail.enabled,
      ...payload,
    }));
  };

  return (
    <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
      {failed ? <ResourceStatus loading={false} failed onReload={onRetry} /> : null}
      <SettingsSection title={detail.name}>
        <Button label={t("tasks.runNow")} disabled={busy} onPress={() => void onMutate(() => remote.runTask(detail.id))} />
        <Button label={detail.enabled ? t("tasks.disable") : t("tasks.enable")} disabled={busy} onPress={() => void onMutate(() => remote.setTaskEnabled(detail.id, !detail.enabled))} />
        <Button label={t("common.back")} onPress={onBack} />
      </SettingsSection>

      <SettingsSection title={t("tasks.settings")}>
        <Text style={settingsStyles.description}>
          {t("tasks.colModel")}
          {": "}
          {detail.modelId ?? t("tasks.modelDefault")}
        </Text>
        <Text style={settingsStyles.description}>
          {t("tasks.colThinking")}
          {": "}
          {detail.thinkingLevel ? t(`tasks.thinkingLabels.${detail.thinkingLevel}`) : t("tasks.thinkingDefault")}
        </Text>
      </SettingsSection>

      <SettingsSection title={t("tasks.form.prompt")}>
        <TextInput multiline style={settingsStyles.input} value={prompt} onChangeText={setPrompt} />
        <Button label={t("tasks.form.save")} disabled={busy || prompt === detail.prompt} onPress={save} />
      </SettingsSection>

      <SettingsSection title={t("tasks.form.cwd")}>
        <View style={settingsStyles.actions}>
          {remote.workspaces.slice(0, 6).map(workspace => (
            <Pressable
              accessibilityRole="radio"
              accessibilityState={{ selected: cwd === workspace.path }}
              disabled={busy}
              key={workspace.id}
              onPress={() => setCwd(workspace.path)}
              style={[styles.choice, cwd === workspace.path && styles.choiceSelected]}
            >
              <Text numberOfLines={1} style={settingsStyles.label}>{workspace.name || workspace.path}</Text>
            </Pressable>
          ))}
        </View>
        <SettingsField label={t("tasks.form.cwdPath")} hint={t("tasks.form.cwdHint")}>
          <TextInput
            accessibilityLabel={t("tasks.form.cwdPath")}
            autoCapitalize="none"
            style={settingsStyles.input}
            value={cwd}
            onChangeText={setCwd}
          />
        </SettingsField>
      </SettingsSection>

      <SettingsSection title={t("tasks.form.conversation")}>
        <View style={settingsStyles.actions}>
          {(["workspace", "chat"] as const).map(mode => (
            <Pressable
              accessibilityRole="radio"
              accessibilityState={{ selected: conversationMode === mode }}
              disabled={busy}
              key={mode}
              onPress={() => setConversationMode(mode)}
              style={[styles.choice, conversationMode === mode && styles.choiceSelected]}
            >
              <Text style={settingsStyles.label}>{t(`tasks.form.conversation${mode === "chat" ? "Chat" : "Workspace"}`)}</Text>
            </Pressable>
          ))}
        </View>
        <Text style={settingsStyles.description}>{t("tasks.form.conversationHint")}</Text>
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
                <View key={dep.upstreamTaskId} style={styles.depCard}>
                  <Text style={settingsStyles.label} numberOfLines={1}>{dep.upstreamName}</Text>
                  <Text style={settingsStyles.description}>
                    {t(`tasks.on.${dep.on}`)}
                    {" · "}
                    {dep.satisfied ? t("tasks.depsReady") : t("tasks.depsWaiting")}
                  </Text>
                </View>
              ))}
            </SettingsSection>
          )
        : null}

      {runs.length > 0
        ? (
            <SettingsSection title={t("tasks.runs")}>
              {runs.map(run => (
                <View key={run.id} style={styles.runCard}>
                  <Text style={settingsStyles.description}>
                    {t(`tasks.kind.${run.kind}`)}
                    {" · "}
                    {t(`tasks.status.${run.status}`)}
                  </Text>
                  <Text style={styles.runSummary}>
                    {run.errorMessage ?? run.resultSummary ?? t("tasks.runNoSummary")}
                  </Text>
                </View>
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
  choice: { minHeight: layout.touchTarget, justifyContent: "center", paddingHorizontal: spacing.md, paddingVertical: spacing.sm, borderWidth: 1, borderColor: colors.line, borderRadius: radius.md, backgroundColor: colors.surface },
  choiceSelected: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  stack: { gap: spacing.md },
  taskCard: { gap: spacing.sm, padding: spacing.lg, borderWidth: 1, borderColor: colors.line, borderRadius: radius.lg, backgroundColor: colors.surface },
  cardHeader: { flexDirection: "row", alignItems: "center", gap: spacing.md },
  cardTitle: { flex: 1, minWidth: 0, color: colors.inkStrong, fontSize: 15, fontWeight: "600" },
  runCard: { gap: spacing.sm, padding: spacing.lg, borderWidth: 1, borderColor: colors.line, borderRadius: radius.lg, backgroundColor: colors.surface },
  runSummary: { color: colors.inkSoft, fontSize: 13, lineHeight: 20 },
  depCard: { gap: spacing.xs, padding: spacing.md, borderWidth: 1, borderColor: colors.line, borderRadius: radius.md, backgroundColor: colors.surface },
});
