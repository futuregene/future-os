import { useFusionAvailability } from "../../../remote/useFusionAvailability";
import { IS_RELEASE } from "../../../version.generated";
import { INPUT_MIN_HEIGHT, INPUT_MAX_HEIGHT } from "../composer/composerMetrics";
import { ArrowDown, ArchiveRestore, CircleAlert, FileText, Paperclip, Pencil, X } from "lucide-react-native";
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
import { QUEUED_SEND_POLICY, type SendMessageIntent } from "../useSendMessage";
import { SkillDetailsDialog } from "./SkillDetailsDialog";
import { SkillSuggestionCard } from "./SkillSuggestionCard";
import { ComposerToolbar } from "../composer/ComposerToolbar";
import { choiceAnswerLabel, pickChoiceOption, type ChoiceAnswer } from "../choiceAnswer";
import { ChoiceSheet } from "./ChoiceSheet";
import { useStyles } from "../composer/composerStyles";
import { SendChoiceSheet } from "./SendChoiceSheet";
import { SendQueuePanel, sendQueueListHeight, type SendQueueAttachmentItem, type SendQueuePanelItem } from "./SendQueuePanel";
import { SkillPicker, skillPickerHeight } from "./SkillPicker";
import { SessionPicker, sessionPickerHeight } from "./SessionPicker";
import { FloatingTimelineButton } from "./FloatingTimelineButton";

type Remote = RemoteControls;

type PendingApproval = Extract<TimelineItem, { kind: "approval"; }>;

