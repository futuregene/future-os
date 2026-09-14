import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { History } from "lucide-react-native";
import {
  ActivityIndicator,
  BackHandler,
  FlatList,
  Keyboard,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import {
  SafeAreaView,
  useSafeAreaInsets,
} from "react-native-safe-area-context";
import { TimelineCard } from "../../components/TimelineCard";
import { ErrorBanner } from "../../components/ErrorBanner";
import { useRemote, useRemoteControls } from "../../remote/RemoteContext";
import { modelReference, type TimelineItem } from "../../remote/types";
import { colors, layout, radius, spacing } from "../../theme/tokens";
import { useComposerDraft } from "./useComposerDraft";
import { useAttachmentPicker } from "./useAttachmentPicker";
import { useFileDownload } from "./useFileDownload";
import { useMarkdownImageLoader } from "./useMarkdownImageLoader";
import { MarkdownImageLoaderContext } from "../../components/MarkdownImage";
import { useChatScroll } from "./useChatScroll";
import { useTimelinePaging } from "./useTimelinePaging";
import { useRename } from "./useRename";
import { useSendMessage } from "./useSendMessage";
import { ChatTopBar } from "./components/ChatTopBar";
import { SessionFilesPanel } from "./components/SessionFilesPanel";
import { ComposerDock } from "./components/ComposerDock";
import { FloatingTimelineButton } from "./components/FloatingTimelineButton";
import { ModelSelectorSheet } from "./components/ModelSelectorSheet";
import { DownloadProgressModal } from "./components/DownloadProgressModal";
import { PreviewModal } from "./components/PreviewModal";
import { RenameModal } from "./components/RenameModal";
import { NativeFileActionSheet } from "./components/NativeFileActionSheet";
import { COMPOSER_FADE_CLEARANCE } from "./utils";
import { newestFirst } from "./timelineListModel";

function TimelineItemGap() {
  return <View style={styles.itemGap} />;
}

function TimelineFlexSpacer() {
  return <View />;
}

export function ChatScreen() {
  const { t } = useTranslation();
  const remote = useRemote();
  const controls = useRemoteControls();
  const connection = controls.connectionPresentation;
  const { closeConversation, decideApproval: submitApproval } = remote;
  const insets = useSafeAreaInsets();

  const [transferProgress, setTransferProgress] = useState<number | null>(null);
  const [selector, setSelector] = useState<"model" | "thinking" | null>(null);
  const [filesSession, setFilesSession] = useState<string | null>(null);
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
  const contextLabel =
    mode === "workspace" || workspaceId
      ? workspace?.name
        ? t("chat.workspaceNamed", { name: workspace.name })
        : t("chat.workspaceConversation")
      : !remote.draft && !selectedSession
        ? t("chat.contextLoading")
        : t("chat.nonWorkspaceConversation");
  const activeModel = remote.models.find(
    (model) => modelReference(model) === remote.modelId,
  );
  const activeModelLabel =
    activeModel?.label || activeModel?.id || remote.modelId || t("chat.model");
  const supportsImages = activeModel
    ? activeModel.supportsImages !== false
    : true;

  const { message, setMessage, attachments, setAttachments } = useComposerDraft(
    remote,
    t,
  );
  const { openAttachmentMenu, attachmentMenu } = useAttachmentPicker(
    attachments,
    setAttachments,
    t,
  );
  const fileDownload = useFileDownload(remote, t, setTransferProgress);
  const markdownImageLoader = useMarkdownImageLoader(remote, t);
  const openTimelineAttachment = fileDownload.openAttachment;
  const openTimelineFile = fileDownload.openFileLink;
  const { send, retryMessage, continueMessage } = useSendMessage(
    remote,
    t,
    message,
    attachments,
    setMessage,
    setAttachments,
    setTransferProgress,
  );
  const rename = useRename(remote, t);

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
        (item): item is Extract<TimelineItem, { kind: "approval" }> =>
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
  const sendFromComposer = useCallback(async () => {
    if (!message.trim() && attachments.length === 0) return;
    scrollToLatest();
    await send();
  }, [attachments, message, scrollToLatest, send]);
  const {
    showLoadOlderHint,
    pagingActive,
    pagingFailed,
    loadOlder,
    onRowLayout,
    onListLayout,
    onContentSizeChange,
    onScrollBeginDrag,
    onScrollEndDrag,
    onMomentumScrollBegin,
    onMomentumScrollEnd,
    onScroll: onPagedScroll,
  } = useTimelinePaging(
    remote.selectedSessionId,
    remote.canLoadOlderTimeline,
    remote.loadingOlderTimeline,
    remote.loadOlderTimeline,
    onScroll,
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
  });
  useEffect(() => {
    timelineActionsRef.current = {
      openAttachment: openTimelineAttachment,
      openFile: openTimelineFile,
      retry: retryMessage,
      continue: continueMessage,
    };
  }, [continueMessage, openTimelineAttachment, openTimelineFile, retryMessage]);
  const handleTimelineAttachment = useCallback(
    (attachment: Parameters<typeof openTimelineAttachment>[0]) =>
      void timelineActionsRef.current.openAttachment(attachment),
    [],
  );
  const handleTimelineFile = useCallback(
    (path: string) => void timelineActionsRef.current.openFile(path),
    [],
  );
  const handleTimelineRetry = useCallback(
    (item: TimelineItem) => timelineActionsRef.current.retry(item),
    [],
  );
  const handleTimelineContinue = useCallback(
    (item: TimelineItem) => timelineActionsRef.current.continue(item),
    [],
  );

  const renderTimelineItem = useCallback(
    ({ item }: { item: TimelineItem }) => (
      <View onLayout={onRowLayout}>
        <TimelineCard
          item={item}
          isLatestAssistant={item.id === latestAssistantId}
          onOpenAttachment={handleTimelineAttachment}
          onOpenFile={handleTimelineFile}
          onRetry={handleTimelineRetry}
          onContinue={handleTimelineContinue}
        />
      </View>
    ),
    [
      handleTimelineAttachment,
      handleTimelineContinue,
      handleTimelineFile,
      handleTimelineRetry,
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
    <MarkdownImageLoaderContext value={markdownImageLoader}>
      <SafeAreaView style={styles.safe}>
        {attachmentMenu}
        <KeyboardAvoidingView
          behavior={Platform.OS === "ios" ? "padding" : undefined}
          style={styles.keyboard}
        >
          <ChatTopBar
            title={title}
            contextLabel={contextLabel}
            draft={remote.draft}
            backLabel={t("common.back")}
            renameLabel={t("chat.rename")}
            onBack={goBack}
            onRename={rename.openRename}
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
              <View style={styles.timelineViewport}>
                {/* Inverted data puts latest at offset zero. Reading-mode native
                  anchoring handles subsequent row layout changes. */}
                <FlatList
                  contentContainerStyle={[
                    styles.timeline,
                    {
                      // The scroll container is inverted, so logical top padding is
                      // rendered at the visual bottom. The composer itself now
                      // participates in flex layout; only its overlaid fade needs
                      // clearance inside the list.
                      paddingTop: COMPOSER_FADE_CLEARANCE + spacing.lg,
                    },
                    timelineItems.length === 0 && styles.emptyTimeline,
                  ]}
                  data={invertedTranscriptItems}
                  initialNumToRender={10}
                  inverted
                  key={remote.selectedSessionId || "draft"}
                  keyExtractor={(item) => item.id}
                  ListHeaderComponent={
                    invertedTranscriptItems.length > 0
                      ? TimelineFlexSpacer
                      : null
                  }
                  ListHeaderComponentStyle={styles.timelineFlexSpacer}
                  ListEmptyComponent={
                    remote.timelineError ? (
                      <View style={styles.loadingState}>
                        <Text style={styles.historyError}>
                          {t("chat.historyLoadTimedOut")}
                        </Text>
                        <Pressable
                          accessibilityRole="button"
                          onPress={() => void remote.retryTimeline()}
                          style={({ pressed }) => [
                            styles.retryButton,
                            pressed && styles.retryPressed,
                          ]}
                        >
                          <Text style={styles.retryLabel}>
                            {t("common.retry")}
                          </Text>
                        </Pressable>
                      </View>
                    ) : loadingHistory ? (
                      <View style={styles.loadingState}>
                        <ActivityIndicator color={colors.accent} />
                        <Text style={styles.empty}>
                          {t("chat.loadingHistory")}
                        </Text>
                      </View>
                    ) : (
                      <Text style={styles.empty}>{t("chat.noHistory")}</Text>
                    )
                  }
                  maintainVisibleContentPosition={
                    scroll.maintainVisibleContentPosition
                  }
                  keyboardDismissMode={
                    Platform.OS === "ios" ? "interactive" : "on-drag"
                  }
                  keyboardShouldPersistTaps="handled"
                  // Android selectable Text requests focus/rectangle visibility on
                  // long press. Its native auto-scroll ignores our inverted reading
                  // anchor and jumps the transcript; selection itself stays enabled.
                  scrollsChildToFocus={false}
                  automaticallyAdjustContentInsets={false}
                  contentInsetAdjustmentBehavior="never"
                  automaticallyAdjustKeyboardInsets={false}
                  maxToRenderPerBatch={8}
                  onContentSizeChange={(width, height) => {
                    scroll.onContentSizeChange();
                    onContentSizeChange(width, height);
                  }}
                  onLayout={(event) => {
                    scroll.onLayout();
                    onListLayout(event.nativeEvent.layout.height);
                  }}
                  onScrollBeginDrag={(event) => {
                    scroll.onScrollBeginDrag();
                    onScrollBeginDrag(event);
                  }}
                  onMomentumScrollBegin={onMomentumScrollBegin}
                  onMomentumScrollEnd={onMomentumScrollEnd}
                  onScroll={onPagedScroll}
                  onScrollEndDrag={onScrollEndDrag}
                  ref={listRef}
                  renderItem={renderTimelineItem}
                  scrollEventThrottle={16}
                  scrollIndicatorInsets={{ bottom: 0 }}
                  style={styles.timelineList}
                  updateCellsBatchingPeriod={32}
                  windowSize={7}
                  ItemSeparatorComponent={TimelineItemGap}
                />

                {showLoadOlderHint && (
                  <FloatingTimelineButton
                    label={t(
                      pagingFailed
                        ? "common.retry"
                        : pagingActive || remote.loadingOlderTimeline
                          ? "chat.loadingOlder"
                          : "chat.loadOlder",
                    )}
                    icon={<History color={colors.inkSoft} size={16} />}
                    busy={pagingActive || remote.loadingOlderTimeline}
                    onPress={() => {
                      scroll.onScrollBeginDrag();
                      loadOlder();
                    }}
                    style={styles.loadOlder}
                  />
                )}

                {!remote.draft &&
                  timelineItems.length > 0 &&
                  (remote.timelineSyncStatus === "syncing" ||
                    remote.timelineSyncStatus === "retrying") && (
                    <View
                      pointerEvents="none"
                      style={[
                        styles.syncNotice,
                        showLoadOlderHint && styles.syncNoticeBelowHistory,
                      ]}
                      accessibilityLiveRegion="polite"
                    >
                      {remote.desktopOnline && (
                        <ActivityIndicator color={colors.accent} size="small" />
                      )}
                      <Text style={styles.syncNoticeText}>
                        {t(
                          !remote.desktopOnline
                            ? "chat.syncWaitingNetwork"
                            : remote.timelineSyncStatus === "retrying"
                              ? "chat.syncRetrying"
                              : "chat.syncingLatest",
                        )}
                      </Text>
                    </View>
                  )}
              </View>

              <ComposerDock
                key={`${conversationKey}:${remote.draft}:${remote.draftWorkspaceId}`}
                keyboardHeight={keyboardHeight}
                message={message}
                setMessage={setMessage}
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
              />
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
                onOpenFile={(path) => fileDownload.openFileLink(path, true)}
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
            action={fileDownload.fileAction}
            onClose={() => fileDownload.setFileAction(null)}
            onSelect={(action, operation) => {
              void fileDownload.openOrShare(
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
            preview={fileDownload.preview}
            activeDownload={fileDownload.activeDownload}
            closePreview={fileDownload.closePreview}
            dismissPreviewThen={fileDownload.dismissPreviewThen}
            downloadOriginal={fileDownload.downloadOriginal}
            flushPendingPreviewAction={fileDownload.flushPendingPreviewAction}
            t={t}
          />

          <DownloadProgressModal
            activeDownload={fileDownload.activeDownload}
            activeDownloadFraction={fileDownload.activeDownloadFraction}
            cancelActiveDownload={fileDownload.cancelActiveDownload}
            flushPendingDownloadModal={fileDownload.flushPendingDownloadModal}
            onDownloadModalShow={fileDownload.onDownloadModalShow}
            t={t}
          />

          <ModelSelectorSheet
            selector={selector}
            setSelector={setSelector}
            remote={remote}
            t={t}
          />

          <RenameModal
            renameOpen={rename.renameOpen}
            renameValue={rename.renameValue}
            setRenameValue={rename.setRenameValue}
            submitRename={rename.submitRename}
            onClose={() => rename.setRenameOpen(false)}
            t={t}
          />
        </KeyboardAvoidingView>
      </SafeAreaView>
    </MarkdownImageLoaderContext>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.surface },
  keyboard: {
    flex: 1,
    width: "100%",
    maxWidth: layout.contentMaxWidth,
    alignSelf: "center",
    backgroundColor: colors.surface,
  },
  chatContent: { flex: 1, minHeight: 0 },
  timelineList: { flex: 1, minHeight: 0 },
  // Overlay, not a list row: changing sync state must not shift the viewport.
  syncNotice: {
    position: "absolute",
    top: spacing.xs,
    left: layout.gutter,
    right: layout.gutter,
    zIndex: 2,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: spacing.sm,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    backgroundColor: colors.surfaceSubtle,
    borderRadius: radius.md,
  },
  syncNoticeText: { flexShrink: 1, color: colors.inkMuted, fontSize: 12 },
  // `inverted` flips the visual axis. A flexible physical header consumes only
  // the unused height of an underfilled list and therefore becomes visual
  // bottom space, leaving short conversations at the visual top. It collapses
  // to zero once message rows overflow the viewport.
  timeline: {
    flexGrow: 1,
    paddingHorizontal: layout.gutter,
    paddingVertical: spacing.sm,
  },
  timelineFlexSpacer: { flexGrow: 1 },
  emptyTimeline: {
    flexGrow: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  empty: { color: colors.inkMuted, fontSize: 14 },
  loadingState: {
    alignItems: "center",
    gap: spacing.sm,
    paddingVertical: spacing.xl,
  },
  historyError: { color: colors.inkMuted, fontSize: 14, textAlign: "center" },
  retryButton: {
    minHeight: layout.touchTarget,
    justifyContent: "center",
    backgroundColor: colors.accent,
    borderRadius: radius.pill,
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.sm,
  },
  retryPressed: { opacity: 0.75 },
  retryLabel: { color: colors.surface, fontSize: 14, fontWeight: "600" },
  itemGap: { height: spacing.md },
  timelineViewport: { flex: 1 },
  loadOlder: { top: spacing.sm },
  syncNoticeBelowHistory: { top: spacing.sm + layout.touchTarget + spacing.sm },
  transferTrack: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: 2,
    overflow: "hidden",
  },
  transferFill: {
    height: 2,
    borderRadius: radius.pill,
    backgroundColor: colors.accent,
  },
});
