import { useEffect, useRef, useState } from "react";
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
  onGenerate,
  generationKey,
  t,
}: {
  renameOpen: boolean;
  renameValue: string;
  setRenameValue: (value: string) => void;
  submitRename: () => Promise<void>;
  onClose: () => void;
  onGenerate?: () => Promise<string>;
  generationKey?: string;
  t: TFunction;
}) {
  const [generating, setGenerating] = useState(false);
  const [generationError, setGenerationError] = useState<string | null>(null);
  const epoch = useRef(0);
  const busy = useRef(false);
  const scope = useRef({ open: renameOpen, key: generationKey });
  const [previousScope, setPreviousScope] = useState({ open: renameOpen, key: generationKey });
  if (previousScope.open !== renameOpen || previousScope.key !== generationKey) {
    setPreviousScope({ open: renameOpen, key: generationKey });
    setGenerating(false);
    setGenerationError(null);
  }
  useEffect(() => {
    scope.current = { open: renameOpen, key: generationKey };
    epoch.current += 1;
    busy.current = false;
    return () => { epoch.current += 1; };
  }, [renameOpen, generationKey]);
  const close = () => {
    epoch.current += 1;
    busy.current = false;
    onClose();
  };
  const generate = async () => {
    if (!onGenerate || busy.current) return;
    const request = ++epoch.current;
    const key = generationKey;
    busy.current = true;
    setGenerating(true);
    setGenerationError(null);
    const current = () => epoch.current === request && scope.current.open && scope.current.key === key;
    try {
      const title = await onGenerate();
      if (current()) setRenameValue(title);
    } catch (error) {
      if (current()) setGenerationError(String(error));
    } finally {
      if (current()) { busy.current = false; setGenerating(false); }
    }
  };
  const pending = useRef<(() => void) | null>(null);
  const flush = () => {
    const action = pending.current;
    pending.current = null;
    action?.();
  };
  const save = () => {
    if (!renameValue.trim() || pending.current || busy.current) return;
    pending.current = () => void submitRename();
    close();
    if (Platform.OS !== "ios") setTimeout(flush, 0);
  };
  return (
    <Modal
      animationType="fade"
      onRequestClose={close}
      onDismiss={flush}
      transparent
      visible={renameOpen}
    >
      <DialogSurface>
          <Text style={styles.dialogTitle}>{t("chat.renameTitle")}</Text>
          {/* Keep borders off the native EditText so Android's underline tint
              applies to its original background, not a layered RN border. */}
          <View style={styles.inputBorder}>
            <TextInput
              autoFocus
              editable={!generating}
              accessibilityLabel={t("chat.renameTitle")}
              onChangeText={setRenameValue}
              onSubmitEditing={save}
              placeholder={t("sessions.unnamed")}
              placeholderTextColor={colors.inkMuted}
              returnKeyType="done"
              style={styles.nameInput}
              underlineColorAndroid="transparent"
              value={renameValue}
            />
          </View>
          {onGenerate ? (
            <View>
              <Button compact disabled={generating} label={t(generating ? "chat.generatingTitle" : "chat.generateTitle")} onPress={() => void generate()} variant="secondary" />
              <Text style={styles.hint}>{t("chat.generateTitleHint")}</Text>
            </View>
          ) : null}
          {generationError ? <Text accessibilityRole="alert" style={styles.error}>{t("chat.titleGenerationFailed", { message: generationError })}</Text> : null}
          <View style={styles.dialogActions}>
            <View style={styles.dialogAction}>
              <Button compact label={t("chat.cancel")} onPress={close} variant="secondary" />
            </View>
            <View style={styles.dialogAction}>
              <Button
                compact
                disabled={!renameValue.trim() || generating}
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
  hint: { color: colors.inkMuted, fontSize: 12, lineHeight: 18, marginTop: spacing.sm },
  error: { color: colors.danger, fontSize: 12 },
  dialogTitle: { color: colors.inkStrong, fontSize: 20, fontWeight: "700" },
  inputBorder: {
    minWidth: 0,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
    overflow: "hidden",
  },
  nameInput: {
    minHeight: 48,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    fontSize: 15,
    color: colors.ink,
  },
  dialogActions: { flexDirection: "row", gap: spacing.md },
  dialogAction: { flex: 1 },
});
