import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { TaskBackend } from "./taskBackend";
import type { TaskDepView, TaskInput, TaskView } from "./useTasks";
import { open } from "@tauri-apps/plugin-dialog";
import { ChevronDown, ChevronLeft, ChevronRight, FolderOpen, Play, Plus, RotateCcw, Trash2 } from "lucide-react";
import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";
import { defaultThinkingLevel, modelSupportsThinking, normalizeThinkingLevel } from "../../integrations/agent/agentClient";
import { formatDateTime } from "../../lib/date";
import { useTauriEvent } from "../../lib/useTauriEvent";
import { useTasks } from "./useTasks";

/** Task times are epoch milliseconds; the date helpers take ISO strings. */
function formatEpoch(ms: number | null, locale: string): string {
  return ms == null ? "" : formatDateTime(new Date(ms).toISOString(), locale);
}

const WEEKDAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] as const;

/** Trigger modes the form can edit (mirrors the kernel's schedule modes). */
type TriggerMode = "manual" | "dependency" | "once" | "interval" | "daily" | "weekly" | "monthly" | "yearly";

interface DraftTrigger {
  mode: TriggerMode;
  date: string;
  time: string;
  everyMinutes: number;
  days: string[];
  day: number;
  month: number;
}

const emptyTrigger: DraftTrigger = {
  mode: "manual",
  date: "",
  time: "09:00",
  everyMinutes: 60,
  days: ["mon"],
  day: 1,
  month: 12,
};

/**
 * One dependency edge as the editor holds it: the upstream, the label to show
 * for it, and the condition it fires on. The label is carried because an edge's
 * upstream is not necessarily in the candidate list the editor was handed (a
 * task filtered out of it, or simply not loaded yet) — and a row that shows a
 * bare `tsk_…` id tells the user nothing.
 */
interface DraftDep {
  upstreamTaskId: string;
  name: string;
  on: string;
}

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
  sessionRetention: "keep",
  conversationMode: "chat",
  enabled: true,
  trigger: { ...emptyTrigger },
  // The dependencies the form wants, as a whole set: saving reconciles the
  // stored edges against it (see `useTasks.saveDeps`). A new task starts with
  // none — it runs on its own trigger until the user says otherwise.
  deps: [] as DraftDep[],
  depJoin: "all",
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
  if (task.triggerKind !== "schedule") {
    // A task that is started by its upstreams reads as "dependency", not as
    // "manual": nobody runs it by hand, and the schedule fields are not the
    // thing that fires it.
    if (task.depCount > 0)
      return { ...emptyTrigger, mode: "dependency" };
    return { ...emptyTrigger };
  }
  const trigger = task.trigger as Record<string, unknown>;
  const mode = String(trigger.mode ?? "daily") as TriggerMode;
  return {
    mode,
    date: String(trigger.date ?? ""),
    time: String(trigger.time ?? "09:00"),
    everyMinutes: Number(trigger.every_minutes ?? 60),
    days: Array.isArray(trigger.days) ? (trigger.days as string[]) : ["mon"],
    day: Number(trigger.day ?? 1),
    month: Number(trigger.month ?? 12),
  };
}

