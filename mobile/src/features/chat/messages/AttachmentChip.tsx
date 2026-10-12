import { FileText, Paperclip } from "lucide-react-native";
import { Pressable, Text } from "react-native";
import type { HistoryAttachment } from "../../../remote/types";
import { colors } from "../../../theme/tokens";
import { styles } from "./messageStyles";

export function AttachmentChip({
  attachment,
  onOpen,
}: {
  attachment: HistoryAttachment;
  onOpen?(): void;
}) {
  return (
    <Pressable
      accessibilityLabel={attachment.name}
      accessibilityRole="button"
      disabled={!onOpen}
      onPress={onOpen}
      style={({ pressed }) => [styles.attachmentChip, pressed && styles.attachmentChipPressed]}
    >
      {attachment.kind === "file" ? (
        <FileText color={colors.inkSoft} size={13} />
      ) : (
        <Paperclip color={colors.inkSoft} size={13} />
      )}
      <Text numberOfLines={1} style={styles.attachmentName}>
        {attachment.name}
      </Text>
    </Pressable>
  );
}
