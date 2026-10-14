import { formatJsonForPreview, rawJsonLines, tokenizeJsonLine } from "@future-os/json-preview";
import { useMemo } from "react";
import { FlatList, StyleSheet, Text, View } from "react-native";
import { spacing } from "../theme/tokens";
import { createThemedStyles } from "../theme/ThemeContext";

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
  const styles = useStyles();
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

const useStyles = createThemedStyles(theme => StyleSheet.create({
  content: { padding: spacing.md, paddingBottom: spacing.xl },
  notices: { gap: spacing.sm, marginBottom: spacing.md },
  notice: { color: theme.colors.inkMuted, fontSize: theme.font(13), lineHeight: theme.leading(19) },
  error: { color: theme.colors.danger, fontSize: theme.font(13), lineHeight: theme.leading(19) },
  row: { flexDirection: "row", alignItems: "flex-start", minHeight: 21 },
  lineNumber: {
    width: 44,
    paddingRight: spacing.sm,
    color: theme.colors.inkSoft,
    fontFamily: "monospace",
    fontSize: theme.font(12),
    lineHeight: theme.leading(21),
    textAlign: "right",
  },
  code: { flex: 1, color: theme.colors.ink, fontFamily: "monospace", fontSize: theme.font(13), lineHeight: theme.leading(21) },
  plain: { color: theme.colors.ink },
  key: { color: theme.codeColors.jsonKey },
  string: { color: theme.codeColors.jsonString },
  number: { color: theme.codeColors.jsonNumber },
  literal: { color: theme.codeColors.jsonLiteral },
}));
