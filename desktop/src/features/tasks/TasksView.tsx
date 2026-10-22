import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { TaskBackend } from "./taskBackend";
import type { TaskInput, TaskView } from "./useTasks";
import { open } from "@tauri-apps/plugin-dialog";
import { ChevronDown, ChevronLeft, ChevronRight, FolderOpen, Plus, Trash2 } from "lucide-react";
import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";
import { defaultThinkingLevel, modelSupportsThinking, normalizeThinkingLevel } from "../../integrations/agent/agentClient";
import { useProviderNames } from "../../integrations/agent/useProviderNames";
import { formatDateTime } from "../../lib/date";
import { useTauriEvent } from "../../lib/useTauriEvent";
import { MarkdownContent } from "../markdown/MarkdownContent";
import { useTasks } from "./useTasks";

/**
 * Task times are epoch milliseconds; the date helpers take ISO strings.
 * `timeZone` is the task's own zone: the same instant must read as the time the
 * schedule means, even when the person looking at it sits elsewhere.
 */
function formatEpoch(ms: number | null, locale: string, timeZone?: string): string {
  return ms == null ? "" : formatDateTime(new Date(ms).toISOString(), locale, timeZone);
}

const WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;

/**
 * Trigger modes the form can edit (mirrors the kernel's schedule modes). The
 * empty string is a new task's unchosen state: with the run-now button gone,
 * a manual task can no longer be fired from anywhere in the UI, so the form
 * stops offering "manual" as a fresh choice — it must be picked, like the
 * model is. A task that already is manual keeps the option so an edit
 * round-trips instead of silently rewriting it into a scheduled one.
 */
type TriggerMode = "" | "manual" | "once" | "interval" | "daily" | "weekly" | "monthly" | "yearly";

/**
 * Whether a schedule's times are wall-clock times. Only these read a time zone:
 * "every N minutes" counts from the last run, and a manual task has no schedule
 * at all, so the form asks which clock the times belong to after one of these
 * modes is chosen — never before the trigger that uses it.
 */
function triggerReadsClock(mode: TriggerMode): boolean {
  return mode === "once" || mode === "daily" || mode === "weekly" || mode === "monthly" || mode === "yearly";
}

interface DraftTrigger {
  mode: TriggerMode;
  date: string;
  time: string;
  everyMinutes: number;
  days: string[];
  day: number;
  month: number;
}

/**
 * Zones offered by the time-zone picker: every zone this webview knows, which
 * is the IANA database the runtime ships. `Intl.supportedValuesOf` is missing on
 * an older webview, so a short list of common zones keeps the picker usable
 * there instead of leaving one option; whatever the task already carries is
 * added either way, so an edit never silently rewrites a zone the list lacks.
 */
let knownTimezones: string[] | null = null;

/** The runtime's zone list, read once: it does not change within a session. */
function allTimezones(): string[] {
  if (knownTimezones)
    return knownTimezones;
  let zones: string[] = [];
  try {
    const supported = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] })
      .supportedValuesOf?.("timeZone");
    if (Array.isArray(supported))
      zones = supported;
  }
  catch {
    // Treated as "not supported" below.
  }
  if (zones.length === 0) {
    zones = [
      "UTC",
      "Asia/Shanghai",
      "Asia/Tokyo",
      "Asia/Singapore",
      "Asia/Hong_Kong",
      "Asia/Taipei",
      "Asia/Seoul",
      "Asia/Kolkata",
      "Asia/Dubai",
      "Europe/London",
      "Europe/Paris",
      "Europe/Berlin",
      "Europe/Moscow",
      "America/New_York",
      "America/Chicago",
      "America/Denver",
      "America/Los_Angeles",
      "America/Sao_Paulo",
      "Australia/Sydney",
    ];
  }
  knownTimezones = zones;
  return zones;
}

/** Every zone, with whatever the task already carries added if the list lacks it. */
function timezoneOptions(current: string): string[] {
  const zones = allTimezones();
  return current && !zones.includes(current) ? [current, ...zones] : zones;
}

const emptyTrigger: DraftTrigger = {
  mode: "",
  date: "",
  time: "",
  everyMinutes: 60,
  days: ["mon"],
  day: 1,
  month: 12,
};

/**
 * A new task's starting point.
 *
 * It deliberately leaves the model and the thinking level unset: both are
 * per-run spending decisions that belong to the user, so the form makes them
 * pick (and refuses a save until they have) instead of inheriting an app
 * default nobody chose. The conversation mode is chat, which is what a task
 * that only reads, writes text or calls tools needs — and, unlike a workspace
 * conversation, it needs no working directory.
 */
const emptyDraft = {
  name: "",
  prompt: "",
  cwd: "",
  modelId: "",
  thinkingLevel: "",
  /**
   * The zone the schedule's wall-clock times are read in. Empty means "this
   * machine's zone", which the host resolves when it stores the task (Beijing
   * when it cannot read one); every other value is a concrete IANA name.
   */
  timezone: "",
  sessionPolicy: "new",
  sessionRetention: "keep",
  conversationMode: "chat",
  enabled: true,
  trigger: { ...emptyTrigger },
};

