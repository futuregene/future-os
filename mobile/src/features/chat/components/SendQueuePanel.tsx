import { Check, Pencil, Play, Trash2 } from "lucide-react-native";
import { useState } from "react";
import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";

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
  /** How many files travel with it, when the sender knows. */
  attachmentCount?: number;
  /**
   * Parked by the session: it waited too long to run by itself and now starts
   * only when the reader says so. Absent means "runs at the next turn boundary".
   */
  parked?: boolean;
}

/**
 * The send queue, above the box that filled it.
 *
 * A queued message is not yet gone, so this is the one place to read it, fix a
 * typo or take it back before the run it waits for ends. It disappears the moment
 * the queue empties, because an empty panel would only take space.
 *
 * The rows are the conversation's, not this phone's: the same list the desktop's
 * own window shows, because both read it from the session that holds it. That is
 * why a removal is a cancel of a run id and an edit is "take it back, send the
 * new words" — there is no local record here to rename.
 */
export function SendQueuePanel({
  items,
  onRelease,
  onRemove,
  onUpdate,
}: {
  items: readonly SendQueuePanelItem[];
  /** Start a parked row now. Offered only when the session can park one. */
  onRelease?: (id: string) => void;
  onRemove: (id: string) => void;
  onUpdate: (id: string, content: string) => void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  // One row is edited at a time — the panel is a list of what is about to leave,
  // not a second composer.
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");

  if (items.length === 0)
    return null;

  function saveEditing(id: string) {
    onUpdate(id, editingText);
    setEditingId(null);
    setEditingText("");
  }

  return (
    <View accessibilityLabel={t("chat.queue.title")} style={styles.panel} testID="send-queue">
      <View style={styles.header}>
        <Text style={styles.title}>{t("chat.queue.title")}</Text>
        <Text style={styles.count}>{t("chat.queue.count", { count: items.length })}</Text>
        <View style={styles.headerSpacer} />
        <Text numberOfLines={1} style={styles.hint}>{t("chat.queue.hint")}</Text>
      </View>
      {items.map((item, index) => {
        const editing = editingId === item.id;
        return (
          <View key={item.id} style={styles.row} testID="send-queue-item">
            <Text style={styles.index}>{index + 1}</Text>
            {editing
              ? (
                  <View style={styles.editArea}>
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
                      {item.attachmentCount !== undefined && item.attachmentCount > 0 && (
                        <Text style={styles.attachments}>
                          {t("chat.queue.attachments", { count: item.attachmentCount })}
                        </Text>
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
    flexDirection: "row",
    alignItems: "baseline",
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
  attachments: {
    marginTop: 1,
    color: theme.colors.inkMuted,
    fontSize: theme.font(11),
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
