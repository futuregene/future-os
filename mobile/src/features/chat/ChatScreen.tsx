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
import { modelReference, type TimelineItem } from "../../remote/types";
import { spacing } from "../../theme/tokens";
import { useTransferProgress } from "./useTransferProgress";
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
import { useSendMessage } from "./useSendMessage";
import { useSkillRecommendation } from "./useSkillRecommendation";
import { useDesktopResource } from "../settings/useDesktopResource";
import { ChatTimeline, type SyncNoticeState } from "./components/ChatTimeline";
import { ChatTopBar } from "./components/ChatTopBar";
import { SessionFilesPanel } from "./components/SessionFilesPanel";
import { ComposerDock } from "./components/ComposerDock";
import { ModelSelectorSheet } from "./components/ModelSelectorSheet";
import { DownloadProgressModal } from "./components/DownloadProgressModal";
import { PreviewModal } from "./components/PreviewModal";
import { ImageViewer } from "./components/ImageViewer";
import { SessionUsageSheet } from "./components/SessionUsageSheet";
import { NativeFileActionSheet } from "./components/NativeFileActionSheet";
import { newestFirst } from "./timelineListModel";

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
    sessionRefs, compactionPending: compactContext.pending,
  });
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
      remote, t, message, attachments, restoring, setMessage, send,
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
  });
  useEffect(() => {
    timelineActionsRef.current = {
      openAttachment: openTimelineAttachment,
      openFile: openTimelineFile,
      retry: retryMessage,
      continue: continueMessage,
      fork: forkMessage,
      resolveToolTarget: remote.resolveToolCallTarget,
    };
  }, [
    continueMessage,
    forkMessage,
    openTimelineAttachment,
    openTimelineFile,
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
          onReplyTailAnchor={questionNav.onReplyTailAnchor}
        />
      </View>
    ),
    [
      forkSupported,
      handleResolveToolTarget,
      handleTimelineAttachment,
      handleTimelineContinue,
      handleTimelineFile,
      handleTimelineFork,
      handleTimelineRetry,
      handleTimelineSession,
      latestAssistantId,
      onRowLayout,
      questionNav.onReplyTailAnchor,
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
                    skillSuggestion={skillRecommendation.suggestion}
                    skillEvaluating={skillRecommendation.evaluating}
                    skillInstalling={installingSkill}
                    onInstallSkill={installSuggestedSkill}
                    onDismissSkill={dismissSuggestedSkill}
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