/**
 * Thinking levels a task may pin, in the composer's order.
 *
 * Deliberately the full list rather than the selected model's declared
 * `ThinkingLevelMap`: those maps are sparse (several models declare none, and
 * `deepseek-flash` declares only `high`/`xhigh`), while an unmapped level is
 * passed through to the provider as-is and is accepted. Filtering by the map
 * would make the app default (`medium`) unselectable on most models. What the
 * form does mirror is the composer's one real rule: a model with no reasoning
 * support at all (`reasoning: false`) gets the control disabled.
 */
const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"] as const;

type Draft = typeof emptyDraft;

function triggerFromTask(task: TaskView): DraftTrigger {
  // A stored manual task keeps its mode so the edit round-trips: with the
  // run-now button gone it can no longer be fired by hand, but silently
  // rewriting it into a schedule would move a task the user did not touch.
  if (task.triggerKind !== "schedule")
    return { ...emptyTrigger, mode: "manual" };
  const trigger = task.trigger as Record<string, unknown>;
  const mode = String(trigger.mode ?? "daily") as TriggerMode;
  return {
    mode,
    date: String(trigger.date ?? ""),
    time: String(trigger.time ?? ""),
    everyMinutes: Number(trigger.every_minutes ?? 60),
    days: Array.isArray(trigger.days) ? (trigger.days as string[]) : ["mon"],
    day: Number(trigger.day ?? 1),
    month: Number(trigger.month ?? 12),
  };
}

function triggerPayload(draft: DraftTrigger): { kind: string; trigger: Record<string, unknown> } | null {
  // No mode chosen yet: the caller reports the missing field, the way it does
  // for an unchosen model. (A stored manual task keeps its own mode below.)
  if (!draft.mode)
    return null;
  if (draft.mode === "manual")
    return { kind: "manual", trigger: {} };
  switch (draft.mode) {
    case "once":
      // Both fields are required: an unset one is reported by the caller
      // (`missingTriggerField`), never sent as an empty string.
      return { kind: "schedule", trigger: { mode: "once", date: draft.date, time: draft.time } };
    case "interval":
      return { kind: "schedule", trigger: { mode: "interval", every_minutes: draft.everyMinutes } };
    case "weekly":
      return { kind: "schedule", trigger: { mode: "weekly", days: draft.days, time: draft.time } };
    case "monthly":
      return { kind: "schedule", trigger: { mode: "monthly", day: draft.day, time: draft.time } };
    case "yearly":
      return {
        kind: "schedule",
        trigger: { mode: "yearly", month: draft.month, day: draft.day, time: draft.time },
      };
    default:
      return { kind: "schedule", trigger: { mode: "daily", time: draft.time } };
  }
}

/**
 * The one-off trigger's first missing field, or null when both are set.
 * The kernel needs a real local date-time to schedule from, so an empty
 * date or time is a save refusal with its own message, not a silent
 * fallback to a moment the user never chose.
 */
function missingTriggerField(draft: DraftTrigger): "date" | "time" | null {
  if (!draft.date.trim())
    return "date";
  if (!draft.time.trim())
    return "time";
  return null;
}

/**
 * "Every N minutes", promoting exact hours and days to their own wording — a
 * 720-minute cadence reads as "every 12 hours", not as a wall of minutes.
 */
function intervalLabel(
  t: (key: string, options?: Record<string, unknown>) => string,
  minutes: number,
): string {
  if (minutes > 0 && minutes % (24 * 60) === 0)
    return t("trigger.everyDays", { days: minutes / (24 * 60) });
  if (minutes > 0 && minutes % 60 === 0)
    return t("trigger.everyHours", { hours: minutes / 60 });
  return t("trigger.every", { minutes });
}

function summarizeTrigger(
  t: (key: string, options?: Record<string, unknown>) => string,
  task: TaskView,
  locale: string,
): string {
  if (task.triggerKind !== "schedule")
    return t("trigger.manual");
  const trigger = task.trigger as Record<string, unknown>;
  const time = String(trigger.time ?? "");
  switch (String(trigger.mode ?? "")) {
    case "once": {
      // The stored value is an ISO day; a task list that prints `2026-12-24
      // 09:00` verbatim is untranslated in a locale that writes dates its own
      // way. Format it, and fall back to the raw pair if it is unparsable.
      const date = String(trigger.date ?? "");
      if (!/^\d{4}-\d{2}-\d{2}$/.test(date))
        return `${date} ${time}`.trim();
      return formatDateTime(`${date}T${time || "00:00"}:00`, locale);
    }
    case "interval":
      return intervalLabel(t, Number(trigger.every_minutes ?? 0));
    case "weekly": {
      // The stored codes are `mon`/`fri`; a list that shows them raw is
      // untranslated in a Chinese UI. Localize each selected day.
      const days = Array.isArray(trigger.days) ? (trigger.days as string[]) : [];
      const labels = days.map(day => t(`weekday.${day}`));
      return `${labels.join(", ")} ${time}`;
    }
    case "monthly":
      return t("trigger.monthly", {
        day: Number(trigger.day ?? 1),
        time,
      });
    case "yearly":
      return t("trigger.yearly", {
        month: Number(trigger.month ?? 1),
        day: Number(trigger.day ?? 1),
        time,
      });
    default:
      return `${t("trigger.daily")} ${time}`;
  }
}

