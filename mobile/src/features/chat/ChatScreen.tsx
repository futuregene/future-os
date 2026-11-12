import { useStyles } from "./chatStyles";
import { useMinimumVisible } from "./useMinimumVisible";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { BackHandler, Keyboard, KeyboardAvoidingView, Platform, View } from "react-native";
import { SafeAreaView, useSafeAreaInsets } from "react-native-safe-area-context";
import { TimelineCard } from "./messages/TimelineCard";
import { PausedTimeline } from "../../components/PausedTimeline";
import { ErrorBanner } from "../../components/ErrorBanner";
import { StandbyNotice } from "../../components/StandbyNotice";
import { useRemote, useRemoteControls } from "../../remote/RemoteContext";
import { readableDeviceName } from "../../remote/deviceName";
import { modelReference, type MobileAttachment, type TimelineItem } from "../../remote/types";
import { spacing } from "../../theme/tokens";
import { useTransferProgress } from "./useTransferProgress";
import { File } from "expo-file-system";
import { mimeFor, pickAttachments, pickFromAlbum, takePhoto } from "../../remote/files";
import { useConversationOperation } from "./useConversationOperation";
import { useComposerSubmission } from "./composer/useComposerSubmission";
import { useComposerDraft } from "./useComposerDraft";
import { useAttachmentPicker } from "./useAttachmentPicker";
import { useChatFiles } from "./files/useChatFiles";
import { useMarkdownImageLoader } from "./useMarkdownImageLoader";
import { MarkdownImageLoaderContext } from "../../components/MarkdownImage";
import { ImageViewerContext, type ImageViewerTarget } from "../../components/imageViewer";
import { useChatScroll } from "./useChatScroll";
import { useQuestionNav } from "./useQuestionNav";
import { useTimelinePaging } from "./useTimelinePaging";
import { useCompactContext } from "./useCompactContext";
import { useSendMessage, sendFailureText, QUEUED_SEND_POLICY, type SendMessageIntent } from "./useSendMessage";
import { useSkillRecommendation } from "./useSkillRecommendation";
import { useDesktopResource } from "../settings/useDesktopResource";
import { ChatTimeline, type SyncNoticeState } from "./components/ChatTimeline";
import { ChatTopBar } from "./components/ChatTopBar";
import { SessionFilesPanel } from "./components/SessionFilesPanel";
import { ComposerDock } from "./components/ComposerDock";
import { AttachmentSourceMenu, type AttachmentSource } from "./components/AttachmentSourceMenu";
import type { SendQueueAttachmentItem } from "./components/SendQueuePanel";
import { ModelSelectorSheet } from "./components/ModelSelectorSheet";
import { DownloadProgressModal } from "./components/DownloadProgressModal";
import { PreviewModal } from "./components/PreviewModal";
import { ImageViewer } from "./components/ImageViewer";
import { SessionUsageSheet } from "./components/SessionUsageSheet";
import { NativeFileActionSheet } from "./components/NativeFileActionSheet";
import { newestFirst } from "./timelineListModel";
import { isParkedSessionQueueRow, sessionQueueNoticeReason, useSessionQueue, type SessionQueueAttachment, type SessionQueueRow } from "../../remote/sessionQueue";
import { showToast } from "./utils";

const SYNC_NOTICE_MIN_MS = 750;

