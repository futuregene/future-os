import { useTranslation } from "react-i18next";
import { MessageCircle, X } from "lucide-react-native";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import type { SessionMentionOption } from "../sessionMention";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";
/** One conversation row: title and the workspace it lives in, on one line. */
export const SESSION_ROW_HEIGHT = 44;
const ROW_PITCH = SESSION_ROW_HEIGHT + 1;
const HEADER_HEIGHT = 44;
const PANEL_BORDER = 1;
/** The menu shows this many conversations without scrolling. */
export const VISIBLE_SESSION_ROWS = 4;

/**
 * How tall the `#` menu above the composer is: as tall as the conversations it
 * has to show, and no taller than the room the keyboard leaves. Same contract as
 * `skillPickerHeight` — the caller uses it for the panel; the conversation keeps
 * the rest.
 *
 * There is no heading allowance: the rows are one recency-ordered list (see
 * `sessionMentionOptions`), so every line of the panel is a conversation.
 */
export function sessionPickerHeight(
  viewportHeight: number,
  keyboardHeight: number,
  rowCount: number,
) {
  const wanted = 2 * PANEL_BORDER + HEADER_HEIGHT
    + Math.max(1, Math.min(rowCount, VISIBLE_SESSION_ROWS)) * ROW_PITCH;
  const space = viewportHeight - keyboardHeight - 100;
  return Math.max(120, Math.min(wanted, space));
}

/**
 * The `#` conversation picker, docked above the composer like the `/` skill
 * picker. One list, newest activity first (see `sessionMentionOptions`), each
 * row naming the workspace it lives in; the typed query — not a field of its own
 * — is what narrows it, so this stays a pure view over `sessions`.
 */
export function SessionPicker({ sessions, onSelect, onClose, maxHeight }: {
  sessions: SessionMentionOption[];
  onSelect: (session: { sessionId: string; title: string }) => void;
  onClose: () => void;
  maxHeight: number;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  return (
    <View style={[styles.panel, { maxHeight }]}>
      <View style={styles.header}>
        <Text accessibilityRole="header" style={styles.title}>{t("sessions.reference")}</Text>
        <Pressable accessibilityRole="button" accessibilityLabel={t("skills.close")}
          onPress={onClose} style={styles.close}>
          <X color={colors.inkSoft} size={18} />
        </Pressable>
      </View>
      <ScrollView keyboardShouldPersistTaps="always" bounces={false}>
        {sessions.length === 0
          ? <Text style={styles.hint}>{t("sessions.noResults")}</Text>
          : sessions.map(session => (
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={session.title.trim() || t("sessions.unnamed")}
              accessibilityHint={t("sessions.referenceHint")}
              key={session.sessionId}
              onPress={() => onSelect({
                sessionId: session.sessionId,
                title: session.title.trim() || t("sessions.unnamed"),
              })}
              style={({ pressed }) => [styles.row, pressed && styles.pressed]}
            >
              <MessageCircle color={colors.inkSoft} size={18} />
              <Text numberOfLines={1} style={styles.name}>
                {session.title.trim() || t("sessions.unnamed")}
              </Text>
              {session.workspace
                ? (
                    <Text numberOfLines={1} style={styles.workspace}>
                      {session.workspace.name.trim() || t("sessions.workspace")}
                    </Text>
                  )
                : null}
            </Pressable>
          ))}
      </ScrollView>
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  panel: { marginBottom: spacing.xs, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.lg,
    backgroundColor: theme.colors.surface, overflow: "hidden" },
  header: { height: HEADER_HEIGHT, flexDirection: "row", alignItems: "center", paddingLeft: spacing.lg },
  title: { flex: 1, fontSize: theme.font(13), fontWeight: "600", color: theme.colors.inkSoft },
  close: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center" },
  row: { minHeight: SESSION_ROW_HEIGHT, flexDirection: "row", alignItems: "center", gap: spacing.sm,
    borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: theme.colors.lineSoft,
    paddingLeft: spacing.lg, paddingRight: spacing.md },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  name: { flex: 1, minWidth: 0, fontSize: theme.font(14), fontWeight: "600", color: theme.colors.ink },
  // The workspace name rides on the row (the list has no headings), muted and
  // capped so a long workspace name cannot squeeze the conversation's own title.
  workspace: { flexShrink: 0, maxWidth: "40%", fontSize: theme.font(12), color: theme.colors.inkMuted },
  hint: { fontSize: theme.font(13), color: theme.colors.inkSoft, padding: spacing.md, flexShrink: 1 },
}));
