import { Check, ChevronDown, ChevronUp, Image as ImageIcon, Paperclip, Pencil, Play, Plus, Trash2, X } from "lucide-react-native";
import { useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";
import { formatBytes } from "../utils";

/** One file a waiting message carries, as the panel draws it. */
export interface SendQueueAttachmentItem {
  /** The desktop path — the identity a removal addresses. */
  path: string;
  /** "image" | "file". */
  kind: string;
  name: string;
  /** Byte size, or 0 when the desktop did not report one. */
  size: number;
}

/**
 * One row of the panel: a message the session is holding behind its running turn.
 *
 * `id` is the session's own run id, because that is what a cancel or a release
 * addresses; the phone keeps no row identity of its own any more.
 */
export interface SendQueuePanelItem {
  id: string;
  /** The text as the session holds it (empty for an attachment-only message). */
  content: string;
  /**
   * The files the message carries.
   *
   * `undefined` means this row's queue did not report them (a desktop older than
   * the field): the panel then says so in the editor and offers no file controls
   * rather than guessing at a count or silently dropping what it cannot see.
   */
  attachments?: readonly SendQueueAttachmentItem[];
  /**
   * Parked by the session: it waited too long to run by itself and now starts
   * only when the reader says so. Absent means "runs at the next turn boundary".
   */
  parked?: boolean;
}

/**
 * How many waiting messages the panel shows without being asked.
 *
 * The panel is docked above the composer, so every row it draws is a row of the
 * conversation that is no longer on screen: a burst of queued messages used to
 * cover the reply they were queued behind. Two rows are enough to see what is
 * waiting and that the queue is moving; past that the panel opens as a header the
 * reader expands when they want the list.
 */
export const QUEUE_ROWS_SHOWN_BY_DEFAULT = 2;

/** The tallest the open list may be, and the shortest that still shows a row. */
const OPEN_LIST_MAX_HEIGHT = 240;
const OPEN_LIST_MIN_HEIGHT = 96;

/**
 * How tall the open list may be: a few rows, but never more than a third of the
 * room the keyboard leaves — the reply above keeps the rest. Same contract as
 * `sessionPickerHeight`: the caller measures the screen, the panel draws inside
 * what it is given.
 */
export function sendQueueListHeight(viewportHeight: number, keyboardHeight: number): number {
  const room = viewportHeight - keyboardHeight;
  return Math.max(
    OPEN_LIST_MIN_HEIGHT,
    Math.min(OPEN_LIST_MAX_HEIGHT, Math.round(room * 0.3)),
  );
}

/**
 * The send queue, above the box that filled it.
 *
 * A queued message is not yet gone, so this is the one place to read it, fix a
 * typo, open a photo it carries or take one off before the run it waits for ends.
 * It disappears the moment the queue empties, because an empty panel would only
 * take space. A long queue starts collapsed to its header — the conversation it
 * sits over is what the reader was looking at — and the header expands it.
 *
 * The rows are the conversation's, not this phone's: the same list the desktop's
 * own window shows, because both read it from the session that holds it. That is
 * why a removal is a cancel of a run id and an edit is "take it back, send the
 * new words" — there is no local record here to rename.
 */
export function SendQueuePanel({
  items,
  maxListHeight,
  onAddAttachments,
  onPreviewAttachment,
  onRelease,
  onRemove,
  onRemoveAttachment,
  onUpdate,
}: {
  items: readonly SendQueuePanelItem[];
  /**
   * The tallest the open list may draw, from `sendQueueListHeight`. Past it the
   * rows scroll inside the panel instead of pushing the conversation off screen.
   */
  maxListHeight?: number;
  /** Start a parked row now. Offered only when the session can park one. */
  onRelease?: (id: string) => void;
  onRemove: (id: string) => void;
  onUpdate: (id: string, content: string) => void;
  /** Open one file of a waiting message, exactly as the transcript would. */
  onPreviewAttachment?: (attachment: SendQueueAttachmentItem) => void;
  /** Take one file off a waiting message (the rest are re-sent with it). */
  onRemoveAttachment?: (id: string, path: string) => void;
  /** Add photos/files to a waiting message before it leaves. */
  onAddAttachments?: (id: string) => void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  // One row is edited at a time — the panel is a list of what is about to leave,
  // not a second composer.
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  // Whether the rows are showing. `null` means the reader has not decided, and
  // the panel keeps following the queue: open while it is short enough to sit
  // above the composer without covering the reply, collapsed once it is not. A
  // choice made by hand outlives the count it was made at — the panel unmounts
  // with an empty queue anyway, so the next one starts from the rule again.
  const [open, setOpen] = useState<boolean | null>(null);
  const expanded = open ?? items.length <= QUEUE_ROWS_SHOWN_BY_DEFAULT;

  if (items.length === 0)
    return null;

  function saveEditing(id: string) {
    onUpdate(id, editingText);
    setEditingId(null);
    setEditingText("");
  }

  return (
    <View accessibilityLabel={t("chat.queue.title")} style={styles.panel} testID="send-queue">
      {/* The whole header is the toggle: it is a line of text, and collapsing a
          queue should not mean hitting a 14pt arrow. */}
      <Pressable
        accessibilityLabel={expanded ? t("chat.queue.collapse") : t("chat.queue.expand")}
        accessibilityRole="button"
        accessibilityState={{ expanded }}
        onPress={() => setOpen(!expanded)}
        style={({ pressed }) => [styles.header, pressed && styles.rowPressed]}
        testID="send-queue-toggle"
      >
        <Text style={styles.title}>{t("chat.queue.title")}</Text>
        <Text style={styles.count}>{t("chat.queue.count", { count: items.length })}</Text>
        <View style={styles.headerSpacer} />
        {/* A parked row is not going to run when the reply ends — it never will
            on its own — so the header must not promise the merge it describes. */}
        <Text numberOfLines={1} style={styles.hint} testID="send-queue-hint">
          {items.some(item => item.parked) ? t("chat.queue.hintParked") : t("chat.queue.hint")}
        </Text>
        {expanded
          ? <ChevronUp color={colors.inkMuted} size={14} />
          : <ChevronDown color={colors.inkMuted} size={14} />}
      </Pressable>
      {expanded && (
        <ScrollView
          bounces={false}
          contentContainerStyle={styles.listContent}
          keyboardShouldPersistTaps="handled"
          style={[styles.list, { maxHeight: maxListHeight ?? OPEN_LIST_MAX_HEIGHT }]}
          testID="send-queue-list"
        >
          {items.map((item, index) => {
            const editing = editingId === item.id;
            return (
              <View key={item.id} style={styles.row} testID="send-queue-item">
                <Text style={styles.index}>{index + 1}</Text>
                {editing
                  ? (
                      <View style={styles.editArea}>
                        {item.attachments === undefined ? (
                          <Text style={styles.editNote}>{t("chat.queue.editDropsFiles")}</Text>
                        ) : null}
                        <TextInput
                          accessibilityLabel={t("chat.queue.editLabel")}
                          autoFocus
                          multiline
                          onChangeText={setEditingText}
                          style={styles.editField}
                          value={editingText}
                        />
                        <View style={styles.editActions}>
                          <Pressable
                            accessibilityRole="button"
                            onPress={() => {
                              setEditingId(null);
                              setEditingText("");
                            }}
                            style={({ pressed }) => [styles.editTextButton, pressed && styles.rowPressed]}
                          >
                            <Text style={styles.editTextButtonLabel}>{t("chat.queue.cancel")}</Text>
                          </Pressable>
                          <Pressable
                            accessibilityRole="button"
                            onPress={() => saveEditing(item.id)}
                            style={({ pressed }) => [styles.save, pressed && styles.rowPressed]}
                            testID="send-queue-save"
                          >
                            <Check color={colors.surface} size={13} />
                            <Text style={styles.saveLabel}>{t("chat.queue.save")}</Text>
                          </Pressable>
                        </View>
                      </View>
                    )
                  : (
                      <>
                        <View style={styles.rowCopy}>
                          {item.content
                            ? (
                                <Text numberOfLines={3} style={styles.content}>
                                  {item.content}
                                </Text>
                              )
                            : <Text style={styles.attachmentsOnly}>{t("chat.queue.attachmentsOnly")}</Text>}
                          {(item.attachments && item.attachments.length > 0
                            || (onAddAttachments && item.attachments !== undefined)) && (
                            <View style={styles.chips} testID="send-queue-attachments">
                              {(item.attachments ?? []).map(attachment => (
                                <View key={attachment.path} style={styles.chip}>
                                  <Pressable
                                    accessibilityLabel={t("chat.queue.previewAttachment", {
                                      name: attachment.name,
                                    })}
                                    accessibilityRole="button"
                                    disabled={!onPreviewAttachment}
                                    onPress={() => onPreviewAttachment?.(attachment)}
                                    style={({ pressed }) => [styles.chipBody, pressed && styles.rowPressed]}
                                    testID="send-queue-attachment-preview"
                                  >
                                    {attachment.kind === "image"
                                      ? <ImageIcon color={colors.inkMuted} size={11} />
                                      : <Paperclip color={colors.inkMuted} size={11} />}
                                    <Text numberOfLines={1} style={styles.chipName}>
                                      {attachment.name}
                                    </Text>
                                    {attachment.size > 0 && (
                                      <Text style={styles.chipSize}>{formatBytes(attachment.size)}</Text>
                                    )}
                                  </Pressable>
                                  {onRemoveAttachment && (
                                    <Pressable
                                      accessibilityLabel={t("chat.queue.removeAttachment", {
                                        name: attachment.name,
                                      })}
                                      accessibilityRole="button"
                                      hitSlop={6}
                                      onPress={() => onRemoveAttachment(item.id, attachment.path)}
                                      style={({ pressed }) => [styles.chipRemove, pressed && styles.rowPressed]}
                                      testID="send-queue-attachment-remove"
                                    >
                                      <X color={colors.inkMuted} size={10} />
                                    </Pressable>
                                  )}
                                </View>
                              ))}
                              {onAddAttachments && (
                                <Pressable
                                  accessibilityLabel={t("chat.queue.addAttachment")}
                                  accessibilityRole="button"
                                  onPress={() => onAddAttachments(item.id)}
                                  style={({ pressed }) => [styles.chipAdd, pressed && styles.rowPressed]}
                                  testID="send-queue-attachment-add"
                                >
                                  <Plus color={colors.accent} size={11} />
                                </Pressable>
                              )}
                            </View>
                          )}
                          {item.parked && onRelease
                            ? (
                                <Pressable
                                  accessibilityLabel={t("chat.queue.release")}
                                  accessibilityRole="button"
                                  onPress={() => onRelease(item.id)}
                                  style={({ pressed }) => [styles.release, pressed && styles.rowPressed]}
                                  testID="send-queue-release"
                                >
                                  <Play color={colors.warning} size={11} />
                                  <Text style={styles.releaseLabel}>{t("chat.queue.release")}</Text>
                                </Pressable>
                              )
                            : null}
                        </View>
                        {/* Nothing to edit in a message that is only files: an empty
                            save is a removal, and the bin beside it says so plainly. */}
                        {item.content
                          ? (
                              <Pressable
                                accessibilityLabel={t("chat.queue.edit")}
                                accessibilityRole="button"
                                hitSlop={8}
                                onPress={() => {
                                  setEditingId(item.id);
                                  setEditingText(item.content);
                                }}
                                style={({ pressed }) => [styles.rowButton, pressed && styles.rowPressed]}
                                testID="send-queue-edit"
                              >
                                <Pencil color={colors.inkMuted} size={14} />
                              </Pressable>
                            )
                          : null}
                        <Pressable
                          accessibilityLabel={t("chat.queue.remove")}
                          accessibilityRole="button"
                          hitSlop={8}
                          onPress={() => onRemove(item.id)}
                          style={({ pressed }) => [styles.rowButton, pressed && styles.rowPressed]}
                          testID="send-queue-remove"
                        >
                          <Trash2 color={colors.inkMuted} size={14} />
                        </Pressable>
                      </>
                    )}
              </View>
            );
          })}
        </ScrollView>
      )}
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  panel: {
    marginHorizontal: spacing.md,
    marginBottom: spacing.sm,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.sm,
    gap: spacing.xs,
    borderWidth: 1,
    borderColor: theme.colors.lineSoft,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surfaceSubtle,
  },
  header: {
    // A touch target in its own right: this row is the expand/collapse control.
    minHeight: 32,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
  },
  title: {
    color: theme.colors.ink,
    fontSize: theme.font(12),
    fontWeight: "700",
  },
  count: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
    fontVariant: ["tabular-nums"],
  },
  headerSpacer: { flex: 1 },
  hint: {
    flexShrink: 1,
    color: theme.colors.inkMuted,
    fontSize: theme.font(11),
    textAlign: "right",
  },
  list: { flexGrow: 0, flexShrink: 1 },
  listContent: { gap: spacing.xs },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: spacing.sm,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.xs,
    borderRadius: radius.sm,
    backgroundColor: theme.colors.surface,
  },
  index: {
    minWidth: 14,
    color: theme.colors.inkMuted,
    fontSize: theme.font(11),
    lineHeight: theme.font(20),
    textAlign: "right",
    fontVariant: ["tabular-nums"],
  },
  rowCopy: { minWidth: 0, flex: 1 },
  content: {
    color: theme.colors.ink,
    fontSize: theme.font(12),
    lineHeight: theme.font(18),
  },
  attachmentsOnly: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
    fontStyle: "italic",
  },
  chips: {
    marginTop: 2,
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: spacing.xs,
  },
  chip: {
    maxWidth: "100%",
    flexDirection: "row",
    alignItems: "center",
    borderRadius: radius.sm,
    backgroundColor: theme.colors.surfaceSubtle,
  },
  chipBody: {
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 3,
    paddingLeft: spacing.xs,
    paddingVertical: 2,
  },
  chipName: {
    flexShrink: 1,
    color: theme.colors.ink,
    fontSize: theme.font(11),
  },
  chipSize: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(10),
    fontVariant: ["tabular-nums"],
  },
  chipRemove: {
    paddingHorizontal: 3,
    paddingVertical: 3,
  },
  chipAdd: {
    width: 22,
    height: 20,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: theme.colors.line,
  },
  release: {
    marginTop: 2,
    flexDirection: "row",
    alignItems: "center",
    alignSelf: "flex-start",
    gap: spacing.xs,
    paddingHorizontal: spacing.xs,
    paddingVertical: 1,
    borderRadius: radius.sm,
  },
  releaseLabel: {
    color: theme.colors.warning,
    fontSize: theme.font(11),
    fontWeight: "600",
  },
  rowButton: {
    width: 28,
    height: 28,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.sm,
  },
  rowPressed: { opacity: 0.6 },
  editArea: { minWidth: 0, flex: 1, gap: spacing.xs },
  editNote: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(11),
    lineHeight: theme.font(15),
  },
  editField: {
    maxHeight: 120,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.xs,
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.sm,
    color: theme.colors.ink,
    fontSize: theme.font(12),
    lineHeight: theme.font(18),
  },
  editActions: {
    flexDirection: "row",
    justifyContent: "flex-end",
    gap: spacing.sm,
  },
  editTextButton: {
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.xs,
    borderRadius: radius.sm,
  },
  editTextButtonLabel: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
  },
  save: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.xs,
    borderRadius: radius.sm,
    backgroundColor: theme.colors.accent,
  },
  saveLabel: {
    color: theme.colors.surface,
    fontSize: theme.font(12),
    fontWeight: "600",
  },
}));
