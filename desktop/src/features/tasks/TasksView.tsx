import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { TaskInput, TaskView } from "./useTasks";
import { ChevronLeft, Play, Plus, RotateCcw, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Select } from "../../components/ui/Select";
import { TextInput } from "../../components/ui/TextInput";
import { loadAgentModelOptions } from "../../integrations/agent/agentClient";
import { formatDateTime } from "../../lib/date";
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

function summarizeTrigger(t: (key: string, options?: Record<string, unknown>) => string, task: TaskView): string {
  if (task.triggerKind !== "schedule")
    return t("trigger.manual");
  const trigger = task.trigger as Record<string, unknown>;
  const mode = String(trigger.mode ?? "");
  switch (mode) {
    case "once":
      return `${trigger.date} ${trigger.time}`;
    case "interval":
      return t("trigger.every", { minutes: Number(trigger.every_minutes ?? 0) });
    case "weekly": {
      // The stored codes are `mon`/`fri`; a list that shows them raw is
      // untranslated in a Chinese UI. Localize each selected day.
      const days = Array.isArray(trigger.days) ? (trigger.days as string[]) : [];
      const labels = days.map(day => t(`weekday.${day}`));
      return `${labels.join(", ")} ${trigger.time}`;
    }
    case "monthly":
      return t("trigger.monthly", { day: Number(trigger.day ?? 1), time: String(trigger.time ?? "") });
    default:
      return `${t("trigger.daily")} ${trigger.time}`;
  }
}

