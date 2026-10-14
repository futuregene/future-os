import { ArrowLeft, FolderOpen, ReceiptText } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";

export function ChatTopBar({
  title,
  contextLabel,
  draft,
  machine,
  backLabel,
  usageLabel,
  onBack,
  onUsage,
  filesLabel,
  filesOpen,
  onFiles,
}: {
  title: string;
  contextLabel: string;
  draft: boolean;
  /**
   * The desktop this conversation is on: its glyph and the user's name for it.
   *
   * The phone has no conversations of its own — every one of these lives on a
   * paired desktop — so the bar says which, on the line that already answers
   * "where is this" (the workspace, or the chat section). One line, as that line
   * is: the machine qualifies the conversation, it does not get a row of its own.
   */
  machine?: { glyph: string; label: string };
  backLabel: string;
  /**
   * Opens the conversation's token/amount breakdown. An icon rather than the
   * amount: the bar belongs to the conversation, and a number that changes
   * every turn competes with the title for attention. Renaming is not here
   * either — the session list already owns that, so repeating it in the
   * conversation only lengthens the row.
   */
  usageLabel: string;
  onBack: () => void;
  onUsage: () => void;
  filesLabel: string;
  filesOpen: boolean;
  onFiles: () => void;
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
                  which computer the conversation they are in lives on. */}
              <Text
                accessibilityLabel={machine.label}
                numberOfLines={1}
                style={styles.machine}
                testID="chat-machine"
              >
                {`${machine.glyph} ${machine.label}`}
              </Text>
              <Text accessibilityElementsHidden importantForAccessibility="no" style={styles.contextSeparator}>
                ·
              </Text>
            </>
          ) : null}
          <Text numberOfLines={1} style={styles.context} accessibilityLabel={contextLabel}>
            {contextLabel}
          </Text>
        </View>
      </View>
      {!draft && (
        <Pressable
          accessibilityLabel={filesLabel}
          accessibilityRole="button"
          accessibilityState={{ expanded: filesOpen }}
          onPress={onFiles}
          style={({ pressed }) => [styles.iconButton, (pressed || filesOpen) && styles.pressed]}
        >
          <FolderOpen color={filesOpen ? colors.accent : colors.ink} size={20} />
        </Pressable>
      )}
      {!draft && (
        <Pressable
          accessibilityLabel={usageLabel}
          accessibilityRole="button"
          onPress={onUsage}
          style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
        >
          <ReceiptText color={colors.ink} size={20} />
        </Pressable>
      )}
      {/* A draft has no session, so neither icon applies; the two spacers keep
          the centered title from jumping sideways when the first message is
          sent and they appear. */}
      {draft && <View style={styles.iconButton} />}
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
  machine: { color: theme.colors.inkMuted, fontSize: theme.font(12), flexShrink: 0, maxWidth: "70%" },
  contextSeparator: { color: theme.colors.inkMuted, fontSize: theme.font(12), marginHorizontal: 4 },
  context: { color: theme.colors.inkSoft, fontSize: theme.font(12), flexShrink: 1 },
  title: { color: theme.colors.inkStrong, fontSize: theme.font(16), fontWeight: "700", maxWidth: "90%" },
}));
