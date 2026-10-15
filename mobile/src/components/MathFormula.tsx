import { memo, useContext, useMemo } from "react";
import { Platform, ScrollView, StyleSheet, Text, useWindowDimensions } from "react-native";
import { SvgXml } from "react-native-svg";
import { spacing } from "../theme/tokens";
import { createThemedStyles, useTheme } from "../theme/ThemeContext";
import { renderMathSvg } from "./mathSvg";
import { MarkdownToneContext } from "./markdownTone";

export const MathFormula = memo(function MathFormula({ code, inline = false }: {
  code: string;
  inline?: boolean;
}) {
  const styles = useStyles();
  const { chat, colors } = useTheme();
  // A formula is tinted, not just positioned: on the user's accent-filled bubble
  // the ink the SVG is drawn in would be unreadable, so it takes that bubble's
  // contrast colour instead (see `MarkdownTone`).
  const onAccent = useContext(MarkdownToneContext) === "onAccent";
  const { fontScale } = useWindowDimensions();
  const formula = useMemo(() => renderMathSvg(code, !inline), [code, inline]);
  if (!formula) return <Text selectable style={[styles.source, onAccent && { color: colors.surface }]}>{code}</Text>;
  const fontSize = chat.fontSize * fontScale;
  const svg = <SvgXml
    xml={formula.xml}
    width={formula.width * fontSize}
    height={formula.height * fontSize}
    color={onAccent ? colors.surface : colors.ink}
    accessibilityRole="image"
    accessibilityLabel={code}
  />;
  // Inline SVG is an inline native text attachment; display equations scroll
  // rather than clipping or shrinking their glyphs on narrow phone screens.
  return inline ? svg : <ScrollView horizontal contentContainerStyle={styles.equation}>{svg}</ScrollView>;
});

const useStyles = createThemedStyles(theme => StyleSheet.create({
  source: { color: theme.colors.ink, fontSize: theme.chat.fontSize, fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace" },
  equation: { paddingVertical: spacing.xs },
}));
