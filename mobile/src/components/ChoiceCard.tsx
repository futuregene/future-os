import type { ChoiceCardMode, ChoiceCardNode, ChoiceCardOption } from "@future-os/markdown";
import { ListChecks } from "lucide-react-native";
import { createContext, useContext } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { layout, radius, spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";

/**
 * What a card may do with the draft's answer: read the picked rows, and write a
 * pick back.
 *
 * Provided by the reply that owns the thread's **last** card (see `TimelineCard`)
 * and read here, so the plain block renderers in `MarkdownText` do not have to
 * carry a prop for it — the same reason `#` references read their opener from a
 * context.
 */
export interface ChoiceCardAnswer {
  /** A row of `card` was picked. The reply turns it into the draft's answer. */
  onPick: (card: ChoiceCardNode, optionId: string) => void;
  /** The draft answer's picked option ids. */
  selectedIds: string[];
  /**
   * The question the draft's answer belongs to. Rows are marked only on the card
   * that asked it: one thread can hold several cards, and an old answer must not
   * light up a question it never answered.
   */
  title: string;
}

/**
 * The answer surface a reply offers its own card.
 *
 * Exported because `MarkdownText` reads it to decide *which* card in a body is
 * answerable: the document's last one. Reading it (rather than a prop) is what
 * `#` references do too — the block renderers are plain functions, deep inside
 * the body, and cannot be handed a prop per card.
 */
export const ChoiceCardAnswerContext = createContext<ChoiceCardAnswer | undefined>(undefined);

export const ChoiceCardAnswerProvider = ChoiceCardAnswerContext.Provider;

/**
 * A `futureos-choice` block: the reply's closing question, drawn as a card.
 *
 * The phone's card (design `docs/design/choice-cards.zh-CN.md` §5.7) is the
 * desktop's grouped-row card at a narrower measure: full width, one row per
 * option carrying the line on what picking it changes, and — unlike §5.7's v1 —
 * exactly **one** cue for the picked row. v1 tinted the row, outlined it and put
 * a white tick on a blue disc, which at 390pt read as "done" rather than
 * "picked"; the mark is now a permanent ring that just thickens and takes a dot
 * (§5.7.1).
 *
 * `live` says whether this is the thread's answerable card. Every other card —
 * an earlier reply's, one nested in a quote, the same fence in a file preview —
 * is read-only: its question has been answered, and letting it change would make
 * "which pick am I editing" unanswerable (§2).
 */
export function ChoiceCard({ live, node }: { live: boolean; node: ChoiceCardNode }) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const answer = useContext(ChoiceCardAnswerContext);
  const picked = live && answer?.title === node.title ? answer.selectedIds : [];
  const multi = node.mode === "multi";
  const onPick = live ? answer?.onPick : undefined;
  return (
    <View style={styles.card} testID="choice-card">
      <View style={styles.head}>
        <ListChecks color={colors.accent} size={15} />
        <Text accessibilityRole="header" style={styles.title}>{node.title}</Text>
        <Text style={styles.mode}>
          {multi ? t("choice.mode.multi") : t("choice.mode.single")}
        </Text>
      </View>
      <Text style={styles.hint}>
        {live
          ? node.hint || (multi ? t("choice.hint.multi") : t("choice.hint.single"))
          : t("choice.hint.readonly")}
      </Text>
      <View
        accessibilityLabel={multi ? undefined : node.title}
        accessibilityRole={multi ? undefined : "radiogroup"}
        style={styles.rows}
      >
        {node.options.map(option => (
          <ChoiceOptionRow
            key={option.id}
            mode={node.mode}
            onPick={onPick ? () => onPick(node, option.id) : undefined}
            option={option}
            picked={picked.includes(option.id)}
            testID={`choice-option-${option.id}`}
          />
        ))}
      </View>
      {live && picked.length > 0
        ? (
            <Text style={styles.foot} testID="choice-card-footer">
              {t("choice.foot.queued")}
            </Text>
          )
        : null}
    </View>
  );
}

/**
 * One option row, shared by the card and the chip's bottom sheet so both answer
 * the same 44pt touch target (§5.7).
 *
 * The mark is a ring that is always drawn — empty until picked, then thicker with
 * a dot inside — so a row does not jump as it becomes the answer, and the control
 * reads as a radio/checkbox rather than as a badge.
 *
 * `onPick` absent means read-only: the row keeps its mark (a past card still says
 * what was picked there) but offers no press and tells a screen reader it is
 * disabled, because a row that looks tappable and answers nothing is worse than
 * an inert one.
 */
