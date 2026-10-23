import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import { useRemoteControls } from "../../remote/RemoteContext";
import type { DesktopSettings, RemoteModel } from "../../remote/types";
import { modelReference, modelThinkingLevels, resolveModelThinkingLevel } from "../../remote/types";
import type { RemoteTaskDetail, RemoteTaskRow, RemoteTaskRun } from "../../remote/taskTypes";
import { layout, radius, spacing } from "../../theme/tokens";
import { createThemedStyles } from "../../theme/ThemeContext";
import { ResourceStatus, SettingsField, SettingsSection, useSettingsStyles } from "./SettingsPrimitives";
import { DateField } from "./DateField";
import { TimeField } from "./TimeField";
import { TimezoneField } from "./TimezoneField";
import type { SettingsPageBack } from "./pageBack";
import { clearTaskDraft, loadTaskDraft, saveTaskDraft, taskDraftKey } from "./taskDraftStorage";
import { useDesktopResource } from "./useDesktopResource";

/** Local draft of the trigger, mirroring the desktop form. */
interface DraftTrigger {
  /** Empty is "not chosen yet": a new task must pick one (see `draftProblem`). */
  mode: "" | "manual" | "once" | "interval" | "daily" | "weekly" | "monthly" | "yearly";
  date: string;
  time: string;
  everyMinutes: string;
  days: string;
  day: string;
  month: string;
}

/** The weekdays a weekly trigger can run on, in the order both clients store. */
const WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;

/** The trigger modes a *new* task may pick, in the desktop's order. */
const TRIGGER_MODES = ["once", "interval", "daily", "weekly", "monthly", "yearly"] as const;

/**
 * The trigger a new task starts on: none chosen.
 *
 * The desktop dropped "manual" as a fresh choice when it dropped the run-now
 * button — a task whose trigger never fires, and which nothing can start, is not
 * a scheduled task. The phone's form used to start there, so the task it built
 * by default was exactly that: `TriggerKind::Manual` has no next due time, and
 * the phone has no run-now either. Asking is the fix; the old option stays
 * reachable for a task that is already manual, so an edit round-trips.
 */
const defaultTrigger: DraftTrigger = {
  mode: "",
  date: "",
  time: "09:00",
  everyMinutes: "60",
  days: "mon",
  day: "1",
  month: "12",
};

/** Whether a mode's times are read on a clock, and so needs a zone. */
function triggerReadsClock(mode: DraftTrigger["mode"]): boolean {
  return mode === "once" || mode === "daily" || mode === "weekly"
    || mode === "monthly" || mode === "yearly";
}

/** The weekdays a weekly trigger runs on, as the list the payload takes. */
function selectedDays(days: string): string[] {
  return days.split(",").map(day => day.trim()).filter(Boolean);
}

/** Flip one weekday on or off, stored in week order rather than tap order. */
function toggleDay(days: string, weekday: string): string {
  const chosen = selectedDays(days);
  const next = chosen.includes(weekday)
    ? chosen.filter(day => day !== weekday)
    : [...chosen, weekday];
  // Week order, not tap order: both clients then build the same payload for the
  // same set, and the summary reads the same whichever order the user tapped.
  return WEEKDAYS.filter(day => next.includes(day)).join(",");
}

/** Thinking levels a task may pin, in the composer's order. */
const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"] as const;

/**
 * Everything the editor can change. The desktop keeps the same shape in its
 * own draft, so the two forms send the same payload.
 */
interface Draft {
  name: string;
  prompt: string;
  cwd: string;
  modelId: string;
  thinkingLevel: string;
  /** Empty = the desktop's own zone (resolved there, Beijing fallback). */
  timezone: string;
  sessionPolicy: string;
  sessionRetention: string;
  conversationMode: string;
  enabled: boolean;
  trigger: DraftTrigger;
}

function triggerFrom(detail: RemoteTaskDetail): DraftTrigger {
  if (detail.triggerKind !== "schedule")
    // Already manual: the option comes back on the form so an edit round-trips,
    // rather than silently turning the task into something that runs.
    return { ...defaultTrigger, mode: "manual" };
  const trigger = detail.trigger ?? {};
  const mode = String(trigger.mode ?? "daily") as DraftTrigger["mode"];
  return {
    mode,
    date: String(trigger.date ?? ""),
    time: String(trigger.time ?? "09:00"),
    everyMinutes: String(trigger.every_minutes ?? 60),
    days: Array.isArray(trigger.days) ? (trigger.days as string[]).join(",") : "mon",
    day: String(trigger.day ?? 1),
    month: String(trigger.month ?? 12),
  };
}

