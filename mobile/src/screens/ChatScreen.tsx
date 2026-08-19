import {
  ArrowDown,
  ArrowLeft,
  Check,
  ChevronDown,
  CircleAlert,
  Download,
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
  ToastAndroid,
  TouchableWithoutFeedback,
  View,
} from "react-native";
import * as Network from "expo-network";
import * as Sharing from "expo-sharing";
import { openFile as openAndroidFile } from "future-file-handler";
import { showActionSheet as showAndroidActionSheet } from "future-native-ui";
import { File } from "expo-file-system";
import * as LegacyFileSystem from "expo-file-system/legacy";
import { SafeAreaView, useSafeAreaInsets } from "react-native-safe-area-context";
import Svg, { Defs, LinearGradient, Rect, Stop } from "react-native-svg";
import { PendingApprovalCard, TimelineCard } from "../components/TimelineCard";
import { Button } from "../components/Button";
import { ErrorBanner } from "../components/ErrorBanner";
import { JsonPreview } from "../components/JsonPreview";
import { MarkdownText } from "../components/MarkdownText";
import { useRemote } from "../remote/RemoteContext";
import { loadSessionDraft, saveSessionDraft } from "../remote/draftStorage";
import {
  deleteTemporaryAttachment,
  MAX_FILE_BYTES,
  mimeFor,
  pickAttachments,
  pickFromAlbum,
  recoverPendingImagePickerAttachments,
  takePhoto,
  TransferCancelledError,
  namedExternalFile,
} from "../remote/files";
import { basename } from "../remote/localPath";
import { mobileFileType, mobilePreviewRoute } from "../remote/fileTypes";
import { supportedExternalMime } from "../remote/fileHandler";
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

// Transient failures (attachment pick, send) surface as a platform-native
// toast instead of pinned red text above the composer. iOS has no native
// toast, so it falls back to a plain Alert like the rest of the app's errors.
function showToast(message: string): void {
  if (Platform.OS === "android") {
    ToastAndroid.show(message, ToastAndroid.SHORT);
  } else {
    Alert.alert(message);
  }
}

// How close to the bottom counts as "at latest" (px). Shared by the atLatest
// detection and the scroll target so the two never disagree.
const AT_LATEST_THRESHOLD = 32;

// The fade band above the composer dock (styles.composerFade) is part of the
// dock's visual footprint: the list's bottom padding must clear it too, or a
// settled reply's footer ("time · tokens" + copy) rests under the
// semi-transparent gradient.
const COMPOSER_FADE_CLEARANCE = 48;
const MARKDOWN_RENDER_BYTES = 2 * 1024 * 1024;

type DownloadPhase =
  | "preparing"
  | "downloading"
  | "waiting_network"
  | "verifying"
  | "saving"
  | "opening"
  | "cancelling";

interface ActiveDownload {
  id: string;
  fileName: string;
  phase: DownloadPhase;
  completedBytes: number;
  totalBytes: number;
}

interface DownloadHandle {
  id: string;
  fileName: string;
  visible: boolean;
  controller: AbortController;
}

interface FileAction {
  info: DownloadInfo;
  cachedFile: File | null;
  openMimeType: string;
}

function deferPresentation(action: () => void): void {
  // UIKit invokes action-sheet callbacks before the dismissal animation has
  // fully released its presentation controller. A short delay avoids racing
  // the next native controller. InteractionManager is deliberately avoided:
  // it can remain pending while a Modal is itself transitioning.
  setTimeout(action, Platform.OS === "ios" ? 350 : 0);
}

