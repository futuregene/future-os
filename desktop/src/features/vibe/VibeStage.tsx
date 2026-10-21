import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { ThreadRunInfo } from "../../components/layout/hooks/useThreadStore";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { StoredApprovalRequest, StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { AgentThreadChrome } from "../agent/AgentThread";
import type { SessionMentionOption } from "../agent/sessionMention";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Expand, Keyboard, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useApprovals } from "../../components/layout/hooks/useApprovals";
import { usePendingApprovalCounts } from "../../components/layout/hooks/usePendingApprovalCounts";
import { Badge } from "../../components/ui/Badge";
import { IconButton } from "../../components/ui/IconButton";
import { useCachedAgentState } from "../../integrations/agent/agentStateCache";
import { cn } from "../../lib/cn";
import { isMacOS } from "../../lib/platform";
import { AgentThread } from "../agent/AgentThread";
import { useCommand } from "../keyboard/commands";
import { useShortcutScope } from "../keyboard/scopes";
import { bindingLabel, useShortcutOverrides } from "../keyboard/shortcuts";
import { cellPosition, gridOf, MAX_CELLS, moveInGrid } from "./layout";
import { moveInOrder, resolveOrder, vibeKeyOfLocal } from "./order";
import { VibeHintBar } from "./VibeHintBar";
import { readVibePreferences, writeVibeColumns, writeVibeOrder } from "./vibePreferences";

/** Everything a conversation's view needs that does not vary per conversation. */
export interface VibeThreadViewProps {
  agentConnection: AgentConnectionState;
  approvalTier: ApprovalTier;
  loadingStore: boolean;
  /** The shell's current model, used when a conversation has not reported its own. */
  modelId: string;
  modelOptions: AgentModelOption[];
  thinkingLevel: string;
  onChangeApprovalTier: (value: ApprovalTier) => void;
  /**
   * Called after the panel's own approval queue has persisted a decision, so the
   * shell can refresh what it shows (the rail's badge, the store).
   *
   * A notification, not the decision itself: the queue that owns the thread is the
   * one `useApprovals` holds below, and asking a second owner to write the same
   * row would decide it twice.
   */
  onApprovalDecided: () => void;
  onForked: (threadId: string) => void;
  onModelChange: (modelId: string) => void;
  onOpenAccount: () => void;
  onOpenModels: () => void;
  onOpenProviders: () => void;
  onRetryAgentConnection: () => void;
  onThinkingLevelChange: (thinkingLevel: string) => void;
  onThreadActivity: () => void;
  sessionMentions: SessionMentionOption[];
  skillRecommend: boolean;
  futureSessionStatus: string;
  futureBalance: number | null;
}

export interface VibeStageProps {
  threads: StoredThread[];
  workspaces: StoredWorkspace[];
  runStatuses: Record<string, ThreadRunInfo | undefined>;
  unreadThreadIds: Set<string>;
  view: VibeThreadViewProps;
  onClose: () => void;
  /** Open a conversation in the main window (the panel's "go there" escape hatch). */
  onOpenInShell: (thread: StoredThread) => void;
  /** Unpin, which is what "remove from the panel" means. */
  onTogglePin: (thread: StoredThread) => void;
}

/**
 * Which layer of the panel owns the keyboard.
 *
 * `grid` → `read` → `input`, and `Esc`/Back walks back out the same way. One
 * layer at a time, which is what lets the same physical key mean three things
 * without ambiguity.
 */
type VibeLayer = "grid" | "read" | "input";

function isRunning(status: ThreadRunInfo | undefined): boolean {
  return status?.status === "queued"
    || status?.status === "running"
    || status?.status === "waiting_approval";
}

/**
 * The Vibe panel: pinned conversations in a full-screen grid, operated with a
 * remote control and dictated into.
 *
 * Product and interaction contract: `docs/internals/desktop/VIBE.md`. Keys:
 * `KEYBINDINGS.md` § Modes. This component owns three things and no more — which
 * conversations are members and in what order, which layer is on top, and the
 * geometry. Everything a conversation *is* stays in `AgentThread`.
 */
