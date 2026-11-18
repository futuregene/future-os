import type { ChoiceAnswerEcho } from "@future-os/markdown";
import { ListChecks } from "lucide-react-native";
import { StyleSheet, Text, View, type StyleProp, type ViewStyle } from "react-native";
import { radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";

/**
 * The answer a `futureos-choice` card printed into a prompt, drawn as a tag.
 *
 * Sending writes the answer as plain text (`[接下来做哪一块？ → 把归因搬进页面]`,
 * design `docs/design/choice-cards.zh-CN.md` §7), so a prompt that picked one row
 * would otherwise read as a bracket sandwich inside the reader's own words. The
 * sentence is not rewritten here — the copy button still yields it — it is drawn
 * as what it is: a pick, one step in from the prose typed around it (the desktop
 * appends its pill to the end of the editor, so an answer picked after typing
 * lands at the end of a sentence).
 *
 * The bubble behind this is the accent fill, not the app surface, so the tag is
 * mixed from the same two colours `MarkdownText`'s `onAccent` tone uses: an
 * inset wash of the bubble's own ink (a dozen percent of it) and the strong ink
 * for the picked row. That keeps it legible in every theme without a colour of
 * its own — a tag that invented one would be the only element on the screen
 * that ignores the accent.
 */
/**
 * `style` carries the gap to the prompt below it: a bubble that holds only the
 * tag has nothing to separate it from, so the caller drops the margin there.
 */
export function ChoiceAnswerTags({ echoes, style }: { echoes: readonly ChoiceAnswerEcho[]; style?: StyleProp<ViewStyle> }) {
  const styles = useStyles();
  const { colors } = useTheme();
  return (
    <View style={[styles.tags, style]}>
      {echoes.map((echo, index) => (
        <View key={`${index}:${echo.question}`} style={styles.tag} testID="choice-answer-tag">
          <ListChecks color={colors.surface} size={13} />
          <Text style={styles.text}>
            <Text style={styles.question}>{echo.question}</Text>
            <Text style={styles.arrow}>{" → "}</Text>
            <Text style={styles.answer}>{echo.answer}</Text>
          </Text>
        </View>
      ))}
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  // The run of answers stacks: a bubble is narrow, and two tags side by side
  // would wrap mid-sentence.
  tags: { gap: spacing.xs },
  tag: {
    flexDirection: "row",
    alignItems: "flex-start",
    alignSelf: "flex-start",
    maxWidth: "100%",
    gap: spacing.xs,
    paddingHorizontal: 6,
    paddingVertical: 3,
    borderRadius: radius.sm,
    // The bubble's own ink, a wash of it: an inset, not a second surface.
    borderWidth: 1,
    borderColor: `${theme.colors.surface}33`,
    backgroundColor: `${theme.colors.surface}1f`,
  },
  text: { flexShrink: 1, color: theme.colors.surface, fontSize: theme.font(12), lineHeight: theme.leading(16) },
  question: { color: theme.colors.surface, opacity: 0.82 },
  arrow: { color: theme.colors.surface, opacity: 0.6 },
  answer: { color: theme.colors.surface, fontWeight: "600" },
}));
