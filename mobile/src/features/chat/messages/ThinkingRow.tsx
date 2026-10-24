import { Brain, ChevronDown, ChevronUp } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { useTheme } from "../../../theme/ThemeContext";
import { useStyles } from "./messageStyles";
import { ROW_HIT_SLOP } from "./touchTargets";

/**
 * Inline thinking block inside a reply bubble. The mobile product decision
 * (audit D4) is that reasoning always renders collapsed — a muted one-line
 * label, expandable on tap — so long reasoning never floods the bubble or the
 * FlatList. The shared projection captures the full text; the collapse is a
 * render concern only. The label reads "thinking" while the run is streaming
 * (the block is still mid-reasoning) and "thought completed" once settled.
 *
 * The glyph matches the desktop's thinking activity glyph (Brain), so an
 * expanded reasoning row and a tool row are the same shape: icon, then words.
 * Like a tool row, its collapsed header starts at the reply's left edge and
 * carries no rule; its revealed text indents under the label the same way a
 * revealed target does — see ToolRow.
 */
export function ThinkingRow({ text, streaming, opened }: {
  text: string;
  streaming?: boolean;
  opened: boolean;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  // A lean feed carries no body at all: the row is then a status label, not a
  // disclosure. A chevron that opens an empty box promises content that never
  // arrives, so a bodyless row must not read as expandable.
  //
  // `opened` is the layout (a row inside a run sits on the open rail); the body
  // is the reader's own expansion. Only the disclosure affordance is gated on
  // having something to disclose — the two must not be conflated.
  const hasBody = text.trim().length > 0;
  const open = opened || expanded;
  const header = (
    <>
      <Brain color={colors.inkMuted} size={14} />
      <Text style={styles.inlineThinkingLabel}>
        {t(streaming ? "chat.thinking" : "chat.thoughtCompleted")}
      </Text>
      {hasBody &&
        (expanded ? (
          <ChevronUp color={colors.inkMuted} size={14} />
        ) : (
          <ChevronDown color={colors.inkMuted} size={14} />
        ))}
    </>
  );
  return (
    <View style={[styles.inlineThinking, !open && styles.railBlock]}>
      {hasBody ? (
        <Pressable
          accessibilityRole="button"
          hitSlop={ROW_HIT_SLOP}
          onPress={() => setExpanded(value => !value)}
          style={[styles.inlineThinkingHeader, !open && styles.railRow]}
        >
          {header}
        </Pressable>
      ) : (
        <View style={[styles.inlineThinkingHeader, !open && styles.railRow]}>{header}</View>
      )}
      {expanded && <Text style={styles.inlineThinkingText}>{text}</Text>}
    </View>
  );
}
