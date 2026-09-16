import { memo, useMemo } from "react";
import { Platform, ScrollView, StyleSheet, Text, useWindowDimensions } from "react-native";
import { SvgXml } from "react-native-svg";
import { chatTypography, colors, spacing } from "../theme/tokens";
import { renderMathSvg } from "./mathSvg";

export const MathFormula = memo(function MathFormula({ code, inline = false }: {
  code: string;
  inline?: boolean;
}) {
  const { fontScale } = useWindowDimensions();
  const formula = useMemo(() => renderMathSvg(code, !inline), [code, inline]);
  if (!formula) return <Text selectable style={styles.source}>{code}</Text>;
  const fontSize = chatTypography.fontSize * fontScale;
  const svg = <SvgXml
    xml={formula.xml}
    width={formula.width * fontSize}
    height={formula.height * fontSize}
    color={colors.ink}
    accessibilityRole="image"
    accessibilityLabel={code}
  />;
  // Inline SVG is an inline native text attachment; display equations scroll
  // rather than clipping or shrinking their glyphs on narrow phone screens.
  return inline ? svg : <ScrollView horizontal contentContainerStyle={styles.equation}>{svg}</ScrollView>;
});

const styles = StyleSheet.create({
  source: { color: colors.ink, fontSize: chatTypography.fontSize, fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace" },
  equation: { paddingVertical: spacing.xs },
});
