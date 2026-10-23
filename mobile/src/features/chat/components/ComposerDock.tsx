import { INPUT_MIN_HEIGHT, INPUT_MAX_HEIGHT } from "../composer/composerMetrics";
import { ArrowDown, CircleAlert, FileText, Paperclip, X } from "lucide-react-native";
import { Pressable, ScrollView, Text, TextInput, useWindowDimensions, View, type TextInputInstance } from "react-native";
import { memo, useCallback, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type { TFunction } from "i18next";
import Svg, { Defs, LinearGradient, Rect, Stop } from "react-native-svg";
import { PendingApprovalCard } from "../messages/PendingApprovalCard";
import type { RemoteControls } from "../../../remote/RemoteContext";
import { deleteTemporaryAttachment } from "../../../remote/files";
import type { MobileAttachment, RemoteSkill, SessionReferenceMap, SessionReferenceTarget, TimelineItem } from "../../../remote/types";
import { useTheme } from "../../../theme/ThemeContext";
import { formatBytes } from "../utils";
import { useComposerCompletion } from "../useComposerCompletion";
import { sessionMentionOptions } from "../sessionMention";
import type { PendingSuggestion } from "../useSkillRecommendation";
import type { SlashAction } from "../skillCompletion";
import { useStopRequest } from "../useStopRequest";
import { SkillDetailsDialog } from "./SkillDetailsDialog";
import { SkillSuggestionCard } from "./SkillSuggestionCard";
import { ComposerToolbar } from "../composer/ComposerToolbar";
import { useStyles } from "../composer/composerStyles";
import { SkillPicker, skillPickerHeight } from "./SkillPicker";
import { SessionPicker, sessionPickerHeight } from "./SessionPicker";
import { FloatingTimelineButton } from "./FloatingTimelineButton";

type Remote = RemoteControls;

type PendingApproval = Extract<TimelineItem, { kind: "approval"; }>;

function ComposerDockView({
  message,
  setMessage,
  sessionRefs,
  rememberSessionRef,
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
  draftRestoring = false,
  keyboardHeight = 0,
  skillSuggestion = null,
  skillEvaluating = false,
  skillInstalling = false,
  onInstallSkill,
  onDismissSkill,
}: {
  message: string;
  setMessage: Dispatch<SetStateAction<string>>;
  /**
   * The draft's conversation references: the `#title` tokens it shows and the
   * conversations they stand for (see `useComposerDraft`).
   */
  sessionRefs: SessionReferenceMap;
  rememberSessionRef: (token: string, target: SessionReferenceTarget) => void;
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
  approvalError: { id: string; message: string; } | null;
  decideApproval: (
    id: string,
    decision: "approved" | "rejected",
  ) => Promise<void>;
  selector: "model" | "thinking" | null;
  setSelector: (value: "model" | "thinking" | null) => void;
  /** Manual context compaction, offered as a `/` action (never a toolbar button). */
  onCompactContext?: () => void;
  compactionPending?: boolean;
  draftRestoring?: boolean;
  keyboardHeight?: number;
  /** A skill suggestion holding the draft, or null (see the card component). */
  skillSuggestion?: PendingSuggestion | null;
  /** True while the recommender is being asked: locks input + send (spinner). */
  skillEvaluating?: boolean;
  skillInstalling?: boolean;
  onInstallSkill?: () => void;
  onDismissSkill?: () => void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const [contentHeight, setContentHeight] = useState(INPUT_MIN_HEIGHT);
  // The skill whose description is open. Owned here rather than by the picker,
  // which is unmounted as soon as the composer loses its slash token.
  const [skillDetails, setSkillDetails] = useState<RemoteSkill | null>(null);
  const { width, height, fontScale } = useWindowDimensions();
  const compactToolbar = width < 380 || fontScale > 1.2;
  // A running reply blocks sending, not drafting the next message. Keep the
  // short send/upload busy phase locked so its acknowledgement cannot clear edits.
  // The recommendation wait locks drafting too (see `skillEvaluating`): the
  // message that goes out must be the one the recommender was asked about.
  const editable = !remote.busy && !skillEvaluating && !skillInstalling && !draftRestoring;
  const compacting = compactionPending || remote.compacting;
  const canSend = !skillInstalling && !draftRestoring && !skillEvaluating && !remote.streaming && !compacting && !remote.busy && remote.desktopOnline &&
    (!!message.trim() || attachments.length > 0);
  // A run in flight rejects compaction, so the tool stays hidden rather than
  // offering an action the Agent will refuse (desktop parity).
  const compactionActionEnabled = !!onCompactContext
    && (remote.capabilities?.has?.("compaction_v1") ?? false)
    && !compacting && !remote.busy && !remote.draft && !!remote.selectedSessionId
    && remote.desktopOnline && remote.connectionPresentation.level === "connected"
    && !remote.streaming;
  // Whether any conversation can be referenced at all (the same filter the `#`
  // menu's own grouping applies). The `/` menu offers the reference entry only
  // when it leads somewhere — a row that opens an empty list is a dead end.
  const canReference = useMemo(
    () => (remote.sessions ?? []).some(
      session => session.sessionId && session.sessionId !== remote.selectedSessionId,
    ),
    [remote.sessions, remote.selectedSessionId],
  );
  const slashActions = useMemo<SlashAction[]>(() => {
    const actions: SlashAction[] = [];
    if (canReference) {
      // The phone has no `#` key to speak of (it sits on the keyboard's symbol
      // page) while `/` has a button of its own, so the reference would be
      // unreachable in practice. Leaving `#` in the draft hands over to the
      // conversation menu — the same hand-off the desktop gets by typing it.
      actions.push({
        id: "reference",
        label: t("chat.referenceConversation"),
        description: t("chat.referenceConversationDescription"),
        searchText: "reference session conversation 引用 会话 对话 #",
        insert: "#",
      });
    }
    if (compactionActionEnabled) {
      actions.push({
        id: "compact",
        label: t("chat.compactContext"),
        description: t("chat.compactContextDescription"),
        searchText: "compact compaction compress context 压缩 上下文",
      });
    }
    return actions;
  }, [canReference, compactionActionEnabled, t]);
  const handleSlashAction = useCallback((action: SlashAction) => {
    if (action.id === "compact" && compactionActionEnabled) onCompactContext?.();
  }, [compactionActionEnabled, onCompactContext]);
  const stopRequest = useStopRequest(
    remote.streaming,
    remote.selectedSessionId,
    remote.abort,
  );
  const inputRef = useRef<TextInputInstance>(null);
  const completion = useComposerCompletion(
    message,
    setMessage,
    editable && selector === null,
    inputRef,
    handleSlashAction,
    { refs: sessionRefs, remember: rememberSessionRef },
  );
  const pickerHeight = skillPickerHeight(height, keyboardHeight, slashActions.length);
  // The `#` menu's conversations, newest activity first — computed here (not in
  // the picker) so its height can account for the rows it will draw.
  const sessionMentions = useMemo(
    () => sessionMentionOptions(
      remote.sessions ?? [],
      remote.workspaces ?? [],
      completion.session?.query ?? "",
      remote.selectedSessionId,
    ),
    [completion.session?.query, remote.sessions, remote.workspaces, remote.selectedSessionId],
  );
  const sessionHeight = sessionPickerHeight(
    height,
    keyboardHeight,
    sessionMentions.length,
  );
  const maxInputHeight = Math.max(
    INPUT_MIN_HEIGHT,
    Math.min(
      completion.query || completion.session ? 90 : INPUT_MAX_HEIGHT,
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
        {completion.session && (
          <SessionPicker
            sessions={sessionMentions}
            maxHeight={sessionHeight}
            onClose={completion.close}
            onSelect={completion.selectSession}
          />
        )}
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
                    disabled={!editable}
                    onPress={() => {
                      // File I/O belongs to the event, never a state updater.
                      try { deleteTemporaryAttachment(attachment); } catch { /* best-effort cleanup */ }
                      setAttachments(current => current.filter(item => item.localUri !== attachment.localUri));
                    }}
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
          <ComposerToolbar
            remote={remote}
            t={t}
            compactToolbar={compactToolbar}
            compacting={compacting}
            activeModelLabel={activeModelLabel}
            selector={selector}
            setSelector={setSelector}
            completion={completion}
            editable={editable}
            openAttachmentMenu={openAttachmentMenu}
            stopRequest={stopRequest}
            skillEvaluating={skillEvaluating}
            canSend={canSend}
            send={send}
          />
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
