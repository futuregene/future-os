import { CameraView, useCameraPermissions } from "expo-camera";
import type { TFunction } from "i18next";
import { ArrowLeft, Clipboard, ScanLine } from "lucide-react-native";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ActivityIndicator,
  AppState,
  BackHandler,
  Keyboard,
  Linking,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  useWindowDimensions,
  View,
} from "react-native";
import { SafeAreaView, useSafeAreaInsets } from "react-native-safe-area-context";
import { Button } from "../components/Button";
import { DialogSurface } from "../components/DialogSurface";
import { pairingCodeFromQr } from "../remote/codec";
import { RemoteApiError } from "../remote/connectionState";
import { useRemote } from "../remote/RemoteContext";
import { colors, layout, radius, spacing } from "../theme/tokens";
import { VERSION } from "../version.generated";
import { LanguageSettings } from "../i18n/LanguageSettings";

function pairingErrorMessage(error: unknown, t: TFunction): string {
  if (error instanceof RemoteApiError) {
    // The server returned a machine `code` + HTTP status; map those rather than
    // the human message (the old `HTTP 401/403/404` message sniff is gone).
    if (/invalid_pairing_code|invalid_jwt|expired/.test(error.code ?? "")) {
      return t("pairing.invalid");
    }
    if (error.status === 401 || error.status === 403 || error.status === 404) {
      return t("pairing.invalid");
    }
    if (error.status === 429 || error.status >= 500) {
      return t("pairing.service");
    }
    return t("pairing.failed");
  }
  const rawMessage = error instanceof Error ? error.message : error;
  const message = (typeof rawMessage === "string" ? rawMessage.trim() : "") || "unknown error";
  if (message === "unexpected_pairing_host") return t("pairing.host");
  if (/invalid_pairing_code|invalid_jwt|expired/.test(message)) {
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

export function PairingScreen({ onPaired, onBack }: {
  onPaired?(): void;
  onBack?(): void;
}) {
  const { t } = useTranslation();
  const dimensions = useWindowDimensions();
  const insets = useSafeAreaInsets();
  const remote = useRemote();
  const [permission, requestPermission, getPermission] = useCameraPermissions();
  const scanLocked = useRef(false);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [manualOpen, setManualOpen] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [manualCode, setManualCode] = useState("");
  const [manualError, setManualError] = useState<string | null>(null);
  const [toastMessage, setToastMessage] = useState<string | null>(null);
  const scannerSize = Math.max(
    280,
    Math.min(dimensions.width, layout.formMaxWidth) - layout.gutter * 2,
  );
  const scanBoxSize = Math.max(
    160,
    scannerSize - spacing.lg * 2 - spacing.md - 22 * dimensions.fontScale,
  );
  const availableHeight = dimensions.height - insets.top - insets.bottom;
  const fixedContentHeight = 360 + Math.max(0, dimensions.fontScale - 1) * 120;
  const compactManualEntry = availableHeight < scannerSize + fixedContentHeight;

  const openManual = () => {
    setManualCode("");
    setManualError(null);
    setManualOpen(true);
  };

  useEffect(() => {
    if (!onBack) return;
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      if (!scanning) onBack();
      return true;
    });
    return () => subscription.remove();
  }, [onBack, scanning]);

  useEffect(
    () => () => {
      if (toastTimer.current) clearTimeout(toastTimer.current);
    },
    [],
  );

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") void getPermission();
    });
    return () => subscription.remove();
  }, [getPermission]);

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
      setScanning(true);
      try {
        await remote.pair(code);
        onPaired?.();
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
      } finally {
        setScanning(false);
      }
    },
    [remote, onPaired, showToast, t],
  );

  const handleScan = useCallback(
    async ({ data }: { data: string }) => {
      if (scanLocked.current || scanning) return;
      const code = pairingCodeFromQr(data);
      if (!code) {
        showToast(t("pairing.invalid"));
        return;
      }
      scanLocked.current = true;
      try {
        await doPair(code);
      } catch {
        // doPair already presents the error; camera callbacks cannot await it.
      } finally {
        setTimeout(() => {
          scanLocked.current = false;
        }, 1200);
      }
    },
    [doPair, scanning, showToast, t],
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

  return (
    <SafeAreaView style={styles.safe}>
        <View style={styles.page}>
          <ScrollView contentContainerStyle={styles.scroll} keyboardShouldPersistTaps="handled">
          <View style={styles.copy}>
            <View style={styles.titleRow}>
              {onBack && (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={t("common.back")}
                  disabled={scanning}
                  onPress={onBack}
                  style={({ pressed }) => [styles.backButton, pressed && styles.manualPressed]}
                >
                  <ArrowLeft color={colors.ink} size={22} />
                </Pressable>
              )}
              <Text accessibilityRole="header" style={styles.title}>{t("pairing.title")}</Text>
              {compactManualEntry && (
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={t("pairing.manual")}
                  accessibilityState={{ disabled: scanning }}
                  disabled={scanning}
                  hitSlop={spacing.sm}
                  onPress={openManual}
                  style={({ pressed }) => [styles.compactManualButton, pressed && styles.manualPressed]}
                >
                  <Clipboard color={colors.inkSoft} size={16} />
                  <Text numberOfLines={1} style={styles.compactManualLabel}>{t("pairing.manual")}</Text>
                </Pressable>
              )}
            </View>
            <Text style={styles.description}>{t("pairing.description")}</Text>
          </View>

          <View style={[styles.scanner, permission?.granted && { height: scannerSize }]}>
            {!permission ? (
              <ActivityIndicator color={colors.accent} />
            ) : !permission.granted ? (
              <View style={styles.permission}>
                <ScanLine color={colors.inkSoft} size={40} />
                <Text style={styles.permissionText}>
                  {t(permission.canAskAgain ? "pairing.permission" : "pairing.permissionDenied")}
                </Text>
                <Button
                  label={t(permission.canAskAgain ? "pairing.continue" : "pairing.openSettings")}
                  onPress={() =>
                    void (permission.canAskAgain ? requestPermission() : Linking.openSettings())
                  }
                />
              </View>
            ) : (
              <>
                <CameraView
                  barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
                  onBarcodeScanned={manualOpen || scanning ? undefined : handleScan}
                  style={StyleSheet.absoluteFill}
                />
                <View pointerEvents="none" style={styles.scanFrame}>
                  <View style={[styles.scanBox, { width: scanBoxSize, height: scanBoxSize }]} />
                  <Text style={styles.scanHint}>
                    {scanning ? t("pairing.claiming") : t("pairing.scanning")}
                  </Text>
                </View>
              </>
            )}
          </View>

          <View style={styles.footer}>
            {!compactManualEntry && (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={t("pairing.manual")}
                accessibilityState={{ disabled: scanning }}
                disabled={scanning}
                onPress={openManual}
                style={({ pressed }) => [styles.manualButton, pressed && styles.manualPressed]}
              >
                <Clipboard color={colors.inkSoft} size={17} />
                <Text style={styles.manualLabel}>{t("pairing.manual")}</Text>
              </Pressable>
            )}
            <LanguageSettings />
            <Text style={styles.version}>{t("common.version", { version: VERSION })}</Text>
          </View>

          </ScrollView>

          {toastMessage && (
            <View accessibilityRole="alert" style={styles.toast}>
              <Text style={styles.toastText}>{toastMessage}</Text>
            </View>
          )}

          <Modal
            animationType="fade"
            onRequestClose={() => { if (!scanning) setManualOpen(false); }}
            transparent
            visible={manualOpen}
          >
            <DialogSurface>
                <Text style={styles.dialogTitle}>{t("pairing.manual")}</Text>
                <TextInput
                  accessibilityLabel={t("pairing.manual")}
                  returnKeyType="go"
                  onSubmitEditing={() => { if (!scanning) void handleManualSubmit(); }}
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
            </DialogSurface>
          </Modal>
        </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.canvas },
  page: { flex: 1, width: "100%", maxWidth: layout.formMaxWidth, alignSelf: "center", paddingHorizontal: layout.gutter },
  scroll: { flexGrow: 1, paddingTop: spacing.lg, paddingBottom: spacing.lg },
  backButton: { width: layout.touchTarget, height: layout.touchTarget, flexShrink: 0, alignItems: "center", justifyContent: "center", borderRadius: radius.md },
  copy: { marginBottom: spacing.xl },
  titleRow: { minHeight: layout.touchTarget, flexDirection: "row", alignItems: "center", gap: spacing.sm },
  title: { flex: 1, minWidth: 0, color: colors.inkStrong, fontSize: 28, fontWeight: "700", letterSpacing: -0.5 },
  description: { color: colors.inkSoft, fontSize: 16, lineHeight: 24, marginTop: spacing.md },
  scanner: {
    width: "100%",
    minHeight: 280,
    flexShrink: 0,
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
    paddingVertical: spacing.lg,
    backgroundColor: "rgba(15, 23, 42, 0.16)",
  },
  scanBox: {
    borderWidth: 3,
    borderColor: colors.surface,
    borderRadius: radius.lg,
  },
  scanHint: {
    marginTop: spacing.md,
    paddingHorizontal: spacing.md,
    textAlign: "center",
    color: colors.surface,
    fontSize: 15,
    lineHeight: 22,
    fontWeight: "600",
    textShadowColor: colors.overlay,
    textShadowRadius: 4,
  },
  manualButton: {
    minHeight: 48,
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
  manualLabel: { flexShrink: 1, textAlign: "center", color: colors.inkSoft, fontSize: 15, fontWeight: "600" },
  compactManualButton: {
    minHeight: 36,
    flexShrink: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.sm,
    borderRadius: radius.md,
  },
  compactManualLabel: { color: colors.inkSoft, fontSize: 13, fontWeight: "600" },
  footer: { marginTop: "auto", paddingTop: spacing.lg, gap: spacing.sm },
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
