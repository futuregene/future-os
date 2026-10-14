import { useTranslation } from "react-i18next";
import { Text, View } from "react-native";
import { useTheme } from "../../../theme/ThemeContext";
import { useStyles } from "./messageStyles";

// Inline divider marking where the agent auto-compacted the conversation
// (history summarized to fit the context window). A hairline rule with a small
// muted label — the only surfacing of compaction in the UI, since the agent
// otherwise continues silently. Shows the pre-compaction token count and the
// agent's estimate for the next turn when known.
export function CompactionDivider({
  tokensBefore,
  tokensAfter,
  status = "completed",
  trigger,
}: {
  tokensBefore?: number;
  tokensAfter?: number;
  status?: "running" | "completed" | "failed";
  trigger?: string;
}) {
  const { t, i18n } = useTranslation();
  const manual = trigger === "manual";
  const format = (count: number) => new Intl.NumberFormat(i18n.language).format(count);
  // Both counts come from the committed checkpoint and are only meaningful as a
  // pair — a lone `tokensBefore` (released journal, legacy row) keeps the
  // older label.
  const delta = tokensBefore && tokensBefore > 0 && tokensAfter && tokensAfter > 0
    ? { before: format(tokensBefore), after: format(tokensAfter) }
    : null;
  const label =
    status === "running"
      ? manual
        ? t("chat.manuallyCompacting")
        : t("chat.compacting")
      : status === "failed"
        ? manual
          ? t("chat.manualCompactionFailed")
          : t("chat.compactionFailed")
        : delta
          ? t(manual ? "chat.manuallyCompactedTokensDelta" : "chat.compactedTokensDelta", delta)
          : manual
            ? t("chat.manuallyCompacted")
            : tokensBefore && tokensBefore > 0
              ? t("chat.compactedTokens", { formattedCount: format(tokensBefore) })
              : t("chat.compacted");
  return <StatusDivider failed={status === "failed"} label={label} />;
}

export function StatusDivider({ label, failed = false }: { label: string; failed?: boolean; }) {
  const styles = useStyles();
  const { colors } = useTheme();
  const color = failed ? colors.danger : colors.inkMuted;
  const lineColor = failed ? colors.dangerLine : colors.line;
  return (
    <View
      accessibilityLabel={label}
      accessibilityRole={failed ? "alert" : "text"}
      style={styles.compactionDivider}
    >
      <View style={[styles.compactionLine, { backgroundColor: lineColor }]} />
      <Text style={[styles.compactionLabel, { color }]}>{label}</Text>
      <View style={[styles.compactionLine, { backgroundColor: lineColor }]} />
    </View>
  );
}
