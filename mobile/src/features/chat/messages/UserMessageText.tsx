import { useTranslation } from "react-i18next";
import { Linking, Text } from "react-native";
import { AppAlert as Alert } from "../../../components/appAlerts";
import { splitUserTextSegments } from "../../../components/userTextSegments";
import { styles } from "./messageStyles";

/**
 * User messages render as plain text (never markdown — the user's `*`/`#`/`1.`
 * stay literal), except `[name](./path)` file mentions and `[label](https://…)`
 * links, recognized exactly like the desktop UserMessageText. The bubble is
 * accent-blue, so both render white: mentions medium-weight, links underlined
 * and tappable (file mentions open like assistant file links, external links in
 * the browser).
 */
export function UserMessageText({ text, onOpenFile, onOpenSession }: {
  text: string;
  onOpenFile?(path: string): void;
  /** Opens the conversation a `#` reference points at (see the composer menu). */
  onOpenSession?(sessionId: string): void;
}) {
  const { t } = useTranslation();
  return (
    <Text selectable style={[styles.messageText, styles.userText]}>
      {splitUserTextSegments(text).map(segment => {
        if (segment.kind === "session") {
          // A reference to another conversation: the `#` marker is what tells it
          // apart from a file mention at a glance. Tapping switches to it.
          return (
            <Text
              accessibilityRole="link"
              key={segment.key}
              onPress={onOpenSession && segment.href ? () => onOpenSession(segment.href!) : undefined}
              style={styles.userMention}
            >
              {`#${segment.text}`}
            </Text>
          );
        }
        if (segment.kind === "mention") {
          return (
            <Text
              key={segment.key}
              onPress={onOpenFile && segment.href ? () => onOpenFile(segment.href!) : undefined}
              style={styles.userMention}
            >
              {segment.text}
            </Text>
          );
        }
        if (segment.kind === "link") {
          return (
            <Text
              key={segment.key}
              onPress={() => {
                void Linking.openURL(segment.href ?? "").catch(() => {
                  Alert.alert(t("attachment.title"), t("attachment.linkOpenFailed"));
                });
              }}
              style={styles.userLink}
            >
              {segment.text}
            </Text>
          );
        }
        return <Text key={segment.key}>{segment.text}</Text>;
      })}
    </Text>
  );
}
