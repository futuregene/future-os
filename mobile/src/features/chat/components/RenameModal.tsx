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
  return (
    <Modal
      animationType="fade"
      onRequestClose={onClose}
      transparent
      visible={Platform.OS !== "ios" && renameOpen}
    >
      <DialogSurface>
          <Text style={styles.dialogTitle}>{t("chat.renameTitle")}</Text>
          <TextInput
            autoFocus
            accessibilityLabel={t("chat.renameTitle")}
            onChangeText={setRenameValue}
            onSubmitEditing={() => void submitRename()}
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
                onPress={() => void submitRename()}
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
    color: colors.ink,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
  },
  dialogActions: { flexDirection: "row", gap: spacing.md },
  dialogAction: { flex: 1 },
});
