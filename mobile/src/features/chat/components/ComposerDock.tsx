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
import { Pressable, ScrollView, StyleSheet, Text, TextInput, useWindowDimensions, View } from "react-native";
import { memo, useState, type Dispatch, type SetStateAction } from "react";
import type { TFunction } from "i18next";
import Svg, { Defs, LinearGradient, Rect, Stop } from "react-native-svg";
import { PendingApprovalCard } from "../../../components/TimelineCard";
import type { RemoteControls } from "../../../remote/RemoteContext";
import { deleteTemporaryAttachment } from "../../../remote/files";
import type { MobileAttachment, TimelineItem } from "../../../remote/types";
import { chatTypography, colors, layout, radius, spacing } from "../../../theme/tokens";
import { COMPOSER_FADE_CLEARANCE, formatBytes } from "../utils";

type Remote = RemoteControls;

type PendingApproval = Extract<TimelineItem, { kind: "approval" }>;

const INPUT_MIN_HEIGHT = 46;
const INPUT_MAX_HEIGHT = 240;

function ComposerDockView({
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
  approvalError: { id: string; message: string } | null;
  decideApproval: (id: string, decision: "approved" | "rejected") => Promise<void>;
  selector: "model" | "thinking" | null;
  setSelector: (value: "model" | "thinking" | null) => void;
}) {
  const [contentHeight, setContentHeight] = useState(INPUT_MIN_HEIGHT);
  const { width, height, fontScale } = useWindowDimensions();
  const stackedToolbar = width < 380 || fontScale > 1.2;
  const maxInputHeight = Math.max(INPUT_MIN_HEIGHT, Math.min(INPUT_MAX_HEIGHT, Math.floor(height * 0.3)));
  const inputHeight = message ? Math.max(INPUT_MIN_HEIGHT, Math.min(maxInputHeight, contentHeight)) : INPUT_MIN_HEIGHT;
  return (
    <View style={styles.composerDock}>
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
      {(showOffline || remote.connectionPresentation.customerState === "devicePreparing") && (
        <Text style={styles.offlineComposer}>
          {t(remote.connectionPresentation.hintKey ?? "connection.offlineHint")}
        </Text>
      )}
      {!remote.draft && remote.desktopOnline && remote.models.length === 0 && !remote.modelId && (
        <Text style={styles.offlineComposer}>{t("connection.noModelsHint")}</Text>
      )}
      {pendingApprovals.map(item => (
        <View key={item.id} style={styles.dockedApproval}>
          <PendingApprovalCard
            error={approvalSubmitting !== item.payload.approval_request_id && approvalError?.id === item.payload.approval_request_id ? approvalError.message : null}
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
                    accessibilityRole="button"
                    style={styles.removeAttachment}
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
          <View>
            {/* Measure unconstrained text at the input's actual width. A fixed
                native TextInput can report a clamped content size, especially
                after pasting/restoring a draft. This also remeasures wrapping
                after rotation and font-size changes without remounting input. */}
            <View
              pointerEvents="none"
              accessibilityElementsHidden
              importantForAccessibility="no-hide-descendants"
              style={styles.inputMeasure}
            >
              <Text
                accessible={false}
                onLayout={event => setContentHeight(Math.ceil(event.nativeEvent.layout.height))}
                style={styles.inputText}
              >
                {`${message}\u200b`}
              </Text>
            </View>
            <TextInput
              accessibilityLabel={t("chat.placeholder")}
              autoCapitalize="none"
              autoCorrect={false}
              editable={remote.desktopOnline && !remote.streaming && !remote.busy}
              multiline
              scrollEnabled={!!message && contentHeight > maxInputHeight}
              onChangeText={setMessage}
              onSubmitEditing={() => void send()}
              placeholder={t("chat.placeholder")}
              placeholderTextColor={colors.inkMuted}
              spellCheck={false}
              style={[styles.inputText, styles.input, { height: inputHeight }]}
              value={message}
            />
          </View>
          <View style={[styles.composerToolbar, stackedToolbar && styles.composerToolbarStacked]}>
            <View style={[styles.composerSelectors, stackedToolbar && styles.composerSelectorsStacked]}>
              <Pressable
                accessibilityLabel={`${t("chat.model")}: ${activeModelLabel}`}
                accessibilityRole="button"
                accessibilityState={{ expanded: selector === "model", disabled: remote.streaming }}
                disabled={remote.streaming}
                onPress={() => setSelector("model")}
                style={({ pressed }) => [
                  styles.selectorTrigger,
                  stackedToolbar && styles.selectorTriggerStacked,
                  pressed && styles.selectorTriggerPressed,
                  remote.streaming && styles.controlDisabled,
                ]}
              >
                <Text numberOfLines={1} style={styles.selectorText}>
                  {activeModelLabel}
                </Text>
                <ChevronDown color={colors.inkMuted} size={14} />
              </Pressable>
              <Pressable
                accessibilityLabel={`${t("chat.thinkingLevel")}: ${t(`thinking.${remote.thinkingLevel}`)}`}
                accessibilityRole="button"
                accessibilityState={{ expanded: selector === "thinking", disabled: remote.streaming }}
                disabled={remote.streaming}
                onPress={() => setSelector("thinking")}
                style={({ pressed }) => [
                  styles.selectorTrigger,
                  stackedToolbar && styles.selectorTriggerStacked,
                  pressed && styles.selectorTriggerPressed,
                  remote.streaming && styles.controlDisabled,
                ]}
              >
                <Text numberOfLines={1} style={styles.selectorText}>
                  {t(`thinking.${remote.thinkingLevel}`)}
                </Text>
                <ChevronDown color={colors.inkMuted} size={14} />
              </Pressable>
            </View>
            <Pressable
              accessibilityLabel={t("attachment.add")}
              accessibilityRole="button"
              disabled={remote.streaming || remote.busy || !remote.fileTransferSupported}
              onPress={openAttachmentMenu}
              style={({ pressed }) => [
                styles.attachmentButton,
                pressed && styles.selectorTriggerPressed,
                (remote.streaming || remote.busy || !remote.fileTransferSupported) &&
                  styles.controlDisabled,
              ]}
            >
              <Paperclip color={colors.inkSoft} size={17} />
            </Pressable>
            {remote.streaming ? (
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

// Filtering the transcript yields a fresh approvals array on each text tick.
// Compare its item identities, but never ignore changed callbacks or controls.
export const ComposerDock = memo(ComposerDockView, (previous, next) => {
  const { pendingApprovals: beforeApprovals, ...before } = previous;
  const { pendingApprovals: afterApprovals, ...after } = next;
  return beforeApprovals.length === afterApprovals.length
    && beforeApprovals.every((item, index) => item === afterApprovals[index])
    && Object.keys(before).length === Object.keys(after).length
    && (Object.keys(before) as (keyof typeof before)[]).every(key => Object.is(before[key], after[key]));
});

const styles = StyleSheet.create({
  composerDock: {
    flexShrink: 0,
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
    top: -56,
    minHeight: layout.touchTarget,
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
    paddingHorizontal: layout.gutter,
    paddingTop: spacing.xs,
    paddingBottom: spacing.sm,
    backgroundColor: "transparent",
  },
  composer: {
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
    shadowColor: colors.inkStrong,
    shadowOpacity: 0.05,
    shadowRadius: 12,
    shadowOffset: { width: 0, height: 3 },
    elevation: 2,
  },
  pendingAttachments: { gap: spacing.sm, paddingHorizontal: spacing.md, paddingTop: spacing.sm },
  pendingAttachment: {
    maxWidth: 260,
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
  pendingAttachmentCopy: { maxWidth: 155, flexShrink: 1 },
  removeAttachment: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center" },
  pendingAttachmentName: { color: colors.ink, fontSize: 12, fontWeight: "600" },
  pendingAttachmentSize: { color: colors.inkMuted, fontSize: 10 },
  attachmentWarning: {
    paddingHorizontal: spacing.md,
    paddingTop: spacing.xs,
    color: colors.warning,
    fontSize: 11,
  },
  inputMeasure: { position: "absolute", top: 0, left: 0, right: 0, opacity: 0 },
  inputText: {
    color: colors.ink,
    ...chatTypography,
    includeFontPadding: false,
    paddingHorizontal: spacing.lg,
    paddingTop: spacing.sm,
    paddingBottom: spacing.sm,
  },
  input: {
    minHeight: INPUT_MIN_HEIGHT,
    textAlignVertical: "top",
  },
  composerToolbar: {
    minHeight: 46,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: spacing.xs,
    paddingHorizontal: spacing.sm,
    paddingBottom: spacing.xs,
  },
  composerSelectors: {
    minWidth: 0,
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
  },
  composerToolbarStacked: { flexWrap: "wrap" },
  composerSelectorsStacked: {
    flexBasis: "100%",
    flexGrow: 0,
    justifyContent: "space-between",
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.lineSoft,
    paddingBottom: spacing.xs,
  },
  selectorTriggerStacked: { flex: 1, maxWidth: "100%" },
  selectorTrigger: {
    minWidth: 0,
    maxWidth: 154,
    flexShrink: 1,
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: 3,
    paddingHorizontal: spacing.sm,
    borderRadius: radius.sm,
  },
  selectorTriggerPressed: { backgroundColor: colors.surfaceSubtle },
  selectorText: { flexShrink: 1, color: colors.inkSoft, fontSize: 12, fontWeight: "600" },
  attachmentButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
    flexShrink: 0,
  },
  controlDisabled: { opacity: 0.5 },
  sendButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
    backgroundColor: colors.accent,
  },
  stopButton: { backgroundColor: colors.danger },
  sendDisabled: { backgroundColor: colors.accentDisabled },
  sendPressed: { opacity: 0.78 },
});
