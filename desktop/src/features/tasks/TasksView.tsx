import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { TaskInput, TaskView } from "./useTasks";
import { open } from "@tauri-apps/plugin-dialog";
import { ChevronLeft, FolderOpen, Play, Plus, RotateCcw, Trash2 } from "lucide-react";
import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";
import { formatDateTime } from "../../lib/date";
import { useTauriEvent } from "../../lib/useTauriEvent";
import { useTasks } from "./useTasks";

/** Task times are epoch milliseconds; the date helpers take ISO strings. */
function formatEpoch(ms: number | null, locale: string): string {
  return ms == null ? "" : formatDateTime(new Date(ms).toISOString(), locale);
}

const WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;

/** Trigger modes the form can edit (mirrors the kernel's schedule modes). */
type TriggerMode = "manual" | "once" | "interval" | "daily" | "weekly" | "monthly";

interface DraftTrigger {
  mode: TriggerMode;
  date: string;
  time: string;
  everyMinutes: number;
  days: string[];
  day: number;
}

const emptyTrigger: DraftTrigger = {
  mode: "manual",
  date: "",
  time: "09:00",
  everyMinutes: 60,
  days: ["mon"],
  day: 1,
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
  sessionPolicy: "new",
  conversationMode: "chat",
  reflection: "ask",
  enabled: true,
  trigger: { ...emptyTrigger },
};

/** Thinking levels the agent accepts, in the composer's order. */
const THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"] as const;

type Draft = typeof emptyDraft;

function triggerFromTask(task: TaskView): DraftTrigger {
  if (task.triggerKind !== "schedule")
    return { ...emptyTrigger };
  const trigger = task.trigger as Record<string, unknown>;
  const mode = String(trigger.mode ?? "daily") as TriggerMode;
  return {
    mode,
    date: String(trigger.date ?? ""),
    time: String(trigger.time ?? "09:00"),
    everyMinutes: Number(trigger.every_minutes ?? 60),
    days: Array.isArray(trigger.days) ? (trigger.days as string[]) : ["mon"],
    day: Number(trigger.day ?? 1),
  };
}

function triggerPayload(draft: DraftTrigger): { kind: string; trigger: Record<string, unknown> } {
  if (draft.mode === "manual")
    return { kind: "manual", trigger: {} };
  switch (draft.mode) {
    case "once":
      return { kind: "schedule", trigger: { mode: "once", date: draft.date, time: draft.time } };
    case "interval":
      return { kind: "schedule", trigger: { mode: "interval", every_minutes: draft.everyMinutes } };
    case "weekly":
      return { kind: "schedule", trigger: { mode: "weekly", days: draft.days, time: draft.time } };
    case "monthly":
      return { kind: "schedule", trigger: { mode: "monthly", day: draft.day, time: draft.time } };
    default:
      return { kind: "schedule", trigger: { mode: "daily", time: draft.time } };
  }
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
    default:
      return `${t("trigger.daily")} ${time}`;
  }
}

/** Left-nav "Tasks" panel: list, edit, runs, revisions. */
export function TasksView({
  leftPanelExpanded,
  modelOptions,
  onToggleLeftPanel,
  onOpenThread,
}: {
  leftPanelExpanded: boolean;
  /** Models the user has enabled (Settings → Models), the same list the composer offers. */
  modelOptions: AgentModelOption[];
  onToggleLeftPanel: () => void;
  onOpenThread: (threadId: string) => void;
}) {
  const { t, i18n } = useTranslation("tasks");
  const locale = i18n.language;
  const store = useTasks();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Draft>({ ...emptyDraft });
  const [saveError, setSaveError] = useState<string | null>(null);

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
      sessionPolicy: task.sessionPolicy,
      conversationMode: task.conversationMode,
      reflection: task.reflection,
      enabled: task.enabled,
      trigger: triggerFromTask(task),
    });
  }, []);

  const save = useCallback(async () => {
    const { kind, trigger } = triggerPayload(draft.trigger);
    const input: TaskInput = {
      name: draft.name.trim(),
      prompt: draft.prompt,
      cwd: draft.cwd.trim(),
      modelId: draft.modelId || null,
      thinkingLevel: draft.thinkingLevel || null,
      sessionPolicy: draft.sessionPolicy,
      conversationMode: draft.conversationMode,
      reflection: draft.reflection,
      triggerKind: kind,
      trigger,
      depJoin: selected?.depJoin ?? "all",
      enabled: draft.enabled,
    };
    // One message per missing decision: a task that names no model would run on
    // whatever the app happens to default to, and the form does not offer that
    // choice, so saving one is refused rather than silently reinterpreted.
    const problem = !input.name || !input.prompt
      ? t("form.required")
      : draft.conversationMode === "workspace" && !input.cwd
        ? t("form.cwdRequired")
        : !input.modelId
            ? t("form.modelRequired")
            : !input.thinkingLevel
                ? t("form.thinkingRequired")
                : null;
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
  }, [draft, selected, selectedId, store, t]);

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
                  {task.nextDueAt ? ` · ${formatEpoch(task.nextDueAt, locale)}` : ""}
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
                  onCancel={() => setEditing(false)}
                  onChange={patch => setDraft(current => ({ ...current, ...patch }))}
                  onSave={() => void save()}
                />
              )
            : selected
              ? (
                  <TaskDetail
                    store={store}
                    task={selected}
                    onEdit={() => startEdit(selected)}
                    onOpenThread={onOpenThread}
                  />
                )
              : <p className="px-6 py-10 text-sm text-ink-muted">{t("selectHint")}</p>}
        </div>
      </div>
    </section>
  );
}