function ComposerDockView({
  message,
  setMessage,
  choice = null,
  onChangeChoice,
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
  queueItems = [],
  queueNotice,
  onQueueUpdate,
  onQueueUpdateChoice,
  onQueueRemove,
  onQueueRelease,
  onQueuePreviewAttachment,
  onQueueRemoveAttachment,
  onQueueAddAttachments,
  archived = false,
  onRestore,
  restoringArchived = false,
}: {
  message: string;
  setMessage: Dispatch<SetStateAction<string>>;
  /**
   * The draft's answer to a `futureos-choice` card, or null.
   *
   * Not a decoration: the phone's field is a native `textarea` and cannot hold
   * the desktop's in-editor pill, so the answer rides in the chip row above it
   * **and** counts as the draft's content (§5.7.1). That is what makes "pick one
   * and send without typing" reachable — the gate below reads this value, not the
   * chip's pixels.
   */
  choice?: ChoiceAnswer | null;
  /** Replace or drop the draft's answer (a row, a pick in the sheet, the chip's ×). */
  onChangeChoice?: (next: ChoiceAnswer | null) => void;
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
  send: (intent?: SendMessageIntent) => Promise<boolean>;
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
  /**
   * This conversation's send queue (see `remote/sessionQueue`): what the session
   * is holding behind its running turn, shown above the box so it can be read,
   * fixed or taken back before it leaves.
   */
  queueItems?: readonly SendQueuePanelItem[];
  /**
   * This conversation is archived (read-only).
   *
   * The desktop refuses a prompt into an archived session, so the phone does not
   * offer one: the editor and its controls lock, and a banner says where to undo
   * it. Display-only, matching the desktop's read-only detail.
   */
  archived?: boolean;
  /**
   * Bring the shelved conversation back, so it can be written to again.
   *
   * Offered where the machine answered `archive_v1` and the page has the row to
   * address: the phone can shelve and unshelve from its own lists (see the
   * archived shelf), so a conversation opened from the shelf is not a dead end
   * waiting for someone at the desktop. Absent on a machine too old for the
   * command, which is when the banner keeps naming the desktop instead.
   */
  onRestore?: () => void;
  /** The restore the reader asked for is in flight. */
  restoringArchived?: boolean;
  /**
   * One sentence about a row that left the queue without running.
   *
   * Drawn outside the panel because it matters most when the panel is empty: the
   * row it is about is the one that is no longer there.
   */
  queueNotice?: string;
  /** Take one waiting message back out (a cancel of the session's run id). */
  onQueueRemove?: (id: string) => void;
  /** Replace one waiting message: the session has no rename, so cancel + send. */
  onQueueUpdate?: (id: string, content: string) => void;
  /** Offer a different pick for a queued answer (design §5.8.2). */
  /**
   * Change the pick a queued message carries.
   *
   * Takes the whole answer, not the ids: the card's question and its other
   * options have to survive a change or the chip could offer them once and never
   * again. The desktop panel's `onUpdateChoice` is the same signature, so the two
   * surfaces cannot drift into meaning different things by the same name.
   */
  onQueueUpdateChoice?: (id: string, choice: ChoiceAnswer) => void;
  /** Start a parked row now. */
  onQueueRelease?: (id: string) => void;
  /** Open a file a waiting message carries, as the transcript would. */
  onQueuePreviewAttachment?: (attachment: SendQueueAttachmentItem) => void;
  /** Take one file off a waiting message (the rest are re-sent with it). */
  onQueueRemoveAttachment?: (id: string, path: string) => void;
  /** Add photos/files to a waiting message before it leaves. */
  onQueueAddAttachments?: (id: string) => void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const [contentHeight, setContentHeight] = useState(INPUT_MIN_HEIGHT);
  // The skill whose description is open. Owned here rather than by the picker,
  // which is unmounted as soon as the composer loses its slash token.
  const [skillDetails, setSkillDetails] = useState<RemoteSkill | null>(null);
  // The "what does send mean now" sheet, from the toolbar while a run streams.
  const [queueChoiceOpen, setQueueChoiceOpen] = useState(false);
  const { width, height, fontScale } = useWindowDimensions();
  const compactToolbar = width < 380 || fontScale > 1.2;
  // A running reply now blocks neither drafting nor sending: a message sent
  // while the turn streams is handed to the session, which holds it and runs it
  // at the boundary. Keep the short send/upload busy phase locked so its
  // acknowledgement cannot clear edits.
  // A compaction does not lock the send either: a manual compaction accepts the
  // prompt and holds it in the session's queue, so what the reader writes
  // mid-summary leaves with the run that starts when the summary settles.
  // The recommendation wait locks drafting too (see `skillEvaluating`): the
  // message that goes out must be the one the recommender was asked about.
  const editable = !remote.busy && !skillEvaluating && !skillInstalling && !draftRestoring && !archived;
  const compacting = compactionPending || remote.compacting;
  // The sheet the chip's pencil opens: the bottom-sheet answer to "which one".
  const [choiceSheetOpen, setChoiceSheetOpen] = useState(false);
  // Which queued message's answer the sheet is open for. The queue panel is a
  // list of rows, so it asks the dock to open the sheet rather than owning one
  // (§5.8.2 — the chip is the button, the sheet is the dock's).
  const [queueAnswerRow, setQueueAnswerRow] = useState<string | null>(null);
  // A queued answer reshaped as the sheet's own type. Same fields, so the sheet
  // draws a waiting message's card exactly as it draws the draft's.
  const queueAnswerSource = queueAnswerRow === null
    ? undefined
    : queueItems.find(item => item.id === queueAnswerRow)?.choice;
  const queueAnswer: ChoiceAnswer | null = queueAnswerSource
    ? { ...queueAnswerSource }
    : null;
  /**
   * Whether the draft holds a card's answer at all.
   *
   * Zero ticks is no answer (§7): `answerOrNull` already drops one before it
   * reaches this component, and this asks the same question so the three places
   * that decide — the send gate, `hasDraft`, and whether to draw the chip — cannot
   * drift apart. A chip drawn from an unpicked answer would name the question and
   * answer nothing; counted but undrawn, it would grey out with the answer still
   * held.
   */
  const hasAnswer = choice !== null && choice.selectedIds.length > 0;
  const canSend = !archived && !skillInstalling && !draftRestoring && !skillEvaluating && !remote.busy && remote.desktopOnline &&
    (!!message.trim() || attachments.length > 0 || hasAnswer);
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
  const fusionAvailable = useFusionAvailability();
  const slashActions = useMemo<SlashAction[]>(() => {
    const actions: SlashAction[] = !IS_RELEASE && fusionAvailable ? [{ id: "fusion", label: "/fusion", description: t("chat.fusionDescription"), searchText: "fusion 讨论 多模型 思考", insert: "/fusion " }] : [];
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
  }, [canReference, compactionActionEnabled, t, fusionAvailable]);
  const handleSlashAction = useCallback((action: SlashAction) => {
    if (action.id === "fusion") setMessage(value => `/fusion ${value.replace(/(^|\s)\/fusion(?=\s|$)\s*/u, "$1").trimStart()}`);
    if (action.id === "compact" && compactionActionEnabled) onCompactContext?.();
  }, [compactionActionEnabled, onCompactContext, setMessage]);
  const stopRequest = useStopRequest(
    remote.streaming,
    remote.selectedSessionId,
    remote.abort,
  );
  // A picked answer is content of its own: with nothing typed and nothing
  // attached, a card's answer is still something to send (§5.4, requirement d), so
  // it is counted exactly where an attachment is.
  const hasDraft = !!message.trim() || attachments.length > 0 || hasAnswer;
  /**
   * Hand the draft to the session, and end the run it waited behind.
   *
   * The message goes first, with the queueing policy, and the interrupt second —
   * the order the desktop uses, for the reason it uses it. Whatever the run is
   * doing, the prompt is the session's the moment this returns: queued behind a
   * run still finishing, or starting at once if it has already ended. Sent the
   * other way round, the stop can win the race and the prompt then arrives at a
   * host that still reads the session as busy — refused with `session_busy`, the
   * draft back in the field, and an interrupt the reader asked to be followed by
   * a send that never leaves. And a send that did not leave stops nothing: the
   * draft is still there to fix, and ending the reply on top of it would cost the
   * reader both.
   */
  const interruptAndSend = useCallback(async () => {
    setQueueChoiceOpen(false);
    const handedOver = await send({ busyPolicy: QUEUED_SEND_POLICY });
    if (handedOver)
      await stopRequest.stop();
  }, [send, stopRequest]);
  const sendQueued = useCallback(() => {
    setQueueChoiceOpen(false);
    // The other row of the same sheet: it exists only because a reply streams,
    // so it names the queueing policy rather than leaving the send to read that
    // from this screen's own flag.
    void send({ busyPolicy: QUEUED_SEND_POLICY });
  }, [send]);
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
  // How tall an expanded send queue may draw before its rows scroll: the reply
  // above it keeps the rest of the room (see `sendQueueListHeight`).
  const queueListHeight = sendQueueListHeight(height, keyboardHeight);
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
      {archived && (
        <View style={styles.archivedBanner} testID="composer-archived-banner">
          <Text style={styles.archivedBannerText}>
            {t(onRestore ? "chat.archivedRestoreHint" : "chat.archivedReadOnly")}
          </Text>
          {onRestore && (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("sessions.restore")}
              accessibilityState={{ disabled: restoringArchived }}
              disabled={restoringArchived}
              onPress={onRestore}
              style={({ pressed }) => [
                styles.archivedRestore,
                (pressed || restoringArchived) && styles.archivedRestoreDim,
              ]}
            >
              <ArchiveRestore size={14} color={colors.accent} />
              <Text style={styles.archivedRestoreLabel}>{t("sessions.restore")}</Text>
            </Pressable>
          )}
        </View>
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
      {queueNotice ? (
        <Text style={styles.queueNotice} testID="send-queue-notice">{queueNotice}</Text>
      ) : null}
      {queueItems.length > 0 && (
        <SendQueuePanel
          items={queueItems}
          maxListHeight={queueListHeight}
          onAddAttachments={id => onQueueAddAttachments?.(id)}
          onPreviewAttachment={attachment => onQueuePreviewAttachment?.(attachment)}
          onRelease={id => onQueueRelease?.(id)}
          onRemove={id => onQueueRemove?.(id)}
          onRemoveAttachment={(id, path) => onQueueRemoveAttachment?.(id, path)}
          onUpdate={(id, content) => onQueueUpdate?.(id, content)}
          onRequestChoiceChange={setQueueAnswerRow}
        />
      )}
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
          {/*
            The answer to a `futureos-choice` card, above the field (design
            `docs/design/choice-cards.zh-CN.md` §5.7). The field is a native
            `textarea`, so unlike the desktop's editor it cannot hold an atomic
            pill; this row is where the answer lives, and the dock *counts* it as
            content (`canSend`/`hasDraft`) rather than only drawing it.

            The question stays, muted and truncating first, because one thread can
            hold several cards and "重画森林图" alone would not say what it
            answers. Both actions wear one ink: editing and removing are the same
            weight of act (§5.7.1).
          */}
          {hasAnswer && (
            <View style={styles.choiceRow} testID="composer-choice-row">
              <View style={styles.choiceChip} testID="composer-choice-chip">
                <Text numberOfLines={1} style={styles.choiceChipTitle}>{choice.title}</Text>
                <Text style={styles.choiceChipDot}>·</Text>
                <Text numberOfLines={1} style={styles.choiceChipAnswer}>
                  {choiceAnswerLabel(choice)}
                </Text>
                <Pressable
                  accessibilityLabel={t("choice.chip.change", { title: choice.title })}
                  accessibilityRole="button"
                  disabled={!editable}
                  onPress={() => setChoiceSheetOpen(true)}
                  style={styles.choiceChipAction}
                  testID="composer-choice-edit"
                >
                  <Pencil color={colors.inkSoft} size={12} />
                </Pressable>
                <Pressable
                  accessibilityLabel={t("choice.chip.remove", { title: choice.title })}
                  accessibilityRole="button"
                  disabled={!editable}
                  onPress={() => onChangeChoice?.(null)}
                  style={styles.choiceChipAction}
                  testID="composer-choice-remove"
                >
                  <X color={colors.inkSoft} size={13} />
                </Pressable>
              </View>
            </View>
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
            hasDraft={hasDraft}
            onOpenQueueChoice={() => setQueueChoiceOpen(true)}
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
      {/* One sheet, two sources. The draft's chip and a queued row's chip both
          answer the same question, so they share the sheet: whichever row is
          named wins, and the sheet is open while either source has an answer
          to offer. Two instances would mean two `Modal`s and two chances to
          pick the wrong `open`. */}
      <ChoiceSheet
        answer={queueAnswer ?? choice}
        onClose={() => {
          setChoiceSheetOpen(false);
          setQueueAnswerRow(null);
        }}
        onPick={(optionId) => {
          const source = queueAnswer ?? choice;
          if (!source)
            return;
          // A single-choice pick is the whole act, so the sheet closes on it; a
          // multi-choice one keeps the sheet up so several can be ticked.
          if (queueAnswer && queueAnswerRow !== null) {
            const picked = pickChoiceOption(queueAnswer, optionId);
            // Zero ticks is no answer (§7): a multi-choice row whose last pick was
            // just removed has nothing to rewrite the waiting message with, so the
            // sheet stays up rather than closing on a refused change.
            if (picked.selectedIds.length === 0)
              return;
            onQueueUpdateChoice?.(queueAnswerRow, picked);
            if (queueAnswer.mode === "single")
              setQueueAnswerRow(null);
            return;
          }
          onChangeChoice?.(pickChoiceOption(source, optionId));
          if (source.mode === "single")
            setChoiceSheetOpen(false);
        }}
        open={choiceSheetOpen || queueAnswer !== null}
        t={t}
      />
      <SendChoiceSheet
        open={queueChoiceOpen}
        t={t}
        onClose={() => setQueueChoiceOpen(false)}
        onInterruptAndSend={() => void interruptAndSend()}
        onSend={sendQueued}
      />
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
