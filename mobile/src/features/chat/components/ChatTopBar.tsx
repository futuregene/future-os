import { ArrowLeft, Ellipsis } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { PeerIconGlyph } from "../../../remote/PeerIconGlyph";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";

export function ChatTopBar({
  title,
  contextLabel,
  draft,
  machine,
  backLabel,
  menuLabel,
  menuOpen,
  onBack,
  onMenu,
}: {
  title: string;
  contextLabel: string;
  draft: boolean;
  /**
   * The desktop this conversation is on: its icon id and the user's name for it.
   *
   * The phone has no conversations of its own — every one of these lives on a
   * paired desktop — so the bar says which, on the line that already answers
   * "where is this" (the workspace, or the chat section). One line, as that line
   * is: the machine qualifies the conversation, it does not get a row of its own.
   */
  machine?: { icon: string | undefined; label: string };
  backLabel: string;
  /**
   * Opens the conversation's own menu: the session's files, its token/amount
   * breakdown, and — when the workspace is a Git repository — its history.
   *
   * One control rather than one per action: the bar belongs to the conversation
   * and every action in it is occasional, so a row of icons spent the title's
   * width on things the reader opens once a session. What is inside is the
   * caller's: this bar draws a button, not a list.
   */
  menuLabel: string;
  menuOpen: boolean;
  onBack: () => void;
  onMenu: () => void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  return (
    <View style={styles.topbar}>
      <Pressable
        accessibilityLabel={backLabel}
        accessibilityRole="button"
        onPress={onBack}
        style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
      >
        <ArrowLeft color={colors.ink} size={22} />
      </Pressable>
      <View style={styles.titleWrap}>
        <Text numberOfLines={1} style={styles.title}>
          {title}
        </Text>
        <View style={styles.contextLine}>
          {machine ? (
            <>
              {/* Named for the screen reader and for the tests: the glyph alone is
                  not a name, and the element is the only place a reader can learn
                  which computer the conversation they are in lives on. The mark
                  itself is drawn from the shared icon set, so it is the same
                  picture the session list and the machine's own page show. */}
              <View accessibilityLabel={machine.label} style={styles.machine} testID="chat-machine">
                <PeerIconGlyph id={machine.icon} size={12} />
                <Text numberOfLines={1} style={styles.machineLabel}>{machine.label}</Text>
              </View>
              <Text accessibilityElementsHidden importantForAccessibility="no" style={styles.contextSeparator}>
                ·
              </Text>
            </>
          ) : null}
          {/* Two lines, like the device trigger on the session list: the machine
              name caps at 70% of this row, so on a phone the workspace half has
              ~80pt and a single line cut 「工作区 · 多巴胺与决策」 to 「多巴…」 —
              the one fact that says *which* workspace this conversation is in.
              The bar has the room (it is a 60pt minimum, and the line under the
              title was using about half of it), so the name wraps instead of
              disappearing. Beyond two lines it still ellipsizes: a workspace
              called by a paragraph is not what this line is for. */}
          <Text numberOfLines={2} style={styles.context} accessibilityLabel={contextLabel}>
            {contextLabel}
          </Text>
        </View>
      </View>
      {!draft && (
        <Pressable
          accessibilityLabel={menuLabel}
          accessibilityRole="button"
          accessibilityState={{ expanded: menuOpen }}
          onPress={onMenu}
          style={({ pressed }) => [styles.iconButton, (pressed || menuOpen) && styles.pressed]}
        >
          <Ellipsis color={menuOpen ? colors.accent : colors.ink} size={22} />
        </Pressable>
      )}
      {/* A draft has no session, so no action applies; the spacer keeps the
          centered title from jumping sideways when the first message is sent and
          the button appears. */}
      {draft && <View style={styles.iconButton} />}
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  topbar: {
    minHeight: 60,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: spacing.md,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.lineSoft,
    backgroundColor: theme.colors.surface,
  },
  iconButton: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  titleWrap: { flex: 1, minWidth: 0, alignItems: "center" },
  // One line for "where this conversation is": machine, then workspace or chat
  // section. The machine keeps its width up to a cap and the rest truncates —
  // whose conversation this is matters more than repeating the workspace the
  // reader just came from, and a line truncating *both* halves reads as a row
  // that did not fit.
  contextLine: { flexDirection: "row", alignItems: "center", justifyContent: "center", maxWidth: "100%", marginTop: 2 },
  machine: { flexDirection: "row", alignItems: "center", gap: 3, flexShrink: 0, maxWidth: "70%" },
  machineLabel: { color: theme.colors.inkMuted, fontSize: theme.font(12), flexShrink: 1 },
  contextSeparator: { color: theme.colors.inkMuted, fontSize: theme.font(12), marginHorizontal: 4 },
  context: { color: theme.colors.inkSoft, fontSize: theme.font(12), flexShrink: 1 },
  title: { color: theme.colors.inkStrong, fontSize: theme.font(16), fontWeight: "700", maxWidth: "90%" },
}));
