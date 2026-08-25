import {
  ArrowDown,
  ChevronDown,
  CircleAlert,
  FileText,
  Paperclip,
  Send,
  Square,
  X,
} from "lucide-react-native";
import { Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import type { Dispatch, SetStateAction } from "react";
import type { TFunction } from "i18next";
import Svg, { Defs, LinearGradient, Rect, Stop } from "react-native-svg";
import { PendingApprovalCard } from "../../../components/TimelineCard";
import { useRemote } from "../../../remote/RemoteContext";
import { deleteTemporaryAttachment } from "../../../remote/files";
import type { MobileAttachment, TimelineItem } from "../../../remote/types";
import { colors, radius, spacing } from "../../../theme/tokens";
import { COMPOSER_FADE_CLEARANCE, formatBytes } from "../utils";

type Remote = ReturnType<typeof useRemote>;

type PendingApproval = Extract<TimelineItem, { kind: "approval" }>;

export function ComposerDock({
  message,
  setMessage,
  attachments,
  setAttachments,
  supportsImages,
  activeModelLabel,
  remote,
  t,
  openAttachmentMenu,
  send,
  atLatest,
  scrollToLatest,
  showOffline,
  pendingApprovals,
  approvalSubmitting,
  approvalError,
  decideApproval,
  setComposerHeight,
  keyboardLift,
  selector,
  setSelector,
}: {
  message: string;
  setMessage: Dispatch<SetStateAction<string>>;
  attachments: MobileAttachment[];
  setAttachments: Dispatch<SetStateAction<MobileAttachment[]>>;
  supportsImages: boolean;
  activeModelLabel: string;
  remote: Remote;
  t: TFunction;
  openAttachmentMenu: () => void;
  send: () => Promise<void>;
  atLatest: boolean;
  scrollToLatest: () => void;
  showOffline: boolean;
  pendingApprovals: PendingApproval[];
  approvalSubmitting: string | null;
  approvalError: string | null;
  decideApproval: (id: string, decision: "approved" | "rejected") => Promise<void>;
  setComposerHeight: (value: number) => void;
  keyboardLift: number;
  selector: "model" | "thinking" | null;
  setSelector: (value: "model" | "thinking" | null) => void;
}) {
  return (
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
      {showOffline && <Text style={styles.offlineComposer}>{t("connection.offlineHint")}</Text>}
      {!remote.draft && remote.desktopOnline && remote.models.length === 0 && !remote.modelId && (
        <Text style={styles.offlineComposer}>{t("connection.noModelsHint")}</Text>
      )}
      {pendingApprovals.map(item => (
        <View key={item.id} style={styles.dockedApproval}>
          <PendingApprovalCard
            error={approvalSubmitting === item.payload.approval_request_id ? null : approvalError}
            onDecision={decision => void decideApproval(item.payload.approval_request_id, decision)}
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
                <View key={`${attachment.localUri}:${index}`} style={styles.pendingAttachment}>
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
              disabled={remote.timeline.streaming || remote.busy || !remote.fileTransferSupported}
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
  );
}

const styles = StyleSheet.create({
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
  dockedApproval: {
    marginHorizontal: spacing.md,
    marginBottom: spacing.sm,
  },
  composerArea: {
    paddingHorizontal: spacing.md,
    paddingTop: spacing.xs,
    paddingBottom: spacing.md,
    backgroundColor: "transparent",
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
  attachmentButton: {
    width: 32,
    height: 32,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
    marginRight: "auto",
  },
  controlDisabled: { opacity: 0.5 },
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
});