export function ChoiceOptionRow({
  mode,
  onPick,
  option,
  picked,
  testID,
}: {
  mode: ChoiceCardMode;
  onPick?: () => void;
  option: ChoiceCardOption;
  picked: boolean;
  testID?: string;
}) {
  const styles = useStyles();
  return (
    <Pressable
      accessibilityLabel={option.description ? `${option.label}，${option.description}` : option.label}
      accessibilityRole={mode === "multi" ? "checkbox" : "radio"}
      /**
       * `aria-checked` rather than `accessibilityState={{ checked }}`: the phone
       * draws this on react-native-web as well as on a device, and RNW forwards
       * the `aria-*` prop (RN maps it back to `accessibilityState.checked` on
       * native) while it drops `accessibilityState` entirely. One spelling, so a
       * screen reader and a DOM assertion read the same truth.
       */
      aria-checked={picked}
      disabled={!onPick}
      onPress={onPick}
      style={({ pressed }) => [
        styles.row,
        picked ? styles.rowPicked : null,
        pressed && onPick ? styles.rowPressed : null,
      ]}
      testID={testID}
    >
      <View style={[styles.ring, picked ? styles.ringPicked : null]}>
        {picked ? <View style={styles.dot} /> : null}
      </View>
      <View style={styles.rowCopy}>
        <Text style={[styles.rowLabel, picked ? styles.rowLabelPicked : null]}>
          {option.label}
        </Text>
        {option.description
          ? (
              <Text style={[styles.rowDescription, picked ? styles.rowDescriptionPicked : null]}>
                {option.description}
              </Text>
            )
          : null}
      </View>
    </Pressable>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  card: {
    width: "100%",
    borderWidth: 1,
    borderColor: theme.colors.line,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surface,
    // A card sits in the reply, not above the composer, so its elevation stays a
    // hairline: the approval card is the floating material, this one is content.
    shadowColor: theme.colors.inkStrong,
    shadowOpacity: 0.05,
    shadowRadius: 2,
    shadowOffset: { width: 0, height: 1 },
    elevation: 1,
  },
  head: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingTop: 10,
    paddingHorizontal: spacing.md,
    paddingBottom: 6,
  },
  title: {
    flex: 1,
    minWidth: 0,
    color: theme.colors.ink,
    fontSize: theme.font(13),
    fontWeight: "600",
  },
  mode: { color: theme.colors.inkMuted, fontSize: theme.font(11) },
  hint: {
    paddingHorizontal: spacing.md,
    paddingBottom: spacing.sm,
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
    lineHeight: theme.leading(17),
  },
  rows: { gap: 2, paddingHorizontal: 6, paddingBottom: spacing.sm },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 10,
    minHeight: layout.touchTarget,
    paddingHorizontal: spacing.md,
    paddingVertical: 10,
    borderRadius: 10,
  },
  rowPicked: { backgroundColor: theme.colors.accentSoft },
  rowPressed: { opacity: 0.72 },
  ring: {
    width: 18,
    height: 18,
    marginTop: 1,
    flexShrink: 0,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.pill,
    borderWidth: 1.5,
    borderColor: theme.colors.line,
  },
  // Only the stroke thickens and turns: the box keeps its size, so the row does
  // not shift as it becomes the answer.
  ringPicked: { borderWidth: 2, borderColor: theme.colors.accent },
  dot: {
    width: 8,
    height: 8,
    borderRadius: radius.pill,
    backgroundColor: theme.colors.accent,
  },
  rowCopy: { flex: 1, minWidth: 0, gap: 1 },
  rowLabel: {
    color: theme.colors.ink,
    fontSize: theme.font(14),
    lineHeight: theme.leading(20),
  },
  rowLabelPicked: { fontWeight: "500" },
  rowDescription: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
    lineHeight: theme.leading(17),
  },
  rowDescriptionPicked: { color: theme.colors.inkSoft },
  foot: {
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderTopWidth: 1,
    borderTopColor: theme.colors.lineSoft,
    backgroundColor: theme.colors.surfaceSubtle,
    color: theme.colors.inkSoft,
    fontSize: theme.font(11),
    lineHeight: theme.leading(16),
  },
}));
