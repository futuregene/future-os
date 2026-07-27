import { CameraView, useCameraPermissions } from "expo-camera";
import { Clipboard, ScanLine, ShieldCheck } from "lucide-react-native";
import { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ActivityIndicator,
  Keyboard,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  SafeAreaView,
  StyleSheet,
  Text,
  TextInput,
  TouchableWithoutFeedback,
  View,
} from "react-native";
import { Button } from "../components/Button";
import { pairingCodeFromQr } from "../remote/codec";
import { useRemote } from "../remote/RemoteContext";
import { colors, radius, spacing } from "../theme/tokens";
import { VERSION } from "../version.generated";

function pairingErrorKey(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (message === "unexpected_pairing_host") return "pairing.host";
  if (message === "invalid_pairing_code") return "pairing.invalid";
  return "common.error";
}

export function PairingScreen() {
  const { t } = useTranslation();
  const remote = useRemote();
  const [permission, requestPermission] = useCameraPermissions();
  const [scanError, setScanError] = useState<string | null>(null);
  const scanLocked = useRef(false);
  const [manualOpen, setManualOpen] = useState(false);
  const [manualCode, setManualCode] = useState("");
  const [manualError, setManualError] = useState<string | null>(null);

  const doPair = useCallback(
    async (code: string) => {
      setScanError(null);
      setManualError(null);
      try {
        await remote.pair(code);
      } catch (error) {
        const key = pairingErrorKey(error);
        setScanError(t(key));
        setManualError(t(key));
        throw error;
      }
    },
    [remote, t],
  );

  const handleScan = useCallback(
    async ({ data }: { data: string }) => {
      if (scanLocked.current || remote.busy) return;
      const code = pairingCodeFromQr(data);
      if (!code) {
        setScanError(t("pairing.invalid"));
        return;
      }
      scanLocked.current = true;
      try {
        await doPair(code);
      } finally {
        setTimeout(() => {
          scanLocked.current = false;
        }, 1200);
      }
    },
    [doPair, remote.busy, t],
  );

  const handleManualSubmit = useCallback(async () => {
    Keyboard.dismiss();
    const trimmed = manualCode.trim();
    if (!trimmed) return;
    const code = pairingCodeFromQr(trimmed);
    if (!code) {
      setManualError(t("pairing.invalid"));
      return;
    }
    try {
      await doPair(code);
      setManualOpen(false);
    } catch {
      // error shown via manualError in doPair
    }
  }, [doPair, manualCode, t]);

  const scanning = remote.phase === "claiming" || remote.busy;

  return (
    <SafeAreaView style={styles.safe}>
      <TouchableWithoutFeedback onPress={Keyboard.dismiss}>
        <View style={styles.page}>
          <View style={styles.header}>
            <View style={styles.brandMark}>
              <ShieldCheck color={colors.accent} size={24} strokeWidth={2.2} />
            </View>
            <Text style={styles.brand}>{t("appName")}</Text>
            <Text style={styles.kicker}>{t("remote")}</Text>
          </View>

          <View style={styles.copy}>
            <Text style={styles.title}>{t("pairing.title")}</Text>
            <Text style={styles.description}>{t("pairing.description")}</Text>
          </View>

          <View style={styles.scanner}>
            {!permission ? (
              <ActivityIndicator color={colors.accent} />
            ) : !permission.granted ? (
              <View style={styles.permission}>
                <ScanLine color={colors.inkSoft} size={40} />
                <Text style={styles.permissionText}>{t("pairing.permission")}</Text>
                <Button label={t("pairing.grant")} onPress={() => void requestPermission()} />
              </View>
            ) : (
              <>
                <CameraView
                  barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
                  onBarcodeScanned={handleScan}
                  style={StyleSheet.absoluteFill}
                />
                <View pointerEvents="none" style={styles.scanFrame}>
                  <View style={styles.scanBox} />
                  <Text style={styles.scanHint}>
                    {scanning ? t("pairing.claiming") : t("pairing.scanning")}
                  </Text>
                </View>
              </>
            )}
          </View>

          <Pressable
            accessibilityRole="button"
            disabled={scanning}
            onPress={() => {
              setManualCode("");
              setManualError(null);
              setManualOpen(true);
            }}
            style={({ pressed }) => [styles.manualButton, pressed && styles.manualPressed]}
          >
            <Clipboard color={colors.inkSoft} size={17} />
            <Text style={styles.manualLabel}>{t("pairing.manual")}</Text>
          </Pressable>

          {(scanError || remote.error) && (
            <Text accessibilityRole="alert" style={styles.error}>
              {scanError ?? t(pairingErrorKey(remote.error))}
            </Text>
          )}
          <Text style={styles.version}>{t("common.version", { version: VERSION })}</Text>

          <Modal
            animationType="fade"
            onRequestClose={() => setManualOpen(false)}
            transparent
            visible={manualOpen}
          >
            <KeyboardAvoidingView
              behavior={Platform.OS === "ios" ? "padding" : undefined}
              style={styles.overlay}
            >
              <View style={styles.dialog}>
                <Text style={styles.dialogTitle}>{t("pairing.manual")}</Text>
                <TextInput
                  autoCapitalize="none"
                  autoCorrect={false}
                  autoFocus
                  editable={!scanning}
                  onChangeText={setManualCode}
                  placeholder={t("pairing.manualPlaceholder")}
                  placeholderTextColor={colors.inkMuted}
                  selectTextOnFocus
                  style={styles.codeInput}
                  value={manualCode}
                />
                {manualError && (
                  <Text style={styles.manualError}>{manualError}</Text>
                )}
                <View style={styles.dialogActions}>
                  <View style={styles.dialogAction}>
                    <Button
                      disabled={scanning}
                      label={t("chat.cancel")}
                      onPress={() => setManualOpen(false)}
                      variant="secondary"
                    />
                  </View>
                  <View style={styles.dialogAction}>
                    <Button
                      disabled={!manualCode.trim() || scanning}
                      label={t("pairing.manualSubmit")}
                      loading={scanning}
                      onPress={() => void handleManualSubmit()}
                    />
                  </View>
                </View>
              </View>
            </KeyboardAvoidingView>
          </Modal>
        </View>
      </TouchableWithoutFeedback>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.canvas },
  page: { flex: 1, paddingHorizontal: spacing.xl, paddingTop: spacing.xl },
  header: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  brandMark: {
    width: 40,
    height: 40,
    borderRadius: radius.md,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: colors.accentSoft,
  },
  brand: { color: colors.inkStrong, fontSize: 19, fontWeight: "700" },
  kicker: { color: colors.inkMuted, fontSize: 14 },
  copy: { marginTop: spacing.xxl, marginBottom: spacing.xl },
  title: { color: colors.inkStrong, fontSize: 28, fontWeight: "700", letterSpacing: -0.5 },
  description: { color: colors.inkSoft, fontSize: 16, lineHeight: 24, marginTop: spacing.md },
  scanner: {
    flex: 1,
    maxHeight: 430,
    minHeight: 300,
    overflow: "hidden",
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.surface,
    alignItems: "center",
    justifyContent: "center",
  },
  permission: { padding: spacing.xl, alignItems: "center", gap: spacing.lg },
  permissionText: { color: colors.inkSoft, fontSize: 15, lineHeight: 22, textAlign: "center" },
  scanFrame: {
    position: "absolute",
    inset: 0,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "rgba(15, 23, 42, 0.16)",
  },
  scanBox: {
    width: 224,
    height: 224,
    borderWidth: 3,
    borderColor: colors.surface,
    borderRadius: radius.lg,
  },
  scanHint: {
    marginTop: spacing.lg,
    color: colors.surface,
    fontSize: 15,
    fontWeight: "600",
    textShadowColor: colors.overlay,
    textShadowRadius: 4,
  },
  manualButton: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: spacing.sm,
    marginTop: spacing.md,
    paddingVertical: spacing.md,
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.surface,
  },
  manualPressed: { backgroundColor: colors.surfaceSubtle },
  manualLabel: { color: colors.inkSoft, fontSize: 14, fontWeight: "600" },
  error: {
    marginTop: spacing.md,
    padding: spacing.md,
    borderRadius: radius.md,
    color: colors.danger,
    backgroundColor: colors.dangerSoft,
    borderWidth: 1,
    borderColor: colors.dangerLine,
  },
  version: {
    color: colors.inkMuted,
    fontSize: 12,
    textAlign: "center",
    paddingVertical: spacing.lg,
  },
  overlay: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: spacing.xl,
    backgroundColor: colors.overlay,
  },
  dialog: {
    width: "100%",
    maxWidth: 420,
    padding: spacing.xl,
    gap: spacing.lg,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  dialogTitle: { color: colors.inkStrong, fontSize: 20, fontWeight: "700" },
  codeInput: {
    minHeight: 48,
    paddingHorizontal: spacing.md,
    color: colors.ink,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
    fontSize: 13,
  },
  manualError: {
    color: colors.danger,
    fontSize: 13,
    marginTop: -spacing.sm,
  },
  dialogActions: { flexDirection: "row", gap: spacing.md },
  dialogAction: { flex: 1 },
});
