import {
  ArrowDown,
  ArrowLeft,
  Camera,
  Check,
  ChevronDown,
  CircleAlert,
  FileText,
  Paperclip,
  Pencil,
  Send,
  Square,
  X,
} from "lucide-react-native";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ActivityIndicator,
  Alert,
  ActionSheetIOS,
  BackHandler,
  FlatList,
  Image,
  Keyboard,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  TouchableWithoutFeedback,
  View,
} from "react-native";
import * as Network from "expo-network";
import { File } from "expo-file-system";
import { SafeAreaView, useSafeAreaInsets } from "react-native-safe-area-context";
import Svg, { Defs, LinearGradient, Rect, Stop } from "react-native-svg";
import { PendingApprovalCard, TimelineCard } from "../components/TimelineCard";
import { Button } from "../components/Button";
import { ErrorBanner } from "../components/ErrorBanner";
import { MarkdownText } from "../components/MarkdownText";
import { useRemote } from "../remote/RemoteContext";
import { loadSessionDraft, saveSessionDraft } from "../remote/draftStorage";
import { deleteTemporaryAttachment, pickAttachments, takePhoto } from "../remote/files";
import {
  modelReference,
  type DownloadInfo,
  type HistoryAttachment,
  type MobileAttachment,
  type ThinkingLevel,
  type TimelineItem,
} from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";

const thinkingLevels: ThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh"];

// How close to the bottom counts as "at latest" (px). Shared by the atLatest
// detection and the scroll target so the two never disagree.
const AT_LATEST_THRESHOLD = 32;

