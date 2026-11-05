import { Brain, ChevronDown, ChevronUp } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { useTheme } from "../../../theme/ThemeContext";
import { useStyles } from "./messageStyles";
import { ROW_HIT_SLOP } from "./touchTargets";

/**
 * Where a bodyless reasoning row's text stands.
 *
 * The lean lanes send a reasoning block's boundaries and never its body: the
 * text is the largest thing a reply carries that nobody has opened yet, so it
 * is fetched when the row is opened (`onLoad`) rather than pushed with every
 * reply. That makes "no body" mean *not loaded yet*, which is a different state
 * from "there is nothing to show" — and the only one that can fail.
 */
type FetchState =
  | { status: "idle"; }
  | { status: "loading"; }
  | { status: "loaded"; text: string; }
  | { status: "unavailable"; };

/**
 * Inline thinking block inside a reply bubble. The mobile product decision
 * (audit D4) is that reasoning always renders collapsed — a muted one-line
 * label, expandable on tap — so long reasoning never floods the bubble or the
 * FlatList. The collapse is a render concern only. The label reads "thinking"
 * while the run is streaming (the block is still mid-reasoning) and "thought
 * completed" once settled.
 *
 * A row that arrives without its body (the lean lanes) is still expandable: the
 * first open asks `onLoad` for the text and shows it, and a second tap collapses
 * it again. Only a row with neither a body nor a way to fetch one is a plain
 * label — a chevron that opens an empty box promises content that never arrives.
 *
 * The glyph matches the desktop's thinking activity glyph (Brain), so an
 * expanded reasoning row and a tool row are the same shape: icon, then words.
 * Like a tool row, its collapsed header starts at the reply's left edge and
 * carries no rule; its revealed text indents under the label the same way a
 * revealed target does — see ToolRow.
 */
export function ThinkingRow({ text, streaming, opened, onLoad }: {
  text: string;
  streaming?: boolean;
  opened: boolean;
  /**
   * Fetch this row's reasoning on demand. Absent for a row whose body was sent
   * with the reply (a full feed), and for a reader with no connection to ask.
   */
  onLoad?(): Promise<string | null>;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [fetched, setFetched] = useState<FetchState>({ status: "idle" });
  const hasText = text.trim().length > 0;
  // A body is what earns the chevron — either the one that came with the reply,
  // or the one this row can go and read.
  const disclosable = hasText || !!onLoad;
  const open = opened || expanded;
  const body = hasText ? text : fetched.status === "loaded" ? fetched.text : null;

  const toggle = () => {
    if (expanded) {
      setExpanded(false);
      return;
    }
    setExpanded(true);
    if (hasText || !onLoad) return;
    if (fetched.status === "loading") return;
    if (fetched.status === "loaded") return;
    setFetched({ status: "loading" });
    void onLoad()
      .then((loaded) => {
        const trimmed = loaded?.trim() ?? "";
        setFetched(trimmed.length > 0
          ? { status: "loaded", text: loaded ?? "" }
          : { status: "unavailable" });
      })
      .catch(() => setFetched({ status: "unavailable" }));
  };

  const header = (
    <>
      <Brain color={colors.inkMuted} size={14} />
      <Text style={styles.inlineThinkingLabel}>
        {t(streaming ? "chat.thinking" : "chat.thoughtCompleted")}
      </Text>
      {disclosable &&
        (expanded ? (
          <ChevronUp color={colors.inkMuted} size={14} />
        ) : (
          <ChevronDown color={colors.inkMuted} size={14} />
        ))}
    </>
  );
  return (
    <View style={[styles.inlineThinking, !open && styles.railBlock]}>
      {disclosable ? (
        <Pressable
          accessibilityRole="button"
          accessibilityState={{ expanded }}
          hitSlop={ROW_HIT_SLOP}
          onPress={toggle}
          style={[styles.inlineThinkingHeader, !open && styles.railRow]}
        >
          {header}
        </Pressable>
      ) : (
        <View style={[styles.inlineThinkingHeader, !open && styles.railRow]}>{header}</View>
      )}
      {expanded && body ? <Text style={styles.inlineThinkingText}>{body}</Text> : null}
      {expanded && !body && fetched.status === "loading" ? (
        <Text style={styles.inlineThinkingText}>{t("chat.thoughtLoading")}</Text>
      ) : null}
      {expanded && !body && fetched.status === "unavailable" ? (
        <Text style={styles.inlineThinkingText}>{t("chat.thoughtUnavailable")}</Text>
      ) : null}
    </View>
  );
}