export function ChatScreen() {
  const styles = useStyles();
  const { t, i18n } = useTranslation();
  const remote = useRemote();
  const controls = useRemoteControls();
  const connection = controls.connectionPresentation;
  const { closeConversation, decideApproval: submitApproval } = remote;
  const insets = useSafeAreaInsets();

  const composerOperation = useConversationOperation(remote);
  const { transferProgress, setTransferProgress } = useTransferProgress(composerOperation.scope);
  const [selector, setSelector] = useState<"model" | "thinking" | null>(null);
  const [filesSession, setFilesSession] = useState<string | null>(null);
  // The spend icon in the top bar opens the conversation's account book.
  const [usageOpen, setUsageOpen] = useState(false);
  const conversationKey = `${remote.credentials?.expectedDesktopId ?? ""}:${remote.selectedSessionId}`;
  const filesOpen = !remote.draft && filesSession === conversationKey;
  const goBack = useCallback(() => {
    if (filesOpen) setFilesSession(null);
    else {
      Keyboard.dismiss();
      closeConversation();
    }
  }, [filesOpen, closeConversation]);
  // Android edge-to-edge: the built-in KeyboardAvoidingView is a no-op here
  // (behavior is undefined on Android) and RN's KAV mis-measures the keyboard
  // under edge-to-edge, so we inset the chat flex column by the measured
  // keyboard height ourselves. iOS keeps relying on KAV padding.
  const [keyboardHeight, setKeyboardHeight] = useState(0);

  const title = remote.draft
    ? t("chat.new")
    : remote.selectedTitle || t("sessions.unnamed");
  const selectedSession = remote.sessions.find(
    (session) => session.sessionId === remote.selectedSessionId,
  );
  /**
   * Bring this conversation back out of the shelf, from where it is read.
   *
   * A shelved conversation opens with its composer locked, and the banner used to
   * send the reader to the desktop to undo that — the phone now has the same
   * restore the shelf's rows carry, so the one place a reader can be sure they
   * are looking at the right conversation is the place to offer it. The host
   * refuses a few trees (one with a running member), so a refusal is reported
   * rather than left looking like a dead button.
   */
  const [restoringArchived, setRestoringArchived] = useState(false);
  const restoreArchived = useCallback(() => {
    const session = selectedSession;
    if (!session?.threadId || restoringArchived) return;
    setRestoringArchived(true);
    void remote.restoreSession(session.sessionId, session.threadId)
      .catch(() => showToast(t("sessions.archiveFailed")))
      .finally(() => setRestoringArchived(false));
  }, [restoringArchived, remote, selectedSession, t]);
  const mode = remote.draft ? remote.draftMode : selectedSession?.mode;
  const workspaceId = remote.draft
    ? remote.draftWorkspaceId
    : selectedSession?.workspaceId;
  const workspace = remote.workspaces.find((item) => item.id === workspaceId);
  // Chat threads also own an internal storage workspace. Trust the explicit
  // mode; only infer from a user-visible workspace for legacy rows without it.
  const isWorkspaceConversation =
    mode === "workspace" || (mode === undefined && workspace !== undefined);
  const contextLabel =
    isWorkspaceConversation
      ? workspace?.name
        ? t("chat.workspaceNamed", { name: workspace.name })
        : t("chat.workspaceConversation")
      : !remote.draft && !selectedSession
        ? t("chat.contextLoading")
        : t("sessions.conversations");
  /**
   * The desktop this conversation is on, as the top bar names it.
   *
   * Every conversation on this phone lives on a paired desktop — the phone has no
   * agent of its own — and the bar said nothing about which one, so a user who
   * paired two could not tell them apart from inside a conversation. The glyph and
   * the user's own name for the machine are the session list's vocabulary, so the
   * same desktop reads the same way in both places; the id is the fallback for a
   * machine the user never renamed.
   */
  const desktopId = remote.credentials?.expectedDesktopId;
  const pairedDesktop = remote.desktops.find(item => item.desktopId === desktopId);
  const machine = desktopId
    ? { icon: pairedDesktop?.icon, label: pairedDesktop?.name || desktopId }
    : undefined;
  const activeModel = remote.models.find(
    (model) => modelReference(model) === remote.modelId,
  );
  const activeModelLabel =
    activeModel?.label || activeModel?.id || remote.modelId || t("chat.model");
  const supportsImages = activeModel
    ? activeModel.supportsImages !== false
    : true;
  // Forking is a Desktop-hosted operation; an older host that does not
  // advertise it leaves the affordance hidden rather than failing on tap.
  const forkSupported = remote.capabilities.has("fork_v1");
  // A lean reply's reasoning row opens by fetching its body. An older host
  // advertises nothing for that read — and would answer "Unsupported command",
  // i.e. a row that promises content and produces an error — so the row stays a
  // plain label there rather than offering an expansion it cannot deliver.
  const reasoningFetchSupported = remote.capabilities.has("reasoning_fetch_v1");

  const { message, setMessage, attachments, setAttachments, sessionRefs, rememberSessionRef, restoring } = useComposerDraft(
    remote,
    t,
  );
  const { openAttachmentMenu, attachmentMenu, albumPicker } = useAttachmentPicker(
    attachments,
    setAttachments,
    t,
    composerOperation,
  );
  const chatFiles = useChatFiles(remote, t, setTransferProgress);
  const markdownImageLoader = useMarkdownImageLoader(remote, t);
  // The picture the viewer is showing, if any. Held here rather than per image:
  // one viewer covers every picture on the screen, and the modal has to sit
  // above the transcript and the file preview both.
  const [viewedImage, setViewedImage] = useState<ImageViewerTarget | null>(null);
  const openTimelineAttachment = chatFiles.openAttachment;
  const openTimelineFile = chatFiles.openFileLink;
  const compactContext = useCompactContext(remote, t);
  const { send, retryMessage, continueMessage, forkMessage } = useSendMessage({
    remote, t, message, attachments, setMessage, setAttachments, setTransferProgress,
    sessionRefs, compactionPending: compactContext.pending, streaming: controls.streaming,
  });

  // ── Send queue ──────────────────────────────────────────────────────────
  // What is waiting belongs to the conversation, not to this screen. A message
  // written while a reply streams is handed to the desktop at once with a
  // queueing policy (`useSendMessage`), the session holds it on a durable
  // receipt, and the rows below are the session's own (`get_queue`) — the same
  // list the desktop's window shows, still there after this app is gone, and
  // visible to every device on the conversation.
  const sessionId = remote.selectedSessionId;
  const {
    forget: forgetQueuedRow,
    notice: queueNotice,
    rows: queuedRows,
    refresh: refreshQueue,
  } = useSessionQueue({
    read: () => remote.getQueue(sessionId),
    sessionId: remote.draft ? "" : sessionId,
    // A run in flight can take a new waiting row, and a row already waiting can
    // start or be parked on its own: both are reasons to keep reading.
    live: (controls.streaming || remote.busy) && !remote.draft,
    enabled: remote.desktopOnline && !remote.draft,
  });

  // Derived from the rows, which are already keyed to the session they were read
  // for — the panel cannot draw one conversation's queue under another's box.
  // The files travel with the row (`get_queue` reports their paths), so the panel
  // can show them and offer to open or drop one before it leaves. A desktop too
  // old to report them leaves `attachments` undefined, and the panel then offers
  // no file controls rather than guessing at what it cannot see.
  const queuePanelItems = useMemo(
    () => queuedRows.map(row => ({
      attachments: row.attachments,
      content: row.text,
      id: row.runId,
      parked: isParkedSessionQueueRow(row),
    })),
    [queuedRows],
  );

  /**
   * What to say about a row that left the queue without running.
   *
   * Without it a row cancelled elsewhere or folded into the turn ahead simply
   * disappears, and the reader believes the message went out.
   */
  const queueNoticeText = useMemo(() => {
    if (!queueNotice)
      return undefined;
    const sentence = t(`chat.queue.notice.${sessionQueueNoticeReason(queueNotice.reason)}`, {
      reason: queueNotice.reason,
    });
    return queueNotice.count > 1
      ? `${sentence} · ${t("chat.queue.notice.more", { count: queueNotice.count })}`
      : sentence;
  }, [queueNotice, t]);

  /**
   * Act on one waiting row, then re-read the queue whatever the answer was.
   *
   * The session named is the one these rows were read for: a tap that lands after
   * the reader moved on cancels in the conversation it belongs to, never in the
   * one now on screen.
   */
  const changeQueue = useCallback((action: () => Promise<void>) => {
    void action()
      .catch((error: unknown) => {
        const failure = sendFailureText(error, readableDeviceName(remote.presence?.holder?.name));
        showToast(t(failure.key, failure.params));
      })
      .finally(refreshQueue);
  }, [refreshQueue, remote.presence?.holder?.name, t]);

  /** Delete: a cancel of the session's own run id. */
  const removeQueuedMessage = useCallback((id: string) => {
    // This screen's own act: its ack is confirmation, not news.
    forgetQueuedRow(id);
    changeQueue(() => remote.cancelQueuedRun(sessionId, id));
  }, [changeQueue, forgetQueuedRow, remote, sessionId]);

  /** "Send this one now": a parked row runs only when a person releases it. */
  const releaseQueuedMessage = useCallback((id: string) => {
    changeQueue(() => remote.releaseQueuedRun(sessionId, id));
  }, [changeQueue, remote, sessionId]);

  /**
   * Replace a waiting message.
   *
   * The session has no rename and no attachment edit, so an edit is the row taken
   * back and sent again: its files are copied down FIRST (a row cancelled before
   * its files arrived would lose them), then the queued run is cancelled, then the
   * new text and files are sent — which also queues it behind whatever else waits,
   * because that is where a new message goes. A refused re-send puts the text and
   * files back in the box: the row is gone on the desktop by then, and losing the
   * correction as well would leave nothing to retry.
   */
  const replaceQueuedMessage = useCallback((
    row: SessionQueueRow,
    text: string,
    loadAttachments: () => Promise<MobileAttachment[]>,
  ) => {
    changeQueue(async () => {
      const attachments = await loadAttachments();
      await remote.cancelQueuedRun(sessionId, row.runId);
      forgetQueuedRow(row.runId);
      try {
        await remote.sendMessage(text, attachments, undefined, { busyPolicy: QUEUED_SEND_POLICY });
      }
      catch (error) {
        setMessage(current => (current.trim() ? current : text));
        setAttachments(current => [...attachments, ...current]);
        throw error;
      }
    });
  }, [changeQueue, forgetQueuedRow, remote, sessionId, setAttachments, setMessage]);

  /** A queued file as this phone can send it: a local copy, downloaded on demand. */
  const materializeQueuedAttachment = useCallback(async (
    attachment: SessionQueueAttachment,
  ): Promise<MobileAttachment> => {
    const kind: MobileAttachment["kind"] = attachment.kind === "image" ? "image" : "file";
    // A path that is already a URI (an optimistic row from this phone) needs no
    // transfer: the bytes are here.
    if (/^[a-z][a-z0-9+.-]*:\/\//i.test(attachment.path)) {
      const file = new File(attachment.path);
      return {
        kind,
        localUri: file.uri,
        mimeType: mimeFor(attachment.name),
        name: attachment.name,
        originalSize: file.size,
        transferSize: file.size,
      };
    }
    const reference = { kind, name: attachment.name, path: attachment.path };
    const info = await remote.prepareAttachment(reference);
    const cached = remote.cachedAttachment(reference);
    const file = cached?.file ?? (await remote.downloadAttachment(info));
    return {
      kind,
      localUri: file.uri,
      mimeType: info.mimeType,
      name: attachment.name,
      originalSize: info.size,
      transferName: info.name,
      transferSize: info.size,
    };
  }, [remote]);

  /**
   * Rewrite a waiting message, keeping the files it carries.
   *
   * The files used to be dropped here, which is what the editor warned about. Now
   * the row reports them, so the correction keeps them: text changes, photos stay.
   */
  const updateQueuedMessage = useCallback((id: string, content: string) => {
    const text = content.trim();
    const row = queuedRows.find(item => item.runId === id);
    // An empty edit is a deletion, and the panel already offers that.
    if (!row || !text)
      return;
    const reported = row.attachments;
    replaceQueuedMessage(row, text, () => reported === undefined
      ? Promise.resolve([])
      : Promise.all(reported.map(materializeQueuedAttachment)));
  }, [materializeQueuedAttachment, queuedRows, replaceQueuedMessage]);

  /** Open one file a waiting message carries, exactly as the transcript would. */
  const previewQueuedAttachment = useCallback((attachment: SendQueueAttachmentItem) => {
    void chatFiles.openAttachment({
      kind: attachment.kind === "image" ? "image" : "file",
      name: attachment.name,
      path: attachment.path,
    });
  }, [chatFiles]);

  /** Drop one file from a waiting message; the rest are re-sent with it. */
  const removeQueuedAttachment = useCallback((id: string, path: string) => {
    const row = queuedRows.find(item => item.runId === id);
    if (!row || row.attachments === undefined)
      return;
    const kept = row.attachments.filter(attachment => attachment.path !== path);
    replaceQueuedMessage(row, row.text, () => Promise.all(kept.map(materializeQueuedAttachment)));
  }, [materializeQueuedAttachment, queuedRows, replaceQueuedMessage]);

  /** Which waiting row's "add files" menu is open (it belongs to that row). */
  const [queueAttachRow, setQueueAttachRow] = useState<SessionQueueRow | null>(null);
  const openQueueAttachments = useCallback((id: string) => {
    const row = queuedRows.find(item => item.runId === id);
    // A row that does not report its files cannot be added to: the new ones would
    // be sent and the ones this phone cannot see would be lost.
    if (!row || row.attachments === undefined)
      return;
    Keyboard.dismiss();
    setQueueAttachRow(row);
  }, [queuedRows]);

  const chooseQueueAttachments = useCallback((source: AttachmentSource) => {
    const row = queueAttachRow;
    setQueueAttachRow(null);
    if (!row || row.attachments === undefined)
      return;
    const kept = row.attachments;
    // The picker's quotas are counted against what the row already carries, from
    // kinds and sizes alone: the kept files are copied down only once the reader
    // actually picked something, so opening the picker costs nothing on a slow
    // link. `localUri` is the desktop path here — nothing reads it before the
    // real copies are made.
    const existing: MobileAttachment[] = kept.map(attachment => ({
      kind: attachment.kind === "image" ? "image" : "file",
      localUri: attachment.path,
      mimeType: mimeFor(attachment.name),
      name: attachment.name,
      originalSize: attachment.size,
      transferSize: attachment.size,
    }));
    const launch = source === "camera"
      ? takePhoto
      : source === "album" ? pickFromAlbum : pickAttachments;
    void (async () => {
      let picked: MobileAttachment[];
      try {
        picked = await launch(existing);
      }
      catch (error) {
        const key = error instanceof Error ? error.message : "attachment_failed";
        showToast(t(`attachment.errors.${key}`));
        return;
      }
      const known = new Set(existing.map(item => item.localUri));
      const added = picked.filter(item => !known.has(item.localUri));
      // The picker answers with the whole list; nothing new means the reader
      // backed out, and the row is left exactly as it was.
      if (added.length === 0)
        return;
      replaceQueuedMessage(row, row.text, async () => [
        ...(await Promise.all(kept.map(materializeQueuedAttachment))),
        ...added,
      ]);
    })();
  }, [materializeQueuedAttachment, queueAttachRow, replaceQueuedMessage, t]);

  /** Send, then read the queue: the row that send may have made is now real. */
  const sendAndReadQueue = useCallback(async (
    override?: string,
    intent?: SendMessageIntent,
  ) => {
    const delivered = intent ? await send(override, intent) : await send(override);
    refreshQueue();
    return delivered;
  }, [refreshQueue, send]);

  // Approvals live docked above the composer (not inline in the transcript), and
  // only while undecided — once a decision lands the card disappears.
  const timelineItems = useMemo(() => remote.timeline.items, [remote.timeline]);
  const transcriptItems = useMemo(
    () => timelineItems.filter((item) => item.kind !== "approval"),
    [timelineItems],
  );
  // FlatList's physical start is the stable latest-message anchor. Reversing
  // only the view data keeps the remote timeline chronological while making
  // older history append at the far end instead of shifting visible rows.
  const invertedTranscriptItems = useMemo(
    () => newestFirst(transcriptItems),
    [transcriptItems],
  );
  const latestAssistantId = useMemo(() => {
    for (let index = transcriptItems.length - 1; index >= 0; index -= 1) {
      const item = transcriptItems[index];
      if (item?.kind === "message")
        return item.role === "assistant" ? item.id : null;
    }
    return null;
  }, [transcriptItems]);
  const pendingApprovals = useMemo(
    () =>
      timelineItems.filter(
        (item): item is Extract<TimelineItem, { kind: "approval"; }> =>
          item.kind === "approval" && !item.decision,
      ),
    [timelineItems],
  );
  const [approvalSubmitting, setApprovalSubmitting] = useState<string | null>(
    null,
  );
  const [approvalError, setApprovalError] = useState<{
    id: string;
    message: string;
  } | null>(null);
  // History load is in flight (selectSession holds busy until it lands) — show
  // a spinner instead of flashing the "no history" empty state.
  const loadingHistory =
    !remote.timelineError &&
    !remote.draft &&
    (remote.busy || remote.timelinePending) &&
    timelineItems.length === 0;
  // Pull-to-refresh rebuilds the visible window from durable history — the
  // manual escape hatch when the automatic sync left the conversation wrong.
  // Its spinner belongs to the pull, not to the lane: an automatic sync already
  // says so in the notice above the transcript, and spinning here as well would
  // report the same wait twice, at the one spot a finger has to be able to tell
  // apart (the inverted list draws the indicator at the visual bottom).
  const laneAtWork =
    !remote.draft &&
    !remote.timelinePending &&
    !remote.timelineError &&
    remote.timelineSyncStatus !== "idle";
  const [refreshingByPull, setRefreshingByPull] = useState(false);
  // The pull is over when the lane it restarted settles; `laneAtWork` is that
  // lane's own signal, so a pull that restarts nothing (offline, no session)
  // cannot leave the spinner up.
  useEffect(() => {
    if (!laneAtWork) {
      // The flag is owned by a signal that lives outside React, and retracting
      // it one commit after the lane settles is what this effect is for.
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setRefreshingByPull(false);
    }
  }, [laneAtWork]);
  const pullRefreshActive = refreshingByPull && laneAtWork;
  const pullRefresh = useCallback(() => {
    // `reloadTimeline` runs the lane's restart synchronously, so the status is
    // already "syncing" in the render this state update rides in — the spinner
    // is up by the time the finger leaves and the effect above leaves it alone.
    setRefreshingByPull(true);
    remote.reloadTimeline();
  }, [remote]);

  // One place decides which wait the notice reports, so the gate below reads the
  // same decision the wording does.
  const syncNoticeState: SyncNoticeState | null =
    remote.timelineSyncStatus === "syncing" ||
      remote.timelineSyncStatus === "retrying"
      ? remote.desktopOnline
        ? remote.timelineSyncStatus
        : "waitingNetwork"
      : null;
  // A pull restarts the very lane the notice reports, so its native spinner at
  // the visual bottom and the notice above the transcript come up together and
  // say the same sentence twice. The pull wins that overlap: the finger is still
  // there. The two richer states stay, because the spinner cannot say them.
  const pullOwnsTheWait = pullRefreshActive && syncNoticeState === "syncing";
  // Live generation reports its own progress: the transcript grows with the
  // run's streaming row while the lane backfills. A reconcile behind live
  // output is routine traffic — the engine heals every gap from the journal
  // while the run streams — so the pill would announce a wait the reader is
  // already watching, for a state that needs no action. The two richer states
  // stay for the same reason as under a pull: the streaming text cannot say
  // "retrying" or "waiting for the connection".
  const liveOutputOwnsTheWait =
    controls.streaming && syncNoticeState === "syncing";
  const showSyncNotice = useMinimumVisible(
    !remote.draft &&
    timelineItems.length > 0 &&
    syncNoticeState !== null &&
    !pullOwnsTheWait &&
    !liveOutputOwnsTheWait,
    SYNC_NOTICE_MIN_MS,
    conversationKey,
  );
  // The pill outlives its status by the minimum-visible window, and that tail
  // has no status of its own: it goes on saying the plain one.
  const noticeState: SyncNoticeState = syncNoticeState ?? "syncing";

  const decideApproval = useCallback(
    async (id: string, decision: "approved" | "rejected") => {
      setApprovalSubmitting(id);
      setApprovalError(null);
      try {
        await submitApproval(id, decision);
      } catch {
        setApprovalError({ id, message: t("approval.submitFailed") });
      } finally {
        setApprovalSubmitting(null);
      }
    },
    [submitApproval, t],
  );

  const scroll = useChatScroll(
    remote.selectedSessionId,
    transcriptItems.length,
  );
  const { listRef, atLatest, scrollToLatest, onScroll } = scroll;
  // A jump leaves the tail behind, and `useChatScroll` owns that handover: it
  // is what keeps the list from pinning the viewport back to the bottom on the
  // next streaming commit.
  const questionNav = useQuestionNav({
    sessionId: remote.selectedSessionId,
    items: invertedTranscriptItems,
    listRef,
    atLatest,
    onTakeOver: scroll.onScrollBeginDrag,
    hasOlderHistory: remote.canLoadOlderTimeline,
    loadingOlder: remote.loadingOlderTimeline,
    loadOlder: remote.loadOlderTimeline,
  });

  // Skill recommendation (PRD v1.6): the desktop's toggle decides whether the
  // phone asks at all. The evaluator holds the draft while it asks, so `send`
  // below cannot race it.
  const desktopSettings = useDesktopResource(
    remote.getDesktopSettings,
    remote.desktopSettingsRevision,
    remote.desktopOnline,
  );
  const skillRecommendation = useSkillRecommendation(
    desktopSettings.data?.skillRecommend ?? false,
    remote.desktopOnline,
    i18n.language,
  );
  const { sendFromComposer, dismissSuggestedSkill, installSuggestedSkill, installingSkill }
    = useComposerSubmission({
      remote, t, message, attachments, restoring, setMessage, send: sendAndReadQueue,
      recommendation: skillRecommendation, scrollToLatest
    });

  const paging = useTimelinePaging(
    remote.selectedSessionId,
    remote.canLoadOlderTimeline,
    remote.loadingOlderTimeline,
    remote.loadOlderTimeline,
    onScroll,
  );

  const { onRowLayout, onScroll: onPagedScroll } = paging;

  // The list keeps one scroll handler: paging owns fetching more history, the
  // question control owns the reading position.
  const questionScroll = questionNav.onScroll;
  const onChatScroll = useCallback(
    (event: Parameters<typeof onPagedScroll>[0]) => {
      onPagedScroll(event);
      questionScroll(event);
    },
    [onPagedScroll, questionScroll],
  );

  // Keep FlatList row callbacks referentially stable while still dispatching
  // through the newest controller closures. Remote context changes on every
  // streaming commit; passing those closures directly would defeat memoized
  // settled TimelineCards and is one source of VirtualizedList update work.
  const timelineActionsRef = useRef({
    openAttachment: openTimelineAttachment,
    openFile: openTimelineFile,
    retry: retryMessage,
    continue: continueMessage,
    fork: forkMessage,
    resolveToolTarget: remote.resolveToolCallTarget,
    resolveReasoning: remote.resolveReasoning,
  });
  useEffect(() => {
    timelineActionsRef.current = {
      openAttachment: openTimelineAttachment,
      openFile: openTimelineFile,
      retry: retryMessage,
      continue: continueMessage,
      fork: forkMessage,
      resolveToolTarget: remote.resolveToolCallTarget,
      resolveReasoning: remote.resolveReasoning,
    };
  }, [
    continueMessage,
    forkMessage,
    openTimelineAttachment,
    openTimelineFile,
    remote.resolveReasoning,
    remote.resolveToolCallTarget,
    retryMessage,
  ]);
  const handleTimelineAttachment = useCallback(
    (attachment: Parameters<typeof openTimelineAttachment>[0]) =>
      void timelineActionsRef.current.openAttachment(attachment),
    [],
  );
  const handleTimelineFile = useCallback(
    (path: string) => void timelineActionsRef.current.openFile(path),
    [],
  );
  // Tapping a `#` reference in a sent message opens that conversation, the same
  // way picking one from the composer menu referenced it.
  const { selectSession } = controls;
  const handleTimelineSession = useCallback(
    (sessionId: string) => void selectSession(sessionId),
    [selectSession],
  );
  const handleTimelineRetry = useCallback(
    (item: TimelineItem) => timelineActionsRef.current.retry(item),
    [],
  );
  const handleTimelineContinue = useCallback(
    (item: TimelineItem) => timelineActionsRef.current.continue(item),
    [],
  );
  const handleTimelineFork = useCallback(
    (item: TimelineItem) => timelineActionsRef.current.fork(item),
    [],
  );
  const handleResolveToolTarget = useCallback(
    (toolCallId: string, runId: string, fusion?: { stepId?: string; offset?: number }) =>
      fusion ? timelineActionsRef.current.resolveToolTarget(toolCallId, runId, fusion) : timelineActionsRef.current.resolveToolTarget(toolCallId, runId),
    [],
  );

  const handleResolveReasoning = useCallback(
    (runId: string, index: number, settled?: boolean) =>
      timelineActionsRef.current.resolveReasoning(runId, index, settled),
    [],
  );

  const renderTimelineItem = useCallback(
    ({ item }: { item: TimelineItem; }) => (
      <View onLayout={onRowLayout}>
        <TimelineCard
          item={item}
          isLatestAssistant={item.id === latestAssistantId}
          onOpenAttachment={handleTimelineAttachment}
          onOpenFile={handleTimelineFile}
          onOpenSession={handleTimelineSession}
          onRetry={handleTimelineRetry}
          onContinue={handleTimelineContinue}
          onFork={forkSupported ? handleTimelineFork : undefined}
          onResolveToolTarget={handleResolveToolTarget}
          onResolveReasoning={reasoningFetchSupported ? handleResolveReasoning : undefined}
        />
      </View>
    ),
    [
      forkSupported,
      handleResolveReasoning,
      reasoningFetchSupported,
      handleResolveToolTarget,
      handleTimelineAttachment,
      handleTimelineContinue,
      handleTimelineFile,
      handleTimelineFork,
      handleTimelineRetry,
      handleTimelineSession,
      latestAssistantId,
      onRowLayout,
    ],
  );

  useEffect(() => {
    // While browsing files, the panel owns system back so it can pop a folder
    // first. Do not rely on child/parent effect subscription order.
    if (filesOpen) return;
    const subscription = BackHandler.addEventListener(
      "hardwareBackPress",
      () => {
        goBack();
        return true;
      },
    );
    return () => subscription.remove();
  }, [filesOpen, goBack]);

  // Measure both platforms to bound skill suggestions above the keyboard.
  // Only Android applies this as compensation below; iOS already uses KAV.
  useEffect(() => {
    const showSub = Keyboard.addListener(
      Platform.OS === "ios" ? "keyboardWillShow" : "keyboardDidShow",
      (event) => setKeyboardHeight(event.endCoordinates.height),
    );
    const hideSub = Keyboard.addListener(
      Platform.OS === "ios" ? "keyboardWillHide" : "keyboardDidHide",
      () => setKeyboardHeight(0),
    );
    return () => {
      showSub.remove();
      hideSub.remove();
    };
  }, []);

  // Inset the whole flex column so the composer *card* clears the keyboard. The
  // dock includes composerArea's bottom padding, so retain a little additional
  // clearance for the card border, rounded corners, and shadow above Gboard.
  const KEYBOARD_CLEARANCE = spacing.md + spacing.sm;
  const keyboardLift =
    Platform.OS === "android" && keyboardHeight > 0
      ? Math.max(0, keyboardHeight - insets.bottom + KEYBOARD_CLEARANCE)
      : 0;

  return (
    <ImageViewerContext value={setViewedImage}>
      <MarkdownImageLoaderContext value={markdownImageLoader}>
        <SafeAreaView style={styles.safe}>
          {attachmentMenu}
          {albumPicker}
          {/* Mounted only while open: a sheet that is not showing is not a
              surface, and a permanently-mounted one would sit above the
              transcript and answer for someone else's dismissal. */}
          {queueAttachRow !== null && (
            <AttachmentSourceMenu
              visible
              onClose={() => setQueueAttachRow(null)}
              onPick={chooseQueueAttachments}
            />
          )}
          <KeyboardAvoidingView
            behavior={Platform.OS === "ios" ? "padding" : undefined}
            style={styles.keyboard}
          >
            <ChatTopBar
              title={title}
              contextLabel={contextLabel}
              draft={remote.draft}
              machine={machine}
              backLabel={t("common.back")}
              usageLabel={t("chat.usageOpen")}
              onBack={goBack}
              onUsage={() => {
                Keyboard.dismiss();
                // The sheet shows the session's spend, so read it as it opens: the
                // figure is only as fresh as the last get_state, and this is the
                // moment it is being looked at.
                void remote.refreshSessionUsage();
                setUsageOpen(true);
              }}
              filesLabel={t("files.title")}
              filesOpen={filesOpen}
              onFiles={() => {
                Keyboard.dismiss();
                setFilesSession(filesOpen ? null : conversationKey);
              }}
            />

            {remote.error && connection.level === "connected" && (
              <ErrorBanner
                message={remote.error}
                onDismiss={
                  remote.connectionPresentation.level === "disconnected" &&
                    remote.connectionPresentation.supportCode
                    ? undefined
                    : remote.clearError
                }
              />
            )}

            <View
              style={[
                styles.chatContent,
                keyboardLift > 0 ? { paddingBottom: keyboardLift } : null,
              ]}
            >
              <View
                style={styles.chatContent}
                accessibilityElementsHidden={filesOpen}
                importantForAccessibility={
                  filesOpen ? "no-hide-descendants" : "auto"
                }
              >
                <PausedTimeline
                  key={conversationKey}
                  paused={filesOpen || chatFiles.preview !== null || chatFiles.activeDownload !== null}
                >
                  <ChatTimeline
                    remote={remote}
                    t={t}
                    invertedTranscriptItems={invertedTranscriptItems}
                    scroll={scroll}
                    questionNav={questionNav}
                    paging={paging}
                    renderTimelineItem={renderTimelineItem}
                    onChatScroll={onChatScroll}
                    loadingHistory={loadingHistory}
                    pullRefresh={pullRefresh}
                    pullRefreshActive={pullRefreshActive}
                    showSyncNotice={showSyncNotice}
                    noticeState={noticeState}
                  />

                  {/* Where the sending happens, not just in the header: a user
                      composing here will not tap a small dot to find out why the
                      send button does nothing. */}
                  {remote.presence?.control === "standby" && (
                    <StandbyNotice
                      holder={remote.presence?.holder ?? null}
                      onTakeControl={remote.takeControl}
                    />
                  )}

                  <ComposerDock
                    key={`${conversationKey}:${remote.draft}:${remote.draftWorkspaceId}`}
                    draftRestoring={restoring}
                    keyboardHeight={keyboardHeight}
                    message={message}
                    setMessage={setMessage}
                    sessionRefs={sessionRefs}
                    rememberSessionRef={rememberSessionRef}
                    attachments={attachments}
                    setAttachments={setAttachments}
                    supportsImages={supportsImages}
                    activeModelLabel={activeModelLabel}
                    remote={controls}
                    t={t}
                    openAttachmentMenu={openAttachmentMenu}
                    send={sendFromComposer}
                    atLatest={atLatest}
                    scrollToLatest={scrollToLatest}
                    pendingApprovals={pendingApprovals}
                    approvalSubmitting={approvalSubmitting}
                    approvalError={approvalError}
                    decideApproval={decideApproval}
                    selector={selector}
                    setSelector={setSelector}
                    onCompactContext={compactContext.compact}
                    compactionPending={compactContext.pending}
                    archived={selectedSession?.archived === true}
                    onRestore={selectedSession?.archived === true
                      && Boolean(selectedSession.threadId)
                      && remote.capabilities.has("archive_v1")
                      ? restoreArchived
                      : undefined}
                    restoringArchived={restoringArchived}
                    skillSuggestion={skillRecommendation.suggestion}
                    skillEvaluating={skillRecommendation.evaluating}
                    skillInstalling={installingSkill}
                    onInstallSkill={installSuggestedSkill}
                    onDismissSkill={dismissSuggestedSkill}
                    queueItems={queuePanelItems}
                    queueNotice={queueNoticeText}
                    onQueueUpdate={updateQueuedMessage}
                    onQueueRemove={removeQueuedMessage}
                    onQueueRelease={releaseQueuedMessage}
                    onQueuePreviewAttachment={previewQueuedAttachment}
                    onQueueRemoveAttachment={removeQueuedAttachment}
                    onQueueAddAttachments={openQueueAttachments}
                  />
                </PausedTimeline>
              </View>

              {filesOpen && (
                <SessionFilesPanel
                  key={conversationKey}
                  online={remote.desktopOnline}
                  supported={remote.capabilities.has("session_files_v1")}
                  isWorkspace={
                    remote.sessions.find(
                      (session) => session.sessionId === remote.selectedSessionId,
                    )?.mode === "workspace"
                  }
                  listFiles={remote.listSessionFiles}
                  onOpenFile={(path) => chatFiles.openFileLink(path, true)}
                  onClose={() => setFilesSession(null)}
                />
              )}

              {transferProgress != null && (
                <View pointerEvents="none" style={styles.transferTrack}>
                  <View
                    style={[
                      styles.transferFill,
                      { width: `${Math.max(2, transferProgress * 100)}%` },
                    ]}
                  />
                </View>
              )}
            </View>

            <NativeFileActionSheet
              action={chatFiles.fileAction}
              onClose={() => chatFiles.setFileAction(null)}
              onSelect={(action, operation) => {
                void chatFiles.openOrShare(
                  action.info,
                  action.cachedFile,
                  operation,
                );
              }}
              openLabel={t("attachment.open")}
              saveLabel={t("attachment.save")}
              shareLabel={t("attachment.share")}
            />

            <PreviewModal
              activeDownload={chatFiles.activeDownload}
              closePreview={chatFiles.closePreview}
              dismissPreviewThen={chatFiles.dismissPreviewThen}
              downloadOriginal={chatFiles.downloadOriginal}
              flushPendingPreviewAction={chatFiles.flushPendingPreviewAction}
              openLinkedFile={chatFiles.openLinkedFile}
              popPreview={chatFiles.popPreview}
              previews={chatFiles.previews}
              t={t}
            />

            <DownloadProgressModal
              activeDownload={chatFiles.activeDownload}
              activeDownloadFraction={chatFiles.activeDownloadFraction}
              cancelActiveDownload={chatFiles.cancelActiveDownload}
              flushPendingDownloadModal={chatFiles.flushPendingDownloadModal}
              onDownloadModalShow={chatFiles.onDownloadModalShow}
              t={t}
            />

            <ModelSelectorSheet
              selector={selector}
              setSelector={setSelector}
              remote={remote}
              t={t}
            />

            <SessionUsageSheet
              title={title}
              usage={remote.sessionUsage}
              visible={usageOpen}
              onClose={() => setUsageOpen(false)}
              t={t}
            />
            {/* Last of the surfaces so the viewer stacks above a file preview it
                was opened from. */}
            <ImageViewer onClose={() => setViewedImage(null)} target={viewedImage} />
          </KeyboardAvoidingView>
        </SafeAreaView>
      </MarkdownImageLoaderContext>
    </ImageViewerContext>
  );
}
