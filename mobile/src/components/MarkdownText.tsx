import { Linking, StyleSheet, Text, View } from "react-native";
import { colors, radius, spacing } from "../theme/tokens";

interface MarkdownTextProps {
  text: string;
}

type Block =
  | { kind: "paragraph"; text: string }
  | { kind: "heading"; level: number; text: string }
  | { kind: "code"; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "rule" };

function blocksFromMarkdown(text: string): Block[] {
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index] ?? "";
    if (!line.trim()) {
      index += 1;
      continue;
    }
    if (line.trimStart().startsWith("```")) {
      const code: string[] = [];
      index += 1;
      while (index < lines.length && !(lines[index] ?? "").trimStart().startsWith("```")) {
        code.push(lines[index] ?? "");
        index += 1;
      }
      if (index < lines.length) index += 1;
      blocks.push({ kind: "code", text: code.join("\n") });
      continue;
    }
    if (/^\s{0,3}([-*_])\1\1+\s*$/.test(line)) {
      blocks.push({ kind: "rule" });
      index += 1;
      continue;
    }
    const heading = line.match(/^\s{0,3}(#{1,6})\s+(.+)$/);
    if (heading) {
      blocks.push({ kind: "heading", level: heading[1]?.length ?? 1, text: heading[2] ?? "" });
      index += 1;
      continue;
    }
    const list = line.match(/^\s*([-*+]|\d+\.)\s+(.+)$/);
    if (list) {
      const ordered = /\d+\./.test(list[1] ?? "");
      const items: string[] = [];
      while (index < lines.length) {
        const item = (lines[index] ?? "").match(/^\s*([-*+]|\d+\.)\s+(.+)$/);
        if (!item || /\d+\./.test(item[1] ?? "") !== ordered) break;
        items.push(item[2] ?? "");
        index += 1;
      }
      blocks.push({ kind: "list", ordered, items });
      continue;
    }
    const paragraph = [line];
    index += 1;
    while (index < lines.length && (lines[index] ?? "").trim()) {
      const nextLine = lines[index] ?? "";
      if (
        nextLine.trimStart().startsWith("```") ||
        /^\s{0,3}([-*_])\1\1+\s*$/.test(nextLine) ||
        /^\s{0,3}(#{1,6})\s+/.test(nextLine) ||
        /^\s*([-*+]|\d+\.)\s+/.test(nextLine)
      ) {
        break;
      }
      paragraph.push(nextLine);
      index += 1;
    }
    blocks.push({ kind: "paragraph", text: paragraph.join("\n") });
  }
  return blocks;
}

function InlineMarkdown({ text }: { text: string }) {
  const parts = text.split(/(\*\*[^*]+\*\*|`[^`]+`|\*[^*]+\*|\[[^\]]+\]\([^\s)]+\))/g);
  return (
    <>
      {parts.map((part, index) => {
        if (/^\*\*[^*]+\*\*$/.test(part)) {
          return (
            <Text key={index} style={styles.bold}>
              {part.slice(2, -2)}
            </Text>
          );
        }
        if (/^`[^`]+`$/.test(part)) {
          return (
            <Text key={index} style={styles.inlineCode}>
              {part.slice(1, -1)}
            </Text>
          );
        }
        if (/^\*[^*]+\*$/.test(part)) {
          return (
            <Text key={index} style={styles.italic}>
              {part.slice(1, -1)}
            </Text>
          );
        }
        const link = part.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/);
        if (link) {
          return (
            <Text
              key={index}
              onPress={() => void Linking.openURL(link[2] ?? "")}
              style={styles.link}
            >
              {link[1] ?? ""}
            </Text>
          );
        }
        return part;
      })}
    </>
  );
}

export function MarkdownText({ text }: MarkdownTextProps) {
  return (
    <View>
      {blocksFromMarkdown(text).map((block, index) => {
        if (block.kind === "rule") return <View key={index} style={styles.rule} />;
        if (block.kind === "code") {
          return (
            <Text key={index} selectable style={styles.code}>
              {block.text}
            </Text>
          );
        }
        if (block.kind === "heading") {
          return (
            <Text
              key={index}
              selectable
              style={[styles.heading, block.level <= 2 ? styles.headingLarge : null]}
            >
              <InlineMarkdown text={block.text} />
            </Text>
          );
        }
        if (block.kind === "list") {
          return (
            <View key={index} style={styles.list}>
              {block.items.map((item, itemIndex) => (
                <Text key={itemIndex} selectable style={styles.listItem}>
                  {block.ordered ? `${itemIndex + 1}. ` : "• "}
                  <InlineMarkdown text={item} />
                </Text>
              ))}
            </View>
          );
        }
        return (
          <Text key={index} selectable style={styles.paragraph}>
            <InlineMarkdown text={block.text} />
          </Text>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  paragraph: { color: colors.ink, fontSize: 17, lineHeight: 26, marginBottom: spacing.md },
  heading: {
    color: colors.inkStrong,
    fontSize: 19,
    fontWeight: "700",
    lineHeight: 26,
    marginBottom: spacing.sm,
  },
  headingLarge: { fontSize: 23, lineHeight: 30 },
  bold: { fontWeight: "700" },
  italic: { fontStyle: "italic" },
  inlineCode: {
    color: colors.inkStrong,
    backgroundColor: colors.surfaceSubtle,
    fontFamily: "monospace",
  },
  link: { color: colors.accent, textDecorationLine: "underline" },
  rule: { height: 1, marginVertical: spacing.md, backgroundColor: colors.line },
  code: {
    marginBottom: spacing.md,
    padding: spacing.md,
    color: colors.ink,
    borderRadius: radius.md,
    backgroundColor: colors.surfaceSubtle,
    fontFamily: "monospace",
    fontSize: 13,
    lineHeight: 20,
  },
  list: { marginBottom: spacing.md },
  listItem: { color: colors.ink, fontSize: 17, lineHeight: 26 },
});
