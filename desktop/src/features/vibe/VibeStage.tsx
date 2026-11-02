import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { ContextTab } from "../../components/layout/ContextPanel";
import type { ThreadRunInfo } from "../../components/layout/hooks/useThreadStore";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { StoredApprovalRequest, StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import type { AgentThreadChrome } from "../agent/AgentThread";
import type { ComposerSendPayload } from "../agent/Composer";
import type { SessionMentionOption } from "../agent/sessionMention";
import type { RemoteCatalog } from "../remote-peer/mergeConversations";
import type { RemotePeer } from "../remote-peer/remotePeerClient";
import type { VibeAttentionReason } from "./attention";
import type { VibeMenuRow } from "./menuNav";
import type { ReadingPanelPreference } from "./readingPanel";
import type { VibeCell, VibeDeviceFilter } from "./vibeCells";
import { Expand, MoreHorizontal, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ContextPanel } from "../../components/layout/ContextPanel";
import { useApprovals } from "../../components/layout/hooks/useApprovals";
import { usePendingApprovalCounts } from "../../components/layout/hooks/usePendingApprovalCounts";
import { useWindowWidth } from "../../components/layout/hooks/useWindowWidth";
import { Badge } from "../../components/ui/Badge";
import { IconButton } from "../../components/ui/IconButton";
import { modelKey, modelThinkingLevels, thinkingLevelName } from "../../integrations/agent/agentClient";
import { useCachedAgentState } from "../../integrations/agent/agentStateCache";
import { cn } from "../../lib/cn";
import { emitFutureEvent } from "../../lib/futureEvents";
import { isMacOS } from "../../lib/platform";
import { useIsFullscreen } from "../../lib/useIsFullscreen";
import { startWindowDrag } from "../../lib/windowDrag";
import { AgentThread } from "../agent/AgentThread";
import { useCommand } from "../keyboard/commands";
import { useShortcutScope } from "../keyboard/scopes";
import { bindingLabel, useShortcutOverrides } from "../keyboard/shortcuts";
import { RemoteFilesPanel } from "../remote-peer/RemoteFilesPanel";
import { AddToVibeDialog } from "./AddToVibeDialog";
import { attentionOf, nextAttention } from "./attention";
import { BAR_HEIGHT, barStart, reserveTrafficLights as shouldReserveTrafficLights } from "./barChrome";
import { ADD_SLOT_KEY, clampPage, hasAddSlot, PAGE_SIZE, pageCount, pageSlice, recentChoices } from "./emptySlot";
import { toggleNativeFullscreen } from "./fullscreen";
import { gridAreaOf, gridOf, moveInGrid } from "./layout";
import { moveInOrder, resolveOrder } from "./order";
import { cyclePreference, nextStickyDefault, PANEL_WIDTH, preferenceLabelKey, readPanelPreference, showReadingPanel, toggleHidden, togglePinned } from "./readingPanel";
import { scrollToLiveEnd, scrollTranscript, transcriptOf } from "./scrollCell";
import { vibeCellsOf } from "./vibeCells";
import { setVibeCellModel, setVibeCellThinkingLevel, useVibeCellSettings } from "./vibeCellSettings";
import { VibeHintBar } from "./VibeHintBar";
import { VibeKeyProbe } from "./VibeKeyProbe";
import { VibeMenu } from "./VibeMenu";
import { readVibePreferences, writeVibeColumns, writeVibeDevice, writeVibeOrder, writeVibePanel } from "./vibePreferences";
import { useVibeQueue } from "./vibeQueue";
import { VibeRemoteCell } from "./VibeRemoteCell";
import { VibeSettingsLine } from "./VibeSettings";

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
  /** Every paired host's catalogue: the remote half of the panel's membership. */
  catalogs: RemoteCatalog[];
  /** Labelled hosts, for the machine filter and a cell's badge. */
  peers: RemotePeer[];
  onClose: () => void;
  /** Open a conversation in the main window (the panel's "go there" escape hatch). */
  onOpenInShell: (thread: StoredThread) => void;
  /** The same, for a host's conversation. */
  onOpenRemoteInShell: (desktopId: string, sessionId: string) => void;
  /**
   * Pin or unpin, on the machine that owns the conversation.
   *
   * One callback per machine rather than one polymorphic "toggle": the pin is a
   * write to that machine's own database, and the two routes take different ids
   * (a thread id here, a thread id *and* a session id there).
   */
  onPinLocal: (threadId: string, pinned: boolean) => void;
  onPinRemote: (desktopId: string, sessionId: string, pinned: boolean) => void;
  /**
   * Leave the panel for the new-conversation screen, and pin whatever is created there.
   *
   * The panel does not grow its own conversation-creation flow: a first message is what
   * creates a thread, so the screen that already asks for one is the right place. The pin
   * is the panel's promise ("adding is pinning") applied to the `+`.
   */
  onNewConversation: () => void;
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
export function VibeStage({
  catalogs,
  onClose,
  onOpenInShell,
  onOpenRemoteInShell,
  onNewConversation,
  onPinLocal,
  onPinRemote,
  peers,
  runStatuses,
  threads,
  unreadThreadIds,
  view,
  workspaces,
}: VibeStageProps) {
  const { t } = useTranslation("vibe");
  const approvalCounts = usePendingApprovalCounts();
  const overrides = useShortcutOverrides();
  const isMac = isMacOS;
  const [layer, setLayer] = useState<VibeLayer>("grid");
  /**
   * Whether the menu is up.
   *
   * Its own state rather than a layer: the menu is not a place the reader is in,
   * it is a sheet over whatever layer they were using — and it comes back to that
   * same layer when it closes.
   */
  const [menuOpen, setMenuOpen] = useState(false);
  /**
   * The window's own fullscreen state, and the one way to change it.
   *
   * Three surfaces ask for this (the bar's icon, the menu's row, `vibe.fullscreen`), so it
   * is one function that also reports failure — the old version swallowed the rejection,
   * which is why the button did nothing for a release and said nothing about why.
   */
  const isFullscreen = useIsFullscreen();
  const toggleFullscreen = useCallback(() => {
    void toggleNativeFullscreen().then((result) => {
      if (!result.ok)
        emitFutureEvent("toast", { message: t("fullscreenFailed"), tone: "error" });
    });
  }, [t]);
  /** Whether the "add a conversation" dialog is up. */
  const [addOpen, setAddOpen] = useState(false);
  /**
   * Whether the add tile's own menu is up: the recent list, plus the two ways out of it.
   *
   * A menu rather than jumping straight into the three-column dialog, because the question
   * a free slot asks is "which conversation", and the answer is usually one of the five the
   * reader was just in — not "browse every workspace on every machine".
   */
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const preferences = useMemo(() => readVibePreferences(), []);
  const [order, setOrder] = useState<string[]>(preferences.order);
  const [columns, setColumns] = useState<number | null>(preferences.columns);
  /** Each conversation's second-column choice (`readingPanel.ts`), remembered by key. */
  const [panelPrefs, setPanelPrefs] = useState<Record<string, string>>(preferences.panel);
  /**
   * The sticky `hidden`/`auto` choice for the second column.
   *
   * What makes "close it here, it stays closed in the next cell" true: a conversation with
   * no entry of its own follows this, instead of falling back to `auto` every time
   * (`readingPanel.ts` explains why `pinned` is not sticky).
   */
  const [panelDefault, setPanelDefault] = useState<"auto" | "hidden">(preferences.panelDefault);
  /** The right column's tab, for the panel's own reading layer. */
  const [contextTab, setContextTab] = useState<ContextTab>("files");
  const windowWidth = useWindowWidth();
  const [size, setSize] = useState({ height: 0, width: 0 });
  const areaRef = useRef<HTMLDivElement>(null);
  /** The panel's own root, so a cell's transcript is looked up inside it and nowhere else. */
  const rootRef = useRef<HTMLDivElement>(null);

  // Members: pinned conversations of the machines the filter lets through, this
  // one's and paired hosts' alike (`vibeCells.ts` owns the rule, `order.ts` the
  // order — two questions, two inputs).
  const [deviceFilter, setDeviceFilter] = useState<VibeDeviceFilter>(preferences.device);
  const workspaceNames = useMemo(
    () => new Map(workspaces.map(workspace => [workspace.id, workspace.name])),
    [workspaces],
  );
  const peerLabels = useMemo(
    () => new Map(peers.map(peer => [peer.desktopId, peer.name || peer.desktopId])),
    [peers],
  );
  const members = useMemo(
    () => vibeCellsOf(threads, catalogs, deviceFilter, workspaceNames),
    [catalogs, deviceFilter, threads, workspaceNames],
  );
  const memberByKey = useMemo(() => new Map(members.map(cell => [cell.key, cell])), [members]);
  const resolved = useMemo(
    () => resolveOrder(members.map(cell => ({ createdAt: cell.createdAt, key: cell.key })), order),
    [members, order],
  );
  const cells = useMemo(
    () => resolved.map(key => memberByKey.get(key)).filter((cell): cell is VibeCell => Boolean(cell)),
    [memberByKey, resolved],
  );
  /**
   * Which page of the grid the reader is on.
   *
   * Past twelve pinned conversations the grid pages rather than scrolling or shrinking
   * (`emptySlot.ts`): twelve is what fits at a readable size, and a thirteenth pinned
   * conversation used to be silently unreachable.
   */
  const [pageIndex, setPageIndex] = useState(0);
  const pages = pageCount(cells.length, PAGE_SIZE);
  const page = clampPage(pageIndex, cells.length, PAGE_SIZE);
  const shown = pageSlice(cells, page, PAGE_SIZE);
  /**
   * The add tile: it fills a **free square** of the shape the cells already need, which is
   * where a reader looks when a corner of the grid is empty.
   *
   * That rule is why it does not disturb the layout table: three cells are 2×2 with one
   * square free, so the tile takes it; four cells are a full 2×2 and get none (the menu's
   * add row is the way in there). A tile that always existed would make every grid one slot
   * bigger than its cell count and change the shape of four cells from 2×2 to 3×2.
   */
  const cellLayout = useMemo(
    () => gridOf(Math.max(1, shown.length), size.width || window.innerWidth, size.height || window.innerHeight, columns),
    [columns, shown.length, size.height, size.width],
  );
  const addSlot = hasAddSlot(shown.length, PAGE_SIZE)
    && cellLayout.columns * cellLayout.rows > shown.length;
  const slotCount = shown.length + (addSlot ? 1 : 0);

  // Focus is kept by key, not by index: a reorder, a new pin or an unpin must
  // not move the ring onto a different conversation than the one the reader was
  // looking at.
  const focusIndex = useMemo(() => {
    if (!focusKey)
      return 0;
    if (focusKey === ADD_SLOT_KEY)
      return addSlot ? shown.length : 0;
    const index = shown.findIndex(cell => cell.key === focusKey);
    return index === -1 ? 0 : index;
  }, [addSlot, focusKey, shown]);
  // The ring can be on the add tile, which is a slot and not a conversation: `focused`
  // stays the conversation (or nothing), and `addFocused` says whether the tile has it.
  const focused = focusIndex < shown.length ? shown[focusIndex]! : null;
  const focusedKey = focused?.key ?? null;
  const addFocused = addSlot && focusIndex === shown.length;
  // The focused *local* conversation, if that is what the ring is on: the panel's
  // own approval queue and its input layer are this machine's, and a host's
  // conversation answers its own approvals inside its own cell.
  const focusedLocal = focused?.thread ?? null;
  /**
   * The focused conversation's settings, for the read-only line and the menu's rows.
   *
   * `null` when the ring is on a host's conversation: those are that machine's, and
   * the panel does not pretend to change them from here (its cell says so by having
   * no settings line at all).
   */
  const focusedSettings = useVibeCellSettings(focusedLocal, {
    approvalTier: view.approvalTier,
    modelId: view.modelId,
    modelOptions: view.modelOptions,
    thinkingLevel: view.thinkingLevel,
  });

  // With the tile in place the shape is the one the cells already implied (the tile only
  // fills a square that was empty), so this equals `cellLayout` whenever the tile exists.
  const layout = useMemo(
    () => gridOf(Math.max(1, slotCount), size.width || window.innerWidth, size.height || window.innerHeight, columns),
    [columns, size.height, size.width, slotCount],
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
  const { activeApproval, decideApproval } = useApprovals(layer === "grid" ? null : focusedLocal?.id ?? null);

  /**
   * Which cells need a human, and why.
   *
   * Computed for the cells on screen only: an attention queue that pointed at a
   * conversation the panel is not showing would be a promise the panel cannot keep
   * (`attention.ts` has the rule, and why "running" is not in it).
   */
  const attention = useMemo(() => {
    const reasons = new Map<string, VibeAttentionReason>();
    for (const cell of shown) {
      const thread = cell.thread;
      // A host's answer is narrower than this machine's: its catalogue reports whether
      // a run is in flight but not a failure or an unread mark, and its approvals
      // arrive *at the cell* that draws them (`VibeRemoteCell`), not in the rail's
      // count. So a remote cell can be busy, and that is not attention — the rule is
      // the same one as here, applied to the facts the source actually has.
      const reason = thread
        ? attentionOf({
            approvals: approvalCounts.get(thread.id) ?? 0,
            runStatus: runStatuses[thread.id]?.status ?? null,
            unread: unreadThreadIds.has(thread.id),
          })
        : attentionOf({ approvals: 0, runStatus: null, unread: false });
      if (reason)
        reasons.set(cell.key, reason);
    }
    return reasons;
  }, [approvalCounts, runStatuses, shown, unreadThreadIds]);
  const attentionCount = attention.size;

  /**
   * The send queue: one held sentence per conversation, handed back — never sent —
   * when that conversation stops running (`vibeQueue.ts`, `VIBE.md` §5.4).
   *
   * `runStatus` is read through the same projection the cells' headers draw, so "still
   * running" means the same thing on the badge and in the queue.
   */
  const queue = useVibeQueue(
    (threadId) => {
      const status = runStatuses[threadId]?.status;
      return status ?? undefined;
    },
    // Changes whenever any run's status does — the one thing the hand-back check needs
    // to know that a conversation stopped running.
    useMemo(() => Object.values(runStatuses).map(status => status?.status ?? "").join(","), [runStatuses]),
  );

  /** "Remove from the panel" is "unpin", on whichever machine owns the cell. */
  const removeFromPanel = useCallback((cell: VibeCell) => {
    if (cell.thread)
      onPinLocal(cell.thread.id, false);
    else if (cell.desktopId && cell.session)
      onPinRemote(cell.desktopId, cell.session.sessionId, false);
  }, [onPinLocal, onPinRemote]);

  /**
   * The focused conversation's second-column choice, and whether it is on screen now.
   *
   * Asked three times per render (the render, the keys, the menu row) from one rule, so a
   * key and the thing it changes cannot disagree.
   */
  const panelPreference: ReadingPanelPreference = readPanelPreference(panelPrefs, focusedKey ?? "", panelDefault);
  const panelVisible = layer === "read" && showReadingPanel(panelPreference, windowWidth);

  const setPanelPreference = useCallback((next: ReadingPanelPreference) => {
    if (!focusedKey)
      return;
    // One write for both facts: this conversation's choice, and the default the next
    // conversation follows. Splitting them would let a relaunch land between the two.
    const stickyDefault = nextStickyDefault(panelDefault, next);
    setPanelPrefs(current => ({ ...current, [focusedKey]: next }));
    setPanelDefault(stickyDefault);
    writeVibePanel(focusedKey, next, stickyDefault);
  }, [focusedKey, panelDefault]);

  const persistOrder = useCallback((next: string[]) => {
    setOrder(next);
    writeVibeOrder(next);
  }, []);

  const setFocus = useCallback((cell: VibeCell | null) => {
    setFocusKey(cell?.key ?? null);
  }, []);

  const moveFocus = useCallback((direction: "up" | "down" | "left" | "right") => {
    if (!shown.length)
      return;
    const next = moveInGrid(focusIndex, direction, slotCount, layout);
    if (next === focusIndex)
      return;
    // The last slot can be the add tile, which is a slot and not a conversation: pointing
    // the ring at it is how `⏎` reaches the picker.
    const cell = shown[next];
    setFocusKey(cell ? cell.key : ADD_SLOT_KEY);
  }, [focusIndex, layout, shown, slotCount]);

  const stepFocus = useCallback((delta: number) => {
    if (slotCount === 0)
      return;
    const next = Math.min(slotCount - 1, Math.max(0, focusIndex + delta));
    if (next === focusIndex)
      return;
    const cell = shown[next];
    setFocusKey(cell ? cell.key : ADD_SLOT_KEY);
  }, [focusIndex, shown, slotCount]);

  /**
   * Turn the page, keeping the ring inside it.
   *
   * The same index within the page rather than "the first cell": a reader walking a grid by
   * position expects the second column of page two to be where the second column was. An
   * out-of-range index falls back to the last cell on that page.
   */
  const turnPage = useCallback((delta: -1 | 1) => {
    const next = clampPage(page + delta, cells.length, PAGE_SIZE);
    if (next === page)
      return;
    setPageIndex(next);
    setFocusKey(null);
  }, [cells.length, page]);

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
  // `OK` on a cell opens its **input layer** for this machine's conversations, and
  // its reading layer for a host's: dictation is this machine's (`chrome="input"`
  // mounts a composer, and a remote cell deliberately has none — see `VIBE.md` §5).
  useCommand("vibe.open", () => {
    // `OK` on the add tile asks the picker, which is what that square promises.
    if (addFocused) {
      setAddMenuOpen(true);
      return;
    }
    if (!focused)
      return;
    if (focused.thread)
      queue.acknowledge(focused.thread.id);
    setLayer(focused.kind === "local" ? "input" : "read");
  }, onGrid);
  useCommand("vibe.close", onClose, onGrid);
  useCommand("vibe.cell", (binding) => {
    const digit = Number.parseInt((binding?.code ?? "").replace("Digit", ""), 10);
    if (!Number.isInteger(digit) || digit < 1)
      return;
    // `1`–`9` address slots, so a digit past the last cell lands on the add tile when it is
    // showing — the same "type the number printed on it" rule for both kinds of square.
    if (digit === slotCount && addSlot) {
      setFocusKey(ADD_SLOT_KEY);
      return;
    }
    const target = shown[digit - 1];
    if (target)
      setFocus(target);
  }, onGrid);

  // ── Reading layer ─────────────────────────────────────────────────────────
  const onRead = layer === "read";
  useCommand("vibe.back", () => setLayer("grid"), onRead);
  useCommand("vibe.zoom.previous", () => stepFocus(-1), onRead);
  useCommand("vibe.zoom.next", () => stepFocus(1), onRead);
  // The second column: `⌘\` shows/hides it, `⌘⇧\` pins it. Both were in the table from the
  // first day and had no handler until the column existed — which is why they were never
  // drawn in the hint line (`VIBE.md` §9).
  /**
   * Reading a cell without zooming into it: `PageUp` / `PageDown` move its transcript by a
   * page, and the ring stays where it is. The wheel does the same thing with a pointer;
   * these keys are what makes it work from the keyboard (and from a remote control, if it
   * has page keys — `VIBE-REMOTE-KEYS.md` records whether it does).
   */
  const scrollFocused = useCallback((direction: -1 | 1) => {
    scrollTranscript(transcriptOf(rootRef.current, focusedKey), direction);
  }, [focusedKey]);
  useCommand("vibe.scroll.up", () => scrollFocused(-1));
  useCommand("vibe.scroll.down", () => scrollFocused(1));
  useCommand("vibe.zoom.scroll.up", () => scrollFocused(-1), layer === "read");
  useCommand("vibe.zoom.scroll.down", () => scrollFocused(1), layer === "read");
  // The remote's `Home` (and the same command's documented meaning): back to the newest
  // message, which is where a reader who has scrolled away usually wants to be.
  useCommand("vibe.home", () => scrollToLiveEnd(transcriptOf(rootRef.current, focusedKey)));
  // Turning the grid's page, registered only when there is more than one — the panel's rule
  // for every command: a key that could not do anything stays the app's own.
  useCommand("vibe.page.previous", () => turnPage(-1), pages > 1);
  useCommand("vibe.page.next", () => turnPage(1), pages > 1);
  useCommand("vibe.panel.hide", () => setPanelPreference(toggleHidden(panelPreference)));
  useCommand("vibe.panel.pin", () => setPanelPreference(togglePinned(panelPreference)));
  useCommand("vibe.input.open", () => {
    if (focusedLocal)
      queue.acknowledge(focusedLocal.id);
    setLayer("input");
  }, onRead && focused?.kind === "local");

  // ── Input layer ───────────────────────────────────────────────────────────
  // Sending is **not** here: `⏎` is the composer's own fixed key, which already
  // stands down while an input method composes (`MentionEditor.onSubmit`), and the
  // editor hands an empty draft to this layer's onLeaveInput instead. Closing the
  // layer is the panel's own.
  useCommand("vibe.input.close", () => setLayer("read"), layer === "input");
  /**
   * Take the held sentence back into the box.
   *
   * A separate command from "leave the layer" on purpose: leaving the input layer is how
   * a reader stops dictating, and taking a queued sentence back is how they change their
   * mind about one message. One key doing both would make `Esc` mean two different things
   * depending on a state the reader cannot read off the key.
   *
   * Registered only while something is held — the panel's rule for every command — and
   * in the input layer, because that is where the sentence was dictated.
   */
  useCommand("vibe.queue.cancel", () => {
    if (!focusedLocal)
      return;
    // A refusal means the box already holds something the reader typed since; the entry
    // stays visible and cancellable rather than overwriting it.
    if (queue.cancel(focusedLocal.id) === "busy")
      emitFutureEvent("toast", { message: t("queue.held"), tone: "info" });
  }, layer === "input" && queue.queued.has(focusedLocal?.id ?? ""));

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
  useCommand("vibe.fullscreen", toggleFullscreen);
  useCommand("vibe.remove", () => {
    if (focused)
      removeFromPanel(focused);
  });
  // The remote's `Menu`. It is also the input layer's way out of the editor's
  // chrome, which is why it is one of the few commands allowed to run over it.
  useCommand("vibe.menu", () => setMenuOpen(true));
  // The panel's own "add a conversation", for the pointer and the keyboard: the
  // dialog is a three-column list, which a remote's four arrows cannot drive.
  useCommand("vibe.add", () => setAddOpen(true));
  /**
   * Jump to the next conversation that needs a human.
   *
   * Registered **only while something does**, which is the rule the panel's other
   * keys follow (`chat.stop`): with nothing waiting, the key must stay unclaimed
   * rather than swallow a press for a no-op. Landing goes to the reading layer, not
   * just to the ring, because the whole question is "what does this want from me" —
   * one more press to find out would make the queue pointless.
   */
  useCommand("vibe.attention", () => {
    const next = nextAttention(shown.map(cell => cell.key), attention, focusedKey);
    const target = next ? shown.find(cell => cell.key === next) : null;
    if (!target)
      return;
    setFocus(target);
    setLayer("read");
  }, attentionCount > 0);

  /**
   * Close the menu and put the caret back where it was.
   *
   * The menu takes focus when it opens (a key opened it; the next key should land
   * in it), so closing it from the input layer would otherwise leave the caret
   * nowhere: `⏎` would send nothing and the next dictation would go to the
   * document. The composer already answers `focus-composer` — the same request the
   * terminal's collapse makes — so the panel asks in the same vocabulary instead of
   * growing a second way to focus an editor it does not own.
   */
  const closeMenu = useCallback(() => {
    setMenuOpen(false);
    if (layer === "input")
      emitFutureEvent("focus-composer", undefined);
  }, [layer]);

  const workspacePathOf = useCallback(
    (thread: StoredThread) => workspaces.find(workspace => workspace.id === thread.workspaceId)?.path ?? null,
    [workspaces],
  );

  const threadView = (thread: StoredThread, chrome: AgentThreadChrome, ownsCommands: boolean) => (
    <VibeThreadView
      activeApproval={layer === "grid" ? null : activeApproval}
      chrome={chrome}
      focused={ownsCommands}
      // Queued, never sent: the composer hands the sentence over and clears its box
      // when this returns true. A refusal means one is already held, and the text then
      // stays exactly where the reader can see it.
      onQueue={payload => queue.enqueue(thread.id, {
        attachments: payload.attachments,
        text: payload.content,
      }) === "queued"}
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

  /** A cell's body: this machine's conversation, or a host's. */
  const cellBody = (cell: VibeCell, ownsCommands: boolean) => {
    if (cell.thread)
      return threadView(cell.thread, "read", ownsCommands);
    const catalog = catalogs.find(item => item.desktopId === cell.desktopId);
    if (!catalog || !cell.session)
      return null;
    return (
      <VibeRemoteCell
        catalog={catalog}
        onOpenInShell={() => onOpenRemoteInShell(catalog.desktopId, cell.session!.sessionId)}
        peer={peers.find(peer => peer.desktopId === catalog.desktopId)}
        session={cell.session}
      />
    );
  };

  const arrows = ["vibe.focus.left", "vibe.focus.up", "vibe.focus.down", "vibe.focus.right"]
    .map(id => bindingLabel(id as never, overrides, isMac))
    .filter(Boolean)
    .join(" ");
  // The menu's key(s), shown beside the arrows in the grid: on a remote the menu
  // button is the only way to reach the panel's configuration, so it has to be
  // visible before the reader knows to look for it.
  const menuKeys = bindingLabel("vibe.menu", overrides, isMac);

  /**
   * What the menu offers, in the order a remote reaches it.
   *
   * Built here, not in `VibeMenu`: the panel owns what is configurable, the menu
   * owns how a menu behaves. Every row is real — a row that did nothing would
   * teach the reader that the menu is decoration, which on a seven-key device is
   * the end of the feature.
   */
  const machines = [
    { id: "local", label: t("devices.local") },
    ...catalogs.map(catalog => ({ id: catalog.desktopId, label: peerLabels.get(catalog.desktopId) ?? catalog.desktopId })),
  ];
  // "All machines" is a real choice, not the absence of one: it is what the panel
  // shows before anyone filters, and the row shows it by name.
  const deviceOrder: VibeDeviceFilter[] = [
    { kind: "all" },
    ...machines.map(machine => ({ desktopId: machine.id, kind: "device" as const })),
  ];
  const deviceIndex = deviceOrder.findIndex((entry) => {
    if (entry.kind === "all")
      return deviceFilter.kind === "all";
    return deviceFilter.kind === "device" && deviceFilter.desktopId === entry.desktopId;
  });
  const deviceLabel = (entry: VibeDeviceFilter) => {
    if (entry.kind === "all")
      return t("devices.all");
    return machines.find(machine => machine.id === entry.desktopId)?.label ?? entry.desktopId;
  };
  const chooseDevice = (entry: VibeDeviceFilter) => {
    setDeviceFilter(entry);
    writeVibeDevice(entry);
  };

  /**
   * The add tile's menu: the five conversations the reader was in most recently, then the
   * two ways out of the list.
   *
   * `recentChoices` already excludes what the panel shows, so every row here is an action
   * that changes something. Adding is pinning, on the machine that owns the conversation —
   * the same call the browse dialog makes.
   */
  const addMenuRows: VibeMenuRow[] = [
    ...recentChoices(threads, catalogs, new Set(members.map(cell => cell.key)), peerLabels)
      .map((choice, index) => ({
        id: `recent:${choice.key}`,
        label: choice.title,
        value: choice.machine ?? undefined,
        group: index === 0,
        onRun: () => {
          const cell = cells.find(item => item.key === choice.key);
          const threadId = choice.key.startsWith("local:") ? choice.key.slice("local:".length) : null;
          if (threadId) {
            onPinLocal(threadId, true);
          }
          else {
            const [desktopId, sessionId] = choice.key.slice("remote:".length).split(":");
            if (desktopId && sessionId)
              onPinRemote(desktopId, sessionId, true);
          }
          if (cell)
            setFocus(cell);
          setAddMenuOpen(false);
        },
      })),
    {
      id: "add:browse",
      label: recentChoices(threads, catalogs, new Set(members.map(cell => cell.key)), peerLabels).length > 0
        ? t("add.browse")
        : t("empty.title"),
      group: true,
      onRun: () => {
        setAddOpen(true);
        setAddMenuOpen(false);
      },
    },
    {
      id: "add:new",
      label: t("add.newConversation"),
      description: t("add.newConversationHint"),
      onRun: () => {
        setAddMenuOpen(false);
        onNewConversation();
      },
    },
  ];

  const menuRows: VibeMenuRow[] = [
    // What needs a human, first: on a remote this is the screen's opening question,
    // and a menu whose first row is "columns" answers something nobody asked.
    ...(attentionCount > 0
      ? shown
          .filter(cell => attention.has(cell.key))
          .map((cell, index) => ({
            id: `attention:${cell.key}`,
            label: cell.title,
            value: t(`attention.${attention.get(cell.key)!}`),
            group: index === 0,
            onRun: () => {
              setFocus(cell);
              setLayer("read");
              closeMenu();
            },
          }))
      : [{
          id: "attention:none",
          label: t("attention.none"),
          // Disabled rather than absent: "nothing is waiting" is the answer to the
          // question the reader opened the menu to ask.
          disabled: true,
          group: true,
          onRun: () => {},
        }]),
    {
      id: "add",
      label: t("menu.add"),
      group: true,
      onRun: () => {
        setAddOpen(true);
        closeMenu();
      },
      description: t("menu.addHint"),
    },
    {
      id: "device",
      label: t("devices.label"),
      value: deviceLabel(deviceOrder[deviceIndex] ?? { kind: "all" }),
      onRun: () => chooseDevice({ kind: "all" }),
      // ←/→ walks the machines, wrapping: this computer, then each paired host.
      onAdjust: (delta) => {
        const at = deviceIndex === -1 ? 0 : deviceIndex;
        chooseDevice(deviceOrder[(at + delta + deviceOrder.length) % deviceOrder.length]!);
      },
    },
    {
      id: "layout",
      label: t("menu.layout"),
      value: columns === null ? t("menu.auto") : t("menu.columns", { count: columns }),
      onRun: () => {
        setColumns(null);
        writeVibeColumns(null);
      },
      // 1 → 2 → 3 → 4 → automatic, wrapping. Automatic is a real choice, not the
      // absence of one: it is what follows the window on the next resize.
      onAdjust: (delta) => {
        const order: Array<number | null> = [null, 1, 2, 3, 4];
        const current = order.indexOf(columns);
        const next = order[(current + delta + order.length) % order.length] ?? null;
        setColumns(next);
        writeVibeColumns(next);
      },
      description: t("menu.layoutHint"),
    },
    // This cell's settings: what used to be four dropdowns in the middle of a
    // dictated sentence. `←`/`→` cycles each value — the same gesture as the layout
    // and device rows, so the menu teaches one grammar rather than four.
    ...(focusedLocal
      ? [
          {
            id: "model",
            label: t("menu.model"),
            value: focusedSettings.modelName,
            onRun: () => {},
            onAdjust: (delta: -1 | 1) => {
              const ids = view.modelOptions.map(option => modelKey(option));
              if (ids.length === 0)
                return;
              const at = ids.indexOf(focusedSettings.modelId);
              const next = ids[((at === -1 ? 0 : at) + delta + ids.length) % ids.length]!;
              void setVibeCellModel(focusedLocal.id, next, view.modelOptions);
            },
          },
          ...(focusedSettings.supportsThinking
            ? [{
                id: "thinking",
                label: t("menu.thinking"),
                value: thinkingLevelName(focusedSettings.thinkingLevel),
                onRun: () => {},
                onAdjust: (delta: -1 | 1) => {
                  const levels = modelThinkingLevels(focusedSettings.modelId, view.modelOptions);
                  const at = levels.indexOf(focusedSettings.thinkingLevel);
                  const next = levels[((at === -1 ? 0 : at) + delta + levels.length) % levels.length]!;
                  void setVibeCellThinkingLevel(focusedLocal.id, next);
                },
              }]
            : []),
          {
            id: "approval",
            label: t("menu.approval"),
            value: t(`agent:composer.approvalTier.${focusedSettings.approvalTier}`),
            onRun: () => {},
            onAdjust: (delta: -1 | 1) => {
              // Application-wide, not this cell's: the label says so, and the row
              // writing it goes through the same setter the main window uses.
              const tiers: ApprovalTier[] = ["manual", "sandbox", "auto", "off"];
              const at = tiers.indexOf(focusedSettings.approvalTier);
              view.onChangeApprovalTier(tiers[((at === -1 ? 0 : at) + delta + tiers.length) % tiers.length]!);
            },
            description: t("menu.approvalHint"),
          },
        ]
      : []),
    // The reading layer's second column: a value row, so "is this one's files open, and
    // will it stay open" is answerable without pressing anything.
    {
      id: "panel",
      label: t("menu.panel"),
      value: t(preferenceLabelKey(panelPreference)),
      onRun: () => setPanelPreference(toggleHidden(panelPreference)),
      onAdjust: delta => setPanelPreference(cyclePreference(panelPreference, delta)),
      description: t("menu.panelHint"),
    },
    // Taking a held sentence back, when there is one. A row rather than only a key:
    // the remote's reader is the one this queue exists for, and `Menu` is how they
    // reach anything that is not on a direction key.
    ...(focusedLocal && queue.queued.has(focusedLocal.id)
      ? [{
          id: "queue-cancel",
          label: t("queue.cancel"),
          group: true,
          onRun: () => {
            queue.cancel(focusedLocal.id);
            closeMenu();
          },
        }]
      : []),
    {
      id: "move-earlier",
      label: t("menu.moveEarlier"),
      disabled: !focusedKey || focusIndex === 0,
      onRun: () => focusedKey && persistOrder(moveInOrder(resolved, focusedKey, -1)),
    },
    {
      id: "move-later",
      label: t("menu.moveLater"),
      disabled: !focusedKey || focusIndex >= shown.length - 1,
      onRun: () => focusedKey && persistOrder(moveInOrder(resolved, focusedKey, 1)),
    },
    {
      id: "remove",
      group: true,
      label: t("menu.remove"),
      disabled: !focused,
      onRun: () => focused && removeFromPanel(focused),
      description: t("menu.removeHint"),
    },
    {
      id: "open-in-shell",
      label: t("menu.openInShell"),
      disabled: !focused,
      onRun: () => {
        if (!focused)
          return;
        if (focused.thread)
          onOpenInShell(focused.thread);
        else if (focused.desktopId && focused.session)
          onOpenRemoteInShell(focused.desktopId, focused.session.sessionId);
      },
    },
    {
      id: "fullscreen",
      group: true,
      label: t("menu.fullscreen"),
      onRun: toggleFullscreen,
    },
    {
      id: "close",
      label: t("menu.close"),
      onRun: onClose,
    },
  ];

  /**
   * The grid: one cell per member, in order, left to right.
   *
   * `exclude` is the input layer's: the conversation being dictated into is drawn
   * by the layer itself (with its composer), so leaving it in the backdrop too
   * would mount it twice — two subscriptions, two history reads, one truth.
   */
  function grid(excludeKey: string | null, dimmed: boolean) {
    const visible = shown.filter(cell => cell.key !== excludeKey);
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
        {/* The add tile: the first free square, which is where a reader looks when a corner
            of the grid is empty. It is a slot in the ring, so `→`/`↓` reaches it and `⏎`
            opens the picker, exactly like a cell (`emptySlot.ts`). */}
        {excludeKey === null && addSlot
          ? (
              <button
                className={cn(
                  "flex min-h-0 min-w-0 flex-col items-center justify-center gap-2 rounded-lg border border-dashed text-ink-soft transition-colors",
                  addFocused
                    ? "border-accent bg-accent-soft/30 text-ink ring-2 ring-accent/35"
                    : "border-line hover:bg-surface-subtle hover:text-ink",
                )}
                data-vibe-add-slot=""
                // Placed after the last cell: the grid fills left to right, so the tile
                // lands in the next free square of the current page. Both axes are stated
                // (`gridAreaOf`) — a column alone leaves the row to auto-placement.
                style={gridAreaOf(visible.length, layout)}
                onMouseDown={() => setFocusKey(ADD_SLOT_KEY)}
                onClick={() => setAddMenuOpen(true)}
                type="button"
              >
                <span aria-hidden className="text-2xl leading-none">+</span>
                <span className="max-w-[80%] truncate text-xs">{t("emptyTile")}</span>
              </button>
            )
          : null}
        {visible.map((cell, index) => {
          const focusedCell = cell.key === focusedKey;
          const approvals = cell.thread ? approvalCounts.get(cell.thread.id) ?? 0 : 0;
          return (
            <section
              className={cn(
                "flex min-h-0 min-w-0 flex-col overflow-hidden rounded-lg border bg-surface",
                focusedCell
                  ? "border-accent shadow-panel ring-2 ring-accent/35"
                  : "border-line-soft",
              )}
              data-vibe-cell={index + 1}
              data-vibe-cell-kind={cell.kind}
              key={cell.key}
              onMouseDown={dimmed
                ? undefined
                : () => {
                    setFocus(cell);
                    setLayer("read");
                  }}
              style={gridAreaOf(index, layout)}
            >
              <header
                className={cn(
                  "flex h-7 shrink-0 items-center gap-2 border-b px-2 text-xs",
                  focusedCell ? "border-accent/40 bg-accent-soft/40" : "border-line-soft/60",
                )}
                onDoubleClick={dimmed
                  ? undefined
                  : () => (cell.thread ? onOpenInShell(cell.thread) : cell.desktopId && cell.session && onOpenRemoteInShell(cell.desktopId, cell.session.sessionId))}
                title={cell.title}
              >
                <span className="font-mono text-[11px] text-ink-faint">{index + 1}</span>
                <span className="min-w-0 flex-1 truncate font-medium">{cell.title}</span>
                {cell.desktopId
                  ? (
                      <span className="shrink-0 truncate text-[10px] text-ink-muted">
                        {peerLabels.get(cell.desktopId) ?? cell.desktopId}
                      </span>
                    )
                  : null}
                {cell.thread && isRunning(runStatuses[cell.thread.id])
                  ? <span className="size-1.5 shrink-0 animate-pulse rounded-full bg-accent" title={t("running")} />
                  : null}
                {approvals > 0
                  ? <Badge className="h-5 px-1.5" tone="warning">{approvals}</Badge>
                  : null}
                {cell.thread && unreadThreadIds.has(cell.thread.id)
                  ? <span className="size-1.5 shrink-0 rounded-full bg-info" title={t("unread")} />
                  : null}
                {/* A held sentence, said out loud. "I said it but cannot see it" is the
                    failure this marker exists to prevent (§5.4). */}
                {cell.thread && queue.queued.has(cell.thread.id)
                  ? (
                      <span
                        className="shrink-0 rounded bg-warning-soft px-1 text-[10px] font-medium text-warning"
                        data-vibe-queued=""
                        title={t("queue.queued")}
                      >
                        {t("queue.queued")}
                      </span>
                    )
                  : null}
                {/* …and the moment it goes back into the box, which is the one thing the
                    reader must not miss: the message is now theirs to send again. */}
                {cell.thread && queue.returned.has(cell.thread.id)
                  ? (
                      <span
                        className="shrink-0 rounded bg-info-soft px-1 text-[10px] font-medium text-info"
                        data-vibe-returned=""
                        title={t("queue.returned")}
                      >
                        {t("queue.returned")}
                      </span>
                    )
                  : null}
                {/* Why this cell is in the attention queue. The approval badge above
                    already counts approvals; this is the other two reasons, and it is
                    the only place a *failed* run says so without opening the cell. */}
                {(() => {
                  const reason = attention.get(cell.key);
                  return reason === "failed" || reason === "finished"
                    ? (
                        <span
                          className={cn(
                            "shrink-0 rounded px-1 text-[10px] font-medium",
                            reason === "failed"
                              ? "bg-danger-soft text-danger"
                              : "bg-info-soft text-info",
                          )}
                          data-vibe-attention-reason={reason}
                          title={t(`attention.${reason}`)}
                        >
                          {t(`attention.${reason}`)}
                        </span>
                      )
                    : null;
                })()}
              </header>
              <div className="min-h-0 flex-1" data-vibe-transcript-for={cell.key}>
                {cellBody(cell, false)}
              </div>
            </section>
          );
        })}
      </div>
    );
  }

  /**
   * The reading layer's keys, by cell kind: a host's conversation has no input layer,
   * so the key that would open one is not offered there rather than being offered and
   * doing nothing.
   */
  const readHintIds: Parameters<typeof VibeHintBar>[0]["ids"] = focused?.kind === "local"
    ? ["vibe.zoom.previous", "vibe.zoom.scroll.down", "vibe.attention", "vibe.panel.hide", "vibe.input.open", "vibe.menu", "vibe.back"]
    : ["vibe.zoom.previous", "vibe.zoom.scroll.down", "vibe.attention", "vibe.panel.hide", "vibe.menu", "vibe.back"];

  return (
    <div
      ref={rootRef}
      className="fixed inset-0 z-40 flex flex-col bg-canvas text-ink"
      // Which conversation the ring is on, and which layer is on top. Two facts the
      // panel's own keys move, and the only ones a capture can assert the way a
      // reader sees them (a screenshot cannot show focus).
      data-vibe-focused={focused?.key ?? (addFocused ? ADD_SLOT_KEY : "")}
      data-vibe-layer={layer}
    >
      {/* The bar starts at x=0 and is the only surface in the app that does, so it makes
          the same two reservations the rail's header makes: on macOS, outside fullscreen,
          its content starts past the lights (80px) instead of under them — and it is the
          same height, so the text lands on the line every other top bar and the lights
          already share. It is also the only place left to drag the window from while the
          panel covers everything. */}
      <header
        className={cn(
          "flex shrink-0 select-none items-center gap-3 border-b border-line-soft/60 pr-3",
          BAR_HEIGHT,
          barStart(shouldReserveTrafficLights(isMac, isFullscreen)),
        )}
        data-vibe-drag="bar"
        onMouseDown={startWindowDrag}
      >
        <span className="text-xs font-semibold tracking-wide text-ink-soft">{t("title")}</span>
        <span className="text-xs text-ink-soft">
          {t("count", { count: cells.length })}
        </span>
        {/* Which page, only when there is more than one: a thirteenth pinned conversation
            used to be unreachable and nothing said so. */}
        {pages > 1
          ? (
              <span className="text-xs font-medium text-ink-muted" data-vibe-page="">
                {t("page", { page: page + 1, pages })}
              </span>
            )
          : null}
        {deviceFilter.kind === "device"
          ? (
              <span className="text-xs text-ink-muted" data-vibe-device={deviceFilter.desktopId}>
                {deviceLabel(deviceFilter)}
              </span>
            )
          : null}
        {attentionCount > 0
          ? (
              <span className="text-xs font-medium text-warning" data-vibe-attention="">
                {t("attention.badge", { count: attentionCount })}
              </span>
            )
          : null}
        <span className="font-mono text-xs text-ink-faint">
          {layout.columns}
          ×
          {layout.rows}
          {columns === null ? "" : ` ${t("manualLayout")}`}
        </span>
        {layer === "grid"
          ? (
              <span className="font-mono text-xs text-ink-faint">
                {arrows}
                {"  "}
                {menuKeys}
              </span>
            )
          : null}
        <div className="ml-auto flex items-center gap-1">
          <IconButton
            className="size-7"
            icon={<Expand className="size-3.5" />}
            label={t("fullscreen")}
            onClick={toggleFullscreen}
          />
          {/* The menu, for the pointer (and for the first-time reader): the
              remote's own way in is its `Menu` button. */}
          <IconButton
            className="size-7"
            icon={<MoreHorizontal className="size-4" />}
            label={t("menu.title")}
            onClick={() => setMenuOpen(true)}
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
              <button
                className="mt-2 rounded-md border border-line-soft px-3 py-1 text-xs text-ink-soft hover:bg-surface-subtle"
                onClick={() => setAddOpen(true)}
                type="button"
              >
                {t("menu.add")}
              </button>
            </div>
          )
        : layer === "grid"
          ? (
              <div className="flex min-h-0 flex-1 flex-col">
                <div ref={areaRef} className="min-h-0 flex-1 p-2">{grid(null, false)}</div>
                {/* The grid's own hint line. It carries the scroll keys, which nothing
                    else on screen mentions — the ring keys are in the bar's key strip. */}
                <VibeHintBar
                  ids={pages > 1
                    ? ["vibe.open", "vibe.page.next", "vibe.scroll.down", "vibe.attention", "vibe.menu", "vibe.close"]
                    : ["vibe.open", "vibe.scroll.down", "vibe.attention", "vibe.menu", "vibe.close"]}
                />
              </div>
            )
          : layer === "read"
            ? (
                <div className="flex min-h-0 flex-1 flex-col p-2">
                  <div className="flex h-7 shrink-0 items-center gap-2 px-1 text-xs">
                    <span className="font-mono text-[11px] text-ink-faint">{focusIndex + 1}</span>
                    <span className="min-w-0 flex-1 truncate font-medium">{focused?.title ?? ""}</span>
                    {focused?.desktopId
                      ? <span className="text-[10px] text-ink-muted">{peerLabels.get(focused.desktopId) ?? focused.desktopId}</span>
                      : null}
                    {/* What this conversation runs on, read-only: the same line the
                        input layer shows, in the same place, so the value does not
                        move or vanish between the two layers (§5.6). */}
                    {focusedLocal ? <VibeSettingsLine settings={focusedSettings} /> : null}
                    <span className="ml-auto text-ink-faint">{t("of", { total: shown.length })}</span>
                  </div>
                  {/* One column, or two: the transcript keeps its own measure and the
                      column beside it is the *existing* panel — `ContextPanel` here,
                      `RemoteFilesPanel` for a host — so what the right side shows cannot
                      depend on which surface opened it (`VIBE.md` §6). */}
                  <div className="flex min-h-0 flex-1 gap-2">
                    <div
                      className="min-h-0 min-w-0 flex-1 overflow-hidden rounded-lg border border-line-soft bg-surface"
                      data-vibe-transcript-for={focused?.key ?? ""}
                      onDoubleClick={() => {
                        if (!focused)
                          return;
                        if (focused.thread)
                          onOpenInShell(focused.thread);
                        else if (focused.desktopId && focused.session)
                          onOpenRemoteInShell(focused.desktopId, focused.session.sessionId);
                      }}
                    >
                      {focused ? cellBody(focused, true) : null}
                    </div>
                    {panelVisible && focused
                      ? (
                          <div
                            className="h-full min-h-0 shrink-0 overflow-hidden rounded-lg border border-line-soft bg-surface"
                            data-vibe-panel-column=""
                            style={{ width: PANEL_WIDTH }}
                          >
                            {focused.thread
                              ? (
                                  <ContextPanel
                                    activeThread={focusedLocal}
                                    activeWorkspace={workspaces.find(workspace => workspace.id === focusedLocal?.workspaceId) ?? null}
                                    activeTab={contextTab}
                                    expanded
                                    // Fixed: the reading layer is a view of a conversation, not a
                                    // window with three resizable columns (`VIBE.md` §6).
                                    width={PANEL_WIDTH}
                                    onResizeNudge={() => {}}
                                    onResizeStart={() => {}}
                                    onTabChange={setContextTab}
                                    onToggleExpanded={() => setPanelPreference("hidden")}
                                  />
                                )
                              : (
                                  <RemoteFilesPanel
                                    desktopId={focused.desktopId!}
                                    expanded
                                    onToggleExpanded={() => setPanelPreference("hidden")}
                                    peer={peers.find(peer => peer.desktopId === focused.desktopId)}
                                    sessionId={focused.session!.sessionId}
                                  />
                                )}
                          </div>
                        )
                      : null}
                  </div>
                  <VibeHintBar ids={readHintIds} />
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
                        {focusedLocal ? <Badge className="h-5" tone="neutral">{t("input.local")}</Badge> : null}
                        {focusedLocal
                          ? <span className="ml-auto"><VibeSettingsLine settings={focusedSettings} /></span>
                          : null}
                        <span className={cn("font-mono text-[11px] text-ink-faint", !focusedLocal && "ml-auto")}>{arrows}</span>
                      </div>
                      <div className="min-h-0 flex-1">
                        {focusedLocal
                          ? (
                              <VibeThreadView
                                chrome="input"
                                focused
                                // The input layer draws its own state line and the
                                // menu changes the values (§5.6).
                                composerControls="none"
                                inputFooter={(
                                  <VibeHintBar
                                    fixed={[{ code: "Enter", labelKey: "fixed.send" }]}
                                    ids={queue.queued.has(focusedLocal?.id ?? "")
                                      ? ["vibe.queue.cancel", "vibe.input.close", "vibe.menu"]
                                      : ["vibe.input.close", "vibe.menu"]}
                                  />
                                )}
                                onApprovalDecision={async (approval, status) => {
                                  await decideApproval(approval, status);
                                  view.onApprovalDecided();
                                }}
                                onLeaveInput={() => setLayer("read")}
                                thread={focusedLocal}
                                view={view}
                                workspacePath={workspacePathOf(focusedLocal)}
                              />
                            )
                          : null}
                      </div>
                    </div>
                  </div>
                </div>
              )}

      {menuOpen
        ? <VibeMenu onClose={closeMenu} rows={menuRows} title={t("menu.title")} />
        : null}
      {addMenuOpen
        ? <VibeMenu onClose={() => setAddMenuOpen(false)} rows={addMenuRows} title={t("emptyTile")} />
        : null}
      {/* The key self-check: a read-only readout for filling in the remote-control matrix
          (`VIBE-REMOTE-KEYS.md`). It is not a command and consumes nothing — see
          `keyProbe.ts` — and a build without the opt-in renders nothing at all. */}
      <VibeKeyProbe />
      {addOpen
        ? (
            <AddToVibeDialog
              catalogs={catalogs}
              members={members}
              onAddLocal={thread => onPinLocal(thread.id, true)}
              onAddRemote={(desktopId, sessionId) => onPinRemote(desktopId, sessionId, true)}
              onClose={() => setAddOpen(false)}
              peerLabels={peerLabels}
              threads={threads}
              workspaces={workspaces}
            />
          )
        : null}
    </div>
  );
}

/**
 * One conversation as the panel draws it.
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
  composerControls,
  focused,
  onQueue,
  activeApproval,
  inputFooter,
  onLeaveInput,
  onApprovalDecision,
  view,
  workspacePath,
}: {
  thread: StoredThread;
  chrome: AgentThreadChrome;
  composerControls?: "full" | "none";
  focused: boolean;
  /** What a send means while the run is still going (see `Composer.onQueue`). */
  onQueue?: (payload: { attachments: ComposerSendPayload["attachments"]; content: string }) => boolean;
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
      composerControls={composerControls}
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
      onQueue={onQueue}
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
