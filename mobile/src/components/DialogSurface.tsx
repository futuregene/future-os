import { useEffect, useState, type PropsWithChildren } from "react";
import { Keyboard, KeyboardAvoidingView, Platform, ScrollView, StyleSheet, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { colors, layout, radius, spacing } from "../theme/tokens";

/** Contents of a transparent Modal: safe-area aware and scrollable even with
 * large text, a landscape viewport, or an open keyboard. Modal lifecycle stays
 * with the caller so native presentation sequencing is unchanged. */
export function DialogSurface({ children }: PropsWithChildren) {
  const insets = useSafeAreaInsets();
  const [keyboardHeight, setKeyboardHeight] = useState(0);
  useEffect(() => {
    if (Platform.OS !== "android") return;
    // Like ChatScreen, Android edge-to-edge needs the measured IME inset;
    // KeyboardAvoidingView's Android measurement includes the wrong origin.
    const show = Keyboard.addListener("keyboardDidShow", event =>
      setKeyboardHeight(event.endCoordinates.height),
    );
    const hide = Keyboard.addListener("keyboardDidHide", () => setKeyboardHeight(0));
    return () => { show.remove(); hide.remove(); };
  }, []);

  return (
    <KeyboardAvoidingView
      behavior={Platform.OS === "ios" ? "padding" : undefined}
      style={styles.overlay}
    >
      <View style={[
        styles.viewport,
        {
          paddingTop: insets.top,
          paddingBottom: Math.max(insets.bottom, keyboardHeight),
          paddingLeft: insets.left,
          paddingRight: insets.right,
        },
      ]}>
        <ScrollView
          bounces={false}
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={styles.scroll}
        >
          <View accessibilityViewIsModal style={styles.dialog}>{children}</View>
        </ScrollView>
      </View>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  overlay: { flex: 1, backgroundColor: colors.overlay },
  viewport: { flex: 1 },
  scroll: {
    flexGrow: 1,
    justifyContent: "center",
    alignItems: "center",
    padding: layout.gutter,
  },
  dialog: {
    width: "100%",
    maxWidth: layout.dialogMaxWidth,
    padding: spacing.lg,
    gap: spacing.lg,
    borderRadius: radius.xl,
    backgroundColor: colors.surface,
  },
});
