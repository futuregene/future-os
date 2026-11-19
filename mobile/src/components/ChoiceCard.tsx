import type { ChoiceCardMode, ChoiceCardNode, ChoiceCardOption } from "@future-os/markdown";
import { CHOICE_CONTEXT_VISIBLE_LINES, choiceContextFolds } from "@future-os/markdown";
import { Check, ChevronDown, ChevronUp, ListChecks, X } from "lucide-react-native";
import { createContext, useContext, useState } from "react";
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
  /**
   * Close the question without answering it (§5.9.4): nothing is sent to the
   * model, the draft's answer is dropped, and the desktop stops flagging this
   * conversation as waiting on a pick. Absent where nothing owns that mark.
   */
  onDismiss?: () => void;
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
 * Whether the paired desktop draws `futureos-choice` cards.
 *
 * The phone mirrors the desktop's setting rather than holding its own: a card is
 * drawn by whichever surface the reader is on, and the desktop's switch is the
 * one that also decides whether the agent is taught the fence (`ChatScreen`
 * passes `desktopSettings.choiceCards` down here). The renderer's own default
 * stays `true` so a card is only hidden on a real "off": the switch ships off in
 * the persisted settings, and a surface that reads them gets `false` from there.
 */
export const ChoiceCardsEnabledContext = createContext(true);

export const ChoiceCardsEnabledProvider = ChoiceCardsEnabledContext.Provider;

/**
 * Width units one line of a phone card's context holds: 350pt less the card's
 * 16pt padding, at 13px, is about 24 CJK characters — 48 units by
 * `estimateContextLines`'s accounting. The phone's column is 2.5x narrower than
 * the desktop's, so the same context that fits a desktop card in four lines
 * already folds here, which is the point of passing it in.
 *
 * The visible line count is the shared `CHOICE_CONTEXT_VISIBLE_LINES`, so
 * `numberOfLines` and the fold rule cannot drift apart.
 */
