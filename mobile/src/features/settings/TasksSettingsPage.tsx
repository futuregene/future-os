import { useCallback, useEffect, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import { useRemoteControls } from "../../remote/RemoteContext";
import type { DesktopSettings, RemoteModel } from "../../remote/types";
import { modelReference } from "../../remote/types";
import type { RemoteTaskDep, RemoteTaskDetail, RemoteTaskRevision, RemoteTaskRow, RemoteTaskRun } from "../../remote/taskTypes";
import { colors, layout, radius, spacing } from "../../theme/tokens";
import { ResourceStatus, SettingsField, SettingsSection, settingsStyles } from "./SettingsPrimitives";
import { useDesktopResource } from "./useDesktopResource";

/** Local draft of the trigger, mirroring the desktop form. */
interface DraftTrigger {
  /**
   * `dependency` is a manual task in the store: its edges are what start it
   * (they live in their own table), and choosing it opens the editor below
   * instead of a date/time field.
   */
  mode: "manual" | "dependency" | "once" | "interval" | "daily" | "weekly" | "monthly";
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

/** Thinking levels the agent accepts, in the composer's order. */
const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"] as const;

/**
 * Everything the editor can change. The desktop keeps the same shape in its
 * own draft, so the two forms send the same payload.
 */
/** One dependency edge as the editor holds it (upstream label + condition). */
interface DraftDep {
  upstreamTaskId: string;
  name: string;
  on: string;
}

interface Draft {
  name: string;
  prompt: string;
  cwd: string;
  modelId: string;
  thinkingLevel: string;
  sessionPolicy: string;
  conversationMode: string;
  reflection: string;
  enabled: boolean;
  trigger: DraftTrigger;
  /** The dependency edges the form wants, as a whole set (see `reconcileDeps`). */
  deps: DraftDep[];
  depJoin: string;
}

function triggerFrom(detail: RemoteTaskDetail): DraftTrigger {
  if (detail.triggerKind !== "schedule") {
    // A task its upstreams start reads as `dependency`, not as `manual`:
    // nobody runs it by hand.
    if ((detail.depCount ?? 0) > 0)
      return { ...defaultTrigger, mode: "dependency" };
    return { ...defaultTrigger };
  }
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
  if (draft.mode === "manual" || draft.mode === "dependency")
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

/** A stored task as an editable draft. */
function draftFrom(detail: RemoteTaskDetail, deps: RemoteTaskDep[]): Draft {
  return {
    name: detail.name,
    prompt: detail.prompt,
    cwd: detail.cwd,
    modelId: detail.modelId ?? "",
    thinkingLevel: detail.thinkingLevel ?? "",
    sessionPolicy: detail.sessionPolicy ?? "new",
    conversationMode: detail.conversationMode ?? "workspace",
    reflection: detail.reflection ?? "ask",
    enabled: detail.enabled,
    trigger: triggerFrom(detail),
    deps: deps.map(dep => ({
      upstreamTaskId: dep.upstreamTaskId,
      name: dep.upstreamName,
      on: dep.on,
    })),
    depJoin: detail.depJoin ?? "all",
  };
}

/**
 * A blank draft for a new task.
 *
 * It leaves the model and the thinking level unset on purpose: both are
 * spending decisions the user makes, so the form asks instead of inheriting an
 * app default nobody chose. The conversation type starts on chat, which needs
 * no working directory at all (the phone cannot browse the desktop's
 * filesystem, and a chat conversation brings its own workspace); switching to
 * a workspace conversation is what asks for a path.
 */
function newDraft(): Draft {
  return {
    name: "",
    prompt: "",
    cwd: "",
    modelId: "",
    thinkingLevel: "",
    sessionPolicy: "new",
    conversationMode: "chat",
    reflection: "ask",
    // A new task starts enabled, like the desktop's form does.
    enabled: true,
    trigger: { ...defaultTrigger },
    // …and with no upstream: it runs on its own trigger until the user says
    // otherwise.
    deps: [],
    depJoin: "all",
  };
}

/** The payload `createTask` / `updateTask` take, from the draft. */
function draftPayload(draft: Draft): Record<string, unknown> {
  return {
    name: draft.name.trim(),
    prompt: draft.prompt,
    cwd: draft.cwd.trim(),
    // `null` means "clear it" on the desktop side, which is how the pickers'
    // "default" choice is expressed. An omitted key would mean "leave it".
    modelId: draft.modelId || null,
    thinkingLevel: draft.thinkingLevel || null,
    sessionPolicy: draft.sessionPolicy,
    conversationMode: draft.conversationMode,
    reflection: draft.reflection,
    enabled: draft.enabled,
    depJoin: draft.depJoin,
    ...triggerPayload(draft.trigger),
  };
}

/**
 * Make a task's dependency edges say exactly `wanted`, and nothing else.
 *
 * The editor holds the whole set, so the write is a reconciliation rather than
 * add/remove calls poured out of the UI: an edge that is kept as it was is not
 * written again, a changed condition is one call, an edge the user dropped is
 * removed, and a new one is added. It compares against the desktop's current
 * edges (re-read here), so an edit made elsewhere in between is not silently
 * reverted.
 */
async function reconcileDeps(
  remote: Pick<ReturnType<typeof useRemoteControls>, "listTaskDeps" | "setTaskDep" | "removeTaskDep">,
  taskId: string,
  wanted: DraftDep[],
): Promise<void> {
  const current = await remote.listTaskDeps(taskId);
  const byUpstream = new Map(wanted.map(dep => [dep.upstreamTaskId, dep.on]));
  for (const dep of current) {
    const on = byUpstream.get(dep.upstreamTaskId);
    if (on === undefined)
      await remote.removeTaskDep(taskId, dep.upstreamTaskId);
    else if (on !== dep.on)
      await remote.setTaskDep(taskId, dep.upstreamTaskId, on);
    byUpstream.delete(dep.upstreamTaskId);
  }
  for (const [upstreamTaskId, on] of byUpstream)
    await remote.setTaskDep(taskId, upstreamTaskId, on);
}

/** Why the form cannot be submitted yet, or null when it can. */
function draftProblem(draft: Draft): "name" | "prompt" | "cwd" | "model" | "thinking" | "date" | null {
  if (!draft.name.trim())
    return "name";
  if (!draft.prompt.trim())
    return "prompt";
  // A workspace conversation *is* its directory; a chat one carries its own.
  if (draft.conversationMode === "workspace" && !draft.cwd.trim())
    return "cwd";
  if (!draft.modelId)
    return "model";
  if (!draft.thinkingLevel)
    return "thinking";
  if (draft.trigger.mode === "once" && !/^\d{4}-\d{2}-\d{2}$/.test(draft.trigger.date.trim()))
    return "date";
  return null;
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
    return (task.depCount ?? 0) > 0 ? t("tasks.trigger.dependency") : t("tasks.trigger.manual");
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

/** A one-of-N choice row, sized as a touch target. */
function Choice({ label, selected, disabled, onPress }: {
  label: string;
  selected: boolean;
  disabled: boolean;
  onPress(): void;
}) {
  return (
    <Pressable
      accessibilityRole="radio"
      accessibilityState={{ selected, disabled }}
      disabled={disabled}
      onPress={onPress}
      style={[styles.choice, selected && styles.choiceSelected]}
    >
      <Text numberOfLines={1} style={settingsStyles.label}>{label}</Text>
    </Pressable>
  );
}

/** Task management on the paired desktop (list, create, edit, delete, runs). */
export function TasksSettingsPage({ desktopOnline, settings }: {
  desktopOnline: boolean;
  /** The desktop's settings, for the enabled-model list (Settings → Models). */
  settings: DesktopSettings | null;
}) {
  const { t, i18n } = useTranslation();
  const remote = useRemoteControls();
  const tasks = useDesktopResource(remote.listTasks, 0, desktopOnline);
  const [openId, setOpenId] = useState<string | null>(null);
  /** Creating opens the same editor with no task behind it. */
  const [creating, setCreating] = useState(false);
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
    setCreating(false);
    setOpenId(taskId);
    setDetail(null);
    void loadDetail(taskId).catch(() => {
      if (active.current) setFailed(true);
    });
  }, [loadDetail]);

  const close = useCallback(() => {
    setCreating(false);
    setOpenId(null);
    setDetail(null);
  }, []);

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

  /**
   * Delete, then leave the editor: the task it describes is gone. The task is a
   * parameter rather than read from state, because the only caller renders when
   * a task is loaded — a guard for "no task" would be unreachable code.
   */
  const remove = (task: RemoteTaskDetail) => {
    void mutate(async () => {
      await remote.deleteTask(task.id);
      close();
    });
  };

  if (creating) {
    return (
      <TaskForm
        busy={busy}
        candidates={tasks.data ?? []}
        desktopOnline={desktopOnline}
        failed={failed}
        kind="create"
        settings={settings}
        onCancel={close}
        onCreate={draft => void mutate(async () => {
          const created = await remote.createTask(draftPayload(draft));
          // The edges need both ids, so they are written once the task exists.
          await reconcileDeps(remote, created.id, draft.deps);
          close();
        })}
      />
    );
  }

  if (openId && detail) {
    return (
      <TaskForm
        key={`${detail.id}:${detail.promptVersion}`}
        busy={busy}
        candidates={tasks.data ?? []}
        deps={deps}
        detail={detail}
        desktopOnline={desktopOnline}
        failed={failed}
        kind="edit"
        revisions={revisions}
        runs={runs}
        settings={settings}
        onBack={close}
        onDelete={() => remove(detail)}
        onMutate={mutate}
        onRetry={() => retryTask(detail.id)}
        onSaveDraft={draft => reconcileDeps(remote, detail.id, draft.deps)}
      />
    );
  }

  return (
    <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
      <SettingsSection title={t("tasks.title")}>
        <Button label={t("tasks.new")} disabled={!desktopOnline} onPress={() => setCreating(true)} />
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
                    {task.pendingProposals
                      ? <Text style={styles.suggestionBadge}>{t("tasks.suggestionCount", { count: task.pendingProposals })}</Text>
                      : null}
                    <Text style={settingsStyles.description}>
                      {task.queued
                        ? t("tasks.status.queued")
                        : task.latestRun
                        ? t(`tasks.status.${task.latestRun.status}`)
                        : ""}
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

/**
 * The task editor, for both a new task and an existing one.
 *
 * `create` has no `detail` to read from, so the draft starts blank; `edit`
 * starts from the stored task and keys the component by prompt version, so a
 * save (or a reload) remounts it with fresh values instead of an effect writing
 * state on every incoming snapshot.
 */
function TaskForm({
  kind, detail, settings, candidates = [], busy, deps = [], desktopOnline, failed, revisions = [], runs = [],
  onBack, onCancel, onCreate, onDelete, onMutate, onRetry, onSaveDraft,
}: {
  kind: "create" | "edit";
  detail?: RemoteTaskDetail;
  /** The desktop's own settings — the enabled-model list comes from them. */
  settings: DesktopSettings | null;
  /** The desktop's tasks: the upstreams a dependency can point at. */
  candidates?: RemoteTaskRow[];
  busy: boolean;
  deps?: RemoteTaskDep[];
  desktopOnline: boolean;
  failed: boolean;
  revisions?: RemoteTaskRevision[];
  runs?: RemoteTaskRun[];
  onBack?(): void;
  onCancel?(): void;
  onCreate?(draft: Draft): void;
  onDelete?(): void;
  onMutate?(operation: () => Promise<unknown>): Promise<void>;
  onRetry?(): void;
  /** Write the draft's dependency edges (edit mode; create goes through `onCreate`). */
  onSaveDraft?(draft: Draft): Promise<void>;
}) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const models = useDesktopResource(remote.listSettingsModels, 0, desktopOnline);
  const [draft, setDraft] = useState<Draft>(() => detail ? draftFrom(detail, deps) : newDraft());
  const [confirmDelete, setConfirmDelete] = useState(false);
  // Editing edges needs a desktop that implements them; an older one keeps the
  // read-only list below the form.
  const canEditDeps = remote.capabilities?.has("task_deps_v1") ?? false;
  // Only the models the user has enabled (Settings → Models), like the
  // composer — plus the task's own model when it has since been disabled, so
  // editing another field cannot silently rewrite it.
  const enabledModels = (models.data ?? []).filter(
    model => !(settings?.hiddenModels ?? []).includes(modelReference(model)),
  );
  const pinnedModel = draft.modelId && !enabledModels.some(model => modelKey(model) === draft.modelId)
    ? draft.modelId
    : null;
  const workspaceConversation = draft.conversationMode === "workspace";
  const patch = (values: Partial<Draft>) => setDraft(current => ({ ...current, ...values }));
  const patchTrigger = (values: Partial<DraftTrigger>) =>
    setDraft(current => ({ ...current, trigger: { ...current.trigger, ...values } }));
  // The other tasks a dependency can point at. The desktop refuses a cycle, so
  // this list only has to exclude the task itself.
  const addable = candidates.filter(
    candidate => candidate.id !== detail?.id
      && !draft.deps.some(dep => dep.upstreamTaskId === candidate.id),
  );
  const problem = draftProblem(draft);
  const trigger = draft.trigger;

  const save = () => {
    // Create goes through `onCreate` (the parent owns the request); edit goes
    // through `onMutate`, which also handles the in-flight guard and reload.
    // Requiring both would make either mode silently do nothing. The dependency
    // edges are written inside the same operation, so a refused edge fails the
    // save the user pressed rather than the next unrelated one.
    if (problem) return;
    if (kind === "create")
      onCreate?.(draft);
    else if (onMutate)
      void onMutate(async () => {
        await remote.updateTask(detail!.id, draftPayload(draft));
        await onSaveDraft?.(draft);
      });
  };

  return (
    <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
      {/* Editing can re-read the task, so its failure banner retries that read.
          A failed create has nothing to re-read: the draft is still on screen
          and Save is the retry, so it gets a plain message and no button that
          would pretend to do something else. */}
      {failed
        ? kind === "edit" && onRetry
          ? <ResourceStatus loading={false} failed onReload={onRetry} />
          : <Text accessibilityRole="alert" style={settingsStyles.error}>{t("desktopSettings.loadFailed")}</Text>
        : null}

      <SettingsSection title={kind === "create" ? t("tasks.newTitle") : detail!.name}>
        {/* One compact row, not a stack of full-width buttons: the phone's
            vertical space belongs to the form, and the desktop puts these on
            one line too. Delete stays two-step (armed, then confirmed). */}
        <View style={settingsStyles.actions}>
          {kind === "edit"
            ? (
                <>
                  <Button compact label={t("tasks.runNow")} disabled={busy} onPress={() => void onMutate?.(() => remote.runTask(detail!.id))} />
                  <Button compact label={detail!.enabled ? t("tasks.disable") : t("tasks.enable")} disabled={busy} variant="secondary" onPress={() => void onMutate?.(() => remote.setTaskEnabled(detail!.id, !detail!.enabled))} />
                </>
              )
            : null}
          {kind === "edit"
            ? (
                confirmDelete
                  ? (
                      <>
                        <Button compact label={t("tasks.deleteConfirmAction")} variant="danger" disabled={busy} onPress={() => onDelete?.()} />
                        <Button compact label={t("chat.cancel")} variant="secondary" disabled={busy} onPress={() => setConfirmDelete(false)} />
                      </>
                    )
                  : <Button compact label={t("tasks.delete")} variant="secondary" disabled={busy} onPress={() => setConfirmDelete(true)} />
              )
            : null}
          <Button compact label={kind === "create" ? t("tasks.form.cancel") : t("common.back")} variant="secondary" onPress={kind === "create" ? onCancel! : onBack!} />
        </View>
        {/* The confirmation is a sentence, so it gets its own line under the row. */}
        {kind === "edit" && confirmDelete
          ? <Text style={settingsStyles.description}>{t("tasks.deleteConfirm", { name: detail!.name })}</Text>
          : null}
      </SettingsSection>

      <SettingsSection title={t("tasks.form.details")}>
        <SettingsField label={t("tasks.form.name")}>
          <TextInput
            accessibilityLabel={t("tasks.form.name")}
            style={settingsStyles.input}
            value={draft.name}
            onChangeText={name => patch({ name })}
          />
        </SettingsField>

        <SettingsField label={t("tasks.form.prompt")}>
          <TextInput
            accessibilityLabel={t("tasks.form.prompt")}
            multiline
            style={settingsStyles.input}
            value={draft.prompt}
            onChangeText={prompt => patch({ prompt })}
          />
        </SettingsField>

        {/* The conversation type decides whether a directory is needed at all,
            so it is asked before the directory row (and the row only exists for
            a workspace conversation — a chat one brings its own). */}
        <SettingsField label={t("tasks.form.conversation")} hint={t("tasks.form.conversationHint")}>
          <View style={settingsStyles.actions}>
            {(["chat", "workspace"] as const).map(mode => (
              <Choice
                disabled={busy}
                key={mode}
                label={t(`tasks.form.conversation${mode === "chat" ? "Chat" : "Workspace"}`)}
                onPress={() => patch({ conversationMode: mode })}
                selected={draft.conversationMode === mode}
              />
            ))}
          </View>
        </SettingsField>

        {workspaceConversation
          ? (
              <SettingsField label={t("tasks.form.cwd")} hint={t("tasks.form.cwdHint")}>
                <View style={settingsStyles.actions}>
                  {remote.workspaces.slice(0, 6).map(workspace => (
                    <Choice
                      disabled={busy}
                      key={workspace.id}
                      label={workspace.name || workspace.path}
                      onPress={() => patch({ cwd: workspace.path })}
                      selected={draft.cwd === workspace.path}
                    />
                  ))}
                </View>
                <TextInput
                  accessibilityLabel={t("tasks.form.cwdPath")}
                  autoCapitalize="none"
                  style={settingsStyles.input}
                  value={draft.cwd}
                  onChangeText={cwd => patch({ cwd })}
                />
              </SettingsField>
            )
          : null}

        <SettingsField label={t("tasks.form.model")} hint={models.failed ? t("tasks.form.modelsFailed") : undefined}>
          <View style={settingsStyles.actions}>
            {pinnedModel
              ? (
                  <Choice
                    disabled={busy}
                    key={pinnedModel}
                    label={pinnedModel}
                    onPress={() => patch({ modelId: pinnedModel })}
                    selected={draft.modelId === pinnedModel}
                  />
                )
              : null}
            {enabledModels.map(model => (
              <Choice
                disabled={busy}
                key={modelKey(model)}
                label={model.label || model.id}
                onPress={() => patch({ modelId: modelKey(model) })}
                selected={draft.modelId === modelKey(model)}
              />
            ))}
          </View>
        </SettingsField>

        <SettingsField label={t("tasks.form.thinking")}>
          <View style={settingsStyles.actions}>
            {THINKING_LEVELS.map(level => (
              <Choice
                disabled={busy}
                key={level}
                label={t(`tasks.thinkingLabels.${level}`)}
                onPress={() => patch({ thinkingLevel: level })}
                selected={draft.thinkingLevel === level}
              />
            ))}
          </View>
        </SettingsField>

        <SettingsField label={t("tasks.form.session")} hint={draft.sessionPolicy === "existing" ? t("tasks.form.sessionExistingHint") : undefined}>
          <View style={settingsStyles.actions}>
            {(["new", "existing"] as const).map(policy => (
              <Choice
                disabled={busy}
                key={policy}
                label={t(policy === "existing" ? "tasks.form.sessionExisting" : "tasks.form.sessionNew")}
                onPress={() => patch({ sessionPolicy: policy })}
                selected={draft.sessionPolicy === policy}
              />
            ))}
          </View>
        </SettingsField>

        <SettingsField label={t("tasks.form.enablement")}>
          <View style={settingsStyles.actions}>
            {([true, false] as const).map(on => (
              <Choice
                disabled={busy}
                key={String(on)}
                label={t(on ? "tasks.form.enabled" : "tasks.form.disabled")}
                onPress={() => patch({ enabled: on })}
                selected={draft.enabled === on}
              />
            ))}
          </View>
        </SettingsField>

        <SettingsField label={t("tasks.form.reflection")} hint={t("tasks.form.reflectionHint")}>
          <View style={settingsStyles.actions}>
            {(["off", "ask", "auto"] as const).map(level => (
              <Choice
                disabled={busy}
                key={level}
                label={t(`tasks.form.reflection${level === "off" ? "Off" : level === "auto" ? "Auto" : "Ask"}`)}
                onPress={() => patch({ reflection: level })}
                selected={draft.reflection === level}
              />
            ))}
          </View>
        </SettingsField>
      </SettingsSection>

      <SettingsSection title={t("tasks.form.trigger")}>
        <View style={settingsStyles.actions}>
          {(["manual", "dependency", "once", "interval", "daily", "weekly", "monthly"] as const).map(mode => (
            <Choice
              disabled={busy}
              key={mode}
              label={t(`tasks.triggerMode.${mode}`)}
              onPress={() => patchTrigger({ mode })}
              selected={trigger.mode === mode}
            />
          ))}
        </View>
        {trigger.mode === "once"
          ? (
              <SettingsField label={t("tasks.form.date")}>
                <TextInput accessibilityLabel={t("tasks.form.date")} placeholder="2026-12-24" style={settingsStyles.input} value={trigger.date} onChangeText={date => patchTrigger({ date })} />
              </SettingsField>
            )
          : null}
        {trigger.mode !== "manual" && trigger.mode !== "dependency" && trigger.mode !== "interval"
          ? (
              <SettingsField label={t("tasks.form.time")}>
                <TextInput accessibilityLabel={t("tasks.form.time")} style={settingsStyles.input} value={trigger.time} onChangeText={time => patchTrigger({ time })} />
              </SettingsField>
            )
          : null}
        {trigger.mode === "interval"
          ? (
              <SettingsField label={t("tasks.form.everyMinutes")}>
                <TextInput accessibilityLabel={t("tasks.form.everyMinutes")} keyboardType="number-pad" style={settingsStyles.input} value={trigger.everyMinutes} onChangeText={everyMinutes => patchTrigger({ everyMinutes })} />
              </SettingsField>
            )
          : null}
        {trigger.mode === "weekly"
          ? (
              <SettingsField label={t("tasks.form.days")} hint="mon,tue,wed">
                <TextInput accessibilityLabel={t("tasks.form.days")} autoCapitalize="none" style={settingsStyles.input} value={trigger.days} onChangeText={days => patchTrigger({ days })} />
              </SettingsField>
            )
          : null}
        {trigger.mode === "monthly"
          ? (
              <SettingsField label={t("tasks.form.day")} hint={t("tasks.form.shortMonthHint")}>
                <TextInput accessibilityLabel={t("tasks.form.day")} keyboardType="number-pad" style={settingsStyles.input} value={trigger.day} onChangeText={day => patchTrigger({ day })} />
              </SettingsField>
            )
          : null}
        {problem
          ? <Text accessibilityRole="alert" style={settingsStyles.error}>{t(`tasks.form.problem.${problem}`)}</Text>
          : null}
        <Button label={t("tasks.form.save")} disabled={busy || problem !== null} onPress={save} />
        <Text style={settingsStyles.description}>{t("tasks.form.fullPermissionWarning")}</Text>
      </SettingsSection>

      {/* The dependency editor belongs to the trigger choice, and stays visible
          for a task that already has upstreams (even next to a schedule of its
          own): an edge nobody can see is an edge nobody can remove. */}
      {trigger.mode === "dependency" || draft.deps.length > 0
        ? (
            <SettingsSection title={t("tasks.form.deps")}>
              {trigger.mode !== "dependency"
                ? <Text style={settingsStyles.description}>{t("tasks.form.depsAlongsideSchedule")}</Text>
                : null}
              {!canEditDeps
                ? <Text style={settingsStyles.description}>{t("tasks.form.depsUnsupported")}</Text>
                : (
              <>
                {draft.deps.length === 0
                  ? <Text style={settingsStyles.description}>{t("tasks.form.depsNone")}</Text>
                  : draft.deps.map(dep => (
                      <View key={dep.upstreamTaskId} style={styles.depCard}>
                        <Text style={settingsStyles.label} numberOfLines={1}>{dep.name || dep.upstreamTaskId}</Text>
                        <View style={settingsStyles.actions}>
                          {(["success", "failure", "completed"] as const).map(on => (
                            <Choice
                              disabled={busy}
                              key={on}
                              label={t(`tasks.on.${on}`)}
                              onPress={() => patch({
                                deps: draft.deps.map(item => (
                                  item.upstreamTaskId === dep.upstreamTaskId ? { ...item, on } : item
                                )),
                              })}
                              selected={dep.on === on}
                            />
                          ))}
                        </View>
                        <Button
                          compact
                          disabled={busy}
                          label={t("tasks.form.depRemove", { name: dep.name || dep.upstreamTaskId })}
                          variant="secondary"
                          onPress={() => patch({
                            deps: draft.deps.filter(item => item.upstreamTaskId !== dep.upstreamTaskId),
                          })}
                        />
                      </View>
                    ))}

                {addable.length === 0
                  ? <Text style={settingsStyles.description}>{t("tasks.form.depNoCandidates")}</Text>
                  : (
                      <View style={settingsStyles.actions}>
                        {addable.map(candidate => (
                          <Choice
                            disabled={busy}
                            key={candidate.id}
                            label={candidate.name}
                            // A new edge waits for a successful finish: the
                            // common case, and what the CLI's bare
                            // `--depends-on NAME` means.
                            onPress={() => patch({
                              deps: [
                                ...draft.deps,
                                { upstreamTaskId: candidate.id, name: candidate.name, on: "success" },
                              ],
                            })}
                            selected={false}
                          />
                        ))}
                      </View>
                    )}

                {draft.deps.length > 1
                  ? (
                      <SettingsField label={t("tasks.form.depJoin")}>
                        <View style={settingsStyles.actions}>
                          {(["all", "any"] as const).map(join => (
                            <Choice
                              disabled={busy}
                              key={join}
                              label={t(`tasks.join.${join}`)}
                              onPress={() => patch({ depJoin: join })}
                              selected={draft.depJoin === join}
                            />
                          ))}
                        </View>
                      </SettingsField>
                    )
                  : null}
              </>
                )}
            </SettingsSection>
          )
        : null}

      {/* The dependency editor above owns this list when the desktop supports
          it; an older desktop gets the read-only view instead of nothing. */}
      {!canEditDeps && deps.length > 0
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
              {revisions.map((revision) => {
                // A suggestion is not a version yet: it has no version number,
                // and applying it is what puts it in force (the row then reads
                // "applied").
                const proposed = revision.status === "proposed";
                const confidence = revision.confidence != null
                  ? ` · ${t("tasks.confidence", { value: Math.round(revision.confidence * 100) })}`
                  : "";
                const heading = proposed
                  ? `${t("tasks.suggestion")} · ${t(`tasks.source.${revision.source}`)}${confidence}`
                  : `v${revision.version} · ${t(`tasks.source.${revision.source}`)}${revision.status === "applied" ? ` · ${t("tasks.applied")}` : ""}`;
                return (
                  <View key={revision.id} style={settingsStyles.card}>
                    <Text style={settingsStyles.label}>{heading}</Text>
                    <Text style={settingsStyles.description}>{revision.reason ?? revision.promptPreview}</Text>
                    <Button label={proposed ? t("tasks.applySuggestion") : t("tasks.apply")} disabled={busy} onPress={() => void onMutate?.(() => remote.applyTaskRevision(detail!.id, revision.id))} />
                  </View>
                );
              })}
            </SettingsSection>
          )
        : null}
    </ScrollView>
  );
}

/**
 * The stable identity of a catalogue model. A model id is only unique within its
 * provider, so the two together are what a task stores (`provider/id`).
 */
function modelKey(model: RemoteModel): string {
  return model.provider ? `${model.provider}/${model.id}` : model.id;
}

const styles = StyleSheet.create({
  choice: { minHeight: layout.touchTarget, justifyContent: "center", paddingHorizontal: spacing.md, paddingVertical: spacing.sm, borderWidth: 1, borderColor: colors.line, borderRadius: radius.md, backgroundColor: colors.surface },
  choiceSelected: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  suggestionBadge: { color: colors.accent, fontSize: 12, fontWeight: "600" },
  stack: { gap: spacing.md },
  taskCard: { gap: spacing.sm, padding: spacing.lg, borderWidth: 1, borderColor: colors.line, borderRadius: radius.lg, backgroundColor: colors.surface },
  cardHeader: { flexDirection: "row", alignItems: "center", gap: spacing.md },
  cardTitle: { flex: 1, minWidth: 0, color: colors.inkStrong, fontSize: 15, fontWeight: "600" },
  runCard: { gap: spacing.sm, padding: spacing.lg, borderWidth: 1, borderColor: colors.line, borderRadius: radius.lg, backgroundColor: colors.surface },
  runSummary: { color: colors.inkSoft, fontSize: 13, lineHeight: 20 },
  depCard: { gap: spacing.xs, padding: spacing.md, borderWidth: 1, borderColor: colors.line, borderRadius: radius.md, backgroundColor: colors.surface },
});