/** Left-nav "Scheduled tasks" panel: list, edit, runs. */
export function TasksView({
  backend,
  leftPanelExpanded,
  modelOptions,
  onToggleLeftPanel,
  onOpenThread,
}: {
  /**
   * Which machine's tasks these are. Defaults to this app's own store, so the
   * local panel and this one are the same screen.
   */
  backend?: TaskBackend;
  leftPanelExpanded: boolean;
  /** Models the user has enabled (Settings → Models), the same list the composer offers. */
  modelOptions: AgentModelOption[];
  onToggleLeftPanel: () => void;
  onOpenThread: (threadId: string) => void;
}) {
  const { t, i18n } = useTranslation("tasks");
  const locale = i18n.language;
  // Provider id → display name, so the model picker can say which vendor a
  // model runs on (two providers often expose the same model name).
  const providerNames = useProviderNames();
  const store = useTasks(backend);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Draft>({ ...emptyDraft });
  const [saveError, setSaveError] = useState<string | null>(null);
  /** The delete confirmation lives in a dialog, not a browser `confirm()`. */
  const [deleting, setDeleting] = useState(false);
  const [deleteBusy, setDeleteBusy] = useState(false);

  const selected = useMemo(
    () => store.tasks.find(task => task.id === selectedId) ?? null,
    [store.tasks, selectedId],
  );

  const startCreate = useCallback(() => {
    setSelectedId(null);
    setEditing(true);
    setSaveError(null);
    setDraft({ ...emptyDraft });
  }, []);

  /**
   * Open the editor on a stored task.
   */
  const startEdit = useCallback((task: TaskView) => {
    setSelectedId(task.id);
    setEditing(true);
    setSaveError(null);
    setDraft({
      name: task.name,
      prompt: task.prompt,
      cwd: task.cwd,
      modelId: task.modelId ?? "",
      thinkingLevel: task.thinkingLevel ?? "",
      timezone: task.timezone ?? "",
      sessionPolicy: task.sessionPolicy,
      sessionRetention: task.sessionRetention,
      conversationMode: task.conversationMode,
      enabled: task.enabled,
      trigger: triggerFromTask(task),
    });
  }, []);

  const save = useCallback(async () => {
    const input: TaskInput = {
      name: draft.name.trim(),
      prompt: draft.prompt,
      cwd: draft.cwd.trim(),
      modelId: draft.modelId || null,
      thinkingLevel: draft.thinkingLevel || null,
      // Empty stands for "this machine's zone"; the host resolves and stores a
      // concrete name so the schedule keeps its meaning.
      timezone: draft.timezone,
      sessionPolicy: draft.sessionPolicy,
      // Only a per-run conversation can be deleted, so the form never sends the
      // combination that means nothing (see `parse_session_retention`).
      sessionRetention: draft.sessionPolicy === "new" ? draft.sessionRetention : "keep",
      conversationMode: draft.conversationMode,
      enabled: draft.enabled,
    };
    // One message per missing decision, in the form's own order: a task that
    // names no model would run on whatever the app happens to default to, and
    // the form does not offer that choice, so saving one is refused rather
    // than silently reinterpreted. The trigger comes last — with the run-now
    // button gone it is the one decision that must exist before a save, but
    // it sits at the bottom of the form.
    let problem: string | null = null;
    if (!input.name || !input.prompt) {
      problem = t("form.required");
    }
    else if (draft.conversationMode === "workspace" && !input.cwd) {
      problem = t("form.cwdRequired");
    }
    else if (!input.modelId) {
      problem = t("form.modelRequired");
    }
    else if (!input.thinkingLevel) {
      problem = t("form.thinkingRequired");
    }
    else {
      const payload = triggerPayload(draft.trigger);
      if (!payload) {
        problem = t("form.triggerRequired");
      }
      else if (payload.kind === "schedule" && payload.trigger.mode === "once"
        && missingTriggerField(draft.trigger)) {
        problem = t(missingTriggerField(draft.trigger) === "date" ? "form.dateRequired" : "form.timeRequired");
      }
      else {
        input.triggerKind = payload.kind;
        input.trigger = payload.trigger;
      }
    }
    if (problem) {
      setSaveError(problem);
      return;
    }
    try {
      if (selectedId)
        await store.updateTask(selectedId, input);
      else
        await store.createTask(input);
      setEditing(false);
    }
    catch (caught) {
      setSaveError(caught instanceof Error ? caught.message : String(caught));
    }
  }, [draft, selectedId, store, t]);

  /**
   * Remove the task, then drop the dialog. Deleting only marks the row
   * (`deleted_at`): a run already executing carries its own snapshot and
   * finishes into the ledger untouched, so there is nothing to cancel here —
   * the confirm dialog is about losing the schedule, not stopping a run.
   */
  const confirmDelete = useCallback(async () => {
    if (!selected)
      return;
    setDeleteBusy(true);
    try {
      await store.deleteTask(selected.id);
      setDeleting(false);
      setSelectedId(null);
      setEditing(false);
    }
    catch (caught) {
      setSaveError(caught instanceof Error ? caught.message : String(caught));
      setDeleting(false);
    }
    finally {
      setDeleteBusy(false);
    }
  }, [selected, store]);

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header className="flex h-14 shrink-0 items-center gap-3 border-b border-line-soft px-4">
        {!leftPanelExpanded
          ? (
              <Button aria-label={t("showSidebar")} size="xs" variant="ghost" onClick={onToggleLeftPanel}>
                <ChevronLeft className="size-4" />
              </Button>
            )
          : null}
        <h1 className="min-w-0 flex-1 truncate text-sm font-semibold text-ink">{t("title")}</h1>
        <Button leftIcon={<Plus className="size-3.5" />} size="sm" variant="primary" onClick={startCreate}>
          {t("new")}
        </Button>
      </header>

      <div className="flex min-h-0 flex-1">
        <div className="w-72 shrink-0 overflow-y-auto border-r border-line-soft">
          {store.error
            ? <p className="px-4 py-3 text-xs text-danger">{store.error}</p>
            : null}
          {store.tasks.length === 0 && !store.loading
            ? <p className="px-4 py-6 text-xs text-ink-muted">{t("empty")}</p>
            : null}
          {store.tasks.map((task) => {
            // A queued request wins over the last run's status: it is what the
            // user is waiting on after pressing "run now" while the task was
            // busy, and the panel would otherwise look like the button did
            // nothing until the in-flight run ends.
            const status = task.queued
              ? t("status.queued")
              : task.latestRun
                ? t(`status.${task.latestRun.status}`)
                : null;
            return (
              <button
                key={task.id}
                className={`flex w-full flex-col gap-1.5 border-b border-line-soft px-4 py-3 text-left transition-colors hover:bg-surface-subtle ${task.id === selectedId ? "bg-surface-subtle" : ""}`}
                type="button"
                onClick={() => {
                  setSelectedId(task.id);
                  setEditing(false);
                }}
              >
                <span className="flex items-center gap-2">
                  <span className={`size-1.5 shrink-0 rounded-full ${task.enabled ? "bg-accent" : "bg-line"}`} />
                  <span className="min-w-0 flex-1 truncate text-sm text-ink">{task.name}</span>
                  {status
                    ? <span className={`shrink-0 text-xs ${task.queued ? "text-accent" : "text-ink-muted"}`}>{status}</span>
                    : null}
                </span>
                <span className="truncate pl-3.5 text-xs text-ink-muted">
                  {summarizeTrigger(t, task, locale)}
                  {task.nextDueAt ? ` · ${formatEpoch(task.nextDueAt, locale, task.timezone)}` : ""}
                </span>
              </button>
            );
          })}
        </div>

        <div className="min-w-0 flex-1 overflow-y-auto">
          {editing
            ? (
                <TaskForm
                  draft={draft}
                  error={saveError}
                  models={modelOptions}
                  providerNames={providerNames}
                  onCancel={() => setEditing(false)}
                  onChange={patch => setDraft(current => ({ ...current, ...patch }))}
                  onSave={() => void save()}
                />
              )
            : selected
              ? (
                  <TaskDetail
                    deleteBusy={deleteBusy}
                    onDelete={() => {
                      setSaveError(null);
                      setDeleting(true);
                    }}
                    store={store}
                    task={selected}
                    onEdit={() => startEdit(selected)}
                    onOpenThread={onOpenThread}
                  />
                )
              : <p className="px-6 py-10 text-sm text-ink-muted">{t("selectHint")}</p>}
        </div>
      </div>

      <Dialog
        description={t("deleteConfirm", { name: selected?.name ?? "" })}
        footer={(
          <>
            <Button disabled={deleteBusy} onClick={() => setDeleting(false)} type="button" variant="ghost">
              {t("common:cancel")}
            </Button>
            <Button disabled={deleteBusy} onClick={() => void confirmDelete()} type="button" variant="danger">
              {deleteBusy ? t("deleting") : t("common:delete")}
            </Button>
          </>
        )}
        onClose={() => setDeleting(false)}
        open={deleting}
        title={t("deleteTitle")}
      >
        <p className="text-sm leading-5 text-ink-soft">{t("deleteDetail")}</p>
      </Dialog>
    </section>
  );
}