function NativeFileActionSheet({
  action,
  cancelLabel,
  openLabel,
  saveLabel,
  onClose,
  onSelect,
}: {
  action: FileAction | null;
  cancelLabel: string;
  openLabel: string;
  saveLabel: string;
  onClose: () => void;
  onSelect: (action: FileAction, save: boolean) => void;
}) {
  const shownActionRef = useRef<FileAction | null>(null);

  useEffect(() => {
    if (!action || shownActionRef.current === action) return;
    shownActionRef.current = action;
    const close = () => {
      shownActionRef.current = null;
      onClose();
    };
    const select = (save: boolean) => {
      close();
      deferPresentation(() => onSelect(action, save));
    };
    if (Platform.OS === "ios") {
      // iOS uses the same system share sheet for both "open" and "save".
      // Present it directly instead of asking the user to choose between two
      // actions that lead to the same native surface.
      select(false);
      return;
    }
    Alert.alert(
      action.info.name,
      undefined,
      [
        { text: openLabel, onPress: () => select(false) },
        { text: cancelLabel, style: "cancel", onPress: close },
        { text: saveLabel, onPress: () => select(true) },
      ],
      { cancelable: true, onDismiss: close },
    );
  }, [action, cancelLabel, onClose, onSelect, openLabel, saveLabel]);

  return null;
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${Math.max(1, Math.ceil(bytes / 1024))} KB`;
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
  const [transferProgress, setTransferProgress] = useState<number | null>(null);
  const [activeDownload, setActiveDownload] = useState<ActiveDownload | null>(null);
  const activeDownloadRef = useRef<DownloadHandle | null>(null);
  const downloadModalPresentedRef = useRef(false);
  // UIKit cannot reliably present a second React Native Modal while the
  // download-progress Modal is still dismissing. Keep the next presentation
  // out of render state until `onDismiss` confirms that first Modal is gone.
  const pendingDownloadModalRef = useRef<(() => void) | null>(null);
  const [preview, setPreview] = useState<{
    attachment: HistoryAttachment;
    info: DownloadInfo;
    uri: string;
    markdown?: string;
    text?: string;
    truncated?: boolean;
  } | null>(null);
  // Prepared non-previewable attachment awaiting an Android open/save choice;
  // iOS immediately continues to its system share sheet.
  const [fileAction, setFileAction] = useState<FileAction | null>(null);
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
  const activeDownloadFraction = activeDownload?.totalBytes
    ? Math.min(1, activeDownload.completedBytes / activeDownload.totalBytes)
    : 0;

  const beginDownload = useCallback(
    (key: string, fileName: string, totalBytes = 0, visible = true): DownloadHandle | null => {
      if (activeDownloadRef.current) return null;
      const handle = {
        id: `${key}:${Date.now().toString(36)}`,
        fileName,
        visible,
        controller: new AbortController(),
      };
      activeDownloadRef.current = handle;
      if (visible) {
        setActiveDownload({
          id: handle.id,
          fileName,
          phase: "preparing",
          completedBytes: 0,
          totalBytes,
        });
      }
      return handle;
    },
    [],
  );

  const showDownload = useCallback(
    (handle: DownloadHandle, patch: Partial<Omit<ActiveDownload, "id" | "fileName">> = {}) => {
      if (activeDownloadRef.current?.id !== handle.id) return;
      handle.visible = true;
      setActiveDownload({
        id: handle.id,
        fileName: handle.fileName,
        phase: "preparing",
        completedBytes: 0,
        totalBytes: 0,
        ...patch,
      });
    },
    [],
  );

  const updateDownload = useCallback(
    (handle: DownloadHandle, patch: Partial<Omit<ActiveDownload, "id" | "fileName">>) => {
      if (activeDownloadRef.current?.id !== handle.id || !handle.visible) return;
      setActiveDownload(current =>
        current?.id === handle.id ? { ...current, ...patch } : current,
      );
    },
    [],
  );

  const finishDownload = useCallback((handle: DownloadHandle) => {
    if (activeDownloadRef.current?.id !== handle.id) return;
    activeDownloadRef.current = null;
    setActiveDownload(null);
  }, []);

  const flushPendingDownloadModal = useCallback(() => {
    downloadModalPresentedRef.current = false;
    const present = pendingDownloadModalRef.current;
    pendingDownloadModalRef.current = null;
    present?.();
  }, []);

  const handoffDownloadModal = useCallback(
    (handle: DownloadHandle, present: () => void) => {
      if (!handle.visible) {
        finishDownload(handle);
        present();
        return;
      }
      pendingDownloadModalRef.current = present;
      finishDownload(handle);
      // `onDismiss` is iOS-only. Android continues after the state commit. On
      // iOS, normally wait for Modal.onDismiss; only use the timer if the
      // progress modal never reached onShow at all.
      if (Platform.OS !== "ios") {
        deferPresentation(flushPendingDownloadModal);
      } else if (!downloadModalPresentedRef.current) {
        setTimeout(() => {
          if (!downloadModalPresentedRef.current && pendingDownloadModalRef.current === present) {
            flushPendingDownloadModal();
          }
        }, 500);
      }
    },
    [finishDownload, flushPendingDownloadModal],
  );

  const handoffDownloadAlert = useCallback(
    (handle: DownloadHandle, message: string) => {
      handoffDownloadModal(handle, () => Alert.alert(t("attachment.title"), message));
    },
    [handoffDownloadModal, t],
  );

  const cancelActiveDownload = useCallback(() => {
    const handle = activeDownloadRef.current;
    if (!handle) return;
    handle.controller.abort();
    // Local cancellation must be immediate even while an underlying NATS
    // request is waiting for its transport timeout. Late callbacks are scoped
    // to this handle and are ignored once it has been released.
    finishDownload(handle);
  }, [finishDownload]);

  // Approvals live docked above the composer (not inline in the transcript), and
  // only while undecided — once a decision lands the card disappears.
  const timelineItems = useMemo(() => remote.timeline.items, [remote.timeline]);
  const transcriptItems = useMemo(
    () => timelineItems.filter(item => item.kind !== "approval"),
    [timelineItems],
  );
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
    !remote.draft && (remote.busy || remote.timelinePending) && timelineItems.length === 0;
  // The first content render snaps to the end without animation; only later
  // appends (streaming, new messages) scroll animated.
  const landedRef = useRef(false);
  // A FlatList emits scroll events while iOS/Android are measuring its first
  // content and viewport. Those are layout side effects, not a user decision
  // to read earlier messages, so they must not disable the initial snap.
  const initialScrollPendingRef = useRef(true);
  const initialScrollFrameRef = useRef<number | null>(null);
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

  const renameConversation = async (rawName: string) => {
    const name = rawName.trim();
    if (!name) return;
    try {
      await remote.rename(remote.selectedSessionId, name);
    } catch {
      Alert.alert(t("common.error"));
    }
  };

  const openRename = () => {
    const currentTitle = remote.selectedTitle || "";
    if (Platform.OS === "ios") {
      Alert.prompt(
        t("chat.renameTitle"),
        undefined,
        [
          { text: t("chat.cancel"), style: "cancel" },
          {
            text: t("chat.save"),
            onPress: (value?: string) => {
              if (value?.trim()) void renameConversation(value);
            },
          },
        ],
        "plain-text",
        currentTitle,
      );
      return;
    }
    // Android has no native React Native text-input alert.
    setRenameValue(currentTitle);
    setRenameOpen(true);
  };

  const submitRename = async () => {
    const name = renameValue.trim();
    if (!name) return;
    setRenameOpen(false);
    await renameConversation(name);
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

  useEffect(
    () => () => {
      activeDownloadRef.current?.controller.abort();
    },
    [],
  );

  // Restore this conversation's draft when the screen (re)opens. Uses the
  // sessionId captured at mount — the draft conversation key stays stable
  // across the "" placeholder, and a bound session keeps its own slot.
  useEffect(() => {
    restoringDraftRef.current = true;
    activeDraftKeyRef.current = draftKey;
    const key = draftKey;
    void (async () => {
      const draft = await loadSessionDraft(key);
      let restoredAttachments = draft?.attachments ?? [];
      if (Platform.OS === "android") {
        try {
          restoredAttachments = await recoverPendingImagePickerAttachments(restoredAttachments);
        } catch (error) {
          const errorKey = error instanceof Error ? error.message : "attachment_failed";
          showToast(t(`attachment.errors.${errorKey}`));
        }
      }
      // Guard against a conversation switch racing the async load or Android
      // pending-result recovery after MainActivity reconstruction.
      if (restoringDraftRef.current && activeDraftKeyRef.current === key) {
        setMessage(draft?.text ?? "");
        setAttachments(restoredAttachments);
        restoringDraftRef.current = false;
      }
    })();
  }, [draftKey, t]);

  // Persist edits. The restore-driven update is skipped (the effect above
  // already loaded the draft), and temporary camera/cache files are released
  // when removed so they don't accumulate on disk.
  useEffect(() => {
    if (restoringDraftRef.current) return;
    void saveSessionDraft(draftKey, { text: message, attachments });
  }, [attachments, draftKey, message]);

  const chooseFiles = async () => {
    try {
      setAttachments(await pickAttachments(attachments));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      showToast(t(`attachment.errors.${key}`));
    }
  };

  const capturePhoto = async () => {
    try {
      setAttachments(await takePhoto(attachments));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      showToast(t(`attachment.errors.${key}`));
    }
  };

  const chooseFromAlbum = async () => {
    try {
      setAttachments(await pickFromAlbum(attachments));
    } catch (error) {
      const key = error instanceof Error ? error.message : "attachment_failed";
      showToast(t(`attachment.errors.${key}`));
    }
  };

  const openAttachmentMenu = () => {
    if (Platform.OS === "ios") {
      ActionSheetIOS.showActionSheetWithOptions(
        {
          options: [
            t("attachment.takePhoto"),
            t("attachment.chooseFromAlbum"),
            t("attachment.chooseFiles"),
            t("chat.cancel"),
          ],
          cancelButtonIndex: 3,
        },
        index => {
          // Run after native presentation work has settled so the picker never
          // competes with the action sheet for the current view controller.
          if (index === 0) deferPresentation(() => void capturePhoto());
          if (index === 1) deferPresentation(() => void chooseFromAlbum());
          if (index === 2) deferPresentation(() => void chooseFiles());
        },
      );
      return;
    }
    void showAndroidActionSheet(
      [
        t("attachment.takePhoto"),
        t("attachment.chooseFromAlbum"),
        t("attachment.chooseFiles"),
        t("chat.cancel"),
      ],
      t("attachment.title"),
    )
      .then(index => {
        if (index === 0) deferPresentation(() => void capturePhoto());
        if (index === 1) deferPresentation(() => void chooseFromAlbum());
        if (index === 2) deferPresentation(() => void chooseFiles());
      })
      .catch(() => showToast(t("attachment.errors.attachment_failed")));
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
      showToast(key === "prompt_too_large" ? t("chat.promptTooLarge") : t("chat.sendFailed"));
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
          void (async () => {
            const retryAttachments: MobileAttachment[] = [];
            for (const attachment of prev.attachments ?? []) {
              if (/^[a-z][a-z0-9+.-]*:\/\//i.test(attachment.path)) {
                const file = new File(attachment.path);
                retryAttachments.push({
                  localUri: file.uri,
                  name: attachment.name,
                  mimeType: mimeFor(attachment.name),
                  kind: attachment.kind === "image" ? "image" : "file",
                  originalSize: file.size,
                  transferSize: file.size,
                  mobilePreviewUnsupported: attachment.mobilePreviewUnsupported,
                });
                continue;
              }
              const info = await remote.prepareAttachment(attachment);
              const cached = remote.cachedAttachment(attachment);
              const file = cached?.file ?? (await remote.downloadAttachment(info));
              retryAttachments.push({
                localUri: file.uri,
                name: attachment.name,
                transferName: info.name,
                mimeType: info.mimeType,
                kind: attachment.kind === "image" ? "image" : "file",
                originalSize: info.size,
                transferSize: info.size,
                mobilePreviewUnsupported: attachment.mobilePreviewUnsupported,
              });
            }
            await remote.sendMessage(prev.text, retryAttachments);
          })().catch(() => showToast(t("chat.sendFailed")));
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
        .catch(() => showToast(t("chat.sendFailed")));
    },
    [remote, t],
  );

  const openAttachment = useCallback(
    async (attachment: HistoryAttachment) => {
      let handle: DownloadHandle | null = null;
      try {
        const fileType = mobileFileType(attachment.name);
        if (!fileType) {
          Alert.alert(t("attachment.title"), t("attachment.unsupportedType"));
          return;
        }
        const openMimeType =
          fileType.route === "external" ? await supportedExternalMime(attachment.name) : null;
        if (fileType.route === "external" && !openMimeType) {
          Alert.alert(t("attachment.title"), t("attachment.noHandler"));
          return;
        }
        // The just-sent optimistic bubble still points at this phone's local
        // picker URI. Open it directly; durable history later replaces this
        // with the desktop path used by the NATS download flow.
        if (/^[a-z][a-z0-9+.-]*:\/\//i.test(attachment.path)) {
          const local = new File(attachment.path);
          if (fileType.route === "image" && !attachment.mobilePreviewUnsupported) {
            setPreview({
              info: {
                transferId: "local",
                name: attachment.name,
                mimeType: "image/*",
                size: local.size,
                contentHash: "",
                previewKind: "image",
                variant: "preview",
                chunkBytes: 0,
              },
              attachment,
              uri: local.uri,
            });
          } else if (fileType.route === "external") {
            setFileAction({
              info: {
                transferId: "local",
                name: attachment.name,
                mimeType: fileType.mimeType,
                size: local.size,
                contentHash: "",
                previewKind: "file",
                variant: "original",
                chunkBytes: 0,
              },
              cachedFile: local,
              openMimeType: openMimeType!,
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
            const markdown = fileType.route === "markdown";
            const richJson = mobilePreviewRoute(attachment.name, local.size) === "json";
            setPreview({
              info: {
                transferId: "local",
                name: attachment.name,
                mimeType: fileType.mimeType,
                size: local.size,
                contentHash: "",
                previewKind: richJson ? "json" : markdown ? "markdown" : "text",
                variant: "preview",
                chunkBytes: 0,
              },
              attachment,
              uri: local.uri,
              ...(markdown ? { markdown: previewText } : { text: previewText }),
              truncated: bytes.byteLength > visible.byteLength,
            });
          }
          return;
        }
        handle = beginDownload(attachment.path, attachment.name, 0, false);
        if (!handle) {
          showToast(t("attachment.downloadInProgress"));
          return;
        }
        const variant = fileType.route === "external" ? "original" : "preview";
        const cachedPreview = remote.cachedAttachment(attachment, variant);
        const info =
          cachedPreview?.info ??
          (await remote.prepareAttachment(
            attachment,
            variant,
            handle.controller.signal,
            () => handle && updateDownload(handle, { phase: "waiting_network" }),
          ));
        if (info.size > MAX_FILE_BYTES) {
          handoffDownloadAlert(handle, t("attachment.tooLarge"));
          return;
        }
        if (info.previewKind === "file") {
          if (!openMimeType) {
            handoffDownloadAlert(handle, t("attachment.noHandler"));
            return;
          }
          handoffDownloadModal(handle, () =>
            setFileAction({ info, cachedFile: cachedPreview?.file ?? null, openMimeType }),
          );
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
          // Metadata is intentionally silent. Once we know this is a cache
          // miss and have the real byte size, show 0 / total before the first
          // (possibly only) chunk arrives.
          showDownload(handle, {
            phase: "downloading",
            completedBytes: 0,
            totalBytes: info.size,
          });
        }
        file = await remote.downloadAttachment(
          info,
          (done, total) => {
            if (!handle) return;
            const patch = {
              phase: done >= total ? "verifying" : "downloading",
              completedBytes: done,
              totalBytes: total,
            } as const;
            if (handle.visible) updateDownload(handle, patch);
            else showDownload(handle, patch);
          },
          handle.controller.signal,
          () => {
            if (!handle) return;
            const patch = { phase: "waiting_network" } as const;
            if (handle.visible) updateDownload(handle, patch);
            else showDownload(handle, patch);
          },
        );
        if (info.previewKind === "image") {
          handoffDownloadModal(handle, () => setPreview({ attachment, info, uri: file.uri }));
        } else {
          const bytes = await file.bytes();
          const visible = bytes.slice(0, MARKDOWN_RENDER_BYTES);
          const previewText = new TextDecoder().decode(visible);
          handoffDownloadModal(handle, () =>
            setPreview({
              attachment,
              info,
              uri: file.uri,
              ...(info.previewKind === "markdown"
                ? { markdown: previewText }
                : { text: previewText }),
              truncated: bytes.byteLength > visible.byteLength,
            }),
          );
        }
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        const detail = error instanceof Error ? error.message : "";
        const message =
          detail.includes("view it on desktop") || detail.includes("GIF preview")
            ? t("attachment.previewOnDesktop")
            : t("attachment.downloadFailed");
        if (handle) {
          handoffDownloadAlert(handle, message);
        } else {
          Alert.alert(t("attachment.title"), message);
        }
      } finally {
        if (handle) finishDownload(handle);
      }
    },
    [
      beginDownload,
      finishDownload,
      handoffDownloadAlert,
      handoffDownloadModal,
      remote,
      showDownload,
      t,
      updateDownload,
    ],
  );

  // Download `info` to a cached File, prompting on cellular. Returns the file,
  // or null when the user declines the cellular download.
  const fetchDownload = useCallback(
    async (
      info: DownloadInfo,
      cachedFile: File | null,
      handle?: DownloadHandle,
    ): Promise<File | null> => {
      if (info.transferId === "local" && cachedFile) return cachedFile;
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
        if (!accepted) return null;
      }
      if (handle && !cachedFile) {
        const patch = {
          phase: "downloading",
          completedBytes: 0,
          totalBytes: info.size,
        } as const;
        if (handle.visible) updateDownload(handle, patch);
        else showDownload(handle, patch);
      }
      return remote.downloadAttachment(
        info,
        (done, total) => {
          if (handle) {
            const patch = {
              phase: done >= total ? "verifying" : "downloading",
              completedBytes: done,
              totalBytes: total,
            } as const;
            if (handle.visible) updateDownload(handle, patch);
            else showDownload(handle, patch);
          } else {
            setTransferProgress(total > 0 ? done / total : null);
          }
        },
        handle?.controller.signal,
        () => {
          if (handle) {
            const patch = { phase: "waiting_network" } as const;
            if (handle.visible) updateDownload(handle, patch);
            else showDownload(handle, patch);
          }
        },
      );
    },
    [remote, showDownload, t, updateDownload],
  );

  // Non-previewable file: download then hand it to the OS share sheet, which is
  // the cross-platform "open with external app / save to files" surface.
  const openOrShare = useCallback(
    async (
      info: DownloadInfo,
      cachedFile: File | null,
      save: boolean,
      existingHandle?: DownloadHandle,
      openMimeType = info.mimeType,
    ) => {
      const handle =
        existingHandle ?? beginDownload(info.transferId || info.name, info.name, info.size, false);
      if (!handle) {
        showToast(t("attachment.downloadInProgress"));
        return;
      }
      try {
        const file = await fetchDownload(info, cachedFile, handle);
        if (!file) return;
        if (save && Platform.OS === "android") {
          updateDownload(handle, { phase: "saving" });
          const permission =
            await LegacyFileSystem.StorageAccessFramework.requestDirectoryPermissionsAsync();
          if (!permission.granted) return;
          if (handle.controller.signal.aborted) throw new TransferCancelledError();
          const destination = await LegacyFileSystem.StorageAccessFramework.createFileAsync(
            permission.directoryUri,
            info.name,
            info.mimeType,
          );
          const base64 = await LegacyFileSystem.readAsStringAsync(file.uri, {
            encoding: LegacyFileSystem.EncodingType.Base64,
          });
          if (handle.controller.signal.aborted) throw new TransferCancelledError();
          await LegacyFileSystem.StorageAccessFramework.writeAsStringAsync(destination, base64, {
            encoding: LegacyFileSystem.EncodingType.Base64,
          });
          showToast(t("attachment.downloaded", { name: info.name }));
          return;
        }
        if (!save && Platform.OS === "android") {
          updateDownload(handle, { phase: "opening" });
          const namedFile = await namedExternalFile(file, info.name);
          await openAndroidFile(namedFile.uri, openMimeType);
          return;
        }
        if (!(await Sharing.isAvailableAsync())) {
          handoffDownloadAlert(handle, t("attachment.shareUnavailable"));
          return;
        }
        updateDownload(handle, { phase: save ? "saving" : "opening" });
        const namedFile = await namedExternalFile(file, info.name);
        if (Platform.OS === "ios") {
          // UIActivityViewController cannot be presented reliably while the
          // React Native download Modal is still on screen. Dismiss it first
          // and wait for Modal.onDismiss before handing the file to UIKit.
          // Once handed off, the system share sheet owns cancellation.
          handoffDownloadModal(handle, () => {
            void Sharing.shareAsync(namedFile.uri, {
              mimeType: save ? info.mimeType : openMimeType,
              dialogTitle: save ? t("attachment.save") : t("attachment.open"),
            }).catch(() => Alert.alert(t("attachment.title"), t("attachment.downloadFailed")));
          });
          return;
        }
        await Sharing.shareAsync(namedFile.uri, {
          mimeType: save ? info.mimeType : openMimeType,
          dialogTitle: save ? t("attachment.save") : t("attachment.open"),
        });
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        handoffDownloadAlert(handle, t("attachment.downloadFailed"));
      } finally {
        finishDownload(handle);
      }
    },
    [
      beginDownload,
      fetchDownload,
      finishDownload,
      handoffDownloadAlert,
      handoffDownloadModal,
      t,
      updateDownload,
    ],
  );

  const downloadOriginal = useCallback(
    async (attachment: HistoryAttachment) => {
      const handle = beginDownload(attachment.path, attachment.name, 0, false);
      if (!handle) {
        showToast(t("attachment.downloadInProgress"));
        return;
      }
      if (/^[a-z][a-z0-9+.-]*:\/\//i.test(attachment.path)) {
        const local = new File(attachment.path);
        await openOrShare(
          {
            transferId: "local",
            name: attachment.name,
            mimeType: mimeFor(attachment.name),
            size: local.size,
            contentHash: "",
            previewKind: "file",
            variant: "original",
            chunkBytes: 0,
          },
          local,
          true,
          handle,
        );
        return;
      }
      try {
        const cached = remote.cachedAttachment(attachment, "original");
        const info =
          cached?.info ??
          (await remote.prepareAttachment(attachment, "original", handle.controller.signal, () =>
            updateDownload(handle, { phase: "waiting_network" }),
          ));
        await openOrShare(info, cached?.file ?? null, true, handle);
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        handoffDownloadAlert(handle, t("attachment.downloadFailed"));
      }
    },
    [beginDownload, handoffDownloadAlert, openOrShare, remote, t, updateDownload],
  );

  // A local-file markdown link/image target: prepare, then dispatch by size and
  // preview kind. Over 10 MB → desktop; image/markdown/text/JSON → in-app preview;
  // anything else → open/save action sheet.
  const openFileLink = useCallback(
    async (path: string) => {
      const attachment: HistoryAttachment = { path, name: basename(path) };
      const fileType = mobileFileType(attachment.name);
      if (!fileType) {
        Alert.alert(t("attachment.title"), t("attachment.unsupportedType"));
        return;
      }
      const openMimeType =
        fileType.route === "external" ? await supportedExternalMime(attachment.name) : null;
      if (fileType.route === "external" && !openMimeType) {
        Alert.alert(t("attachment.title"), t("attachment.noHandler"));
        return;
      }
      const handle = beginDownload(path, attachment.name, 0, false);
      if (!handle) {
        showToast(t("attachment.downloadInProgress"));
        return;
      }
      try {
        const variant = fileType.route === "external" ? "original" : "preview";
        const cachedPreview = remote.cachedAttachment(attachment, variant);
        const info =
          cachedPreview?.info ??
          (await remote.prepareAttachment(attachment, variant, handle.controller.signal, () =>
            updateDownload(handle, { phase: "waiting_network" }),
          ));
        if (info.size > MAX_FILE_BYTES) {
          handoffDownloadAlert(handle, t("attachment.tooLarge"));
          return;
        }
        const previewable =
          info.previewKind === "image" ||
          info.previewKind === "markdown" ||
          info.previewKind === "text" ||
          info.previewKind === "json";
        if (!previewable) {
          if (!openMimeType) {
            handoffDownloadAlert(handle, t("attachment.noHandler"));
            return;
          }
          handoffDownloadModal(handle, () =>
            setFileAction({ info, cachedFile: cachedPreview?.file ?? null, openMimeType }),
          );
          return;
        }
        const file = await fetchDownload(info, cachedPreview?.file ?? null, handle);
        if (!file) return;
        if (info.previewKind === "image") {
          handoffDownloadModal(handle, () => setPreview({ attachment, info, uri: file.uri }));
        } else {
          const bytes = await file.bytes();
          const visible = bytes.slice(0, MARKDOWN_RENDER_BYTES);
          const previewText = new TextDecoder().decode(visible);
          handoffDownloadModal(handle, () =>
            setPreview({
              attachment,
              info,
              uri: file.uri,
              ...(info.previewKind === "markdown"
                ? { markdown: previewText }
                : { text: previewText }),
              truncated: bytes.byteLength > visible.byteLength,
            }),
          );
        }
      } catch (error) {
        if (error instanceof TransferCancelledError) return;
        const detail = error instanceof Error ? error.message : "";
        const message =
          detail.includes("view it on desktop") || detail.includes("GIF preview")
            ? t("attachment.previewOnDesktop")
            : t("attachment.downloadFailed");
        handoffDownloadAlert(handle, message);
      } finally {
        finishDownload(handle);
      }
    },
    [
      beginDownload,
      fetchDownload,
      finishDownload,
      handoffDownloadAlert,
      handoffDownloadModal,
      remote,
      t,
      updateDownload,
    ],
  );

  // The bottom-most scroll offset: full content height minus the viewport,
  // never negative. Recomputed from measured sizes so it stays correct as the
  // composer (bottom padding) and content grow.
  const maxScrollOffset = () => Math.max(0, contentHeightRef.current - layoutHeightRef.current);

  // Opening a conversation may lay out the list, its remote history and the
  // floating composer in separate commits. Wait one frame, then repeat on the
  // next frame with the final measurements: this avoids both Android's stale
  // first content size and iOS applying its safe-area/composer inset late.
  const scheduleInitialScroll = () => {
    if (
      !initialScrollPendingRef.current ||
      contentHeightRef.current <= 0 ||
      layoutHeightRef.current <= 0 ||
      composerHeight <= 0
    ) {
      return;
    }
    if (initialScrollFrameRef.current != null) return;
    initialScrollFrameRef.current = requestAnimationFrame(() => {
      initialScrollFrameRef.current = null;
      if (!initialScrollPendingRef.current) return;
      listRef.current?.scrollToOffset({ animated: false, offset: maxScrollOffset() });
      initialScrollFrameRef.current = requestAnimationFrame(() => {
        initialScrollFrameRef.current = null;
        if (!initialScrollPendingRef.current) return;
        listRef.current?.scrollToOffset({ animated: false, offset: maxScrollOffset() });
        initialScrollPendingRef.current = false;
        landedRef.current = true;
        setAtLatest(true);
      });
    });
  };

  // Reset the opening contract if the mounted screen switches between the
  // draft and an established session. In the usual navigation path ChatScreen
  // unmounts, but this also covers a send binding a draft to its new session.
  useEffect(() => {
    landedRef.current = false;
    initialScrollPendingRef.current = true;
    contentHeightRef.current = 0;
    layoutHeightRef.current = 0;
    return () => {
      if (initialScrollFrameRef.current != null) {
        cancelAnimationFrame(initialScrollFrameRef.current);
        initialScrollFrameRef.current = null;
      }
    };
  }, [remote.selectedSessionId]);

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
                <View style={styles.loadingState}>
                  <ActivityIndicator color={colors.accent} />
                  <Text style={styles.empty}>{t("chat.loadingHistory")}</Text>
                </View>
              ) : (
                <Text style={styles.empty}>{t("chat.noHistory")}</Text>
              )
            }
            onContentSizeChange={(_w, h) => {
              contentHeightRef.current = h;
              if (initialScrollPendingRef.current) {
                scheduleInitialScroll();
                return;
              }
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
              scheduleInitialScroll();
            }}
            onScroll={event => {
              // Ignore first-layout offsets; the two-frame snap above owns the
              // initial position on both platforms.
              if (initialScrollPendingRef.current) return;
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
                isLatestAssistant={item.id === latestAssistantId}
                onOpenAttachment={attachment => void openAttachment(attachment)}
                onOpenFile={path => void openFileLink(path)}
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

        <NativeFileActionSheet
          action={fileAction}
          cancelLabel={t("chat.cancel")}
          onClose={() => setFileAction(null)}
          onSelect={(action, save) => {
            void openOrShare(action.info, action.cachedFile, save, undefined, action.openMimeType);
          }}
          openLabel={t("attachment.open")}
          saveLabel={t("attachment.save")}
        />

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
              <Pressable
                accessibilityLabel={t("attachment.save")}
                disabled={activeDownload !== null}
                onPress={() => {
                  if (preview) void downloadOriginal(preview.attachment);
                }}
              >
                {activeDownload !== null ? (
                  <ActivityIndicator color={colors.ink} size="small" />
                ) : (
                  <Download color={colors.ink} size={21} />
                )}
              </Pressable>
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
                <MarkdownText mode="file-preview" text={preview?.markdown ?? ""} />
              </ScrollView>
            ) : preview?.info.previewKind === "json" ? (
              <JsonPreview
                invalidMessage={detail => t("attachment.jsonInvalid", { detail })}
                sourceTruncated={!!preview.truncated}
                text={preview.text ?? ""}
                tooComplexMessage={t("attachment.jsonTooComplex")}
                truncatedMessage={t("attachment.jsonTruncated")}
              />
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
          onDismiss={flushPendingDownloadModal}
          onRequestClose={cancelActiveDownload}
          onShow={() => {
            downloadModalPresentedRef.current = true;
          }}
          transparent
          visible={activeDownload !== null}
        >
          <View style={styles.downloadOverlay}>
            <View style={styles.downloadDialog}>
              <Text style={styles.downloadTitle}>{t("attachment.downloadProgressTitle")}</Text>
              <Text numberOfLines={1} style={styles.downloadFileName}>
                {activeDownload?.fileName}
              </Text>
              <Text style={styles.downloadPhase}>
                {activeDownload ? t(`attachment.downloadPhases.${activeDownload.phase}`) : ""}
              </Text>
              <View style={styles.downloadTrack}>
                <View
                  style={[
                    styles.downloadFill,
                    { width: `${Math.max(2, activeDownloadFraction * 100)}%` },
                  ]}
                />
              </View>
              <View style={styles.downloadMeta}>
                <Text style={styles.downloadBytes}>
                  {activeDownload?.totalBytes
                    ? `${activeDownload.completedBytes === 0 ? "0 KB" : formatBytes(activeDownload.completedBytes)} / ${formatBytes(activeDownload.totalBytes)}`
                    : t("attachment.calculatingSize")}
                </Text>
                <Text style={styles.downloadPercent}>
                  {activeDownload?.totalBytes ? `${Math.round(activeDownloadFraction * 100)}%` : ""}
                </Text>
              </View>
              <Pressable
                disabled={activeDownload?.phase === "cancelling"}
                onPress={cancelActiveDownload}
                style={({ pressed }) => [
                  styles.downloadCancel,
                  pressed && styles.downloadCancelPressed,
                ]}
              >
                <Text style={styles.downloadCancelText}>{t("chat.cancel")}</Text>
              </Pressable>
            </View>
          </View>
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
          visible={Platform.OS !== "ios" && renameOpen}
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
  loadingState: { alignItems: "center", gap: spacing.sm, paddingVertical: spacing.xl },
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
  downloadOverlay: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: spacing.xl,
    backgroundColor: colors.overlay,
  },
  downloadDialog: {
    width: "100%",
    maxWidth: 360,
    gap: spacing.md,
    padding: spacing.xl,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  downloadTitle: { color: colors.inkStrong, fontSize: 17, fontWeight: "700" },
  downloadFileName: { color: colors.ink, fontSize: 14, fontWeight: "600" },
  downloadPhase: { color: colors.inkMuted, fontSize: 13 },
  downloadTrack: {
    height: 7,
    overflow: "hidden",
    borderRadius: radius.pill,
    backgroundColor: colors.surfaceSubtle,
  },
  downloadFill: { height: 7, borderRadius: radius.pill, backgroundColor: colors.accent },
  downloadMeta: { flexDirection: "row", justifyContent: "space-between" },
  downloadBytes: { color: colors.inkMuted, fontSize: 12 },
  downloadPercent: { color: colors.inkSoft, fontSize: 12, fontWeight: "600" },
  downloadCancel: {
    alignSelf: "flex-end",
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
  },
  downloadCancelPressed: { backgroundColor: colors.surfaceSubtle },
  downloadCancelText: { color: colors.accent, fontSize: 14, fontWeight: "600" },
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
