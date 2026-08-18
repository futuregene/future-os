import {
  formatJsonForPreview,
  rawJsonLines,
  tokenizeJsonLine,
} from "@future-os/json-preview";
import { useMemo } from "react";
import { FlatList, StyleSheet, Text, View } from "react-native";
import { colors, spacing } from "../theme/tokens";

export { formatJsonForPreview, tokenizeJsonLine } from "@future-os/json-preview";

interface JsonPreviewProps {
  text: string;
  sourceTruncated: boolean;
  truncatedMessage: string;
  invalidMessage(detail: string): string;
  tooComplexMessage: string;
}

export function JsonPreview({
  text,
  sourceTruncated,
  truncatedMessage,
  invalidMessage,
  tooComplexMessage,
}: JsonPreviewProps) {
  const validationError = useMemo(() => {
    if (sourceTruncated) return null;
    try {
      JSON.parse(text);
      return null;
    } catch (error) {
      return error instanceof Error ? error.message : String(error);
    }
  }, [sourceTruncated, text]);
  const formatted = useMemo(
    () => (validationError ? rawJsonLines(text) : formatJsonForPreview(text)),
    [text, validationError],
  );

  return (
    <FlatList
      contentContainerStyle={styles.content}
      data={formatted.lines}
      initialNumToRender={40}
      keyExtractor={(_, index) => String(index)}
      ListHeaderComponent={
        sourceTruncated || validationError || formatted.limited ? (
          <View style={styles.notices}>
            {sourceTruncated ? <Text style={styles.notice}>{truncatedMessage}</Text> : null}
            {validationError ? (
              <Text style={styles.error}>{invalidMessage(validationError)}</Text>
            ) : null}
            {formatted.limited ? <Text style={styles.notice}>{tooComplexMessage}</Text> : null}
          </View>
        ) : null
      }
      maxToRenderPerBatch={60}
      removeClippedSubviews
      renderItem={({ item, index }) => (
        <View style={styles.row}>
          <Text style={styles.lineNumber}>{index + 1}</Text>
          <Text selectable style={styles.code}>
            {tokenizeJsonLine(item).map((token, tokenIndex) => (
              <Text key={tokenIndex} style={styles[token.kind]}>
                {token.text}
              </Text>
            ))}
          </Text>
        </View>
      )}
      updateCellsBatchingPeriod={25}
      windowSize={9}
    />
  );
}

const styles = StyleSheet.create({
  content: { padding: spacing.md, paddingBottom: spacing.xl },
  notices: { gap: spacing.sm, marginBottom: spacing.md },
  notice: { color: colors.inkMuted, fontSize: 13, lineHeight: 19 },
  error: { color: colors.danger, fontSize: 13, lineHeight: 19 },
  row: { flexDirection: "row", alignItems: "flex-start", minHeight: 21 },
  lineNumber: {
    width: 44,
    paddingRight: spacing.sm,
    color: colors.inkSoft,
    fontFamily: "monospace",
    fontSize: 12,
    lineHeight: 21,
    textAlign: "right",
  },
  code: { flex: 1, color: colors.ink, fontFamily: "monospace", fontSize: 13, lineHeight: 21 },
  plain: { color: colors.ink },
  key: { color: "#1d4ed8" },
  string: { color: "#047857" },
  number: { color: "#b45309" },
  literal: { color: "#7c3aed" },
});
