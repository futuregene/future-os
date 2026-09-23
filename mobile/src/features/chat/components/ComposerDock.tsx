import {
  ArrowDown,
  ChevronDown,
  CircleAlert,
  FileText,
  Paperclip,
  Send,
  Slash,
  Square,
  X,
} from "lucide-react-native";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  useWindowDimensions,
  View,
} from "react-native";
import {
  memo,
  useCallback,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type SetStateAction,
} from "react";
import type { TFunction } from "i18next";
import Svg, { Defs, LinearGradient, Rect, Stop } from "react-native-svg";
import { PendingApprovalCard } from "../../../components/TimelineCard";
import type { RemoteControls } from "../../../remote/RemoteContext";
import { deleteTemporaryAttachment } from "../../../remote/files";
import type { MobileAttachment, RemoteSkill, TimelineItem } from "../../../remote/types";
import {
  chatTypography,
  colors,
  layout,
  radius,
  spacing,
} from "../../../theme/tokens";
import { COMPOSER_FADE_CLEARANCE, formatBytes } from "../utils";
import { useSkillCompletion } from "../useSkillCompletion";
import type { PendingSuggestion } from "../useSkillRecommendation";
import type { SlashAction } from "../skillCompletion";
import { useStopRequest } from "../useStopRequest";
import { SkillDetailsDialog } from "./SkillDetailsDialog";
import { SkillSuggestionCard } from "./SkillSuggestionCard";
import { SkillPicker, skillPickerHeight } from "./SkillPicker";
import { FloatingTimelineButton } from "./FloatingTimelineButton";

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
  pendingApprovals,
  approvalSubmitting,
  approvalError,
  decideApproval,
  selector,
  setSelector,
  onCompactContext,
  compactionPending = false,
  keyboardHeight = 0,
  skillSuggestion = null,
  skillInstalling = false,
  onInstallSkill,
  onDismissSkill,
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
  pendingApprovals: PendingApproval[];
  approvalSubmitting: string | null;
  approvalError: { id: string; message: string } | null;
  decideApproval: (
    id: string,
    decision: "approved" | "rejected",
  ) => Promise<void>;
  selector: "model" | "thinking" | null;
  setSelector: (value: "model" | "thinking" | null) => void;
  /** Manual context compaction, offered as a `/` action (never a toolbar button). */
  onCompactContext?: () => void;
  compactionPending?: boolean;
  keyboardHeight?: number;
  /** A skill suggestion holding the draft, or null (see the card component). */
  skillSuggestion?: PendingSuggestion | null;
  skillInstalling?: boolean;
  onInstallSkill?: () => void;
  onDismissSkill?: () => void;
}) {
  const [contentHeight, setContentHeight] = useState(INPUT_MIN_HEIGHT);
  // The skill whose description is open. Owned here rather than by the picker,
  // which is unmounted as soon as the composer loses its slash token.
  const [skillDetails, setSkillDetails] = useState<RemoteSkill | null>(null);
  const { width, height, fontScale } = useWindowDimensions();
  const compactToolbar = width < 380 || fontScale > 1.2;
  // A running reply blocks sending, not drafting the next message. Keep the
  // short send/upload busy phase locked so its acknowledgement cannot clear edits.
  const editable = !remote.busy;
  const compacting = compactionPending || remote.compacting;
  const canSend = !remote.streaming && !compacting && !remote.busy && remote.desktopOnline &&
    (!!message.trim() || attachments.length > 0);
  // A run in flight rejects compaction, so the tool stays hidden rather than
  // offering an action the Agent will refuse (desktop parity).
  const compactionActionEnabled = !!onCompactContext
    && (remote.capabilities?.has?.("compaction_v1") ?? false)
    && !compacting && !remote.busy && !remote.draft && !!remote.selectedSessionId
    && remote.desktopOnline && remote.connectionPresentation.level === "connected"
    && !remote.streaming;
  const slashActions = useMemo<SlashAction[]>(() => compactionActionEnabled
    ? [{
      id: "compact",
      label: t("chat.compactContext"),
      description: t("chat.compactContextDescription"),
      searchText: "compact compaction compress context 压缩 上下文",
    }]
    : [], [compactionActionEnabled, t]);
  const handleSlashAction = useCallback((action: SlashAction) => {
    if (action.id === "compact" && compactionActionEnabled) onCompactContext?.();
  }, [compactionActionEnabled, onCompactContext]);
  const stopRequest = useStopRequest(
    remote.streaming,
    remote.selectedSessionId,
    remote.abort,
  );
  const inputRef = useRef<TextInput>(null);
  const completion = useSkillCompletion(
    message,
    setMessage,
    editable && selector === null,
    inputRef,
    handleSlashAction,
  );
  const pickerHeight = skillPickerHeight(height, keyboardHeight, slashActions.length);
  const maxInputHeight = Math.max(
    INPUT_MIN_HEIGHT,
    Math.min(
      completion.query ? 90 : INPUT_MAX_HEIGHT,
      Math.floor(height * 0.3),
    ),
  );
  const inputHeight = message
    ? Math.max(INPUT_MIN_HEIGHT, Math.min(maxInputHeight, contentHeight))
    : INPUT_MIN_HEIGHT;
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
        <FloatingTimelineButton
          label={t("chat.backToLatest")}
          icon={<ArrowDown color={colors.inkSoft} size={16} />}
          onPress={scrollToLatest}
          style={styles.backToLatest}
        />
      )}
      {!remote.draft &&
        remote.desktopOnline &&
        remote.models.length === 0 &&
        !remote.modelId && (
          <Text style={styles.offlineComposer}>
            {t("connection.noModelsHint")}
          </Text>
        )}
      {pendingApprovals.map((item) => (
        <View key={item.id} style={styles.dockedApproval}>
          <PendingApprovalCard
            error={
              approvalSubmitting !== item.payload.approval_request_id &&
              approvalError?.id === item.payload.approval_request_id
                ? approvalError.message
                : null
            }
            onDecision={(decision) =>
              void decideApproval(item.payload.approval_request_id, decision)
            }
            payload={item.payload}
            submitting={approvalSubmitting === item.payload.approval_request_id}
          />
        </View>
      ))}
      {skillSuggestion ? (
        <SkillSuggestionCard
          installing={skillInstalling}
          onDismiss={() => onDismissSkill?.()}
          onInstall={() => onInstallSkill?.()}
          suggestion={skillSuggestion}
          t={t}
        />
      ) : null}
      <View style={styles.composerArea}>
        {completion.query && (
          <SkillPicker
            key={`${remote.credentials?.pairId}:${remote.presence?.bridgeInstanceId}:${remote.selectedSessionId}:${remote.draftWorkspaceId}`}
            query={completion.query.query}
            supported={remote.capabilities?.has("skills_v1") ?? false}
            load={remote.listSkills}
            onSelect={completion.select}
            onClose={completion.close}
            onShowDetails={skill => setSkillDetails(current => current?.name === skill.name ? null : skill)}
            detailsName={skillDetails?.name ?? null}
            maxHeight={pickerHeight}
            actions={slashActions}
            onActionSelect={completion.runAction}
          />
        )}
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
                    <Text
                      numberOfLines={1}
                      style={styles.pendingAttachmentName}
                    >
                      {attachment.name}
                    </Text>
                    <Text style={styles.pendingAttachmentSize}>
                      {formatBytes(attachment.originalSize)}
                    </Text>
                  </View>
                  <Pressable
                    accessibilityLabel={t("attachment.remove", {
                      name: attachment.name,
                    })}
                    accessibilityRole="button"
                    style={styles.removeAttachment}
                    onPress={() =>
                      setAttachments((current) => {
                        deleteTemporaryAttachment(current[index]!);
                        return current.filter(
                          (_, itemIndex) => itemIndex !== index,
                        );
                      })
                    }
                  >
                    <X color={colors.inkMuted} size={14} />
                  </Pressable>
                </View>
              ))}
            </ScrollView>
          )}
          {attachments.some((a) => a.kind === "image") && !supportsImages && (
            <Text style={styles.attachmentWarning}>
              {t("attachment.imagesUnsupported")}
            </Text>
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
                onLayout={(event) =>
                  setContentHeight(Math.ceil(event.nativeEvent.layout.height))
                }
                style={styles.inputText}
              >
                {`${message}\u200b`}
              </Text>
            </View>
            <TextInput
              ref={inputRef}
              selection={completion.inputSelection}
              onSelectionChange={(event) =>
                completion.onSelectionChange(event.nativeEvent.selection)
              }
              onFocus={completion.onFocus}
              onBlur={completion.onBlur}
              accessibilityLabel={t("chat.placeholder")}
              autoCapitalize="none"
              autoCorrect={false}
              editable={editable}
              multiline
              scrollEnabled={!!message && contentHeight > maxInputHeight}
              onChangeText={completion.onChangeText}
              onSubmitEditing={() => { if (canSend) void send(); }}
              placeholder={t("chat.placeholder")}
              placeholderTextColor={colors.inkMuted}
              spellCheck={false}
              style={[styles.inputText, styles.input, { height: inputHeight }]}
              value={message}
            />
          </View>
          <View
            style={[
              styles.composerToolbar,
              compactToolbar && styles.composerToolbarCompact,
            ]}
          >
            <View
              style={[
                styles.composerSelectors,
                compactToolbar && styles.composerSelectorsCompact,
              ]}
            >
              <Pressable
                accessibilityLabel={`${t("chat.model")}: ${activeModelLabel}`}
                accessibilityRole="button"
                accessibilityState={{
                  expanded: selector === "model",
                  disabled:
                    remote.streaming || compacting ||
                    remote.connectionPresentation.level !== "connected",
                }}
                disabled={
                  remote.streaming || compacting ||
                  remote.connectionPresentation.level !== "connected"
                }
                onPress={() => setSelector("model")}
                style={({ pressed }) => [
                  styles.selectorTrigger,
                  styles.modelTrigger,
                  compactToolbar && styles.selectorTriggerCompact,
                  pressed && styles.selectorTriggerPressed,
                  (remote.streaming || compacting) && styles.controlDisabled,
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
                accessibilityState={{
                  expanded: selector === "thinking",
                  disabled:
                    remote.streaming || compacting ||
                    remote.connectionPresentation.level !== "connected",
                }}
                disabled={
                  remote.streaming || compacting ||
                  remote.connectionPresentation.level !== "connected"
                }
                onPress={() => setSelector("thinking")}
                style={({ pressed }) => [
                  styles.selectorTrigger,
                  styles.thinkingTrigger,
                  compactToolbar && styles.selectorTriggerCompact,
                  pressed && styles.selectorTriggerPressed,
                  (remote.streaming || compacting) && styles.controlDisabled,
                ]}
              >
                <Text numberOfLines={1} style={styles.selectorText}>
                  {t(`thinking.${remote.thinkingLevel}`)}
                </Text>
                <ChevronDown color={colors.inkMuted} size={14} />
              </Pressable>
            </View>
            <Pressable
              accessibilityLabel={t("skills.choose")}
              accessibilityRole="button"
              accessibilityState={{
                expanded: !!completion.query,
                disabled: !editable,
              }}
              disabled={!editable}
              onPress={completion.insertSlash}
              hitSlop={{ left: 6, right: 6 }}
              style={({ pressed }) => [
                styles.attachmentButton,
                styles.skillButton,
                (pressed || !!completion.query) &&
                  styles.selectorTriggerPressed,
                !editable && styles.controlDisabled,
              ]}
            >
              <Slash
                color={completion.query ? colors.accent : colors.inkSoft}
                size={16}
              />
            </Pressable>
            <Pressable
              accessibilityLabel={t("attachment.add")}
              accessibilityRole="button"
              disabled={
                remote.streaming || remote.busy || !remote.fileTransferSupported
              }
              onPress={openAttachmentMenu}
              style={({ pressed }) => [
                styles.attachmentButton,
                pressed && styles.selectorTriggerPressed,
                (remote.streaming ||
                  remote.busy ||
                  !remote.fileTransferSupported) &&
                  styles.controlDisabled,
              ]}
            >
              <Paperclip color={colors.inkSoft} size={17} />
            </Pressable>
            {remote.streaming ? (
              <Pressable
                accessibilityLabel={t(
                  stopRequest.status === "requesting" ? "chat.stopping" : "chat.stop",
                )}
                accessibilityRole="button"
                accessibilityState={{
                  busy: stopRequest.status === "requesting",
                  disabled: stopRequest.status === "requesting",
                }}
                disabled={stopRequest.status === "requesting"}
                onPress={() => void stopRequest.stop()}
                android_ripple={{ color: "#ffffff40" }}
                style={({ pressed }) => [
                  styles.sendButton,
                  styles.stopButton,
                  pressed && styles.sendPressed,
                ]}
              >
                {stopRequest.status === "requesting" ? (
                  <ActivityIndicator color={colors.surface} size="small" />
                ) : (
                  <Square color={colors.surface} fill={colors.surface} size={14} />
                )}
              </Pressable>
            ) : (
              <Pressable
                accessibilityLabel={t(compacting ? "chat.compacting" : "chat.send")}
                accessibilityRole="button"
                accessibilityState={{ disabled: !canSend, busy: compacting }}
                disabled={!canSend}
                onPress={() => { if (canSend) void send(); }}
                style={({ pressed }) => [
                  styles.sendButton,
                  !canSend && styles.sendDisabled,
                  pressed && styles.sendPressed,
                ]}
              >
                {compacting ? (
                  <ActivityIndicator color={colors.surface} size="small" />
                ) : (
                  <Send color={colors.surface} size={17} />
                )}
              </Pressable>
            )}
          </View>
          {remote.streaming && stopRequest.status !== "idle" && (
            <Text accessibilityLiveRegion="polite" style={styles.stopStatus}>
              {t(
                stopRequest.status === "failed"
                  ? "chat.stopFailed"
                  : stopRequest.status === "requesting"
                    ? "chat.stopping"
                    : "chat.stopRequested",
              )}
            </Text>
          )}
        </View>
      </View>
      <SkillDetailsDialog onClose={() => setSkillDetails(null)} skill={skillDetails} />
    </View>
  );
}

// Filtering the transcript yields a fresh approvals array on each text tick.
// Compare its item identities, but never ignore changed callbacks or controls.
export const ComposerDock = memo(ComposerDockView, (previous, next) => {
  const { pendingApprovals: beforeApprovals, ...before } = previous;
  const { pendingApprovals: afterApprovals, ...after } = next;
  return (
    beforeApprovals.length === afterApprovals.length &&
    beforeApprovals.every((item, index) => item === afterApprovals[index]) &&
    Object.keys(before).length === Object.keys(after).length &&
    (Object.keys(before) as (keyof typeof before)[]).every((key) =>
      Object.is(before[key], after[key]),
    )
  );
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
  backToLatest: { top: -56 },
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
  pendingAttachments: {
    gap: spacing.sm,
    paddingHorizontal: spacing.lg,
    paddingTop: spacing.sm,
  },
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
  removeAttachment: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
  },
  pendingAttachmentName: { color: colors.ink, fontSize: 12, fontWeight: "600" },
  pendingAttachmentSize: { color: colors.inkMuted, fontSize: 10 },
  attachmentWarning: {
    paddingHorizontal: spacing.lg,
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
    paddingTop: spacing.md,
    paddingBottom: spacing.sm,
  },
  input: {
    minHeight: INPUT_MIN_HEIGHT,
    textAlignVertical: "top",
  },
  composerToolbar: {
    minHeight: layout.touchTarget + spacing.sm,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: spacing.xs,
    paddingHorizontal: spacing.sm,
    paddingBottom: spacing.sm,
  },
  composerSelectors: {
    minWidth: 0,
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
  },
  // Keep both selectors visible even on small screens / with large type.
  // Toolbar 8 + trigger 8 matches the text/attachment inset of 16.
  composerToolbarCompact: { gap: spacing.xs },
  composerSelectorsCompact: { gap: spacing.xs },
  selectorTriggerCompact: { gap: spacing.xs },
  modelTrigger: { flex: 1, maxWidth: 154 },
  thinkingTrigger: { flexShrink: 0, maxWidth: "50%" },
  selectorTrigger: {
    minWidth: 0,
    maxWidth: 154,
    flexShrink: 1,
    minHeight: 44,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.sm,
    borderRadius: radius.sm,
  },
  selectorTriggerPressed: { backgroundColor: colors.surfaceSubtle },
  selectorText: {
    flexShrink: 1,
    color: colors.inkSoft,
    fontSize: 12,
    fontWeight: "600",
  },
  attachmentButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
    flexShrink: 0,
  },
  skillButton: { width: 32 },
  controlDisabled: { opacity: 0.5 },
  sendButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
    backgroundColor: colors.accent,
  },
  stopButton: { backgroundColor: colors.danger, overflow: "hidden" },
  stopStatus: {
    color: colors.inkSoft,
    fontSize: 12,
    paddingHorizontal: spacing.lg,
    paddingBottom: spacing.sm,
  },
  sendDisabled: { backgroundColor: colors.accentDisabled },
  sendPressed: { opacity: 0.78 },
});