/** Left-nav "Tasks" panel: list, edit, runs, revisions. */
export function TasksView({
  leftPanelExpanded,
  onToggleLeftPanel,
  onOpenThread,
}: {
  leftPanelExpanded: boolean;
  onToggleLeftPanel: () => void;
  onOpenThread: (threadId: string) => void;
}) {
  const { t, i18n } = useTranslation("tasks");
  const locale = i18n.language;
  const store = useTasks();
  const [models, setModels] = useState<AgentModelOption[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState({
    name: "",
    prompt: "",
    cwd: "",
    modelId: "",
    thinkingLevel: "",
    sessionPolicy: "new",
    reflection: "ask",
    enabled: true,
    trigger: { ...emptyTrigger },
  });
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    void loadAgentModelOptions().then(setModels).catch(() => setModels([]));
  }, []);

  const selected = useMemo(
    () => store.tasks.find(task => task.id === selectedId) ?? null,
    [store.tasks, selectedId],
  );

  const startCreate = useCallback(() => {
    setSelectedId(null);
    setEditing(true);
    setSaveError(null);
    setDraft({
      name: "",
      prompt: "",
      cwd: "",
      modelId: "",
      thinkingLevel: "",
      sessionPolicy: "new",
      reflection: "ask",
      enabled: true,
      trigger: { ...emptyTrigger },
    });
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
      reflection: draft.reflection,
      triggerKind: kind,
      trigger,
      depJoin: selected?.depJoin ?? "all",
      enabled: draft.enabled,
    };
    if (!input.name || !input.prompt || !input.cwd) {
      setSaveError(t("form.required"));
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
      <header className="flex h-14 shrink-0 items-center gap-2 border-b border-line-soft px-3">
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
            ? <p className="p-3 text-xs text-danger">{store.error}</p>
            : null}
          {store.tasks.length === 0 && !store.loading
            ? <p className="p-3 text-xs text-ink-muted">{t("empty")}</p>
            : null}
          {store.tasks.map(task => (
            <button
              key={task.id}
              className={`flex w-full flex-col gap-1 border-b border-line-soft px-3 py-2 text-left transition-colors hover:bg-surface-subtle ${task.id === selectedId ? "bg-surface-subtle" : ""}`}
              type="button"
              onClick={() => {
                setSelectedId(task.id);
                setEditing(false);
              }}
            >
              <span className="flex items-center gap-2">
                <span className={`size-1.5 shrink-0 rounded-full ${task.enabled ? "bg-accent" : "bg-line"}`} />
                <span className="min-w-0 flex-1 truncate text-sm text-ink">{task.name}</span>
                {task.latestRun
                  ? <span className="shrink-0 text-[11px] text-ink-muted">{t(`status.${task.latestRun.status}`)}</span>
                  : null}
              </span>
              <span className="truncate pl-3.5 text-[11px] text-ink-muted">
                {summarizeTrigger(t, task)}
                {task.nextDueAt ? ` · ${formatEpoch(task.nextDueAt, locale)}` : ""}
              </span>
            </button>
          ))}
        </div>

        <div className="min-w-0 flex-1 overflow-y-auto">
          {editing
            ? (
                <TaskForm
                  draft={draft}
                  error={saveError}
                  models={models}
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
              : <p className="p-6 text-sm text-ink-muted">{t("selectHint")}</p>}
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
  draft: {
    name: string;
    prompt: string;
    cwd: string;
    modelId: string;
    thinkingLevel: string;
    sessionPolicy: string;
    reflection: string;
    enabled: boolean;
    trigger: DraftTrigger;
  };
  error: string | null;
  models: AgentModelOption[];
  onChange: (patch: Partial<typeof draft>) => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const { t } = useTranslation("tasks");
  const trigger = draft.trigger;
  const setTrigger = (patch: Partial<DraftTrigger>) => onChange({ trigger: { ...trigger, ...patch } });

  return (
    <div className="space-y-4 p-6">
      <h2 className="text-sm font-semibold text-ink">{t("form.title")}</h2>
      {error ? <p className="text-xs text-danger">{error}</p> : null}

      <label className="block space-y-1">
        <span className="text-xs text-ink-soft">{t("form.name")}</span>
        <TextInput value={draft.name} onChange={e => onChange({ name: e.target.value })} />
      </label>

      <label className="block space-y-1">
        <span className="text-xs text-ink-soft">{t("form.prompt")}</span>
        <textarea
          className="min-h-32 w-full rounded-md border border-line-soft bg-surface px-3 py-2 text-sm text-ink outline-none focus:border-focus focus:ring-2 focus:ring-focus"
          spellCheck={false}
          value={draft.prompt}
          onChange={e => onChange({ prompt: e.target.value })}
        />
      </label>

      <label className="block space-y-1">
        <span className="text-xs text-ink-soft">{t("form.cwd")}</span>
        <TextInput value={draft.cwd} onChange={e => onChange({ cwd: e.target.value })} />
      </label>

      <div className="grid grid-cols-2 gap-3">
        <label className="block space-y-1">
          <span className="text-xs text-ink-soft">{t("form.model")}</span>
          <Select value={draft.modelId} onChange={e => onChange({ modelId: e.target.value })}>
            <option value="">{t("form.modelDefault")}</option>
            {models.map(model => (
              <option key={`${model.provider}/${model.id}`} value={`${model.provider}/${model.id}`}>
                {model.label || model.id}
              </option>
            ))}
          </Select>
        </label>
        <label className="block space-y-1">
          <span className="text-xs text-ink-soft">{t("form.thinking")}</span>
          <Select value={draft.thinkingLevel} onChange={e => onChange({ thinkingLevel: e.target.value })}>
            <option value="">{t("form.thinkingDefault")}</option>
            {["off", "minimal", "low", "medium", "high", "xhigh"].map(level => (
              <option key={level} value={level}>{level}</option>
            ))}
          </Select>
        </label>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <label className="block space-y-1">
          <span className="text-xs text-ink-soft">{t("form.session")}</span>
          <Select value={draft.sessionPolicy} onChange={e => onChange({ sessionPolicy: e.target.value })}>
            <option value="new">{t("form.sessionNew")}</option>
            <option value="existing">{t("form.sessionExisting")}</option>
          </Select>
        </label>
        <label className="block space-y-1">
          <span className="text-xs text-ink-soft">{t("form.reflection")}</span>
          <Select value={draft.reflection} onChange={e => onChange({ reflection: e.target.value })}>
            <option value="off">{t("form.reflectionOff")}</option>
            <option value="ask">{t("form.reflectionAsk")}</option>
            <option value="auto">{t("form.reflectionAuto")}</option>
          </Select>
        </label>
      </div>
      {draft.sessionPolicy === "existing"
        ? <p className="text-[11px] text-ink-muted">{t("form.sessionExistingHint")}</p>
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
                <label className="block space-y-1">
                  <span className="text-xs text-ink-soft">{t("form.date")}</span>
                  <TextInput placeholder="2026-12-24" value={trigger.date} onChange={e => setTrigger({ date: e.target.value })} />
                </label>
                <label className="block space-y-1">
                  <span className="text-xs text-ink-soft">{t("form.time")}</span>
                  <TextInput value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
                </label>
              </div>
            )
          : null}

        {trigger.mode === "interval"
          ? (
              <label className="block space-y-1">
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
              <label className="block space-y-1">
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
                <label className="block space-y-1">
                  <span className="text-xs text-ink-soft">{t("form.time")}</span>
                  <TextInput value={trigger.time} onChange={e => setTrigger({ time: e.target.value })} />
                </label>
              </div>
            )
          : null}

        {trigger.mode === "monthly"
          ? (
              <div className="space-y-2">
                <label className="block space-y-1">
                  <span className="text-xs text-ink-soft">{t("form.day")}</span>
                  <TextInput
                    type="number"
                    max={31}
                    min={1}
                    value={trigger.day}
                    onChange={e => setTrigger({ day: Number(e.target.value) || 1 })}
                  />
                </label>
                <label className="block space-y-1">
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

      <p className="text-[11px] text-warning">{t("form.fullPermissionWarning")}</p>

      <div className="flex gap-2">
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

  return (
    <div className="space-y-5 p-6">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-sm font-semibold text-ink">{task.name}</h2>
          <p className="text-xs text-ink-muted">
            {summarizeTrigger(t, task)}
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

      <section className="space-y-1">
        <h3 className="text-xs font-medium text-ink-soft">{t("prompt")}</h3>
        <pre className="max-h-48 overflow-auto whitespace-pre-wrap rounded-md border border-line-soft bg-surface-subtle p-3 text-xs text-ink">
          {task.prompt}
        </pre>
        <p className="text-[11px] text-ink-muted">
          {t("promptVersion", { version: task.promptVersion })}
        </p>
      </section>

      <section className="space-y-2">
        <h3 className="text-xs font-medium text-ink-soft">{t("deps")}</h3>
        {deps.length === 0
          ? <p className="text-[11px] text-ink-muted">{t("depsNone")}</p>
          : (
              <ul className="space-y-1">
                {deps.map(dep => (
                  <li key={dep.upstreamTaskId} className="flex items-center gap-2 text-xs text-ink">
                    <span className={dep.satisfied ? "text-accent" : "text-ink-muted"}>
                      {dep.satisfied ? t("depsReady") : t("depsWaiting")}
                    </span>
                    <span className="truncate">{dep.upstreamName}</span>
                    <span className="text-ink-muted">
                      {t(`on.${dep.on}`)}
                      {" · "}
                      {t(`join.${task.depJoin}`)}
                    </span>
                  </li>
                ))}
              </ul>
            )}
      </section>

      <section className="space-y-2">
        <h3 className="text-xs font-medium text-ink-soft">{t("runs")}</h3>
        {runs.length === 0
          ? <p className="text-[11px] text-ink-muted">{t("runsNone")}</p>
          : (
              <ul className="space-y-1">
                {runs.map(run => (
                  <li key={run.id} className="flex items-center gap-2 text-xs text-ink">
                    <span className="w-16 shrink-0 text-ink-muted">{t(`kind.${run.kind}`)}</span>
                    <span className="w-16 shrink-0">{t(`status.${run.status}`)}</span>
                    <span className="w-32 shrink-0 text-ink-muted">
                      {run.startedAt ? formatEpoch(run.startedAt, locale) : ""}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-ink-muted">
                      {run.errorMessage ?? run.resultSummary ?? ""}
                    </span>
                    {run.threadId
                      ? (
                          <Button size="xs" variant="ghost" onClick={() => onOpenThread(run.threadId as string)}>
                            {t("openConversation")}
                          </Button>
                        )
                      : null}
                  </li>
                ))}
              </ul>
            )}
      </section>

      <section className="space-y-2">
        <h3 className="text-xs font-medium text-ink-soft">{t("revisions")}</h3>
        {revisions.length === 0
          ? <p className="text-[11px] text-ink-muted">{t("revisionsNone")}</p>
          : (
              <ul className="space-y-1">
                {revisions.map(revision => (
                  <li key={revision.id} className="flex items-center gap-2 text-xs text-ink">
                    <span className="w-10 shrink-0 text-ink-muted">
                      v
                      {revision.version}
                    </span>
                    <span className="w-20 shrink-0">{t(`source.${revision.source}`)}</span>
                    <span className="min-w-0 flex-1 truncate text-ink-muted">
                      {revision.reason ?? revision.prompt.slice(0, 60)}
                    </span>
                    <Button
                      leftIcon={<RotateCcw className="size-3" />}
                      size="xs"
                      variant="ghost"
                      onClick={() => void store.applyRevision(task.id, revision.id).then(refresh)}
                    >
                      {t("apply")}
                    </Button>
                  </li>
                ))}
              </ul>
            )}
      </section>
    </div>
  );
}
