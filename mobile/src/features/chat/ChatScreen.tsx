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
import { SafeAreaView, useSafeAreaInsets } from "react-native-safe-area-context";
import { TimelineCard } from "../../components/TimelineCard";
import { ErrorBanner } from "../../components/ErrorBanner";
import { useRemote } from "../../remote/RemoteContext";
import { modelReference, type TimelineItem } from "../../remote/types";
import { colors, radius, spacing } from "../../theme/tokens";
import { useComposerDraft } from "./useComposerDraft";
import { useAttachmentPicker } from "./useAttachmentPicker";
import { useFileDownload } from "./useFileDownload";
import { useChatScroll } from "./useChatScroll";
import { useTimelinePaging } from "./useTimelinePaging";
import { useRename } from "./useRename";
import { useSendMessage } from "./useSendMessage";
import { ChatTopBar } from "./components/ChatTopBar";
import { ComposerDock } from "./components/ComposerDock";
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

export function ChatScreen() {
  const { t } = useTranslation();
  const remote = useRemote();
  const { closeConversation } = remote;
  const insets = useSafeAreaInsets();

  const [transferProgress, setTransferProgress] = useState<number | null>(null);
  const [selector, setSelector] = useState<"model" | "thinking" | null>(null);
  const [showOffline, setShowOffline] = useState(false);
  // Android edge-to-edge: the built-in KeyboardAvoidingView is a no-op here
  // (behavior is undefined on Android) and RN's KAV mis-measures the keyboard
  // under edge-to-edge, so we lift the floating composer + list padding by the
  // measured keyboard height ourselves. iOS keeps relying on KAV padding.
  const [keyboardHeight, setKeyboardHeight] = useState(0);

  const title = remote.draft ? t("chat.new") : remote.selectedTitle || t("sessions.unnamed");
  const activeModel = remote.models.find(model => modelReference(model) === remote.modelId);
  const activeModelLabel =
    activeModel?.label || activeModel?.id || remote.modelId || t("chat.model");
  const supportsImages = activeModel ? activeModel.supportsImages !== false : true;

  const { message, setMessage, attachments, setAttachments } = useComposerDraft(remote, t);
  const { openAttachmentMenu } = useAttachmentPicker(attachments, setAttachments, t);
  const fileDownload = useFileDownload(remote, t, setTransferProgress);
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
    () => timelineItems.filter(item => item.kind !== "approval"),
    [timelineItems],
  );
  // FlatList's physical start is the stable latest-message anchor. Reversing
  // only the view data keeps the remote timeline chronological while making
  // older history append at the far end instead of shifting visible rows.
  const invertedTranscriptItems = useMemo(() => newestFirst(transcriptItems), [transcriptItems]);
  const latestAssistantId = useMemo(() => {
    for (let index = transcriptItems.length - 1; index >= 0; index -= 1) {
      const item = transcriptItems[index];
      if (item?.kind === "message") return item.role === "assistant" ? item.id : null;
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
  const [approvalSubmitting, setApprovalSubmitting] = useState<string | null>(null);
  const [approvalError, setApprovalError] = useState<string | null>(null);
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
        await remote.decideApproval(id, decision);
      } catch {
        setApprovalError(t("approval.submitFailed"));
      } finally {
        setApprovalSubmitting(null);
      }
    },
    [remote, t],
  );

  const { listRef, atLatest, composerHeight, setComposerHeight, scrollToLatest, onScroll } =
    useChatScroll(remote.selectedSessionId);
  const {
    showLoadOlderHint,
    loadOlder,
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
      <TimelineCard
        item={item}
        isLatestAssistant={item.id === latestAssistantId}
        onOpenAttachment={handleTimelineAttachment}
        onOpenFile={handleTimelineFile}
        onRetry={handleTimelineRetry}
        onContinue={handleTimelineContinue}
      />
    ),
    [
      handleTimelineAttachment,
      handleTimelineContinue,
      handleTimelineFile,
      handleTimelineRetry,
      latestAssistantId,
    ],
  );

  useEffect(() => {
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      closeConversation();
      return true;
    });
    return () => subscription.remove();
  }, [closeConversation]);

  // Keyboard compensation is Android-only: iOS already adapts via KAV padding,
  // and stacking both would double-shift the composer. We subtract the bottom
  // safe-area inset because SafeAreaView's bottom edge already lifts the content
  // above the nav bar — without the subtraction the composer would float a
  // nav-bar-height gap above the keyboard.
  useEffect(() => {
    if (Platform.OS !== "android") return;
    const showSub = Keyboard.addListener("keyboardDidShow", event =>
      setKeyboardHeight(event.endCoordinates.height),
    );
    const hideSub = Keyboard.addListener("keyboardDidHide", () => setKeyboardHeight(0));
    return () => {
      showSub.remove();
      hideSub.remove();
    };
  }, []);

  useEffect(() => {
    const timer = setTimeout(
      () => setShowOffline(!remote.desktopOnline),
      remote.desktopOnline ? 0 : 3_000,
    );
    return () => clearTimeout(timer);
  }, [remote.desktopOnline]);

  // Lift the composer so the floating *card* clears the keyboard, not the dock:
  // the dock's bottom edge includes composerArea's paddingBottom (spacing.md) of
  // empty space below the card, so lifting the dock flush to the keyboard top
  // still hides the card's bottom border + rounded corners + shadow by that
  // padding (exactly the clipping seen with Gboard). Re-add the padding, plus a
  // little shadow clearance, so the whole card floats above the IME.
  const KEYBOARD_CLEARANCE = spacing.md + spacing.sm;
  const keyboardLift =
    Platform.OS === "android" && keyboardHeight > 0
      ? Math.max(0, keyboardHeight - insets.bottom + KEYBOARD_CLEARANCE)
      : 0;

  return (
    <SafeAreaView edges={["top", "bottom"]} style={styles.safe}>
      <KeyboardAvoidingView
        behavior={Platform.OS === "ios" ? "padding" : undefined}
        style={styles.keyboard}
      >
        <ChatTopBar
          title={title}
          draft={remote.draft}
          backLabel={t("common.back")}
          renameLabel={t("chat.rename")}
          onBack={closeConversation}
          onRename={rename.openRename}
        />

        {remote.error && (
          <ErrorBanner
            message={remote.error}
            onDismiss={remote.phase === "failed" ? undefined : remote.clearError}
          />
        )}

        <View style={styles.chatContent}>
          {/* The inverted newest-first view makes offset zero equal "latest".
              Older pages append at the opposite edge, so their async Markdown
              layout cannot move the reader's current viewport. */}
          <FlatList
            contentContainerStyle={[
              styles.timeline,
              {
                // The scroll container is inverted, so logical top padding is
                // rendered at the visual bottom beside the floating composer.
                paddingTop: composerHeight + COMPOSER_FADE_CLEARANCE + spacing.lg + keyboardLift,
              },
              timelineItems.length === 0 && styles.emptyTimeline,
            ]}
            data={invertedTranscriptItems}
            initialNumToRender={10}
            inverted
            key={remote.selectedSessionId || "draft"}
            keyExtractor={item => item.id}
            ListEmptyComponent={
              remote.timelineError ? (
                <View style={styles.loadingState}>
                  <Text style={styles.historyError}>{t("chat.historyLoadTimedOut")}</Text>
                  <Pressable
                    accessibilityRole="button"
                    onPress={() => void remote.retryTimeline()}
                    style={({ pressed }) => [styles.retryButton, pressed && styles.retryPressed]}
                  >
                    <Text style={styles.retryLabel}>{t("common.retry")}</Text>
                  </Pressable>
                </View>
              ) : loadingHistory ? (
                <View style={styles.loadingState}>
                  <ActivityIndicator color={colors.accent} />
                  <Text style={styles.empty}>{t("chat.loadingHistory")}</Text>
                </View>
              ) : (
                <Text style={styles.empty}>{t("chat.noHistory")}</Text>
              )
            }
            maintainVisibleContentPosition={{
              minIndexForVisible: 0,
              autoscrollToTopThreshold: 32,
            }}
            maxToRenderPerBatch={8}
            onScroll={onPagedScroll}
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
            <Pressable
              accessibilityRole="button"
              onPress={loadOlder}
              style={({ pressed }) => [styles.loadOlder, pressed && styles.loadOlderPressed]}
            >
              <History color={colors.inkMuted} size={14} />
              <Text style={styles.loadOlderLabel}>{t("chat.loadOlder")}</Text>
            </Pressable>
          )}

          <ComposerDock
            message={message}
            setMessage={setMessage}
            attachments={attachments}
            setAttachments={setAttachments}
            supportsImages={supportsImages}
            activeModelLabel={activeModelLabel}
            remote={remote}
            t={t}
            openAttachmentMenu={openAttachmentMenu}
            send={send}
            atLatest={atLatest}
            scrollToLatest={scrollToLatest}
            showOffline={showOffline}
            pendingApprovals={pendingApprovals}
            approvalSubmitting={approvalSubmitting}
            approvalError={approvalError}
            decideApproval={decideApproval}
            setComposerHeight={setComposerHeight}
            keyboardLift={keyboardLift}
            selector={selector}
            setSelector={setSelector}
          />

          {transferProgress != null && (
            <View pointerEvents="none" style={styles.transferTrack}>
              <View
                style={[styles.transferFill, { width: `${Math.max(2, transferProgress * 100)}%` }]}
              />
            </View>
          )}
        </View>

        <NativeFileActionSheet
          action={fileDownload.fileAction}
          cancelLabel={t("chat.cancel")}
          onClose={() => fileDownload.setFileAction(null)}
          onSelect={(action, save) => {
            void fileDownload.openOrShare(
              action.info,
              action.cachedFile,
              save,
              undefined,
              action.openMimeType,
            );
          }}
          openLabel={t("attachment.open")}
          saveLabel={t("attachment.save")}
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

        <ModelSelectorSheet selector={selector} setSelector={setSelector} remote={remote} t={t} />

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
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.surface },
  keyboard: { flex: 1, backgroundColor: colors.surface },
  chatContent: { flex: 1 },
  timelineList: { flex: 1 },
  timeline: { padding: spacing.lg },
  emptyTimeline: { flexGrow: 1, alignItems: "center", justifyContent: "center" },
  empty: { color: colors.inkMuted, fontSize: 14 },
  loadingState: { alignItems: "center", gap: spacing.sm, paddingVertical: spacing.xl },
  historyError: { color: colors.inkMuted, fontSize: 14, textAlign: "center" },
  retryButton: {
    backgroundColor: colors.accent,
    borderRadius: radius.pill,
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.sm,
  },
  retryPressed: { opacity: 0.75 },
  retryLabel: { color: colors.surface, fontSize: 14, fontWeight: "600" },
  itemGap: { height: spacing.md },
  loadOlder: {
    position: "absolute",
    top: spacing.sm,
    alignSelf: "center",
    zIndex: 2,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.line,
    borderRadius: radius.pill,
    backgroundColor: colors.surface,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.xs,
  },
  loadOlderPressed: { opacity: 0.72 },
  loadOlderLabel: { color: colors.inkMuted, fontSize: 12 },
  transferTrack: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: 2,
    overflow: "hidden",
  },
  transferFill: { height: 2, borderRadius: radius.pill, backgroundColor: colors.accent },
});