const CONTEXT_UNITS_PER_LINE = 48;

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
  const onDismiss = live ? answer?.onDismiss : undefined;
  // No subtitle of its own any more (§5.1): the mode label and the mark shape
  // already say single or multi, and "the answer lands in the input" described
  // something the composer shows by itself. The model's own line still shows;
  // a past card adds the notice underneath.
  const guidance = node.hint;
  // Nothing bounds what a model writes, and a card carrying twenty lines of
  // context has become the reply it was meant to replace. Above the visible
  // lines the tail folds behind a button. Below it, no control at all.
  const [contextExpanded, setContextExpanded] = useState(false);
  const contextFolds = node.context ? choiceContextFolds(node.context, CONTEXT_UNITS_PER_LINE) : false;
  const contextFolded = contextFolds && !contextExpanded;
  return (
    <View style={styles.card} testID="choice-card">
      <View style={styles.head}>
        <ListChecks color={colors.accent} size={15} />
        <Text accessibilityRole="header" style={styles.title}>{node.title}</Text>
        <Text style={styles.mode}>
          {multi ? t("choice.mode.multi") : t("choice.mode.single")}
        </Text>
      </View>
      {node.context
        ? (
            <>
              <Text
                numberOfLines={contextFolded ? CHOICE_CONTEXT_VISIBLE_LINES : undefined}
                style={styles.context}
                testID="choice-card-context"
              >
                {node.context}
              </Text>
              {contextFolds
                ? (
                    <Pressable
                      accessibilityRole="button"
                      /**
                       * `aria-expanded` for the same reason the rows spell
                       * `aria-checked`: react-native-web forwards `aria-*` and
                       * drops `accessibilityState`, so the button's state would
                       * vanish on the web build while the native tree kept it.
                       */
                      aria-expanded={contextExpanded}
                      hitSlop={8}
                      onPress={() => setContextExpanded(expanded => !expanded)}
                      style={styles.contextToggle}
                      testID="choice-card-context-toggle"
                    >
                      {contextExpanded
                        ? <ChevronUp color={colors.inkMuted} size={13} />
                        : <ChevronDown color={colors.inkMuted} size={13} />}
                      <Text style={styles.contextToggleText}>
                        {contextExpanded ? t("choice.context.collapse") : t("choice.context.expand")}
                      </Text>
                    </Pressable>
                  )
                : null}
            </>
          )
        : null}
      {guidance
        ? <Text style={styles.hint} testID="choice-card-hint">{guidance}</Text>
        : null}
      {live
        ? null
        : (
            <Text style={styles.readonly} testID="choice-card-readonly">
              {t("choice.hint.readonly")}
            </Text>
          )}
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
      {/*
        The third answer: none. Drawn only on the answerable card (a past
        question's exit is long gone) and only where something owns the mark it
        clears — see §5.9.4. It sits outside the option group, because it is not
        a choice and a radiogroup must not contain an action that answers nothing.
      */}
      {live && onDismiss
        ? (
            <Pressable
              accessibilityRole="button"
              hitSlop={8}
              onPress={onDismiss}
              style={({ pressed }) => [styles.dismiss, pressed ? styles.rowPressed : null]}
              testID="choice-card-dismiss"
            >
              <X color={colors.inkMuted} size={16} />
              <View style={styles.dismissCopy}>
                <Text style={styles.dismissLabel}>{t("choice.dismiss")}</Text>
                <Text style={styles.dismissNote}>{t("choice.dismissNote")}</Text>
              </View>
            </Pressable>
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
  const { colors } = useTheme();
  const multi = mode === "multi";
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
      {/*
        Shape carries the mode: a radio ring for a mutually exclusive pick, a
        square checkbox for rows that may be picked together. The mark keeps its
        size either way, so a row does not jump as it becomes the answer.
      */}
      <View style={[styles.mark, multi ? styles.box : styles.ring, picked ? styles.markPicked : null]}>
        {picked
          ? multi
            ? <Check color={colors.accent} size={12} strokeWidth={3} />
            : <View style={styles.dot} />
          : null}
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
  // The situation, above the instruction: the reader answers from the card, not
  // from a scroll back up the reply. A step louder than `hint` — it is content,
  // the line below it is chrome — and pre-line so repeated `context:` lines keep
  // their breaks.
  context: {
    paddingHorizontal: spacing.md,
    paddingBottom: 6,
    color: theme.colors.inkSoft,
    fontSize: theme.font(13),
    lineHeight: theme.leading(19),
  },
  // "展开"/"收起": a row of its own under the context, 44pt tall like every other
  // touch target on this card (hitSlop covers the rest), quiet enough to read as
  // chrome rather than a fifth option.
  contextToggle: {
    flexDirection: "row",
    alignItems: "center",
    gap: 2,
    marginTop: -2,
    paddingHorizontal: spacing.md,
    paddingBottom: 6,
    minHeight: layout.touchTarget,
  },
  contextToggleText: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
    lineHeight: theme.leading(17),
  },
  hint: {
    paddingHorizontal: spacing.md,
    paddingBottom: 6,
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
    lineHeight: theme.leading(17),
  },
  // A past card says why it cannot be picked, after the model's own hint rather
  // than in place of it (§2, revised 2026-11-18).
  readonly: {
    paddingHorizontal: spacing.md,
    paddingBottom: spacing.sm,
    color: theme.colors.inkMuted,
    fontSize: theme.font(11),
    lineHeight: theme.leading(16),
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
  mark: {
    width: 18,
    height: 18,
    marginTop: 1,
    flexShrink: 0,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1.5,
    borderColor: theme.colors.line,
  },
  ring: { borderRadius: radius.pill },
  box: { borderRadius: 4 },
  // Only the stroke thickens and turns: the mark keeps its size, so the row does
  // not shift as it becomes the answer.
  markPicked: { borderWidth: 2, borderColor: theme.colors.accent },
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
  // "都不做": a row of its own below the options, 44pt like every other touch
  // target here (hitSlop covers the rest), quiet enough to read as chrome rather
  // than as a fifth option.
  dismiss: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 10,
    minHeight: layout.touchTarget,
    paddingHorizontal: spacing.md,
    paddingVertical: 10,
    borderTopWidth: 1,
    borderTopColor: theme.colors.lineSoft,
  },
  dismissCopy: { flex: 1, minWidth: 0, gap: 1 },
  dismissLabel: {
    color: theme.colors.inkSoft,
    fontSize: theme.font(14),
    lineHeight: theme.leading(20),
  },
  dismissNote: {
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
    lineHeight: theme.leading(17),
  },
}));