function triggerPayload(draft: DraftTrigger): { triggerKind: string; trigger: Record<string, unknown> } {
  // "" never reaches here: `draftProblem` refuses the save until a mode is
  // picked, the same way the desktop's form does.
  if (draft.mode === "" || draft.mode === "manual")
    return { triggerKind: "manual", trigger: {} };
  const time = draft.time.trim() || "09:00";
  switch (draft.mode) {
    case "once":
      return { triggerKind: "schedule", trigger: { mode: "once", date: draft.date.trim(), time } };
    case "interval":
      return { triggerKind: "schedule", trigger: { mode: "interval", every_minutes: Number(draft.everyMinutes) || 1 } };
    case "weekly": {
      return { triggerKind: "schedule", trigger: { mode: "weekly", days: selectedDays(draft.days), time } };
    }
    case "monthly":
      return { triggerKind: "schedule", trigger: { mode: "monthly", day: Number(draft.day) || 1, time } };
    case "yearly":
      return {
        triggerKind: "schedule",
        trigger: {
          mode: "yearly",
          month: Number(draft.month) || 1,
          day: Number(draft.day) || 1,
          time,
        },
      };
    default:
      return { triggerKind: "schedule", trigger: { mode: "daily", time } };
  }
}

/** A stored task as an editable draft. */
function draftFrom(detail: RemoteTaskDetail): Draft {
  return {
    name: detail.name,
    prompt: detail.prompt,
    cwd: detail.cwd,
    modelId: detail.modelId ?? "",
    thinkingLevel: detail.thinkingLevel ?? "",
    timezone: detail.timezone ?? "",
    sessionPolicy: detail.sessionPolicy ?? "new",
    sessionRetention: detail.sessionRetention ?? "keep",
    conversationMode: detail.conversationMode ?? "workspace",
    enabled: detail.enabled,
    trigger: triggerFrom(detail),
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
    timezone: "",
    sessionPolicy: "new",
    sessionRetention: "keep",
    conversationMode: "chat",
    // A new task starts enabled, like the desktop's form does.
    enabled: true,
    trigger: { ...defaultTrigger },
  };
}

/**
 * A stored draft, with every field held to the shape this form writes.
 *
 * The stored value is read from disk, where it may have come from an older
 * build, a truncated write or another writer entirely; a field of the wrong
 * type falls back to the blank draft's own default instead of reaching an input
 * as `[object Object]`. Unknown keys are dropped rather than carried along.
 */
function restoredDraft(raw: unknown, base: Draft): Draft | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const value = raw as Record<string, unknown>;
  const text = (candidate: unknown, fallback: string) =>
    typeof candidate === "string" ? candidate : fallback;
  const oneOf = <T extends string>(candidate: unknown, allowed: readonly T[], fallback: T): T =>
    typeof candidate === "string" && (allowed as readonly string[]).includes(candidate)
      ? candidate as T
      : fallback;
  const rawTrigger = value.trigger && typeof value.trigger === "object" && !Array.isArray(value.trigger)
    ? value.trigger as Record<string, unknown>
    : {};
  return {
    name: text(value.name, base.name),
    prompt: text(value.prompt, base.prompt),
    cwd: text(value.cwd, base.cwd),
    modelId: text(value.modelId, base.modelId),
    thinkingLevel: text(value.thinkingLevel, base.thinkingLevel),
    timezone: text(value.timezone, base.timezone),
    sessionPolicy: oneOf(value.sessionPolicy, ["new", "existing"] as const, base.sessionPolicy),
    sessionRetention: oneOf(value.sessionRetention, ["keep", "delete"] as const, base.sessionRetention),
    conversationMode: oneOf(value.conversationMode, ["chat", "workspace"] as const, base.conversationMode),
    enabled: typeof value.enabled === "boolean" ? value.enabled : base.enabled,
    trigger: {
      mode: oneOf(
        rawTrigger.mode,
        // "manual" only survives where the base already is manual (an edit of a
        // manual task). A stored *new-task* draft that predates this change must
        // not restore the dead default the form no longer offers.
        base.trigger.mode === "manual"
          ? ["manual", ...TRIGGER_MODES] as const
          : TRIGGER_MODES,
        base.trigger.mode,
      ),
      date: text(rawTrigger.date, base.trigger.date),
      time: text(rawTrigger.time, base.trigger.time),
      everyMinutes: text(rawTrigger.everyMinutes, base.trigger.everyMinutes),
      days: text(rawTrigger.days, base.trigger.days),
      day: text(rawTrigger.day, base.trigger.day),
      month: text(rawTrigger.month, base.trigger.month),
    },
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
    // Empty means "the desktop's own zone"; the desktop resolves and stores a
    // concrete name, so the schedule keeps its meaning.
    timezone: draft.timezone.trim(),
    sessionPolicy: draft.sessionPolicy,
    // Only a per-run conversation can be deleted; the server normalizes the
    // contradiction too, this just keeps the payload honest.
    sessionRetention: draft.sessionPolicy === "new" ? draft.sessionRetention : "keep",
    conversationMode: draft.conversationMode,
    enabled: draft.enabled,
    ...triggerPayload(draft.trigger),
  };
}