// The fade band above the composer dock (styles.composerFade) is part of the
// dock's visual footprint: the list's bottom padding must clear it too, or a
// settled reply's footer ("time · tokens" + copy) rests under the
// semi-transparent gradient.
const COMPOSER_FADE_CLEARANCE = 48;
const MARKDOWN_RENDER_BYTES = 2 * 1024 * 1024;

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${Math.max(1, Math.ceil(bytes / 1024))} KB`;
}

function fileExtension(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

function plainText(bytes: Uint8Array): string | null {
  // Binary formats such as PDF contain NUL or C0 control bytes. The desktop
  // repeats a stricter UTF-8 check before it transfers a durable attachment.
  if (bytes.some(byte => byte === 0 || (byte < 32 && byte !== 9 && byte !== 10 && byte !== 13))) {
    return null;
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

function confirmDownload(title: string, message: string, cancel: string, download: string) {
  return new Promise<boolean>(resolve => {
    Alert.alert(
      title,
      message,
      [
        { text: cancel, style: "cancel", onPress: () => resolve(false) },
        { text: download, onPress: () => resolve(true) },
      ],
      { cancelable: true, onDismiss: () => resolve(false) },
    );
  });
}

export function ChatScreen() {
  const { t } = useTranslation();
  const remote = useRemote();
  const { closeConversation } = remote;
  const insets = useSafeAreaInsets();
  const listRef = useRef<FlatList<TimelineItem>>(null);
  const [message, setMessage] = useState("");
  const [attachments, setAttachments] = useState<MobileAttachment[]>([]);
  const [attachmentMenu, setAttachmentMenu] = useState(false);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  const [transferProgress, setTransferProgress] = useState<number | null>(null);
  const [preview, setPreview] = useState<{
    info: DownloadInfo;
    uri: string;
    markdown?: string;
    text?: string;
    truncated?: boolean;
  } | null>(null);
  const [selector, setSelector] = useState<"model" | "thinking" | null>(null);
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const [showOffline, setShowOffline] = useState(false);
  const [atLatest, setAtLatest] = useState(true);
  // Per-session composer draft: the unsent text/attachments survive leaving the
  // screen and coming back (G6). The draft conversation (no session yet) uses a
  // fixed key so a re-created new-conversation draft restores what was started.
  const draftKey = remote.selectedSessionId || "draft:new";
  const restoringDraftRef = useRef(false);
  const activeDraftKeyRef = useRef(draftKey);
  // Measured height of the floating composer dock — the list's bottom padding
  // tracks it so the last message always lands just above the composer (no
  // gap, no hidden content) regardless of docked approvals or input height.
  const [composerHeight, setComposerHeight] = useState(0);
  // Android edge-to-edge: the built-in KeyboardAvoidingView is a no-op here
  // (behavior is undefined on Android) and RN's KAV mis-measures the keyboard
  // under edge-to-edge, so we lift the floating composer + list padding by the
  // measured keyboard height ourselves. iOS keeps relying on KAV padding.
  const [keyboardHeight, setKeyboardHeight] = useState(0);
  const contentHeightRef = useRef(0);
  const layoutHeightRef = useRef(0);
  const title = remote.draft ? t("chat.new") : remote.selectedTitle || t("sessions.unnamed");
  const activeModel = remote.models.find(model => modelReference(model) === remote.modelId);
  const activeModelLabel =
    activeModel?.label || activeModel?.id || remote.modelId || t("chat.model");
  const supportsImages = activeModel ? activeModel.supportsImages !== false : true;

  // Approvals live docked above the composer (not inline in the transcript), and
  // only while undecided — once a decision lands the card disappears.
  const timelineItems = useMemo(() => remote.timeline.items, [remote.timeline]);
  const transcriptItems = useMemo(
    () => timelineItems.filter(item => item.kind !== "approval"),
    [timelineItems],
  );
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
  const loadingHistory = !remote.draft && remote.busy && timelineItems.length === 0;
  // The first content render snaps to the end without animation; only later
  // appends (streaming, new messages) scroll animated.
  const landedRef = useRef(false);
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

  const openRename = () => {
    // Pre-fill the current title so the user can edit in place.
    setRenameValue(remote.selectedTitle || "");
    setRenameOpen(true);
  };

  const submitRename = async () => {
    const name = renameValue.trim();
    if (!name) return;
    setRenameOpen(false);
    try {
      await remote.rename(remote.selectedSessionId, name);
    } catch {
      Alert.alert(t("common.error"));
    }
  };

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

  // Restore this conversation's draft when the screen (re)opens. Uses the
  // sessionId captured at mount — the draft conversation key stays stable
  // across the "" placeholder, and a bound session keeps its own slot.
  useEffect(() => {
    restoringDraftRef.current = true;
    activeDraftKeyRef.current = draftKey;
    const key = draftKey;
    void loadSessionDraft(key).then(draft => {
      // Guard against a conversation switch racing the async load.
      if (restoringDraftRef.current && activeDraftKeyRef.current === key) {
        setMessage(draft?.text ?? "");
        setAttachments(draft?.attachments ?? []);
        restoringDraftRef.current = false;
      }
    });
  }, [draftKey]);

  // Persist edits. The restore-driven update is skipped (the effect above
  // already loaded the draft), and temporary camera/cache files are released
  // when removed so they don't accumulate on disk.
  useEffect(() => {
    if (restoringDraftRef.current) return;
    void saveSessionDraft(draftKey, { text: message, attachments });
  }, [attachments, draftKey, message]);

  const chooseFiles = async () => {
    setAttachmentMenu(false);
    setAttachmentError(null);
    try {
      // Android's fallback sheet must finish dismissing before UIKit/Android
      // presents another native controller. Otherwise the picker can be
      // rejected as a concurrent presentation and looks like a no-op.
      if (attachmentMenu) await new Promise(resolve => setTimeout(resolve, 200));
      setAttachments(await pickAttachments(attachments));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      setAttachmentError(t(`attachment.errors.${key}`));
    }
  };

  const capturePhoto = async () => {
    setAttachmentMenu(false);
    setAttachmentError(null);
    try {
      if (attachmentMenu) await new Promise(resolve => setTimeout(resolve, 200));
      setAttachments(await takePhoto(attachments));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      setAttachmentError(t(`attachment.errors.${key}`));
    }
  };

  const openAttachmentMenu = () => {
    if (Platform.OS === "ios") {
      ActionSheetIOS.showActionSheetWithOptions(
        {
          options: [t("attachment.chooseFiles"), t("attachment.takePhoto"), t("chat.cancel")],
          cancelButtonIndex: 2,
        },
        index => {
          // Schedule after the action sheet's dismissal animation, so the
          // document/camera controller always gets a presentable view host.
          if (index === 0) setTimeout(() => void chooseFiles(), 0);
          if (index === 1) setTimeout(() => void capturePhoto(), 0);
        },
      );
      return;
    }
    setAttachmentMenu(true);
  };

  const send = async () => {
    const value = message.trim();
    if (!value && attachments.length === 0) return;
    const pendingAttachments = attachments;
    setMessage("");
    setTransferProgress(pendingAttachments.length ? 0 : null);
    try {
      await remote.sendMessage(value, pendingAttachments, (done, total) =>
        setTransferProgress(total > 0 ? done / total : null),
      );
      setAttachments([]);
    } catch (error) {
      // M9: sendMessage now throws for busy/streaming/disconnected instead of
      // swallowing the input — always restore the draft so nothing vanishes.
      setMessage(value);
      const key = error instanceof Error ? error.message : "";
      setAttachmentError(key === "prompt_too_large" ? t("chat.promptTooLarge") : t("chat.sendFailed"));
    } finally {
      setTransferProgress(null);
    }
  };

  const retryMessage = useCallback(
    (item: TimelineItem) => {
      if (item.kind !== "message" || item.role !== "assistant") return;
      const items = remote.timeline.items;
      const index = items.findIndex(entry => entry.id === item.id);
      for (let i = index - 1; i >= 0; i -= 1) {
        const prev = items[i];
        if (prev?.kind === "message" && prev.role === "user") {
          void remote.sendMessage(prev.text).catch(() => setAttachmentError(t("chat.sendFailed")));
          return;
        }
      }
    },
    [remote, t],
  );

  const continueMessage = useCallback(
    (item: TimelineItem) => {
      if (item.kind !== "message" || item.role !== "assistant" || !item.runId) return;
      void remote
        .continueRun(remote.selectedSessionId, item.runId)
        .catch(() => setAttachmentError(t("chat.sendFailed")));
    },
    [remote, t],
  );

  const openAttachment = useCallback(
    async (attachment: HistoryAttachment) => {
      setAttachmentError(null);
      setTransferProgress(0);
      try {
        // The just-sent optimistic bubble still points at this phone's local
        // picker URI. Open it directly; durable history later replaces this
        // with the desktop path used by the NATS download flow.
        if (/^[a-z][a-z0-9+.-]*:\/\//i.test(attachment.path)) {
          const local = new File(attachment.path);
          const ext = fileExtension(attachment.name);
          if (
            attachment.kind === "image" &&
            ext !== "gif" &&
            !attachment.mobilePreviewUnsupported
          ) {
            setPreview({
              info: {
                transferId: "local",
                name: attachment.name,
                mimeType: "image/*",
                size: local.size,
                contentHash: "",
                previewKind: "image",
                chunkBytes: 0,
              },
              uri: local.uri,
            });
          } else {
            const bytes = await local.bytes();
            const text = plainText(bytes);
            if (text === null) {
              Alert.alert(t("attachment.title"), t("attachment.previewOnDesktop"));
              return;
            }
            const visible = bytes.slice(0, MARKDOWN_RENDER_BYTES);
            const previewText =
              visible.byteLength === bytes.byteLength ? text : new TextDecoder().decode(visible);
            const markdown = ext === "md" || ext === "markdown";
            setPreview({
              info: {
                transferId: "local",
                name: attachment.name,
                mimeType: markdown ? "text/markdown" : "text/plain",
                size: local.size,
                contentHash: "",
                previewKind: markdown ? "markdown" : "text",
                chunkBytes: 0,
              },
              uri: local.uri,
              ...(markdown ? { markdown: previewText } : { text: previewText }),
              truncated: bytes.byteLength > visible.byteLength,
            });
          }
          return;
        }
        const cachedPreview = remote.cachedAttachment(attachment);
        const info = cachedPreview?.info ?? (await remote.prepareAttachment(attachment));
        if (
          info.previewKind !== "image" &&
          info.previewKind !== "markdown" &&
          info.previewKind !== "text"
        ) {
          Alert.alert(t("attachment.title"), t("attachment.previewOnDesktop"));
          return;
        }
        let file = cachedPreview?.file ?? null;
        if (!file) {
          const network = await Network.getNetworkStateAsync();
          if (
            network.type === Network.NetworkStateType.CELLULAR ||
            network.type === Network.NetworkStateType.UNKNOWN
          ) {
            const accepted = await confirmDownload(
              t("attachment.downloadTitle"),
              t("attachment.cellularWarning", { size: formatBytes(info.size) }),
              t("chat.cancel"),
              t("attachment.download"),
            );
            if (!accepted) return;
          }
          file = await remote.downloadAttachment(info, (done, total) =>
            setTransferProgress(total > 0 ? done / total : null),
          );
        }
        if (info.previewKind === "image") {
          setPreview({ info, uri: file.uri });
        } else {
          const bytes = await file.bytes();
          const visible = bytes.slice(0, MARKDOWN_RENDER_BYTES);
          const previewText = new TextDecoder().decode(visible);
          setPreview({
            info,
            uri: file.uri,
            ...(info.previewKind === "markdown"
              ? { markdown: previewText }
              : { text: previewText }),
            truncated: bytes.byteLength > visible.byteLength,
          });
        }
      } catch (error) {
        const detail = error instanceof Error ? error.message : "";
        const message =
          detail.includes("view it on desktop") || detail.includes("GIF preview")
            ? t("attachment.previewOnDesktop")
            : t("attachment.downloadFailed");
        Alert.alert(t("attachment.title"), message);
      } finally {
        setTransferProgress(null);
      }
    },
    [remote, t],
  );

  // The bottom-most scroll offset: full content height minus the viewport,
  // never negative. Recomputed from measured sizes so it stays correct as the
  // composer (bottom padding) and content grow.
  const maxScrollOffset = () => Math.max(0, contentHeightRef.current - layoutHeightRef.current);

  // scrollToEnd is unreliable on Android for the first layout of a large
  // history (it can no-op or use a stale content size, leaving a gap). An
  // explicit offset computed from the measured content size is deterministic;
  // the rAF retry catches content that finishes laying out a frame late.
  const scrollToLatest = () => {
    setAtLatest(true);
    listRef.current?.scrollToOffset({ animated: true, offset: maxScrollOffset() });
    requestAnimationFrame(() =>
      listRef.current?.scrollToOffset({ animated: true, offset: maxScrollOffset() }),
    );
  };

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
        <View style={styles.topbar}>
          <Pressable
            accessibilityLabel={t("common.back")}
            accessibilityRole="button"
            onPress={closeConversation}
            style={styles.iconButton}
          >
            <ArrowLeft color={colors.ink} size={22} />
          </Pressable>
          <View style={styles.titleWrap}>
            <Text numberOfLines={1} style={styles.title}>
              {title}
            </Text>
          </View>
          {!remote.draft && (
            <Pressable
              accessibilityLabel={t("chat.rename")}
              accessibilityRole="button"
              onPress={openRename}
              style={styles.iconButton}
            >
              <Pencil color={colors.ink} size={18} />
            </Pressable>
          )}
          {remote.draft && <View style={styles.iconButton} />}
        </View>

        {remote.error && <ErrorBanner message={remote.error} onDismiss={remote.clearError} />}

        <View style={styles.chatContent}>
          <FlatList
            contentContainerStyle={[
              styles.timeline,
              {
                paddingBottom: composerHeight + COMPOSER_FADE_CLEARANCE + spacing.lg + keyboardLift,
              },
              timelineItems.length === 0 && styles.emptyTimeline,
            ]}
            data={transcriptItems}
            keyExtractor={item => item.id}
            ListEmptyComponent={
              loadingHistory ? (
                <ActivityIndicator color={colors.accent} />
              ) : (
                <Text style={styles.empty}>{t("chat.noHistory")}</Text>
              )
            }
            onContentSizeChange={(_w, h) => {
              contentHeightRef.current = h;
              if (!atLatest) return;
              if (!landedRef.current && transcriptItems.length === 0) return;
              listRef.current?.scrollToOffset({
                animated: landedRef.current,
                offset: maxScrollOffset(),
              });
              landedRef.current = true;
            }}
            onLayout={event => {
              layoutHeightRef.current = event.nativeEvent.layout.height;
            }}
            onScroll={event => {
              const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
              setAtLatest(
                contentOffset.y + layoutMeasurement.height >=
                  contentSize.height - AT_LATEST_THRESHOLD,
              );
            }}
            ref={listRef}
            renderItem={({ item }) => (
              <TimelineCard
                item={item}
                onOpenAttachment={attachment => void openAttachment(attachment)}
                onRetry={retryMessage}
                onContinue={continueMessage}
              />
            )}
            scrollEventThrottle={16}
            scrollIndicatorInsets={{ bottom: 0 }}
            style={styles.timelineList}
            ItemSeparatorComponent={() => <View style={styles.itemGap} />}
          />

          <View
            onLayout={event => setComposerHeight(event.nativeEvent.layout.height)}
            style={[styles.composerDock, keyboardLift > 0 ? { bottom: keyboardLift } : null]}
          >
            <View pointerEvents="none" style={styles.composerFade}>
              <Svg height="100%" width="100%">
                <Defs>
                  <LinearGradient id="composerFade" x1="0" x2="0" y1="0" y2="1">
                    <Stop offset="0" stopColor={colors.surface} stopOpacity="0" />
                    <Stop offset="1" stopColor={colors.surface} stopOpacity="0.96" />
                  </LinearGradient>
                </Defs>
                <Rect fill="url(#composerFade)" height="100%" width="100%" />
              </Svg>
            </View>
            {!atLatest && (
              <Pressable
                accessibilityLabel={t("chat.backToLatest")}
                accessibilityRole="button"
                onPress={scrollToLatest}
                style={styles.backToLatest}
              >
                <ArrowDown color={colors.inkSoft} size={16} />
                <Text style={styles.backToLatestText}>{t("chat.backToLatest")}</Text>
              </Pressable>
            )}
            {showOffline && (
              <Text style={styles.offlineComposer}>{t("connection.offlineHint")}</Text>
            )}
            {!remote.draft &&
              remote.desktopOnline &&
              remote.models.length === 0 &&
              !remote.modelId && (
              <Text style={styles.offlineComposer}>{t("connection.noModelsHint")}</Text>
            )}
            {pendingApprovals.map(item => (
              <View key={item.id} style={styles.dockedApproval}>
                <PendingApprovalCard
                  error={
                    approvalSubmitting === item.payload.approval_request_id ? null : approvalError
                  }
                  onDecision={decision =>
                    void decideApproval(item.payload.approval_request_id, decision)
                  }
                  payload={item.payload}
                  submitting={approvalSubmitting === item.payload.approval_request_id}
                />
              </View>
            ))}
            <View style={styles.composerArea}>
              <View style={styles.composer}>
                {attachments.length > 0 && (
                  <ScrollView
                    contentContainerStyle={styles.pendingAttachments}
                    horizontal
                    keyboardShouldPersistTaps="handled"
                    showsHorizontalScrollIndicator={false}
                  >
                    {attachments.map((attachment, index) => (
                      <View
                        key={`${attachment.localUri}:${index}`}
                        style={styles.pendingAttachment}
                      >
                        {attachment.kind === "image" && !supportsImages ? (
                          <CircleAlert color={colors.warning} size={13} />
                        ) : attachment.kind === "image" ? (
                          <Paperclip color={colors.inkSoft} size={13} />
                        ) : (
                          <FileText color={colors.inkSoft} size={13} />
                        )}
                        <View style={styles.pendingAttachmentCopy}>
                          <Text numberOfLines={1} style={styles.pendingAttachmentName}>
                            {attachment.name}
                          </Text>
                          <Text style={styles.pendingAttachmentSize}>
                            {formatBytes(attachment.originalSize)}
                          </Text>
                        </View>
                        <Pressable
                          accessibilityLabel={t("attachment.remove", { name: attachment.name })}
                          hitSlop={8}
                          onPress={() =>
                            setAttachments(current => {
                              deleteTemporaryAttachment(current[index]!);
                              return current.filter((_, itemIndex) => itemIndex !== index);
                            })
                          }
                        >
                          <X color={colors.inkMuted} size={14} />
                        </Pressable>
                      </View>
                    ))}
                  </ScrollView>
                )}
                {attachments.some(a => a.kind === "image") && !supportsImages && (
                  <Text style={styles.attachmentWarning}>{t("attachment.imagesUnsupported")}</Text>
                )}
                {!!attachmentError && <Text style={styles.attachmentError}>{attachmentError}</Text>}
                <TextInput
                  accessibilityLabel={t("chat.placeholder")}
                  editable={remote.desktopOnline && !remote.timeline.streaming && !remote.busy}
                  multiline
                  onChangeText={setMessage}
                  onSubmitEditing={() => void send()}
                  placeholder={t("chat.placeholder")}
                  placeholderTextColor={colors.inkMuted}
                  style={styles.input}
                  value={message}
                />
                <View style={styles.composerToolbar}>
                  <Pressable
                    accessibilityLabel={t("attachment.add")}
                    accessibilityRole="button"
                    disabled={
                      remote.timeline.streaming || remote.busy || !remote.fileTransferSupported
                    }
                    onPress={openAttachmentMenu}
                    style={({ pressed }) => [
                      styles.attachmentButton,
                      pressed && styles.selectorTriggerPressed,
                      (remote.timeline.streaming || remote.busy || !remote.fileTransferSupported) &&
                        styles.controlDisabled,
                    ]}
                  >
                    <Paperclip color={colors.inkSoft} size={17} />
                  </Pressable>
                  <View style={styles.composerSelectors}>
                    <Pressable
                      accessibilityLabel={t("chat.model")}
                      accessibilityRole="button"
                      disabled={remote.timeline.streaming}
                      onPress={() => setSelector("model")}
                      style={({ pressed }) => [
                        styles.selectorTrigger,
                        pressed && styles.selectorTriggerPressed,
                        remote.timeline.streaming && styles.controlDisabled,
                      ]}
                    >
                      <Text numberOfLines={1} style={styles.selectorText}>
                        {activeModelLabel}
                      </Text>
                      <ChevronDown color={colors.inkMuted} size={14} />
                    </Pressable>
                    <Pressable
                      accessibilityLabel={t("chat.thinkingLevel")}
                      accessibilityRole="button"
                      disabled={remote.timeline.streaming}
                      onPress={() => setSelector("thinking")}
                      style={({ pressed }) => [
                        styles.selectorTrigger,
                        pressed && styles.selectorTriggerPressed,
                        remote.timeline.streaming && styles.controlDisabled,
                      ]}
                    >
                      <Text numberOfLines={1} style={styles.selectorText}>
                        {t(`thinking.${remote.thinkingLevel}`)}
                      </Text>
                      <ChevronDown color={colors.inkMuted} size={14} />
                    </Pressable>
                  </View>
                  {remote.timeline.streaming ? (
                    <Pressable
                      accessibilityLabel={t("chat.stop")}
                      accessibilityRole="button"
                      onPress={() => void remote.abort()}
                      style={[styles.sendButton, styles.stopButton]}
                    >
                      <Square color={colors.surface} fill={colors.surface} size={14} />
                    </Pressable>
                  ) : (
                    <Pressable
                      accessibilityLabel={t("chat.send")}
                      accessibilityRole="button"
                      disabled={
                        (!message.trim() && attachments.length === 0) ||
                        remote.busy ||
                        !remote.desktopOnline
                      }
                      onPress={() => void send()}
                      style={({ pressed }) => [
                        styles.sendButton,
                        ((!message.trim() && attachments.length === 0) ||
                          remote.busy ||
                          !remote.desktopOnline) &&
                          styles.sendDisabled,
                        pressed && styles.sendPressed,
                      ]}
                    >
                      <Send color={colors.surface} size={17} />
                    </Pressable>
                  )}
                </View>
              </View>
            </View>
          </View>

          {transferProgress != null && (
            <View pointerEvents="none" style={styles.transferTrack}>
              <View
                style={[styles.transferFill, { width: `${Math.max(2, transferProgress * 100)}%` }]}
              />
            </View>
          )}
        </View>

        <Modal
          animationType="fade"
          onRequestClose={() => setAttachmentMenu(false)}
          transparent
          visible={attachmentMenu}
        >
          <TouchableWithoutFeedback onPress={() => setAttachmentMenu(false)}>
            <View style={styles.attachmentOverlay}>
              <TouchableWithoutFeedback>
                <View style={styles.attachmentMenu}>
                  <Pressable onPress={() => void chooseFiles()} style={styles.attachmentMenuOption}>
                    <FileText color={colors.ink} size={20} />
                    <Text style={styles.attachmentMenuText}>{t("attachment.chooseFiles")}</Text>
                  </Pressable>
                  <Pressable
                    onPress={() => void capturePhoto()}
                    style={styles.attachmentMenuOption}
                  >
                    <Camera color={colors.ink} size={20} />
                    <Text style={styles.attachmentMenuText}>{t("attachment.takePhoto")}</Text>
                  </Pressable>
                </View>
              </TouchableWithoutFeedback>
            </View>
          </TouchableWithoutFeedback>
        </Modal>

        <Modal
          animationType="slide"
          onRequestClose={() => setPreview(null)}
          presentationStyle="pageSheet"
          visible={preview !== null}
        >
          <SafeAreaView style={styles.previewSafe}>
            <View style={styles.previewHeader}>
              <Text numberOfLines={1} style={styles.previewTitle}>
                {preview?.info.name}
              </Text>
              <Pressable accessibilityLabel={t("common.close")} onPress={() => setPreview(null)}>
                <X color={colors.ink} size={22} />
              </Pressable>
            </View>
            {preview?.info.previewKind === "image" ? (
              <Image
                resizeMode="contain"
                source={{ uri: preview.uri }}
                style={styles.previewImage}
              />
            ) : preview?.info.previewKind === "markdown" ? (
              <ScrollView contentContainerStyle={styles.previewMarkdown}>
                {!!preview?.truncated && (
                  <Text style={styles.previewTruncated}>{t("attachment.markdownTruncated")}</Text>
                )}
                <MarkdownText text={preview?.markdown ?? ""} />
              </ScrollView>
            ) : (
              <ScrollView contentContainerStyle={styles.previewMarkdown}>
                {!!preview?.truncated && (
                  <Text style={styles.previewTruncated}>{t("attachment.textTruncated")}</Text>
                )}
                <Text selectable style={styles.previewText}>
                  {preview?.text ?? ""}
                </Text>
              </ScrollView>
            )}
          </SafeAreaView>
        </Modal>

        <Modal
          animationType="fade"
          onRequestClose={() => setSelector(null)}
          transparent
          visible={selector !== null}
        >
          <TouchableWithoutFeedback onPress={() => setSelector(null)}>
            <View style={styles.selectorOverlay}>
              <TouchableWithoutFeedback>
                <View style={styles.selectorMenu}>
                  <Text style={styles.selectorTitle}>
                    {selector === "model" ? t("chat.model") : t("chat.thinkingLevel")}
                  </Text>
                  <ScrollView bounces={false}>
                    {selector === "model"
                      ? remote.models.map(model => {
                          const selected = modelReference(model) === remote.modelId;
                          return (
                            <Pressable
                              key={`${model.provider ?? ""}/${model.id}`}
                              onPress={() => {
                                setSelector(null);
                                void remote.setModel(modelReference(model));
                              }}
                              style={({ pressed }) => [
                                styles.selectorOption,
                                selected && styles.selectorOptionSelected,
                                pressed && styles.selectorOptionPressed,
                              ]}
                            >
                              <View style={styles.selectorOptionCopy}>
                                <Text numberOfLines={1} style={styles.selectorOptionLabel}>
                                  {model.label || model.id}
                                </Text>
                                {model.provider ? (
                                  <Text numberOfLines={1} style={styles.selectorOptionMeta}>
                                    {model.provider}
                                  </Text>
                                ) : null}
                              </View>
                              {selected ? <Check color={colors.accent} size={18} /> : null}
                            </Pressable>
                          );
                        })
                      : thinkingLevels.map(level => {
                          const selected = level === remote.thinkingLevel;
                          return (
                            <Pressable
                              key={level}
                              onPress={() => {
                                setSelector(null);
                                void remote.setThinkingLevel(level);
                              }}
                              style={({ pressed }) => [
                                styles.selectorOption,
                                selected && styles.selectorOptionSelected,
                                pressed && styles.selectorOptionPressed,
                              ]}
                            >
                              <Text style={styles.selectorOptionLabel}>
                                {t(`thinking.${level}`)}
                              </Text>
                              {selected ? <Check color={colors.accent} size={18} /> : null}
                            </Pressable>
                          );
                        })}
                  </ScrollView>
                </View>
              </TouchableWithoutFeedback>
            </View>
          </TouchableWithoutFeedback>
        </Modal>
        <Modal
          animationType="fade"
          onRequestClose={() => setRenameOpen(false)}
          transparent
          visible={renameOpen}
        >
          <View style={styles.overlay}>
            <View style={styles.dialog}>
              <Text style={styles.dialogTitle}>{t("chat.renameTitle")}</Text>
              <TextInput
                autoFocus
                onChangeText={setRenameValue}
                onSubmitEditing={() => void submitRename()}
                placeholder={t("sessions.unnamed")}
                placeholderTextColor={colors.inkMuted}
                returnKeyType="done"
                style={styles.nameInput}
                value={renameValue}
              />
              <View style={styles.dialogActions}>
                <View style={styles.dialogAction}>
                  <Button
                    compact
                    label={t("chat.cancel")}
                    onPress={() => setRenameOpen(false)}
                    variant="secondary"
                  />
                </View>
                <View style={styles.dialogAction}>
                  <Button
                    compact
                    disabled={!renameValue.trim()}
                    label={t("chat.save")}
                    onPress={() => void submitRename()}
                  />
                </View>
              </View>
            </View>
          </View>
        </Modal>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.surface },
  keyboard: { flex: 1, backgroundColor: colors.surface },
  topbar: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: spacing.md,
    borderBottomWidth: 1,
    borderBottomColor: colors.lineSoft,
    backgroundColor: colors.surface,
  },
  iconButton: {
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  titleWrap: { flex: 1, alignItems: "center" },
  title: { color: colors.inkStrong, fontSize: 16, fontWeight: "700", maxWidth: "90%" },
  controlDisabled: { opacity: 0.5 },
  chatContent: { flex: 1 },
  timelineList: { flex: 1 },
  timeline: { padding: spacing.lg },
  emptyTimeline: { flexGrow: 1, alignItems: "center", justifyContent: "center" },
  empty: { color: colors.inkMuted, fontSize: 14 },
  itemGap: { height: spacing.md },
  offlineComposer: {
    marginHorizontal: spacing.md,
    marginBottom: spacing.xs,
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
    color: colors.warning,
    backgroundColor: colors.warningSoft,
    fontSize: 12,
    textAlign: "center",
  },
  composerDock: {
    position: "absolute",
    right: 0,
    bottom: 0,
    left: 0,
    backgroundColor: colors.surface,
  },
  composerFade: {
    position: "absolute",
    top: -COMPOSER_FADE_CLEARANCE,
    right: 0,
    left: 0,
    height: COMPOSER_FADE_CLEARANCE + 4,
  },
  backToLatest: {
    position: "absolute",
    top: -48,
    alignSelf: "center",
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderWidth: 1,
    borderColor: colors.lineSoft,
    borderRadius: radius.pill,
    backgroundColor: colors.surface,
    shadowColor: colors.inkStrong,
    shadowOpacity: 0.08,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 3 },
    elevation: 3,
  },
  backToLatestText: { color: colors.inkSoft, fontSize: 13, fontWeight: "600" },
  composerArea: {
    paddingHorizontal: spacing.md,
    paddingTop: spacing.xs,
    paddingBottom: spacing.md,
    backgroundColor: "transparent",
  },
  pendingAttachments: { gap: spacing.sm, paddingHorizontal: spacing.md, paddingTop: spacing.sm },
  pendingAttachment: {
    maxWidth: 230,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.xs,
    borderWidth: 1,
    borderColor: colors.lineSoft,
    borderRadius: radius.md,
    backgroundColor: colors.surfaceSubtle,
  },
  pendingAttachmentCopy: { maxWidth: 155 },
  pendingAttachmentName: { color: colors.ink, fontSize: 12, fontWeight: "600" },
  pendingAttachmentSize: { color: colors.inkMuted, fontSize: 10 },
  attachmentError: {
    paddingHorizontal: spacing.md,
    paddingTop: spacing.xs,
    color: colors.danger,
    fontSize: 11,
  },
  attachmentWarning: {
    paddingHorizontal: spacing.md,
    paddingTop: spacing.xs,
    color: colors.warning,
    fontSize: 11,
  },
  transferTrack: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: 2,
    overflow: "hidden",
  },
  transferFill: { height: 2, borderRadius: radius.pill, backgroundColor: colors.accent },
  attachmentButton: {
    width: 32,
    height: 32,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
    marginRight: "auto",
  },
  dockedApproval: {
    marginHorizontal: spacing.md,
    marginBottom: spacing.sm,
  },
  composer: {
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
    shadowColor: colors.inkStrong,
    shadowOpacity: 0.1,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 6 },
    elevation: 4,
  },
  input: {
    minHeight: 56,
    maxHeight: 160,
    color: colors.ink,
    fontSize: 15,
    lineHeight: 21,
    paddingHorizontal: spacing.lg,
    paddingTop: spacing.md,
    paddingBottom: spacing.md,
    textAlignVertical: "top",
  },
  composerToolbar: {
    minHeight: 46,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    gap: spacing.sm,
    paddingHorizontal: spacing.md,
    paddingBottom: spacing.md,
  },
  composerSelectors: {
    minWidth: 0,
    flexGrow: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
  },
  selectorTrigger: {
    minWidth: 0,
    maxWidth: 154,
    height: 34,
    flexDirection: "row",
    alignItems: "center",
    gap: 3,
    paddingHorizontal: spacing.sm,
    borderRadius: radius.sm,
  },
  selectorTriggerPressed: { backgroundColor: colors.surfaceSubtle },
  selectorText: { flexShrink: 1, color: colors.inkSoft, fontSize: 12, fontWeight: "600" },
  sendButton: {
    width: 38,
    height: 38,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.sm,
    backgroundColor: colors.accent,
  },
  stopButton: { backgroundColor: colors.danger },
  sendDisabled: { backgroundColor: colors.accentDisabled },
  sendPressed: { opacity: 0.78 },
  selectorOverlay: {
    flex: 1,
    justifyContent: "flex-end",
    padding: spacing.md,
    paddingBottom: spacing.xl,
    backgroundColor: colors.overlay,
  },
  attachmentOverlay: {
    flex: 1,
    justifyContent: "flex-end",
    backgroundColor: colors.overlay,
  },
  attachmentMenu: {
    width: "100%",
    overflow: "hidden",
    borderTopLeftRadius: radius.lg,
    borderTopRightRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  attachmentMenuOption: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.lg,
    borderBottomWidth: 1,
    borderBottomColor: colors.lineSoft,
  },
  attachmentMenuText: { color: colors.ink, fontSize: 15, fontWeight: "600" },
  previewSafe: { flex: 1, backgroundColor: colors.surface },
  previewHeader: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.lg,
    borderBottomWidth: 1,
    borderBottomColor: colors.lineSoft,
  },
  previewTitle: { flex: 1, color: colors.inkStrong, fontSize: 16, fontWeight: "700" },
  previewImage: { flex: 1, width: "100%", height: "100%", backgroundColor: colors.surfaceSubtle },
  previewMarkdown: { padding: spacing.lg },
  previewTruncated: {
    marginBottom: spacing.md,
    padding: spacing.md,
    borderRadius: radius.md,
    color: colors.warning,
    backgroundColor: colors.warningSoft,
    fontSize: 12,
  },
  previewText: { color: colors.ink, fontSize: 14, lineHeight: 21 },
  selectorMenu: {
    maxHeight: "60%",
    overflow: "hidden",
    padding: spacing.sm,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  selectorTitle: {
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.sm,
    color: colors.inkMuted,
    fontSize: 12,
    fontWeight: "700",
  },
  selectorOption: {
    minHeight: 50,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
  },
  selectorOptionSelected: { backgroundColor: colors.accentSoft },
  selectorOptionPressed: { opacity: 0.72 },
  selectorOptionCopy: { minWidth: 0, flex: 1 },
  selectorOptionLabel: { color: colors.ink, fontSize: 14, fontWeight: "600" },
  selectorOptionMeta: { marginTop: 2, color: colors.inkMuted, fontSize: 11 },
  overlay: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: spacing.xl,
    backgroundColor: colors.overlay,
  },
  dialog: {
    width: "100%",
    maxWidth: 420,
    padding: spacing.xl,
    gap: spacing.lg,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  dialogTitle: { color: colors.inkStrong, fontSize: 20, fontWeight: "700" },
  nameInput: {
    minHeight: 48,
    paddingHorizontal: spacing.md,
    color: colors.ink,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
  },
  dialogActions: { flexDirection: "row", gap: spacing.md },
  dialogAction: { flex: 1 },
});
