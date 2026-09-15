import { useCallback, useEffect, useRef, useState } from "react";
import { Modal, Platform, StyleSheet, Text, View, type AlertButton, type AlertOptions } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "./Button";
import { DialogSurface } from "./DialogSurface";
import { colors, spacing } from "../theme/tokens";

interface DialogRequest {
  title: string;
  message?: string;
  buttons?: AlertButton[];
  options?: AlertOptions;
}

/** App confirmations and notices share the same surface as rename/settings.
 * Actions run after dismissal so a failure or navigation cannot compete with
 * the outgoing UIKit presentation. Keep the host mounted while it dismisses. */
export function useAppDialog(active = true) {
  const { t } = useTranslation();
  const [request, setRequest] = useState<DialogRequest | null>(null);
  const [visible, setVisible] = useState(false);
  const [wasActive, setWasActive] = useState(active);
  if (wasActive !== active) {
    setWasActive(active);
    if (!active) setVisible(false);
  }
  const pending = useRef<(() => void) | null>(null);
  const closing = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);
  const flush = () => {
    if (!closing.current) return;
    closing.current = false;
    const action = pending.current;
    pending.current = null;
    action?.();
  };
  const dismiss = (action?: () => void) => {
    if (closing.current) return;
    closing.current = true;
    pending.current = action ?? null;
    setVisible(false);
    if (Platform.OS !== "ios") timer.current = setTimeout(flush, 0);
  };
  const alert = useCallback((title: string, message?: string, buttons?: AlertButton[], options?: AlertOptions) => {
    setRequest({ title, message, buttons, options });
    setVisible(true);
  }, []);
  const buttons = request?.buttons ?? [{ text: t("common.close") }];
  const cancel = () => {
    if (request?.options?.cancelable === false) return;
    dismiss(request?.options?.onDismiss ?? buttons.find(button => button.style === "cancel")?.onPress);
  };
  return {
    alert,
    dialog: (
      <Modal transparent animationType="fade" visible={active && visible} onRequestClose={cancel} onDismiss={flush}>
        <DialogSurface>
          <Text accessibilityRole="header" style={styles.title}>{request?.title}</Text>
          {/* Android/OEM selection can add a native EditText background.
              Keep iOS selection, but render Android notices as plain text. */}
          {!!request?.message && <Text selectable={Platform.OS === "ios"} style={styles.message}>{request.message}</Text>}
          <View style={styles.actions}>
            {buttons.map((button, index) => (
              <View key={index} style={styles.action}>
                <Button
                  compact
                  label={button.text ?? t("common.close")}
                  variant={button.style === "destructive" ? "danger" : button.style === "cancel" ? "secondary" : "primary"}
                  onPress={() => dismiss(button.onPress)}
                />
              </View>
            ))}
          </View>
        </DialogSurface>
      </Modal>
    ),
  };
}

const styles = StyleSheet.create({
  title: { color: colors.inkStrong, fontSize: 20, fontWeight: "700" },
  message: { color: colors.inkSoft, fontSize: 15, lineHeight: 22 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: spacing.md },
  action: { flexGrow: 1, flexBasis: 120 },
});