/** Why the form cannot be submitted yet, or null when it can. */
function draftProblem(draft: Draft): "name" | "prompt" | "cwd" | "model" | "thinking" | "trigger" | "date" | "time" | null {
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
  // A trigger is required, like the model: nothing else starts a scheduled task,
  // and a manual one has no next due time at all.
  if (draft.trigger.mode === "")
    return "trigger";
  if (draft.trigger.mode === "once" && !/^\d{4}-\d{2}-\d{2}$/.test(draft.trigger.date.trim()))
    return "date";
  // A one-off also needs a time: the desktop no longer seeds one the user
  // never picked, so an empty field here would schedule an unparsable instant.
  if (draft.trigger.mode === "once" && !/^\d{2}:\d{2}$/.test(draft.trigger.time.trim()))
    return "time";
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
 * Date + time at minute precision, localized on the task's own clock.
 *
 * `timeZone` is the task's, not the viewer's: a task scheduled for 09:00
 * Asia/Shanghai is 09:00 there whatever the phone is set to, and the desktop
 * already renders it that way (`formatDateTime(…, task.timezone)`). Without it
 * the two clients showed different wall clocks for the same run, which is
 * exactly the reading a schedule must not disagree about. An unknown zone falls
 * back to the phone's clock rather than throwing.
 */
function formatWhen(value: string | number, locale: string, timeZone?: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime()))
    return String(value);
  const options: Intl.DateTimeFormatOptions = {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  };
  if (timeZone)
    options.timeZone = timeZone.trim();
  try {
    return new Intl.DateTimeFormat(locale, options).format(date);
  }
  catch {
    delete options.timeZone;
    return new Intl.DateTimeFormat(locale, options).format(date);
  }
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
    case "yearly":
      return t("tasks.trigger.yearly", {
        month: Number(trigger.month ?? 1),
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
  const settingsStyles = useSettingsStyles();
  const styles = useStyles();
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
export function TasksSettingsPage({ desktopOnline, settings, onOpenConversation, onBackLevel }: {
  desktopOnline: boolean;
  /** The desktop's settings, for the enabled-model list (Settings → Models). */
  settings: DesktopSettings | null;
  /**
   * Open one of a run's conversations in the chat. Wired from the app shell,
   * which closes settings first (the chat is a different screen here).
   */
  onOpenConversation?(sessionId: string): void;
  /**
   * Tell the settings stack how to leave one level of *this* page. The system
   * back gesture otherwise pops the whole Tasks page to the settings home,
   * even with a task open — see `SettingsScreen`'s `goBack`.
   */
  onBackLevel?(handle: SettingsPageBack | null): void;
}) {
  const settingsStyles = useSettingsStyles();
  const styles = useStyles();
  const { t, i18n } = useTranslation();
  const remote = useRemoteControls();
  // `desktopOnline` disables controls; reading needs the machine to be its own
  // controller too (see `RemoteControls.desktopReadable`).
  const tasks = useDesktopResource(remote.listTasks, 0, desktopOnline && remote.desktopReadable);
  const [openId, setOpenId] = useState<string | null>(null);
  /** Creating opens the same editor with no task behind it. */
  const [creating, setCreating] = useState(false);
  /**
   * A task is two pages: the task itself (what it is, what it did) and the form
   * that changes it, one level down. A new task has no history to lead with, so
   * it opens on the form instead.
   */
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [detail, setDetail] = useState<RemoteTaskDetail | null>(null);
  const [runs, setRuns] = useState<RemoteTaskRun[]>([]);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const active = useRef(true);
  // Re-entrancy guard for writes. State updates land after the render that set
  // them, so two presses in one tick would both pass a `busy` check; the ref is
  // set synchronously and drops the second one (same shape as the desktop's
  // `writing` guard).
  const writing = useRef(false);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);

  /**
   * The form's draft slot on this machine.
   *
   * A task id is only meaningful on the desktop that owns it, so the pair is
   * the identity. `unknown` is the brief window before the connection reports
   * which machine it is — the draft still saves, and a later mount on the same
   * machine simply uses the real key.
   */
  const draftKeyFor = useCallback(
    (taskId: string | null) => taskDraftKey(remote.credentials?.expectedDesktopId ?? "unknown", taskId),
    [remote.credentials],
  );
  /**
   * Drop a draft the user has left behind.
   *
   * A form that is saved, cancelled, deleted or backed out of has no unfinished
   * work any more; keeping its draft would hand the next open a page of edits
   * the task itself no longer has.
   */
  const discardDraft = useCallback((taskId: string | null) => {
    void clearTaskDraft(draftKeyFor(taskId));
  }, [draftKeyFor]);

  const loadDetail = useCallback(async (taskId: string) => {
    const [nextDetail, nextRuns] = await Promise.all([
      remote.getTask(taskId),
      remote.listTaskRuns(taskId),
    ]);
    if (!active.current) return;
    setDetail(nextDetail);
    setRuns(nextRuns);
  }, [remote]);

  const open = useCallback((taskId: string) => {
    setCreating(false);
    setSettingsOpen(false);
    setOpenId(taskId);
    setDetail(null);
    void loadDetail(taskId).catch(() => {
      if (active.current) setFailed(true);
    });
  }, [loadDetail]);

  const close = useCallback(() => {
    setCreating(false);
    setSettingsOpen(false);
    setOpenId(null);
    setDetail(null);
  }, []);

  // One step back inside this page: the form first, the task second, the list
  // third. Registered with the settings stack, which is what the system back
  // gesture and the header arrow call — otherwise a swipe out of a task left
  // the whole Tasks page (and the list's scroll position) behind.
  const goBackOneLevel = useCallback(() => {
    // Back is not "throw this away": the phone's system gesture and the header
    // arrow are the way out of a screen, and a form a minute's phone call
    // interrupted is why the draft exists at all. The draft therefore stays —
    // it comes back when the form is opened again — and only the two deliberate
    // endings clear it: Cancel, and a save that landed.
    if (settingsOpen) {
      setSettingsOpen(false);
      return true;
    }
    if (creating || openId) {
      close();
      return true;
    }
    return false;
  }, [close, creating, openId, settingsOpen]);
  useEffect(() => {
    if (!onBackLevel) return;
    onBackLevel({ goBack: goBackOneLevel });
    return () => onBackLevel(null);
  }, [goBackOneLevel, onBackLevel]);

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

  /**
   * Run one write and report whether it landed. The editor uses the answer to
   * know whether it may leave its page: a refused save keeps the draft on
   * screen, where Save is the retry (a second press while one is in flight is
   * dropped, and cannot look like a success either).
   */
  const mutate = async (operation: () => Promise<unknown>): Promise<boolean> => {
    if (writing.current) return false;
    writing.current = true;
    setBusy(true);
    setFailed(false);
    try {
      await operation();
      await tasks.reload();
      if (openId) await loadDetail(openId);
      return true;
    }
    catch {
      if (active.current) setFailed(true);
      return false;
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
      discardDraft(task.id);
      close();
    });
  };

  if (creating) {
    return (
      <TaskForm
        busy={busy}
        desktopOnline={desktopOnline}
        failed={failed}
        kind="create"
        settings={settings}
        onCancel={() => {
          discardDraft(null);
          close();
        }}
        onCreate={draft => void mutate(async () => {
          await remote.createTask(draftPayload(draft));
          discardDraft(null);
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
        detail={detail}
        desktopOnline={desktopOnline}
        failed={failed}
        kind="edit"
        runs={runs}
        settings={settings}
        settingsOpen={settingsOpen}
        onOpenConversation={onOpenConversation}
        onOpenSettings={() => setSettingsOpen(true)}
        onCloseSettings={() => setSettingsOpen(false)}
        onDelete={() => remove(detail)}
        onMutate={mutate}
        onRetry={() => retryTask(detail.id)}
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
                      {task.nextDueAt ? ` · ${formatWhen(task.nextDueAt, i18n.language, task.timezone)}` : ""}
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
 * An existing task is two pages: the task itself (its actions, then what it
 * did) and the form that changes it, one level down and reached from the
 * Settings button. A new task has no history to lead with, so it opens on the
 * form.
 *
 * `create` has no `detail` to read from, so the draft starts blank; `edit`
 * starts from the stored task and keys the component by prompt version, so a
 * save (or a reload) remounts it with fresh values instead of an effect writing
 * state on every incoming snapshot.
 */
function TaskForm({
  kind, detail, settings, busy, desktopOnline, failed, runs = [],
  settingsOpen = false,
  onCancel, onCloseSettings, onCreate, onDelete, onMutate, onOpenConversation, onOpenSettings, onRetry,
}: {
  kind: "create" | "edit";
  detail?: RemoteTaskDetail;
  /** The desktop's own settings — the enabled-model list comes from them. */
  settings: DesktopSettings | null;
  busy: boolean;
  desktopOnline: boolean;
  failed: boolean;
  runs?: RemoteTaskRun[];
  /** Whether the form is on top (edit mode; a new task opens straight on it). */
  settingsOpen?: boolean;
  onCancel?(): void;
  /** Leave the form for the task, once a save has landed (edit mode). */
  onCloseSettings?(): void;
  onCreate?(draft: Draft): void;
  onDelete?(): void;
  onMutate?(operation: () => Promise<unknown>): Promise<boolean>;
  /** Open a run's conversation in the chat (absent while creating a task). */
  onOpenConversation?(sessionId: string): void;
  /** Reveal the form, one level down from the task (edit mode). */
  onOpenSettings?(): void;
  onRetry?(): void;
}) {
  const settingsStyles = useSettingsStyles();
  const styles = useStyles();
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const models = useDesktopResource(remote.listSettingsModels, 0, desktopOnline && remote.desktopReadable);
  // Provider id → display name (same source the Models page groups by), so a
  // model chip can say which vendor it runs on.
  const providers = useDesktopResource(remote.listProviders, 0, desktopOnline && remote.desktopReadable);
  const providerNames = useMemo(() => new Map(
    [...providers.data?.builtin ?? [], ...providers.data?.custom ?? []].map(provider => [provider.id, provider.name]),
  ), [providers.data]);
  const [draft, setDraft] = useState<Draft>(() => detail ? draftFrom(detail) : newDraft());
  const [confirmDelete, setConfirmDelete] = useState(false);
  /**
   * Where this form's unfinished work is kept between mounts.
   *
   * The form is long and the phone leaves the app at a moment's notice; a draft
   * that lives only in state came back empty after the process was killed. It
   * is read once when the form opens and written as it changes, so the next
   * open — after a cold start, or after coming back from another app — finds
   * what was being typed.
   */
  const draftKey = taskDraftKey(
    remote.credentials?.expectedDesktopId ?? "unknown",
    kind === "edit" ? detail!.id : null,
  );
  /**
   * The slot whose stored draft has been read (or found absent).
   *
   * A key rather than a flag: a pairing whose desktop id only arrives after the
   * first render changes `draftKey` without remounting the form, and a bare
   * "loaded" flag would let the write effect replace the new slot's draft before
   * anything had read it.
   */
  const [loadedKey, setLoadedKey] = useState<string | null>(null);
  const draftReady = loadedKey === draftKey;
  const edited = useRef(false);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    let active = true;
    void loadTaskDraft(draftKey).then(stored => {
      if (!active) return;
      // An edit draft written against a prompt that has since changed is not
      // this task's work any more: the desktop's new prompt wins, and the stale
      // slot is dropped rather than offered back.
      const currentVersion = kind === "edit" ? detail?.promptVersion ?? null : null;
      const usable = stored && (kind === "create" || stored.promptVersion === currentVersion);
      if (stored && !usable) void clearTaskDraft(draftKey);
      if (usable && !edited.current) {
        const restored = restoredDraft(stored.draft, draft);
        if (restored) setDraft(restored);
      }
      setLoadedKey(draftKey);
    });
    return () => { active = false; };
    // `draft` is read as the fallback shape only; re-running on it would reload
    // the stored draft over every keystroke.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [draftKey, detail?.promptVersion, kind]);
  useEffect(() => {
    // Until the stored draft has been read, writing would replace the very
    // draft this mount is about to restore with the blank one. And a form the
    // user has only looked at is not a draft at all: storing the task's own
    // values would later be "restored" over a task somebody has since edited
    // elsewhere (the prompt version check catches a prompt change, not a
    // schedule change).
    if (!draftReady || !edited.current) return;
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      void saveTaskDraft(draftKey, draft, kind === "edit" ? detail?.promptVersion ?? null : null);
    }, 250);
    return () => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
    };
  }, [draft, draftKey, draftReady, detail?.promptVersion, kind]);
  // Which of the editor's two pages is on screen: an existing task opens on
  // what it did, with the form one level down behind the Settings button.
  const taskPage = kind === "edit" && !settingsOpen;
  const formPage = kind === "create" || settingsOpen;
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
  // The levels the task's model offers. The agent clamps anything else when the
  // run starts, so the form does not offer a level the task would not run at.
  const thinkingLevels = modelThinkingLevels(
    enabledModels.find(model => modelKey(model) === draft.modelId),
    THINKING_LEVELS,
  );
  /**
   * The level this form shows and saves.
   *
   * A task written when the model offered `medium`, against a model whose
   * `models.json` list has since dropped it, would otherwise show no selected
   * chip while its save wrote a level the run replaces. An *empty* level stays
   * empty: that is "not chosen yet", which the form asks for rather than filling
   * in. Derived rather than written back, so opening the form does not edit the
   * stored task behind the user's back.
   */
  const resolvedLevel = resolveModelThinkingLevel(
    draft.thinkingLevel || null,
    enabledModels.find(model => modelKey(model) === draft.modelId),
  );
  const thinkingLevel = draft.thinkingLevel ? (resolvedLevel ?? draft.thinkingLevel) : "";
  const patch = (values: Partial<Draft>) => {
    // A keystroke makes the on-screen form the user's own: a stored draft that
    // arrives after it must not overwrite what is being typed.
    edited.current = true;
    setDraft(current => ({ ...current, ...values }));
  };
  const patchTrigger = (values: Partial<DraftTrigger>) => {
    edited.current = true;
    setDraft(current => ({ ...current, trigger: { ...current.trigger, ...values } }));
  };
  const problem = draftProblem(draft);
  const trigger = draft.trigger;

  const save = async () => {
    // Create goes through `onCreate` (the parent owns the request); edit goes
    // through `onMutate`, which also handles the in-flight guard and reload.
    // Requiring both would make either mode silently do nothing.
    if (problem) return;
    if (kind === "create") {
      onCreate?.({ ...draft, thinkingLevel });
      return;
    }
    const saved = await onMutate?.(async () => {
      await remote.updateTask(detail!.id, draftPayload({ ...draft, thinkingLevel }));
    });
    // A save that did not land keeps the form open: the draft is its own retry.
    // A save that landed has the task on the desktop, so the stored draft has
    // nothing left to recover.
    if (saved) {
      void clearTaskDraft(draftKey);
      onCloseSettings?.();
    }
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

      {/* The task itself: its actions, then what it did. The form that changes
          it is one level down, so a long run history never buries the fields
          the way a fold at the bottom of this page did. */}
      {taskPage
        ? (
            <SettingsSection title={detail!.name}>
              {/* One compact row, not a stack of full-width buttons: the
                  phone's vertical space belongs to the form, and the desktop
                  puts these on one line too. Delete stays two-step (armed,
                  then confirmed). No run-now control: the desktop's panel
                  dropped it (a task runs when its trigger fires), and the
                  phone shows the same actions. */}
              <View style={settingsStyles.actions}>
                <Button compact label={detail!.enabled ? t("tasks.disable") : t("tasks.enable")} disabled={busy} variant="secondary" onPress={() => void onMutate?.(() => remote.setTaskEnabled(detail!.id, !detail!.enabled))} />
                {confirmDelete
                  ? (
                      <>
                        <Button compact label={t("tasks.deleteConfirmAction")} variant="danger" disabled={busy} onPress={() => onDelete?.()} />
                        <Button compact label={t("chat.cancel")} variant="secondary" disabled={busy} onPress={() => setConfirmDelete(false)} />
                      </>
                    )
                  : <Button compact label={t("tasks.delete")} variant="secondary" disabled={busy} onPress={() => setConfirmDelete(true)} />}
                {/* The way into the form. Leaving it is the same level back as
                    everywhere else (the header arrow, the system back
                    gesture), so it needs no button of its own. */}
                <Button compact label={t("tasks.edit")} variant="secondary" disabled={busy} onPress={() => onOpenSettings?.()} />
              </View>
              {/* The confirmation is a sentence, so it gets its own line under the row. */}
              {confirmDelete
                ? <Text style={settingsStyles.description}>{t("tasks.deleteConfirm", { name: detail!.name })}</Text>
                : null}
            </SettingsSection>
          )
        : null}
      {taskPage && runs.length > 0
        ? (
            <SettingsSection title={t("tasks.runs")}>
              {runs.map(run => (
                <View key={run.id} style={styles.runCard}>
                  <Text style={styles.runHeading}>
                    {t(`tasks.status.${run.status}`)}
                    {" · "}
                    {t(`tasks.kind.${run.kind}`)}
                    {run.promptVersion != null ? ` · ${t("tasks.runPromptVersion", { version: run.promptVersion })}` : ""}
                  </Text>
                  <Text style={styles.runSummary}>
                    {run.errorMessage ?? run.resultSummary ?? t("tasks.runNoSummary")}
                  </Text>
                  {/* The conversation is gone by design: say so, so the missing
                      open link reads as intended and the reader knows the output
                      is saved on the run. */}
                  {run.sessionDeleted
                    ? <Text style={settingsStyles.description}>{t("tasks.runSessionDeleted")}</Text>
                    : null}
                  {/* The desktop links a run to the conversation it ran in; the
                      phone can open it too, in the chat this app already has.
                      A run that never reached an agent has no conversation, and
                      an older desktop does not send the session id. */}
                  {run.sessionId && onOpenConversation
                    ? (
                        <Pressable
                          accessibilityRole="button"
                          disabled={busy}
                          onPress={() => onOpenConversation(run.sessionId as string)}
                        >
                          <Text style={styles.runLink}>{t("tasks.openConversation")}</Text>
                        </Pressable>
                      )
                    : null}
                </View>
              ))}
            </SettingsSection>
          )
        : null}

      {formPage
        ? (
            <>
            {/* The heading says which page this is: a new task, or the form
                that changes an existing one (whose name is its first field). */}
            <SettingsSection title={kind === "create" ? t("tasks.newTitle") : t("tasks.editTitle")}>
              {/* A new task is cancelled from here; an existing one leaves the
                  form with the level back (header arrow / system gesture), or
                  by saving. */}
              {kind === "create"
                ? (
                    <View style={settingsStyles.actions}>
                      <Button compact label={t("tasks.form.cancel")} variant="secondary" onPress={onCancel!} />
                    </View>
                  )
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
                  {enabledModels.map(model => {
                    // The vendor goes on every chip: two providers often expose
                    // the same model name, and a chip that only says the name
                    // cannot tell them apart. A providerless model has no
                    // vendor to add, so it stays the bare id.
                    const label = model.provider
                      ? `${model.label || model.id} · ${providerNames.get(model.provider) ?? model.provider}`
                      : (model.label || model.id);
                    return (
                      <Choice
                        disabled={busy}
                        key={modelKey(model)}
                        label={label}
                        onPress={() => patch({ modelId: modelKey(model) })}
                        selected={draft.modelId === modelKey(model)}
                      />
                    );
                  })}
                </View>
              </SettingsField>

              <SettingsField label={t("tasks.form.thinking")}>
                <View style={settingsStyles.actions}>
                  {thinkingLevels.map(level => (
                    <Choice
                      disabled={busy}
                      key={level}
                      label={t(`tasks.thinkingLabels.${level}`)}
                      onPress={() => patch({ thinkingLevel: level })}
                      selected={thinkingLevel === level}
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

        {/* Only a conversation opened per run can be thrown away after it: a
            reused one is what the next run continues. */}
        {draft.sessionPolicy === "new"
          ? (
              <SettingsField
                label={t("tasks.form.sessionRetention")}
                hint={draft.sessionRetention === "delete" ? t("tasks.form.sessionRetentionHint") : undefined}
              >
                <View style={settingsStyles.actions}>
                  {(["keep", "delete"] as const).map(choice => (
                    <Choice
                      disabled={busy}
                      key={choice}
                      label={t(`tasks.form.sessionRetention${choice === "keep" ? "Keep" : "Delete"}`)}
                      onPress={() => patch({ sessionRetention: choice })}
                      selected={draft.sessionRetention === choice}
                    />
                  ))}
                </View>
              </SettingsField>
            )
          : null}

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
            </SettingsSection>

            <SettingsSection title={t("tasks.form.trigger")}>
              {/* No "manual" as a fresh choice: the desktop dropped it when it
                  dropped the run-now button, and the phone has none either — a
                  task that never fires and that nothing can start is not a
                  scheduled task. A task that is already manual keeps its own
                  chip so an edit round-trips instead of quietly changing what
                  the task does. */}
              <View style={settingsStyles.actions}>
                {(trigger.mode === "manual" ? (["manual", ...TRIGGER_MODES] as const) : TRIGGER_MODES).map(mode => (
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
                      {/* A picker here too: a `2026-12-24` placeholder read as a
                          date already chosen, which is why the desktop replaced
                          its text field with a date picker. A time is required
                          with it, and this payload must match the desktop's. */}
                      <DateField
                        disabled={busy}
                        label={t("tasks.form.date")}
                        onChange={date => patchTrigger({ date })}
                        value={trigger.date}
                      />
                    </SettingsField>
                  )
                : null}
              {triggerReadsClock(trigger.mode)
                ? (
                    <SettingsField label={t("tasks.form.time")}>
                      {/* A picker, like the desktop's native time input: the filed
                          field needed the keyboard, which covered the bottom of
                          this form, and a schedule is a choice from 24 hours
                          rather than something to spell out. */}
                      <TimeField
                        disabled={busy}
                        label={t("tasks.form.time")}
                        onChange={time => patchTrigger({ time })}
                        value={trigger.time}
                      />
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
                    <SettingsField label={t("tasks.form.days")}>
                      {/* Seven toggles, not a text field asking the user to spell
                          `mon,tue,wed`: the desktop's form draws the same seven
                          checkboxes, and a typed list had no way to say which
                          names it would accept. */}
                      <View style={settingsStyles.actions}>
                        {WEEKDAYS.map(day => (
                          <Choice
                            disabled={busy}
                            key={day}
                            label={t(`tasks.weekday.${day}`)}
                            onPress={() => patchTrigger({ days: toggleDay(trigger.days, day) })}
                            selected={selectedDays(trigger.days).includes(day)}
                          />
                        ))}
                      </View>
                    </SettingsField>
                  )
                : null}
              {trigger.mode === "yearly"
                ? (
                    <>
                      <SettingsField label={t("tasks.form.month")}>
                        <TextInput accessibilityLabel={t("tasks.form.month")} keyboardType="number-pad" style={settingsStyles.input} value={trigger.month} onChangeText={month => patchTrigger({ month })} />
                      </SettingsField>
                      <SettingsField label={t("tasks.form.dayOfMonth")} hint={t("tasks.form.shortMonthHint")}>
                        <TextInput accessibilityLabel={t("tasks.form.dayOfMonth")} keyboardType="number-pad" style={settingsStyles.input} value={trigger.day} onChangeText={day => patchTrigger({ day })} />
                      </SettingsField>
                    </>
                  )
                : null}
              {trigger.mode === "monthly"
                ? (
                    <SettingsField label={t("tasks.form.day")} hint={t("tasks.form.shortMonthHint")}>
                      <TextInput accessibilityLabel={t("tasks.form.day")} keyboardType="number-pad" style={settingsStyles.input} value={trigger.day} onChangeText={day => patchTrigger({ day })} />
                    </SettingsField>
                  )
                : null}
              {/* The zone belongs to the schedule, not to the task: it says which
                  clock the times just chosen are read on, so it is asked inside
                  the trigger, after the mode that has times — the desktop asks
                  for it in exactly this place, and only for a mode that has one.
                  Empty means "the desktop's own zone", which is why it is a row
                  of its own rather than a blank field. */}
              {triggerReadsClock(trigger.mode)
                ? (
                    <SettingsField label={t("tasks.form.timezone")}>
                      <TimezoneField
                        disabled={busy}
                        hint={t("tasks.form.timezoneHint")}
                        label={t("tasks.form.timezone")}
                        onChange={timezone => patch({ timezone })}
                        value={draft.timezone}
                      />
                    </SettingsField>
                  )
                : null}
              {problem
                ? <Text accessibilityRole="alert" style={settingsStyles.error}>{t(`tasks.form.problem.${problem}`)}</Text>
                : null}
              <Button label={t("tasks.form.save")} disabled={busy || problem !== null} onPress={() => void save()} />
              <Text style={settingsStyles.description}>{t("tasks.form.fullPermissionWarning")}</Text>
            </SettingsSection>
            </>
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

const useStyles = createThemedStyles(theme => StyleSheet.create({
  choice: { minHeight: layout.touchTarget, justifyContent: "center", paddingHorizontal: spacing.md, paddingVertical: spacing.sm, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.md, backgroundColor: theme.colors.surface },
  choiceSelected: { borderColor: theme.colors.accent, backgroundColor: theme.colors.accentSoft },
  stack: { gap: spacing.md },
  taskCard: { gap: spacing.sm, padding: spacing.lg, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.lg, backgroundColor: theme.colors.surface },
  cardHeader: { flexDirection: "row", alignItems: "center", gap: spacing.md },
  cardTitle: { flex: 1, minWidth: 0, color: theme.colors.inkStrong, fontSize: theme.font(15), fontWeight: "600" },
  runCard: { gap: spacing.sm, padding: spacing.lg, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.lg, backgroundColor: theme.colors.surface },
  runHeading: { color: theme.colors.ink, fontSize: theme.font(13), fontWeight: "600" },
  runSummary: { color: theme.colors.inkSoft, fontSize: theme.font(13), lineHeight: theme.leading(20) },
  runLink: { color: theme.colors.accent, fontSize: theme.font(13), fontWeight: "600" },
}));