function TaskForm({
  draft,
  error,
  models,
  providerNames,
  onChange,
  onCancel,
  onSave,
}: {
  draft: Draft;
  error: string | null;
  /** Enabled models only — the same list the composer offers. */
  models: AgentModelOption[];
  /** Provider id → display name, so each model option can name its vendor. */
  providerNames: Record<string, string>;
  onChange: (patch: Partial<Draft>) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const { t } = useTranslation("tasks");
  const [browseError, setBrowseError] = useState<string | null>(null);
  const zones = useMemo(() => timezoneOptions(draft.timezone), [draft.timezone]);
  const trigger = draft.trigger;
  const workspaceConversation = draft.conversationMode === "workspace";
  const setTrigger = (patch: Partial<DraftTrigger>) => onChange({ trigger: { ...trigger, ...patch } });
  // A task may already point at a model the user has since disabled. It stays
  // selectable (dropping it would silently rewrite the task on the next save),
  // listed after the enabled ones so the difference is visible.
  const enabledKeys = new Set(models.map(model => `${model.provider}/${model.id}`));
  const pinned = draft.modelId && !enabledKeys.has(draft.modelId) ? draft.modelId : null;
  // Whether the chosen model can think at all. A model that cannot
  // (`reasoning: false`) makes the level meaningless, so the control is
  // disabled rather than left offering a choice that does nothing.
  const supportsThinking = modelSupportsThinking(draft.modelId, models);
  /**
   * Switching models normalises the level the way the composer does. A task can
   * hold a level from an earlier model, and saving it unchanged onto a model
   * that cannot think would pin an effort the model never runs at.
   */
  const chooseModel = (modelId: string) =>
    onChange({
      modelId,
      thinkingLevel: modelSupportsThinking(modelId, models)
        ? normalizeThinkingLevel(draft.thinkingLevel)
        : "off",
    });

  /**
   * Pick the working directory in the OS directory chooser. Cancelling leaves
   * the field alone (the dialog answers null, not an error), and a picker that
   * cannot open at all must not clear a path the user already typed.
   */
  const chooseDirectory = async () => {
    try {
      const picked = await open({ directory: true, multiple: false, defaultPath: draft.cwd || undefined });
      if (typeof picked === "string")
        onChange({ cwd: picked });
    }
    catch (error) {
      setBrowseError(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <div className="space-y-5 px-6 py-5 pb-10">
      <h2 className="text-sm font-semibold text-ink">{t("form.title")}</h2>
      {error ? <p className="text-xs text-danger">{error}</p> : null}
      {browseError ? <p className="text-xs text-danger">{browseError}</p> : null}

      <label className="block space-y-1.5">
        <span className="text-xs text-ink-soft">{t("form.name")}</span>
        <TextInput value={draft.name} onChange={e => onChange({ name: e.target.value })} />
      </label>

      <label className="block space-y-1.5">
        <span className="text-xs text-ink-soft">{t("form.prompt")}</span>
        <textarea
          className="min-h-32 w-full rounded-md border border-line-soft bg-surface px-3 py-2 text-sm text-ink outline-none focus:border-focus focus:ring-2 focus:ring-focus"
          spellCheck={false}
          value={draft.prompt}
          onChange={e => onChange({ prompt: e.target.value })}
        />
      </label>

      {/* The directory belongs to the conversation's type, so it is asked for
          where that choice is made: a workspace conversation *is* its folder,
          while a chat conversation runs in the workspace it carries with it. */}
      <div className="block space-y-1.5">
        <span className="text-xs text-ink-soft">{t("form.conversationMode")}</span>
        <Select value={draft.conversationMode} onChange={e => onChange({ conversationMode: e.target.value })}>
          <option value="chat">{t("form.conversationChat")}</option>
          <option value="workspace">{t("form.conversationWorkspace")}</option>
        </Select>
        <p className="text-xs text-ink-muted">{t("form.conversationModeHint")}</p>
      </div>

      {workspaceConversation
        ? (
            <div className="block space-y-1.5">
              <span className="text-xs text-ink-soft">{t("form.cwd")}</span>
              {/* `min-w-0 flex-1` on the field and `shrink-0` on the button:
                  the input's own width used to push the button past the panel
                  edge, leaving "browse" half cut off. */}
              <div className="flex items-center gap-2">
                <TextInput
                  aria-label={t("form.cwd")}
                  className="min-w-0 flex-1"
                  placeholder={t("form.cwdPlaceholder")}
                  value={draft.cwd}
                  onChange={e => onChange({ cwd: e.target.value })}
                />
                <Button
                  className="shrink-0"
                  leftIcon={<FolderOpen className="size-3.5" />}
                  size="md"
                  variant="secondary"
                  onClick={() => void chooseDirectory()}
                >
                  {t("form.browse")}
                </Button>
              </div>
            </div>
          )
        : null}

      <div className="grid grid-cols-2 gap-3">
        <label className="block space-y-1.5">
          <span className="text-xs text-ink-soft">{t("form.model")}</span>
          <Select value={draft.modelId} onChange={e => chooseModel(e.target.value)}>
            {/* No "default" entry: the model is the user's decision, and an
                empty value only ever means "not chosen yet". */}
            <option value="" disabled>{t("form.modelPlaceholder")}</option>
            {pinned ? <option value={pinned}>{pinned}</option> : null}
            {models.map(model => (
              <option key={`${model.provider}/${model.id}`} value={`${model.provider}/${model.id}`}>
                {/* The vendor goes on every row: two providers often expose the
                    same model name, and a picker that only says the name cannot
                    tell them apart. Same shape as the composer's menu. */}
                {`${model.label || model.id} · ${providerNames[model.provider] ?? model.provider}`}
              </option>
            ))}
          </Select>
          {models.length === 0
            ? <p className="text-xs text-ink-muted">{t("form.modelsEmpty")}</p>
            : null}
        </label>
        <label className="block space-y-1.5">
          <span className="text-xs text-ink-soft">{t("form.thinking")}</span>
          <Select
            value={draft.thinkingLevel}
            disabled={!supportsThinking}
            title={supportsThinking ? undefined : t("agent:composer.thinkingUnsupported")}
            onChange={e => onChange({ thinkingLevel: e.target.value })}
          >
            <option value="" disabled>{t("form.thinkingPlaceholder")}</option>
            {THINKING_LEVELS.map(level => (
              <option key={level} value={level}>{t(`agent:composer.thinkingLevelLabels.${level}`)}</option>
            ))}
          </Select>
          {!supportsThinking
            ? <p className="text-xs text-ink-muted">{t("agent:composer.thinkingUnsupported")}</p>
            : null}
        </label>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <label className="block space-y-1.5">
          <span className="text-xs text-ink-soft">{t("form.session")}</span>
          <Select value={draft.sessionPolicy} onChange={e => onChange({ sessionPolicy: e.target.value })}>
            <option value="new">{t("form.sessionNew")}</option>
            <option value="existing">{t("form.sessionExisting")}</option>
          </Select>
        </label>
        {/* Only a conversation that is opened per run can be thrown away after
            it: deleting a reused one would delete what the next run continues,
            so the choice is not offered there. */}
        {draft.sessionPolicy === "new"
          ? (
              <label className="block space-y-1.5">
                <span className="text-xs text-ink-soft">{t("form.sessionRetention")}</span>
                <Select value={draft.sessionRetention} onChange={e => onChange({ sessionRetention: e.target.value })}>
                  <option value="keep">{t("form.sessionRetentionKeep")}</option>
                  <option value="delete">{t("form.sessionRetentionDelete")}</option>
                </Select>
              </label>
            )
          : null}
      </div>
      {draft.sessionPolicy === "existing"
        ? <p className="text-xs text-ink-muted">{t("form.sessionExistingHint")}</p>
        : null}
      {draft.sessionPolicy === "new" && draft.sessionRetention === "delete"
        ? <p className="text-xs text-ink-muted">{t("form.sessionRetentionHint")}</p>
        : null}

      <fieldset className="space-y-3 rounded-md border border-line-soft p-3">
        <legend className="px-1 text-xs text-ink-soft">{t("form.trigger")}</legend>
        <Select
          value={trigger.mode}
          onChange={e => setTrigger({ mode: e.target.value as TriggerMode })}
        >
          {/* An unchosen trigger is the empty option, required like the model:
              the run-now button is gone, so a task that runs only when the
              trigger fires must have one that can fire. A task that is already
              manual keeps its option below so an edit round-trips. */}
          <option value="" disabled>{t("form.triggerPlaceholder")}</option>
          {trigger.mode === "manual"
            ? <option value="manual">{t("triggerMode.manual")}</option>
            : null}
          <option value="once">{t("triggerMode.once")}</option>
          <option value="interval">{t("triggerMode.interval")}</option>
          <option value="daily">{t("triggerMode.daily")}</option>
          <option value="weekly">{t("triggerMode.weekly")}</option>
          <option value="monthly">{t("triggerMode.monthly")}</option>
          <option value="yearly">{t("triggerMode.yearly")}</option>
        </Select>

        {trigger.mode === "once"
          ? (
              <div className="grid grid-cols-2 gap-3">
                <label className="block space-y-1.5">
                  <span className="text-xs text-ink-soft">{t("form.date")}</span>
                  {/* Native date picker: a placeholder like "2026-12-24" read as
                      a date already chosen, and typed days were easy to fumble.
                      The picker starts empty and reports no value until one is
                      actually picked; the save refuses an empty one. */}
                  <TextInput required type="date" value={trigger.date} onChange={e => setTrigger({ date: e.target.value })} />
                </label>
                <label className="block space-y-1.5">
                  <span className="text-xs text-ink-soft">{t("form.time")}</span>
                  {/* Same for the time: no default minute the user never chose. */}
                  <TextInput required type="time" value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
                </label>
              </div>
            )
          : null}

        {trigger.mode === "interval"
          ? (
              <label className="block space-y-1.5">
                <span className="text-xs text-ink-soft">{t("form.everyMinutes")}</span>
                <TextInput
                  type="number"
                  min={1}
                  value={trigger.everyMinutes}
                  onChange={e => setTrigger({ everyMinutes: Number(e.target.value) || 1 })}
                />
              </label>
            )
          : null}

        {trigger.mode === "daily"
          ? (
              <label className="block space-y-1.5">
                <span className="text-xs text-ink-soft">{t("form.time")}</span>
                <TextInput type="time" value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
              </label>
            )
          : null}

        {trigger.mode === "weekly"
          ? (
              <div className="space-y-2">
                <div className="flex flex-wrap gap-2">
                  {WEEKDAYS.map(day => (
                    <label key={day} className="flex items-center gap-1 text-xs text-ink">
                      <input
                        checked={trigger.days.includes(day)}
                        type="checkbox"
                        onChange={(e) => {
                          const days = e.target.checked
                            ? [...trigger.days, day]
                            : trigger.days.filter(value => value !== day);
                          setTrigger({ days });
                        }}
                      />
                      {t(`weekday.${day}`)}
                    </label>
                  ))}
                </div>
                <label className="block space-y-1.5">
                  <span className="text-xs text-ink-soft">{t("form.time")}</span>
                  <TextInput type="time" value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
                </label>
              </div>
            )
          : null}

        {trigger.mode === "monthly"
          ? (
              <div className="space-y-2">
                <label className="block space-y-1.5">
                  <span className="text-xs text-ink-soft">{t("form.day")}</span>
                  <TextInput
                    type="number"
                    max={31}
                    min={1}
                    value={trigger.day}
                    onChange={e => setTrigger({ day: Number(e.target.value) || 1 })}
                  />
                </label>
                <label className="block space-y-1.5">
                  <span className="text-xs text-ink-soft">{t("form.time")}</span>
                  <TextInput type="time" value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
                </label>
                {/* The short-month rule is user-visible by design. */}
                <p className="text-2xs text-ink-muted">{t("form.shortMonthHint")}</p>
              </div>
            )
          : null}

        {trigger.mode === "yearly"
          ? (
              <div className="space-y-2">
                <div className="grid grid-cols-2 gap-3">
                  <label className="block space-y-1.5">
                    <span className="text-xs text-ink-soft">{t("form.month")}</span>
                    <TextInput
                      type="number"
                      max={12}
                      min={1}
                      value={trigger.month}
                      onChange={e => setTrigger({ month: Number(e.target.value) || 1 })}
                    />
                  </label>
                  <label className="block space-y-1.5">
                    <span className="text-xs text-ink-soft">{t("form.dayOfMonth")}</span>
                    <TextInput
                      type="number"
                      max={31}
                      min={1}
                      value={trigger.day}
                      onChange={e => setTrigger({ day: Number(e.target.value) || 1 })}
                    />
                  </label>
                </div>
                <label className="block space-y-1.5">
                  <span className="text-xs text-ink-soft">{t("form.time")}</span>
                  <TextInput type="time" value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
                </label>
                {/* A clamped day is user-visible by design, the same rule the
                    monthly mode states. */}
                <p className="text-2xs text-ink-muted">{t("form.shortMonthHint")}</p>
              </div>
            )
          : null}

        {/* The zone belongs to the schedule, not to the task: it says which
            clock the times just chosen are read on, so it is asked for inside
            the trigger, after the mode that has times. */}
        {triggerReadsClock(trigger.mode)
          ? (
              <div className="block space-y-1.5">
                <span className="text-xs text-ink-soft">{t("form.timezone")}</span>
                <Select value={draft.timezone} onChange={e => onChange({ timezone: e.target.value })}>
                  <option value="">{t("form.timezoneSystem")}</option>
                  {zones.map(zone => (
                    <option key={zone} value={zone}>{zone}</option>
                  ))}
                </Select>
                {/* The hint says what an unset choice means rather than leaving
                    the user to guess whether it follows the machine or UTC. */}
                <p className="text-xs text-ink-muted">{t("form.timezoneHint")}</p>
              </div>
            )
          : null}
      </fieldset>

      <label className="flex items-center gap-2 text-xs text-ink">
        <input checked={draft.enabled} type="checkbox" onChange={e => onChange({ enabled: e.target.checked })} />
        {t("form.enabled")}
      </label>

      <p className="rounded-md border border-warning-line bg-warning-soft px-3 py-2 text-xs text-warning">
        {t("form.fullPermissionWarning")}
      </p>

      <div className="flex gap-2 pt-1">
        <Button size="sm" variant="primary" onClick={onSave}>{t("form.save")}</Button>
        <Button size="sm" variant="ghost" onClick={onCancel}>{t("form.cancel")}</Button>
      </div>
    </div>
  );
}

function TaskDetail({
  deleteBusy,
  onDelete,
  task,
  store,
  onEdit,
  onOpenThread,
}: {
  /** The delete dialog in the parent is working; the row button waits. */
  deleteBusy: boolean;
  /** Ask the parent to open the delete confirmation dialog. */
  onDelete: () => void;
  task: TaskView;
  store: ReturnType<typeof useTasks>;
  onEdit: () => void;
  onOpenThread: (threadId: string) => void;
}) {
  const { t, i18n } = useTranslation("tasks");
  const locale = i18n.language;
  // Closed by default: the page opens on what the task did, not on how it is
  // configured (see the section's comment).
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [runs, setRuns] = useState<Awaited<ReturnType<typeof store.listRuns>>>([]);

  const refresh = useCallback(async () => {
    setRuns(await store.listRuns(task.id));
  }, [store, task.id]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The runs of a task change while it is on screen (a run finishing). The host
  // announces those with the same "threads-updated" event the sidebar listens
  // to, so the detail re-reads its list rather than showing a ledger frozen at
  // open time.
  useTauriEvent("threads-updated", () => {
    void refresh();
  });

  return (
    <div className="space-y-6 px-6 py-5 pb-10">
      <div className="flex items-start gap-2.5">
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-sm font-semibold text-ink">{task.name}</h2>
          <p className="mt-1 text-xs text-ink-muted">
            {summarizeTrigger(t, task, locale)}
            {task.nextDueAt ? ` · ${t("nextDue")} ${formatEpoch(task.nextDueAt, locale, task.timezone)}` : ""}
          </p>
        </div>
        <Button size="sm" variant="secondary" onClick={() => onEdit()}>{t("edit")}</Button>
        <Button
          size="sm"
          variant={task.enabled ? "ghost" : "secondary"}
          onClick={() => void store.setEnabled(task.id, !task.enabled).then(refresh)}
        >
          {task.enabled ? t("disable") : t("enable")}
        </Button>
        <Button
          aria-label={t("delete")}
          disabled={deleteBusy}
          size="sm"
          variant="danger-soft"
          onClick={onDelete}
        >
          <Trash2 className="size-3.5" />
        </Button>
      </div>

      {/* The way into the definition sits above the history, not below it: a
          task that has run many times would otherwise push a one-line control
          far out of reach. The definition itself is still folded — you come
          here to see what happened, and the form is one click away. */}
      <section className="space-y-2">
        <button
          aria-expanded={settingsOpen}
          className="flex w-full cursor-pointer items-center gap-1.5 pl-3.5 text-left text-xs font-medium text-ink-soft hover:text-ink"
          onClick={() => setSettingsOpen(open => !open)}
          type="button"
        >
          {settingsOpen
            ? <ChevronDown className="size-3.5" />
            : <ChevronRight className="size-3.5" />}
          {t("settings")}
        </button>
        {settingsOpen
          ? (
              <div className="space-y-6 pt-1">
                <section className="space-y-2">
                  {/* The disclosure button above *is* this block's heading;
                      repeating it here would read as a second, nested
                      "Settings". */}
                  <dl className="grid grid-cols-[minmax(0,8rem)_minmax(0,1fr)] gap-x-4 gap-y-2.5 rounded-md border border-line-soft bg-surface-subtle px-3.5 py-3.5 text-xs">
                    {[
                      // What the run will actually be: the model and thinking level are
                      // per-task, so a detail page that omits them cannot answer "what
                      // does this run on?".
                      [t("colModel"), task.modelId ?? t("modelDefault")],
                      // An unset level is not "unknown": the agent applies its own
                      // default, so the row names that value rather than saying
                      // "Default" and leaving the user to guess what it means. The
                      // value comes from the constant the composer uses, so it
                      // cannot drift from what a session actually gets.
                      [
                        t("colThinking"),
                        task.thinkingLevel
                          ? t(`agent:composer.thinkingLevelLabels.${task.thinkingLevel}`)
                          : `${t("thinkingDefault")} (${t(`agent:composer.thinkingLevelLabels.${defaultThinkingLevel}`)})`,
                      ],
                      // A chat task may name no directory at all: it runs in its own
                      // conversation's workspace, and an empty row would read as "missing".
                      [t("colCwd"), task.cwd.trim() || (task.conversationMode === "chat" ? t("cwdChatWorkspace") : "—")],
                      // The trigger times are read on this clock: a detail page
                      // that omits it cannot explain why 09:00 printed
                      // differently than the user's own morning.
                      [t("colTimezone"), task.timezone || t("form.timezoneSystem")],
                      [t("colConversation"), t(task.conversationMode === "chat" ? "form.conversationChat" : "form.conversationWorkspace")],
                      [t("form.session"), t(task.sessionPolicy === "existing" ? "form.sessionExisting" : "form.sessionNew")],
                      [
                        t("form.sessionRetention"),
                        t(task.sessionPolicy === "new" && task.sessionRetention === "delete"
                          ? "form.sessionRetentionDelete"
                          : "form.sessionRetentionKeep"),
                      ],
                    ].map(([label, value]) => (
                      <Fragment key={label}>
                        <dt className="text-ink-soft">{label}</dt>
                        <dd className="min-w-0 wrap-break-word text-ink">{value}</dd>
                      </Fragment>
                    ))}
                  </dl>
                </section>

                <section className="space-y-2">
                  <h3 className="pl-3.5 text-xs font-medium text-ink-soft">{t("prompt")}</h3>
                  <pre className="max-h-48 overflow-auto whitespace-pre-wrap rounded-md border border-line-soft bg-surface-subtle px-3.5 py-3.5 text-xs leading-relaxed text-ink">
                    {task.prompt}
                  </pre>
                  <p className="pl-3.5 text-xs text-ink-muted">
                    {t("promptVersion", { version: task.promptVersion })}
                  </p>
                </section>

              </div>
            )
          : null}
      </section>

      <section className="space-y-3">
        <h3 className="pl-3.5 text-xs font-medium text-ink-soft">{t("runs")}</h3>
        {runs.length === 0
          ? <p className="pl-3.5 text-xs text-ink-muted">{t("runsNone")}</p>
          : (
              // One card per run, with space between the cards: consecutive runs
              // used to share a single box separated by hairlines, and a run that
              // ends in a paragraph of summary ran into the next one's header.
              <ul className="space-y-3">
                {runs.map(run => (
                  <li key={run.id} className="overflow-hidden rounded-md border border-line-soft">
                    <div className="flex items-center gap-2 border-b border-line-soft bg-surface-subtle px-3.5 py-2.5 text-xs">
                      <span className="font-medium text-ink">{t(`status.${run.status}`)}</span>
                      <span aria-hidden className="text-line">·</span>
                      <span className="text-ink-muted">{t(`kind.${run.kind}`)}</span>
                      {run.promptVersion != null
                        ? (
                            <>
                              <span aria-hidden className="text-line">·</span>
                              <span className="text-ink-muted">
                                {t("runPromptVersion", { version: run.promptVersion })}
                              </span>
                            </>
                          )
                        : null}
                      <span aria-hidden className="text-line">·</span>
                      <span className="min-w-0 flex-1 truncate text-ink-muted">
                        {run.startedAt ? formatEpoch(run.startedAt, locale, task.timezone) : ""}
                      </span>
                      {run.threadId
                        ? (
                            <Button size="xs" variant="ghost" onClick={() => onOpenThread(run.threadId as string)}>
                              {t("openConversation")}
                            </Button>
                          )
                        : null}
                    </div>
                    <div className="space-y-3 px-3.5 py-3">
                      {/* The summary is the model's markdown answer, not a plain
                          paragraph: headings, lists and tables a task produced
                          render formatted, the way the conversation shows them.
                          An error stays plain text — it is a message from the
                          host, not the model's prose. */}
                      {run.errorMessage
                        ? <p className="text-xs leading-relaxed text-danger">{run.errorMessage}</p>
                        : run.resultSummary
                          ? (
                              <div className="text-xs leading-relaxed">
                                <MarkdownContent content={run.resultSummary} />
                              </div>
                            )
                          : <p className="text-xs leading-relaxed text-ink-soft">{t("runNoSummary")}</p>}
                      {/* The run's conversation is gone by design: say so, so the
                          missing "open conversation" reads as intended rather
                          than broken, and point at where the output is. */}
                      {run.sessionDeleted
                        ? <p className="text-2xs text-ink-muted">{t("runSessionDeleted")}</p>
                        : null}
                    </div>
                  </li>
                ))}
              </ul>
            )}
      </section>

    </div>
  );
}