function triggerPayload(draft: DraftTrigger): { kind: string; trigger: Record<string, unknown> } {
  // `dependency` is a manual task in the store: the edges are what starts it,
  // and they live in their own table (see `saveDeps`).
  if (draft.mode === "manual" || draft.mode === "dependency")
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
    return task.depCount > 0 ? t("trigger.dependency") : t("trigger.manual");
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

/** Left-nav "Tasks" panel: list, edit, runs, revisions. */
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
  const store = useTasks(backend);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<Draft>({ ...emptyDraft });
  const [saveError, setSaveError] = useState<string | null>(null);

  const selected = useMemo(
    () => store.tasks.find(task => task.id === selectedId) ?? null,
    [store.tasks, selectedId],
  );
  // The tasks a dependency can point at: every other live task. A cycle (this
  // task depending on something that already waits on it) is refused by the
  // backend, which sees the whole graph; the editor would have to fetch every
  // task's edges to know better than that.
  const depCandidates = useMemo(
    () => store.tasks.filter(task => task.id !== selectedId).map(task => ({ id: task.id, name: task.name })),
    [store.tasks, selectedId],
  );

  const startCreate = useCallback(() => {
    setSelectedId(null);
    setEditing(true);
    setSaveError(null);
    setDraft({ ...emptyDraft });
  }, []);

  /**
   * Open the editor on a stored task. `deps` comes from the detail view, which
   * has already read them: the editor works on the whole dependency set, so it
   * needs what is there before the user changes anything.
   */
  const startEdit = useCallback((task: TaskView, deps: TaskDepView[]) => {
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
      sessionRetention: task.sessionRetention,
      conversationMode: task.conversationMode,
      enabled: task.enabled,
      trigger: triggerFromTask(task),
      deps: deps.map(dep => ({
        upstreamTaskId: dep.upstreamTaskId,
        name: dep.upstreamName,
        on: dep.on,
      })),
      depJoin: task.depJoin,
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
      // Only a per-run conversation can be deleted, so the form never sends the
      // combination that means nothing (see `parse_session_retention`).
      sessionRetention: draft.sessionPolicy === "new" ? draft.sessionRetention : "keep",
      conversationMode: draft.conversationMode,
      triggerKind: kind,
      trigger,
      depJoin: draft.depJoin,
      enabled: draft.enabled,
    };
    // One message per missing decision: a task that names no model would run on
    // whatever the app happens to default to, and the form does not offer that
    // choice, so saving one is refused rather than silently reinterpreted.
    let problem: string | null = null;
    if (!input.name || !input.prompt)
      problem = t("form.required");
    else if (draft.conversationMode === "workspace" && !input.cwd)
      problem = t("form.cwdRequired");
    else if (!input.modelId)
      problem = t("form.modelRequired");
    else if (!input.thinkingLevel)
      problem = t("form.thinkingRequired");
    if (problem) {
      setSaveError(problem);
      return;
    }
    try {
      let taskId = selectedId;
      if (taskId)
        await store.updateTask(taskId, input);
      else
        taskId = (await store.createTask(input)).id;
      try {
        await store.saveDeps(
          taskId,
          draft.deps.map(dep => ({ upstreamTaskId: dep.upstreamTaskId, on: dep.on })),
        );
      }
      catch (caught) {
        // The task itself is stored; only its edges failed (a cycle, say). Say
        // so and stay in the editor, switched onto update — a retry must not
        // create a second task.
        setSelectedId(taskId);
        setSaveError(caught instanceof Error ? caught.message : String(caught));
        return;
      }
      setEditing(false);
    }
    catch (caught) {
      setSaveError(caught instanceof Error ? caught.message : String(caught));
    }
  }, [draft, selectedId, store, t]);

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
                  candidates={depCandidates}
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
                    onEdit={deps => startEdit(selected, deps)}
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
  candidates,
  models,
  onChange,
  onCancel,
  onSave,
}: {
  draft: Draft;
  error: string | null;
  /** Other live tasks: the upstreams a dependency can point at. */
  candidates: Array<{ id: string; name: string }>;
  /** Enabled models only — the same list the composer offers. */
  models: AgentModelOption[];
  onChange: (patch: Partial<Draft>) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const { t } = useTranslation("tasks");
  const [browseError, setBrowseError] = useState<string | null>(null);
  // The upstream the user has picked but not added yet.
  const [pendingUpstream, setPendingUpstream] = useState("");
  const trigger = draft.trigger;
  const workspaceConversation = draft.conversationMode === "workspace";
  // The dependency editor is part of the trigger choice, and stays visible for a
  // task that already has upstreams (even next to a schedule of its own).
  const depMode = trigger.mode === "dependency" || draft.deps.length > 0;
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
  const nameOf = (dep: DraftDep) => dep.name || dep.upstreamTaskId;
  const addable = candidates.filter(
    candidate => !draft.deps.some(dep => dep.upstreamTaskId === candidate.id),
  );
  const setDepOn = (upstreamTaskId: string, on: string) =>
    onChange({
      deps: draft.deps.map(dep => (dep.upstreamTaskId === upstreamTaskId ? { ...dep, on } : dep)),
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
        <Select value={trigger.mode} onChange={e => setTrigger({ mode: e.target.value as TriggerMode })}>
          <option value="manual">{t("triggerMode.manual")}</option>
          <option value="dependency">{t("triggerMode.dependency")}</option>
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

        {/* The dependency editor appears with the choice that needs it — and,
            for a task that already has upstreams next to a schedule of its own
            (set from the CLI), it stays visible: an edge nobody can see is an
            edge nobody can remove. */}
        {depMode
          ? (
              <div className="space-y-3 border-t border-line-soft pt-3">
                {trigger.mode !== "dependency"
                  ? <p className="text-xs text-ink-muted">{t("form.depsAlongsideSchedule")}</p>
                  : null}

                {draft.deps.length === 0
                  ? <p className="text-xs text-ink-muted">{t("form.depsNone")}</p>
                  : (
                      <ul className="space-y-2">
                        {draft.deps.map(dep => (
                          <li key={dep.upstreamTaskId} className="flex items-center gap-2">
                            <span className="min-w-0 flex-1 truncate text-xs text-ink" title={nameOf(dep)}>
                              {nameOf(dep)}
                            </span>
                            <Select
                              aria-label={t("form.depCondition", { name: nameOf(dep) })}
                              className="w-28 shrink-0"
                              value={dep.on}
                              onChange={e => setDepOn(dep.upstreamTaskId, e.target.value)}
                            >
                              <option value="success">{t("on.success")}</option>
                              <option value="failure">{t("on.failure")}</option>
                              <option value="completed">{t("on.completed")}</option>
                            </Select>
                            <Button
                              aria-label={t("form.depRemove", { name: nameOf(dep) })}
                              className="shrink-0"
                              size="xs"
                              variant="ghost"
                              onClick={() => onChange({
                                deps: draft.deps.filter(item => item.upstreamTaskId !== dep.upstreamTaskId),
                              })}
                            >
                              {t("form.depRemoveShort")}
                            </Button>
                          </li>
                        ))}
                      </ul>
                    )}

                <div className="flex items-center gap-2">
                  <Select
                    aria-label={t("form.depAdd")}
                    className="min-w-0 flex-1"
                    value={pendingUpstream}
                    onChange={e => setPendingUpstream(e.target.value)}
                  >
                    <option value="">{t("form.depAddPlaceholder")}</option>
                    {addable.map(candidate => (
                      <option key={candidate.id} value={candidate.id}>{candidate.name}</option>
                    ))}
                  </Select>
                  <Button
                    className="shrink-0"
                    disabled={!pendingUpstream}
                    size="md"
                    variant="secondary"
                    onClick={() => {
                      // A new edge waits for the upstream to finish
                      // successfully: the common case, and the one the CLI's
                      // bare `--depends-on` means.
                      const chosen = candidates.find(candidate => candidate.id === pendingUpstream);
                      onChange({
                        deps: [
                          ...draft.deps,
                          { upstreamTaskId: pendingUpstream, name: chosen?.name ?? "", on: "success" },
                        ],
                      });
                      setPendingUpstream("");
                    }}
                  >
                    {t("form.depAddAction")}
                  </Button>
                </div>
                {candidates.length === 0
                  ? <p className="text-xs text-ink-muted">{t("form.depNoCandidates")}</p>
                  : null}

                {draft.deps.length > 1
                  ? (
                      <label className="block space-y-1.5">
                        <span className="text-xs text-ink-soft">{t("form.depJoin")}</span>
                        <Select
                          aria-label={t("form.depJoin")}
                          value={draft.depJoin}
                          onChange={e => onChange({ depJoin: e.target.value })}
                        >
                          <option value="all">{t("join.all")}</option>
                          <option value="any">{t("join.any")}</option>
                        </Select>
                      </label>
                    )
                  : null}
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
                  <TextInput value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
                </label>
                {/* A clamped day is user-visible by design, the same rule the
                    monthly mode states. */}
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
  onEdit: (deps: TaskDepView[]) => void;
  onOpenThread: (threadId: string) => void;
}) {
  const { t, i18n } = useTranslation("tasks");
  const locale = i18n.language;
  // Closed by default: the page opens on what the task did, not on how it is
  // configured (see the section's comment).
  const [settingsOpen, setSettingsOpen] = useState(false);
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
        <Button size="sm" variant="secondary" onClick={() => onEdit(deps)}>{t("edit")}</Button>
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
                    <div className="space-y-3 px-3.5 py-3">
                      <p className="text-xs leading-relaxed text-ink-soft">
                        {run.errorMessage ?? run.resultSummary ?? t("runNoSummary")}
                      </p>
                      {/* The run's conversation is gone by design: say so, so the
                          missing "open conversation" reads as intended rather
                          than broken, and point at where the output is. */}
                      {run.sessionDeleted
                        ? <p className="text-[11px] text-ink-muted">{t("runSessionDeleted")}</p>
                        : null}
                    </div>
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
                {revisions.map((revision) => {
                  const active = revision.version === task.promptVersion;
                  return (
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
                        {active
                          ? <span className="text-ink-muted">{t("revisionActive")}</span>
                          : null}
                        <span className="min-w-0 flex-1" />
                        {active
                          ? null
                          : (
                              <Button
                                leftIcon={<RotateCcw className="size-3" />}
                                size="xs"
                                variant="ghost"
                                onClick={() => void store.applyRevision(task.id, revision.id).then(refresh)}
                              >
                                {t("apply")}
                              </Button>
                            )}
                      </div>
                      <p className="text-[11px] text-ink-muted">
                        {revision.reason ?? t("revisionsNoReason")}
                      </p>
                    </li>
                  );
                })}
              </ul>
            )}
      </section>

    </div>
  );
}
