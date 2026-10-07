import { useTranslation } from "react-i18next";
import { MessageCircle, X } from "lucide-react-native";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import type { ShareSessionGroup } from "../../../share/shareSessionGroups";
import { colors, layout, radius, spacing } from "../../../theme/tokens";
/** One conversation row: title only (one line), plus its workspace heading. */
export const SESSION_ROW_HEIGHT = 44;
const ROW_PITCH = SESSION_ROW_HEIGHT + 1;
const GROUP_HEADING_HEIGHT = 28;
const HEADER_HEIGHT = 44;
const PANEL_BORDER = 1;
/** The menu shows this many conversations without scrolling. */
export const VISIBLE_SESSION_ROWS = 4;

/**
 * How tall the `#` menu above the composer is: as tall as the conversations and
 * workspace headings it has to show, and no taller than the room the keyboard
 * leaves. Same contract as `skillPickerHeight` — the caller uses it for the
 * panel; the conversation keeps the rest.
 */
export function sessionPickerHeight(
  viewportHeight: number,
  keyboardHeight: number,
  rowCount: number,
  groupCount: number,
) {
  const wanted = 2 * PANEL_BORDER + HEADER_HEIGHT
    + Math.max(1, Math.min(rowCount, VISIBLE_SESSION_ROWS)) * ROW_PITCH
    + groupCount * GROUP_HEADING_HEIGHT;
  const space = viewportHeight - keyboardHeight - 100;
  return Math.max(120, Math.min(wanted, space));
}

/**
 * The `#` conversation picker, docked above the composer like the `/` skill
 * picker. Conversations are grouped by workspace (the session list's own
 * ordering, via `sessionMentionGroups`), and the typed query — not a field of
 * its own — is what narrows them, so this stays a pure view over `groups`.
 */
export function SessionPicker({ groups, onSelect, onClose, maxHeight }: {
  groups: ShareSessionGroup[];
  onSelect: (session: { sessionId: string; title: string }) => void;
  onClose: () => void;
  maxHeight: number;
}) {
  const { t } = useTranslation();
  const total = groups.reduce((count, group) => count + group.sessions.length, 0);
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
        {total === 0
          ? <Text style={styles.hint}>{t("sessions.noResults")}</Text>
          : groups.map(group => (
            <View key={group.workspace?.id ?? "__chats"}>
              <Text numberOfLines={1} style={styles.heading}>
                {group.workspace
                  ? (group.workspace.name.trim() || t("sessions.workspace"))
                  : t("sessions.conversations")}
              </Text>
              {group.sessions.map(session => (
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
                </Pressable>
              ))}
            </View>
          ))}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  panel: { marginBottom: spacing.xs, borderWidth: 1, borderColor: colors.line, borderRadius: radius.lg,
    backgroundColor: colors.surface, overflow: "hidden" },
  header: { height: HEADER_HEIGHT, flexDirection: "row", alignItems: "center", paddingLeft: spacing.lg },
  title: { flex: 1, fontSize: 13, fontWeight: "600", color: colors.inkSoft },
  close: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center" },
  heading: { height: GROUP_HEADING_HEIGHT, paddingTop: spacing.sm, paddingLeft: spacing.lg,
    fontSize: 12, fontWeight: "600", color: colors.inkMuted },
  row: { minHeight: SESSION_ROW_HEIGHT, flexDirection: "row", alignItems: "center", gap: spacing.sm,
    borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.lineSoft,
    paddingLeft: spacing.lg, paddingRight: spacing.md },
  pressed: { backgroundColor: colors.surfaceSubtle },
  name: { flex: 1, minWidth: 0, fontSize: 14, fontWeight: "600", color: colors.ink },
  hint: { fontSize: 13, color: colors.inkSoft, padding: spacing.md, flexShrink: 1 },
});
