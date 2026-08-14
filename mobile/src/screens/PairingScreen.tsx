import { CameraView, useCameraPermissions } from "expo-camera";
import type { TFunction } from "i18next";
import { Clipboard, ScanLine } from "lucide-react-native";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ActivityIndicator,
  Keyboard,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  TouchableWithoutFeedback,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { Button } from "../components/Button";
import { pairingCodeFromQr } from "../remote/codec";
import { useRemote } from "../remote/RemoteContext";
import { colors, radius, spacing } from "../theme/tokens";
import { VERSION } from "../version.generated";

function pairingErrorMessage(error: unknown, t: TFunction): string {
  const rawMessage = error instanceof Error ? error.message : error;
  const message = (typeof rawMessage === "string" ? rawMessage.trim() : "") || "unknown error";
  if (message === "unexpected_pairing_host") return t("pairing.host");
  if (/invalid_pairing_code|invalid_jwt|expired|HTTP\s*(401|403|404)/i.test(message)) {
    return t("pairing.invalid");
  }
  if (message === "nats_ws_not_tls") return t("pairing.secureEndpoint");
  if (/handshake|signature|confirmation_mismatch/i.test(message)) {
    return t("pairing.verification");
  }
  if (/network|unreachable|load failed|fetch failed|econn|time-?out|nats_connect/i.test(message)) {
    return t("pairing.network");
  }
  if (/HTTP\s*(429|5\d\d)|server|service unavailable/i.test(message)) {
    return t("pairing.service");
  }
  return t("pairing.failed");
}

export function PairingScreen({ revoked = false }: { revoked?: boolean }) {
  const { t } = useTranslation();
  const remote = useRemote();
  const [permission, requestPermission] = useCameraPermissions();
  const scanLocked = useRef(false);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [manualOpen, setManualOpen] = useState(false);
  const [manualCode, setManualCode] = useState("");
  const [manualError, setManualError] = useState<string | null>(null);
  const [toastMessage, setToastMessage] = useState<string | null>(null);

  useEffect(
    () => () => {
      if (toastTimer.current) clearTimeout(toastTimer.current);
    },
    [],
  );

  const showToast = useCallback((message: string) => {
    if (toastTimer.current) clearTimeout(toastTimer.current);
    setToastMessage(message);
    toastTimer.current = setTimeout(() => {
      setToastMessage(null);
      toastTimer.current = null;
    }, 4_500);
  }, []);

  const doPair = useCallback(
    async (code: string) => {
      setManualError(null);
      try {
        await remote.pair(code);
      } catch (error) {
        const message = pairingErrorMessage(error, t);
        const record =
          typeof error === "object" && error !== null ? (error as Record<string, unknown>) : null;
        console.warn("remote pairing failed", {
          name: typeof record?.name === "string" ? record.name : typeof error,
          message: typeof record?.message === "string" ? record.message : "",
          code: record?.code,
          cause: error instanceof Error ? error.cause : undefined,
          stack: error instanceof Error ? error.stack : undefined,
        });
        setManualError(message);
        showToast(message);
        throw error;
      }
    },
    [remote, showToast, t],
  );

  const handleScan = useCallback(
    async ({ data }: { data: string }) => {
      if (scanLocked.current || remote.phase === "claiming") return;
      const code = pairingCodeFromQr(data);
      if (!code) {
        showToast(t("pairing.invalid"));
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
    [doPair, remote.phase, showToast, t],
  );

  const handleManualSubmit = useCallback(async () => {
    Keyboard.dismiss();
    const trimmed = manualCode.trim();
    if (!trimmed) return;
    const code = pairingCodeFromQr(trimmed);
    if (!code) {
      setManualError(t("pairing.invalid"));
      showToast(t("pairing.invalid"));
      return;
    }
    try {
      await doPair(code);
      setManualOpen(false);
    } catch {
      // error shown via manualError in doPair
    }
  }, [doPair, manualCode, showToast, t]);

  const scanning = remote.phase === "claiming";

  return (
    <SafeAreaView edges={["top", "bottom"]} style={styles.safe}>
      <TouchableWithoutFeedback onPress={Keyboard.dismiss}>
        <View style={styles.page}>
          <View style={styles.copy}>
            <Text style={styles.title}>{t("pairing.title")}</Text>
            <Text style={styles.description}>{t("pairing.description")}</Text>
          </View>

          {revoked && (
            <View accessibilityRole="alert" style={styles.revokedBanner}>
              <Text style={styles.revokedText}>{t("pairing.revoked")}</Text>
            </View>
          )}

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

          <View style={styles.footer}>
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
            <Text style={styles.version}>{t("common.version", { version: VERSION })}</Text>
          </View>

          {toastMessage && (
            <View accessibilityRole="alert" style={styles.toast}>
              <Text style={styles.toastText}>{toastMessage}</Text>
            </View>
          )}

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
                {manualError && <Text style={styles.manualError}>{manualError}</Text>}
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
  copy: { marginBottom: spacing.xl },
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
  footer: { marginTop: "auto" },
  version: {
    color: colors.inkMuted,
    fontSize: 12,
    textAlign: "center",
    paddingTop: spacing.lg,
    paddingBottom: spacing.sm,
  },
  toast: {
    position: "absolute",
    right: spacing.xl,
    bottom: spacing.xl,
    left: spacing.xl,
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.md,
    borderRadius: radius.md,
    backgroundColor: colors.inkStrong,
    shadowColor: "#000",
    shadowOpacity: 0.16,
    shadowRadius: 10,
    shadowOffset: { width: 0, height: 4 },
    elevation: 5,
  },
  toastText: { color: colors.surface, fontSize: 14, fontWeight: "600", textAlign: "center" },
  revokedBanner: {
    width: "100%",
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.md,
    borderRadius: radius.md,
    backgroundColor: colors.dangerSoft,
    borderWidth: 1,
    borderColor: colors.dangerLine,
  },
  revokedText: { color: colors.danger, fontSize: 14, fontWeight: "600", textAlign: "center" },
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
