import { useRef } from "react";
import { Modal, Platform, StyleSheet, Text, TextInput, View } from "react-native";
import type { TFunction } from "i18next";
import { Button } from "../../../components/Button";
import { DialogSurface } from "../../../components/DialogSurface";
import { colors, radius, spacing } from "../../../theme/tokens";

export function RenameModal({
  renameOpen,
  renameValue,
  setRenameValue,
  submitRename,
  onClose,
  t,
}: {
  renameOpen: boolean;
  renameValue: string;
  setRenameValue: (value: string) => void;
  submitRename: () => Promise<void>;
  onClose: () => void;
  t: TFunction;
}) {
  const pending = useRef<(() => void) | null>(null);
  const flush = () => {
    const action = pending.current;
    pending.current = null;
    action?.();
  };
  const save = () => {
    if (!renameValue.trim() || pending.current) return;
    pending.current = () => void submitRename();
    onClose();
    if (Platform.OS !== "ios") setTimeout(flush, 0);
  };
  return (
    <Modal
      animationType="fade"
      onRequestClose={onClose}
      onDismiss={flush}
      transparent
      visible={renameOpen}
    >
      <DialogSurface>
          <Text style={styles.dialogTitle}>{t("chat.renameTitle")}</Text>
          <TextInput
            autoFocus
            accessibilityLabel={t("chat.renameTitle")}
            onChangeText={setRenameValue}
            onSubmitEditing={save}
            placeholder={t("sessions.unnamed")}
            placeholderTextColor={colors.inkMuted}
            returnKeyType="done"
            style={styles.nameInput}
            value={renameValue}
          />
          <View style={styles.dialogActions}>
            <View style={styles.dialogAction}>
              <Button compact label={t("chat.cancel")} onPress={onClose} variant="secondary" />
            </View>
            <View style={styles.dialogAction}>
              <Button
                compact
                disabled={!renameValue.trim()}
                label={t("chat.save")}
                onPress={save}
              />
            </View>
          </View>
      </DialogSurface>
    </Modal>
  );
}

const styles = StyleSheet.create({
  dialogTitle: { color: colors.inkStrong, fontSize: 20, fontWeight: "700" },
  nameInput: {
    minHeight: 48,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    fontSize: 15,
    color: colors.ink,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
  },
  dialogActions: { flexDirection: "row", gap: spacing.md },
  dialogAction: { flex: 1 },
});