export function VibeStage({ threads, workspaces, runStatuses, unreadThreadIds, view, onClose, onOpenInShell, onTogglePin }: VibeStageProps) {
  const { t } = useTranslation("vibe");
  const approvalCounts = usePendingApprovalCounts();
  const overrides = useShortcutOverrides();
  const isMac = isMacOS;
  const [layer, setLayer] = useState<VibeLayer>("grid");
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const preferences = useMemo(() => readVibePreferences(), []);
  const [order, setOrder] = useState<string[]>(preferences.order);
  const [columns, setColumns] = useState<number | null>(preferences.columns);
  const [size, setSize] = useState({ height: 0, width: 0 });
  const areaRef = useRef<HTMLDivElement>(null);

  // Members: pinned, alive, and this machine's. P0 is the local machine only;
  // the machine filter and a host's conversations are P1 (`VIBE.md` §14).
  const members = useMemo(
    () => threads
      .filter(thread => thread.pinned && thread.status === "active")
      .map(thread => ({ key: vibeKeyOfLocal(thread.id), thread })),
    [threads],
  );
  const memberByKey = useMemo(() => new Map(members.map(member => [member.key, member.thread])), [members]);
  const resolved = useMemo(
    () => resolveOrder(members.map(member => ({ createdAt: member.thread.createdAt, key: member.key })), order),
    [members, order],
  );
  const cells = useMemo(
    () => resolved.map(key => memberByKey.get(key)).filter((thread): thread is StoredThread => Boolean(thread)),
    [memberByKey, resolved],
  );
  const overflow = Math.max(0, cells.length - MAX_CELLS);
  const shown = cells.slice(0, MAX_CELLS);

  // Focus is kept by key, not by index: a reorder, a new pin or an unpin must
  // not move the ring onto a different conversation than the one the reader was
  // looking at.
  const focusIndex = useMemo(() => {
    if (!focusKey)
      return 0;
    const index = shown.findIndex(thread => vibeKeyOfLocal(thread.id) === focusKey);
    return index === -1 ? 0 : index;
  }, [focusKey, shown]);
  const focused = shown[focusIndex] ?? null;
  const focusedKey = focused ? vibeKeyOfLocal(focused.id) : null;

  const layout = useMemo(
    () => gridOf(shown.length, size.width || window.innerWidth, size.height || window.innerHeight, columns),
    [columns, shown.length, size.height, size.width],
  );

  // The grid area's real size, not the window's: the bar above it and the padding
  // around it are what a cell does not get, and measuring the window would hand
  // every cell a few pixels it cannot use. Guarded like the other observers in
  // the app: jsdom has none, and the layout falls back to the window size.
  useEffect(() => {
    const element = areaRef.current;
    if (!element)
      return;
    const measure = () => setSize({ height: element.clientHeight, width: element.clientWidth });
    measure();
    if (typeof ResizeObserver === "undefined")
      return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  // The panel's own approvals: only the focused conversation can be answered
  // from here (one queue, not twelve). A cell shows its count and, at the focus
  // point, the answerable card.
  const { activeApproval, decideApproval } = useApprovals(layer === "grid" ? null : focused?.id ?? null);

  const persistOrder = useCallback((next: string[]) => {
    setOrder(next);
    writeVibeOrder(next);
  }, []);

  const setFocus = useCallback((thread: StoredThread | null) => {
    setFocusKey(thread ? vibeKeyOfLocal(thread.id) : null);
  }, []);

  const moveFocus = useCallback((direction: "up" | "down" | "left" | "right") => {
    if (!shown.length)
      return;
    const next = moveInGrid(focusIndex, direction, shown.length, layout);
    if (next !== focusIndex)
      setFocus(shown[next] ?? null);
  }, [focusIndex, layout, setFocus, shown]);

  const stepFocus = useCallback((delta: number) => {
    if (!shown.length)
      return;
    const next = Math.min(shown.length - 1, Math.max(0, focusIndex + delta));
    if (next !== focusIndex)
      setFocus(shown[next] ?? null);
  }, [focusIndex, setFocus, shown]);

  // ── Scopes: exactly one layer owns the bare keys ──────────────────────────
  useShortcutScope("vibe", true);
  useShortcutScope("vibeGrid", layer === "grid");
  useShortcutScope("vibeZoom", layer === "read");
  useShortcutScope("vibeInput", layer === "input");

  // ── Grid layer ────────────────────────────────────────────────────────────
  const onGrid = layer === "grid";
  useCommand("vibe.focus.up", () => moveFocus("up"), onGrid);
  useCommand("vibe.focus.down", () => moveFocus("down"), onGrid);
  useCommand("vibe.focus.left", () => moveFocus("left"), onGrid);
  useCommand("vibe.focus.right", () => moveFocus("right"), onGrid);
  // `OK` on a cell opens its **input layer**, not a reading view: speaking is the
  // cheapest thing this panel can do, so it gets the shortest path. Watching
  // without speaking is one more key (`Esc`), which is also how a reader who
  // changed their mind gets out.
  useCommand("vibe.open", () => {
    if (focused)
      setLayer("input");
  }, onGrid);
  useCommand("vibe.close", onClose, onGrid);
  useCommand("vibe.cell", (binding) => {
    const digit = Number.parseInt((binding?.code ?? "").replace("Digit", ""), 10);
    if (!Number.isInteger(digit) || digit < 1)
      return;
    const target = shown[digit - 1];
    if (target)
      setFocus(target);
  }, onGrid);

  // ── Reading layer ─────────────────────────────────────────────────────────
  const onRead = layer === "read";
  useCommand("vibe.back", () => setLayer("grid"), onRead);
  useCommand("vibe.zoom.previous", () => stepFocus(-1), onRead);
  useCommand("vibe.zoom.next", () => stepFocus(1), onRead);
  useCommand("vibe.input.open", () => {
    if (focused)
      setLayer("input");
  }, onRead);

  // ── Input layer ───────────────────────────────────────────────────────────
  // Sending is **not** here: `⏎` is the composer's own fixed key, which already
  // stands down while an input method composes (`MentionEditor.onSubmit`), and the
  // editor hands an empty draft to this layer's onLeaveInput instead. Closing the
  // layer is the panel's own.
  useCommand("vibe.input.close", () => setLayer("read"), layer === "input");

  // ── Panel-wide ────────────────────────────────────────────────────────────
  useCommand("vibe.move.left", () => {
    if (!focusedKey)
      return;
    persistOrder(moveInOrder(resolved, focusedKey, -1));
  });
  useCommand("vibe.move.right", () => {
    if (!focusedKey)
      return;
    persistOrder(moveInOrder(resolved, focusedKey, 1));
  });
  useCommand("vibe.columns.cycle", () => {
    const next = columns === null ? 2 : columns >= 4 ? null : columns + 1;
    setColumns(next);
    writeVibeColumns(next);
  });
  useCommand("vibe.columns.more", () => {
    const next = Math.min(4, (columns ?? layout.columns) + 1);
    setColumns(next);
    writeVibeColumns(next);
  });
  useCommand("vibe.columns.fewer", () => {
    const next = Math.max(1, (columns ?? layout.columns) - 1);
    setColumns(next);
    writeVibeColumns(next);
  });
  useCommand("vibe.columns.auto", () => {
    setColumns(null);
    writeVibeColumns(null);
  });
  useCommand("vibe.fullscreen", () => {
    const win = getCurrentWindow();
    void win.isFullscreen().then(full => win.setFullscreen(!full)).catch(() => {});
  });
  useCommand("vibe.remove", () => {
    if (focused)
      onTogglePin(focused);
  });

  const workspacePathOf = useCallback(
    (thread: StoredThread) => workspaces.find(workspace => workspace.id === thread.workspaceId)?.path ?? null,
    [workspaces],
  );

  const threadView = (thread: StoredThread, chrome: AgentThreadChrome, ownsCommands: boolean) => (
    <VibeThreadView
      activeApproval={layer === "grid" ? null : activeApproval}
      chrome={chrome}
      focused={ownsCommands}
      onApprovalDecision={async (approval, status) => {
        await decideApproval(approval, status);
        view.onApprovalDecided();
      }}
      onLeaveInput={() => setLayer("read")}
      thread={thread}
      view={view}
      workspacePath={workspacePathOf(thread)}
    />
  );

  const arrows = ["vibe.focus.left", "vibe.focus.up", "vibe.focus.down", "vibe.focus.right"]
    .map(id => bindingLabel(id as never, overrides, isMac))
    .filter(Boolean)
    .join(" ");

  /**
   * The grid: one cell per member, in order, left to right.
   *
   * `exclude` is the input layer's: the conversation being dictated into is drawn
   * by the layer itself (with its composer), so leaving it in the backdrop too
   * would mount it twice — two subscriptions, two history reads, one truth.
   */
  function grid(excludeKey: string | null, dimmed: boolean) {
    const visible = shown.filter(thread => vibeKeyOfLocal(thread.id) !== excludeKey);
    return (
      <div
        aria-hidden={dimmed || undefined}
        className={cn(
          "grid h-full w-full gap-2",
          dimmed && "pointer-events-none select-none opacity-30",
        )}
        style={{
          gridTemplateColumns: `repeat(${layout.columns}, minmax(0, 1fr))`,
          gridTemplateRows: `repeat(${layout.rows}, minmax(0, 1fr))`,
        }}
      >
        {visible.map((thread, index) => {
          const key = vibeKeyOfLocal(thread.id);
          const focusedCell = key === focusedKey;
          const approvals = approvalCounts.get(thread.id) ?? 0;
          return (
            <section
              className={cn(
                "flex min-h-0 min-w-0 flex-col overflow-hidden rounded-lg border bg-surface",
                focusedCell
                  ? "border-accent shadow-panel ring-2 ring-accent/35"
                  : "border-line-soft",
              )}
              data-vibe-cell={index + 1}
              key={key}
              onMouseDown={dimmed
                ? undefined
                : () => {
                    setFocus(thread);
                    setLayer("read");
                  }}
              style={{ gridColumn: cellPosition(index, layout).column + 1 }}
            >
              <header
                className={cn(
                  "flex h-7 shrink-0 items-center gap-2 border-b px-2 text-xs",
                  focusedCell ? "border-accent/40 bg-accent-soft/40" : "border-line-soft/60",
                )}
                onDoubleClick={dimmed ? undefined : () => onOpenInShell(thread)}
                title={thread.title}
              >
                <span className="font-mono text-[11px] text-ink-faint">{index + 1}</span>
                <span className="min-w-0 flex-1 truncate font-medium">{thread.title}</span>
                {isRunning(runStatuses[thread.id])
                  ? <span className="size-1.5 shrink-0 animate-pulse rounded-full bg-accent" title={t("running")} />
                  : null}
                {approvals > 0
                  ? <Badge className="h-5 px-1.5" tone="warning">{approvals}</Badge>
                  : null}
                {unreadThreadIds.has(thread.id)
                  ? <span className="size-1.5 shrink-0 rounded-full bg-info" title={t("unread")} />
                  : null}
              </header>
              <div className="min-h-0 flex-1">
                {threadView(thread, "read", false)}
              </div>
            </section>
          );
        })}
      </div>
    );
  }

  return (
    <div className="fixed inset-0 z-40 flex flex-col bg-canvas text-ink">
      <header className="flex h-9 shrink-0 select-none items-center gap-3 border-b border-line-soft/60 px-3">
        <span className="text-xs font-semibold tracking-wide text-ink-soft">{t("title")}</span>
        <span className="text-xs text-ink-soft">
          {t("count", { count: shown.length })}
          {overflow > 0 ? ` · ${t("overflow", { count: overflow })}` : ""}
        </span>
        <span className="font-mono text-xs text-ink-faint">
          {layout.columns}
          ×
          {layout.rows}
          {columns === null ? "" : ` ${t("manualLayout")}`}
        </span>
        {layer === "grid" ? <span className="font-mono text-xs text-ink-faint">{arrows}</span> : null}
        <div className="ml-auto flex items-center gap-1">
          <IconButton
            className="size-7"
            icon={<Expand className="size-3.5" />}
            label={t("fullscreen")}
            onClick={() => {
              const win = getCurrentWindow();
              void win.isFullscreen().then(full => win.setFullscreen(!full)).catch(() => {});
            }}
          />
          <IconButton
            className="size-7"
            icon={<Keyboard className="size-3.5" />}
            label={t("shortcuts")}
            onClick={() => focused && onOpenInShell(focused)}
          />
          <IconButton
            className="size-7"
            icon={<X className="size-4" />}
            label={t("close")}
            onClick={onClose}
          />
        </div>
      </header>

      {shown.length === 0
        ? (
            <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-8 text-center">
              <p className="text-sm font-medium text-ink">{t("empty.title")}</p>
              <p className="max-w-md text-xs leading-5 text-ink-soft">{t("empty.body")}</p>
            </div>
          )
        : layer === "grid"
          ? <div ref={areaRef} className="min-h-0 flex-1 p-2">{grid(null, false)}</div>
          : layer === "read"
            ? (
                <div className="flex min-h-0 flex-1 flex-col p-2">
                  <div className="flex h-7 shrink-0 items-center gap-2 px-1 text-xs">
                    <span className="font-mono text-[11px] text-ink-faint">{focusIndex + 1}</span>
                    <span className="min-w-0 flex-1 truncate font-medium">{focused?.title ?? ""}</span>
                    <span className="text-ink-faint">{t("of", { total: shown.length })}</span>
                  </div>
                  <div
                    className="min-h-0 flex-1 overflow-hidden rounded-lg border border-line-soft bg-surface"
                    onDoubleClick={() => focused && onOpenInShell(focused)}
                  >
                    {focused ? threadView(focused, "read", true) : null}
                  </div>
                  <VibeHintBar ids={["vibe.zoom.previous", "vibe.input.open", "vibe.back"]} />
                </div>
              )
            : (
                // The input layer keeps the *rest* of the panel visible behind it,
                // dimmed: who is running and who is waiting for an approval is
                // exactly what a person watching several conversations must keep
                // seeing while they talk. The focused conversation is drawn here
                // instead (its own composer lives in this card).
                <div className="relative flex min-h-0 flex-1 flex-col">
                  <div className="absolute inset-0 p-2">{grid(focusedKey, true)}</div>
                  <div className="relative flex min-h-0 flex-1 flex-col items-center px-8 pt-[6vh]">
                    <div
                      className="flex w-full max-w-4xl min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-line-soft bg-surface shadow-panel"
                      data-vibe-target=""
                    >
                      <div className="flex h-9 shrink-0 select-none items-center gap-2 border-b border-line-soft/60 bg-surface-subtle px-4 text-xs">
                        <span className="text-ink-soft">{t("input.to")}</span>
                        <span className="min-w-0 truncate text-sm font-medium">{focused?.title ?? ""}</span>
                        {focused ? <Badge className="h-5" tone="neutral">{t("input.local")}</Badge> : null}
                        <span className="ml-auto font-mono text-[11px] text-ink-faint">{arrows}</span>
                      </div>
                      <div className="min-h-0 flex-1">
                        {focused
                          ? (
                              <VibeThreadView
                                chrome="input"
                                focused
                                inputFooter={(
                                  <VibeHintBar
                                    fixed={[{ code: "Enter", labelKey: "fixed.send" }]}
                                    ids={["vibe.input.close", "vibe.menu"]}
                                  />
                                )}
                                onApprovalDecision={async (approval, status) => {
                                  await decideApproval(approval, status);
                                  view.onApprovalDecided();
                                }}
                                onLeaveInput={() => setLayer("read")}
                                thread={focused}
                                view={view}
                                workspacePath={workspacePathOf(focused)}
                              />
                            )
                          : null}
                      </div>
                    </div>
                  </div>
                </div>
              )}
    </div>
  );
}

/**
 * One conversation as the panel draws it.
 *
 * A thin adapter, deliberately: it answers the two questions `AgentThread` cannot
 * answer for itself in the panel — which model this *particular* conversation
 * runs on, and whether this instance may claim `chat.stop` — and then hands
 * everything else over. The panel reuses the conversation view rather than
 * writing a second one, so scrolling, paging, streaming and approvals cannot
 * behave differently here than in the main window.
 */
function VibeThreadView({
  thread,
  chrome,
  focused,
  activeApproval,
  inputFooter,
  onLeaveInput,
  onApprovalDecision,
  view,
  workspacePath,
}: {
  thread: StoredThread;
  chrome: AgentThreadChrome;
  focused: boolean;
  activeApproval?: StoredApprovalRequest | null;
  inputFooter?: React.ReactNode;
  onLeaveInput?: () => void;
  onApprovalDecision: (approval: StoredApprovalRequest, status: "approved" | "rejected") => Promise<void>;
  view: VibeThreadViewProps;
  workspacePath: string | null;
}) {
  // The conversation's own model, as the agent reports it, so a cell sends with
  // the thing that conversation is running on rather than with whatever the shell
  // happens to have selected for the thread the reader came from.
  const state = useCachedAgentState(thread.id);
  const modelId = state?.model ?? view.modelId;
  const thinkingLevel = state?.thinkingLevel ?? view.thinkingLevel;
  return (
    <AgentThread
      activeApproval={activeApproval}
      agentConnection={view.agentConnection}
      approvalTier={view.approvalTier}
      chrome={chrome}
      inputFooter={inputFooter}
      loadingStore={view.loadingStore}
      modelId={modelId}
      modelOptions={view.modelOptions}
      onChangeApprovalTier={view.onChangeApprovalTier}
      onApprovalDecision={onApprovalDecision}
      onForked={view.onForked}
      onLeaveInput={onLeaveInput}
      onModelChange={view.onModelChange}
      onOpenAccount={view.onOpenAccount}
      onOpenModels={view.onOpenModels}
      onOpenProviders={view.onOpenProviders}
      onPromptConsumed={() => {}}
      onRetryAgentConnection={view.onRetryAgentConnection}
      ownsCommands={focused}
      // The panel has no rail to collapse and no shell to report to: a cell's
      // transcript is the conversation, and the left panel's toggle is the shell's.
      leftPanelExpanded
      onThreadActivity={view.onThreadActivity}
      onThinkingLevelChange={view.onThinkingLevelChange}
      onToggleLeftPanel={() => {}}
      pendingPrompt={null}
      sessionMentions={view.sessionMentions}
      skillRecommend={view.skillRecommend}
      thread={thread}
      thinkingLevel={thinkingLevel}
      workspacePath={workspacePath}
      futureBalance={view.futureBalance}
      futureSessionStatus={view.futureSessionStatus}
    />
  );
}