function TaskForm({
  draft,
  error,
  models,
  onChange,
  onCancel,
  onSave,
}: {
  draft: Draft;
  error: string | null;
  /** Enabled models only — the same list the composer offers. */
  models: AgentModelOption[];
  onChange: (patch: Partial<Draft>) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const { t } = useTranslation("tasks");
  const [browseError, setBrowseError] = useState<string | null>(null);
  const trigger = draft.trigger;
  const workspaceConversation = draft.conversationMode === "workspace";
  const setTrigger = (patch: Partial<DraftTrigger>) => onChange({ trigger: { ...trigger, ...patch } });
  // A task may already point at a model the user has since disabled. It stays
  // selectable (dropping it would silently rewrite the task on the next save),
  // listed after the enabled ones so the difference is visible.
  const enabledKeys = new Set(models.map(model => `${model.provider}/${model.id}`));
  const pinned = draft.modelId && !enabledKeys.has(draft.modelId) ? draft.modelId : null;

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
          <Select value={draft.modelId} onChange={e => onChange({ modelId: e.target.value })}>
            {/* No "default" entry: the model is the user's decision, and an
                empty value only ever means "not chosen yet". */}
            <option value="" disabled>{t("form.modelPlaceholder")}</option>
            {pinned ? <option value={pinned}>{pinned}</option> : null}
            {models.map(model => (
              <option key={`${model.provider}/${model.id}`} value={`${model.provider}/${model.id}`}>
                {model.label || model.id}
              </option>
            ))}
          </Select>
          {models.length === 0
            ? <p className="text-xs text-ink-muted">{t("form.modelsEmpty")}</p>
            : null}
        </label>
        <label className="block space-y-1.5">
          <span className="text-xs text-ink-soft">{t("form.thinking")}</span>
          <Select value={draft.thinkingLevel} onChange={e => onChange({ thinkingLevel: e.target.value })}>
            <option value="" disabled>{t("form.thinkingPlaceholder")}</option>
            {THINKING_LEVELS.map(level => (
              <option key={level} value={level}>{t(`agent:composer.thinkingLevelLabels.${level}`)}</option>
            ))}
          </Select>
        </label>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <label className="block space-y-1.5">
          <span className="text-xs text-ink-soft">{t("form.reflection")}</span>
          <Select value={draft.reflection} onChange={e => onChange({ reflection: e.target.value })}>
            <option value="off">{t("form.reflectionOff")}</option>
            <option value="ask">{t("form.reflectionAsk")}</option>
            <option value="auto">{t("form.reflectionAuto")}</option>
          </Select>
        </label>
        <label className="block space-y-1.5">
          <span className="text-xs text-ink-soft">{t("form.session")}</span>
          <Select value={draft.sessionPolicy} onChange={e => onChange({ sessionPolicy: e.target.value })}>
            <option value="new">{t("form.sessionNew")}</option>
            <option value="existing">{t("form.sessionExisting")}</option>
          </Select>
        </label>
      </div>
      {draft.sessionPolicy === "existing"
        ? <p className="text-xs text-ink-muted">{t("form.sessionExistingHint")}</p>
        : null}

      <fieldset className="space-y-3 rounded-md border border-line-soft p-3">
        <legend className="px-1 text-xs text-ink-soft">{t("form.trigger")}</legend>
        <Select value={trigger.mode} onChange={e => setTrigger({ mode: e.target.value as TriggerMode })}>
          <option value="manual">{t("triggerMode.manual")}</option>
          <option value="once">{t("triggerMode.once")}</option>
          <option value="interval">{t("triggerMode.interval")}</option>
          <option value="daily">{t("triggerMode.daily")}</option>
          <option value="weekly">{t("triggerMode.weekly")}</option>
          <option value="monthly">{t("triggerMode.monthly")}</option>
        </Select>

        {trigger.mode === "once"
          ? (
              <div className="grid grid-cols-2 gap-3">
                <label className="block space-y-1.5">
                  <span className="text-xs text-ink-soft">{t("form.date")}</span>
                  <TextInput placeholder="2026-12-24" value={trigger.date} onChange={e => setTrigger({ date: e.target.value })} />
                </label>
                <label className="block space-y-1.5">
                  <span className="text-xs text-ink-soft">{t("form.time")}</span>
                  <TextInput value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
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
                <TextInput value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
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
                  <TextInput value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
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
                  <TextInput value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
                </label>
                {/* The short-month rule is user-visible by design. */}
                <p className="text-[11px] text-ink-muted">{t("form.shortMonthHint")}</p>
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
  task,
  store,
  onEdit,
  onOpenThread,
}: {
  task: TaskView;
  store: ReturnType<typeof useTasks>;
  onEdit: () => void;
  onOpenThread: (threadId: string) => void;
}) {
  const { t, i18n } = useTranslation("tasks");
  const locale = i18n.language;
  const [runs, setRuns] = useState<Awaited<ReturnType<typeof store.listRuns>>>([]);
  const [revisions, setRevisions] = useState<Awaited<ReturnType<typeof store.listRevisions>>>([]);
  const [deps, setDeps] = useState<Awaited<ReturnType<typeof store.listDeps>>>([]);

  const refresh = useCallback(async () => {
    const [nextRuns, nextRevisions, nextDeps] = await Promise.all([
      store.listRuns(task.id),
      store.listRevisions(task.id),
      store.listDeps(task.id),
    ]);
    setRuns(nextRuns);
    setRevisions(nextRevisions);
    setDeps(nextDeps);
  }, [store, task.id]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The runs and versions of a task change while it is on screen (a run
  // finishing, a prompt version being applied). The host announces those with
  // the same "threads-updated" event the sidebar listens to, so the detail
  // re-reads its three lists rather than showing a ledger frozen at open time.
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
            {task.nextDueAt ? ` · ${t("nextDue")} ${formatEpoch(task.nextDueAt, locale)}` : ""}
          </p>
        </div>
        <Button leftIcon={<Play className="size-3.5" />} size="sm" variant="secondary" onClick={() => void store.runNow(task.id).then(refresh)}>
          {t("runNow")}
        </Button>
        <Button size="sm" variant="secondary" onClick={onEdit}>{t("edit")}</Button>
        <Button
          size="sm"
          variant={task.enabled ? "ghost" : "secondary"}
          onClick={() => void store.setEnabled(task.id, !task.enabled).then(refresh)}
        >
          {task.enabled ? t("disable") : t("enable")}
        </Button>
        <Button
          aria-label={t("delete")}
          size="sm"
          variant="danger-soft"
          onClick={() => {
            if (window.confirm(t("deleteConfirm", { name: task.name })))
              void store.deleteTask(task.id);
          }}
        >
          <Trash2 className="size-3.5" />
        </Button>
      </div>

      <section className="space-y-2">
        <h3 className="pl-3.5 text-xs font-medium text-ink-soft">{t("settings")}</h3>
        <dl className="grid grid-cols-[minmax(0,8rem)_minmax(0,1fr)] gap-x-4 gap-y-2.5 rounded-md border border-line-soft bg-surface-subtle px-3.5 py-3.5 text-xs">
          {[
            // What the run will actually be: the model and thinking level are
            // per-task, so a detail page that omits them cannot answer "what
            // does this run on?".
            [t("colModel"), task.modelId ?? t("modelDefault")],
            [t("colThinking"), task.thinkingLevel ? t(`agent:composer.thinkingLevelLabels.${task.thinkingLevel}`) : t("thinkingDefault")],
            // A chat task may name no directory at all: it runs in its own
            // conversation's workspace, and an empty row would read as "missing".
            [t("colCwd"), task.cwd.trim() || t("cwdChatWorkspace")],
            [t("colConversation"), t(task.conversationMode === "chat" ? "form.conversationChat" : "form.conversationWorkspace")],
            [t("form.session"), t(task.sessionPolicy === "existing" ? "form.sessionExisting" : "form.sessionNew")],
            [t("colReflection"), t(`form.reflection${task.reflection === "off" ? "Off" : task.reflection === "auto" ? "Auto" : "Ask"}`)],
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

      <section className="space-y-2">
        <h3 className="pl-3.5 text-xs font-medium text-ink-soft">{t("deps")}</h3>
        {deps.length === 0
          ? <p className="pl-3.5 text-xs text-ink-muted">{t("depsNone")}</p>
          : (
              <ul className="divide-y divide-line-soft overflow-hidden rounded-md border border-line-soft">
                {deps.map(dep => (
                  <li key={dep.upstreamTaskId} className="space-y-1 px-3.5 py-3">
                    <div className="flex items-center gap-2 text-xs">
                      <span className="min-w-0 flex-1 truncate text-ink">{dep.upstreamName}</span>
                      <span className={dep.satisfied ? "text-accent" : "text-ink-muted"}>
                        {dep.satisfied ? t("depsReady") : t("depsWaiting")}
                      </span>
                    </div>
                    <p className="text-[11px] text-ink-muted">
                      {t(`on.${dep.on}`)}
                      {" · "}
                      {t(`join.${task.depJoin}`)}
                    </p>
                  </li>
                ))}
              </ul>
            )}
      </section>

      <section className="space-y-3">
        <h3 className="pl-3.5 text-xs font-medium text-ink-soft">{t("runs")}</h3>
        {runs.length === 0
          ? <p className="pl-3.5 text-xs text-ink-muted">{t("runsNone")}</p>
          : (
              <ul className="divide-y divide-line-soft overflow-hidden rounded-md border border-line-soft">
                {runs.map(run => (
                  // Two lines per run rather than fixed columns: a CJK status and
                  // an English one are different widths, and columns padded to the
                  // wider script leave the other one visibly misaligned.
                  <li key={run.id} className="space-y-1.5 px-3.5 py-3">
                    <div className="flex items-center gap-2 text-xs">
                      <span className="text-ink-muted">{t(`kind.${run.kind}`)}</span>
                      <span aria-hidden className="text-line">·</span>
                      <span className="text-ink">{t(`status.${run.status}`)}</span>
                      <span aria-hidden className="text-line">·</span>
                      <span className="min-w-0 flex-1 truncate text-ink-muted">
                        {run.startedAt ? formatEpoch(run.startedAt, locale) : ""}
                      </span>
                      {run.threadId
                        ? (
                            <Button size="xs" variant="ghost" onClick={() => onOpenThread(run.threadId as string)}>
                              {t("openConversation")}
                            </Button>
                          )
                        : null}
                    </div>
                    <p className="text-xs leading-relaxed text-ink-soft">
                      {run.errorMessage ?? run.resultSummary ?? t("runNoSummary")}
                    </p>
                  </li>
                ))}
              </ul>
            )}
      </section>

      <section className="space-y-3">
        <h3 className="pl-3.5 text-xs font-medium text-ink-soft">{t("revisions")}</h3>
        {revisions.length === 0
          ? <p className="pl-3.5 text-xs text-ink-muted">{t("revisionsNone")}</p>
          : (
              <ul className="divide-y divide-line-soft overflow-hidden rounded-md border border-line-soft">
                {revisions.map(revision => (
                  // Version rows follow the same two-line shape as runs and
                  // dependencies: a fixed column would drift as soon as one
                  // locale's source label is wider than the other's.
                  <li key={revision.id} className="space-y-1.5 px-3.5 py-3">
                    <div className="flex items-center gap-2 text-xs">
                      <span className="text-ink-muted">
                        v
                        {revision.version}
                      </span>
                      <span aria-hidden className="text-line">·</span>
                      <span className="text-ink">{t(`source.${revision.source}`)}</span>
                      <span className="min-w-0 flex-1" />
                      <Button
                        leftIcon={<RotateCcw className="size-3" />}
                        size="xs"
                        variant="ghost"
                        onClick={() => void store.applyRevision(task.id, revision.id).then(refresh)}
                      >
                        {t("apply")}
                      </Button>
                    </div>
                    <p className="text-[11px] text-ink-muted">
                      {revision.reason ?? t("revisionsNoReason")}
                    </p>
                  </li>
                ))}
              </ul>
            )}
      </section>
    </div>
  );
}
