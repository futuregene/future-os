import { StatusBar } from "expo-status-bar";
import { AppDialogHost } from "./src/components/AppDialogHost";
import { useEffect, useState, useSyncExternalStore, type PropsWithChildren } from "react";
import { ActivityIndicator, Animated, Easing, StyleSheet, View } from "react-native";
import { SafeAreaProvider, initialWindowMetrics } from "react-native-safe-area-context";
import { ChatScreen } from "./src/features/chat/ChatScreen";
import { PairingScreen } from "./src/screens/PairingScreen";
import { SessionsScreen } from "./src/screens/SessionsScreen";
import { DesktopsScreen } from "./src/screens/DesktopsScreen";
import { RemoteProvider, useRemoteControls as useRemote } from "./src/remote/RemoteContext";
import { shareLandedRevision, subscribeShareLanded } from "./src/share/shareInbox";
import { ShareIntakeMenu } from "./src/share/ShareIntakeMenu";
import { useUpdateReminder } from "./src/update/useUpdateReminder";
import { colors } from "./src/theme/tokens";
import { useSystemLanguage } from "./src/i18n/useSystemLanguage";

// Enter transition for top-level screen swaps (there is no navigation library
// here — App switches screens by conditional render). The outgoing screen
// unmounts immediately; sliding/fading the incoming one over the shared
// background reads as a push/pop instead of a hard cut.
function EnterTransition({ fromRight, children }: PropsWithChildren<{ fromRight: boolean }>) {
  // Stable holder (never re-set) — useState instead of useRef so the React
  // Compiler ref rules stay happy while the value survives re-renders.
  const [progress] = useState(() => new Animated.Value(0));
  useEffect(() => {
    Animated.timing(progress, {
      toValue: 1,
      duration: 180,
      easing: Easing.out(Easing.cubic),
      useNativeDriver: true,
    }).start();
  }, [progress]);
  const translateX = progress.interpolate({
    inputRange: [0, 1],
    outputRange: [fromRight ? 32 : -32, 0],
  });
  return (
    <Animated.View style={[styles.fill, { opacity: progress, transform: [{ translateX }] }]}>
      {children}
    </Animated.View>
  );
}

function AppContent() {
  const remote = useRemote();
  const [screen, setScreen] = useState<"main" | "desktops" | "pair">("main");
  const showDesktops = () => setScreen("desktops");
  useUpdateReminder();
  useEffect(() => subscribeShareLanded(() => setScreen("main")), []);
  // A share stages its payload in the composer draft; when the app is already
  // showing that same draft, the key below is what makes the composer re-read
  // it (see src/share/shareInbox.ts).
  const shareRevision = useSyncExternalStore(subscribeShareLanded, shareLandedRevision);

  if (remote.phase === "booting") {
    return (
      <View style={styles.loading}>
        <ActivityIndicator color={colors.accent} size="large" />
      </View>
    );
  }
  if (screen === "desktops") return <DesktopsScreen onBack={() => setScreen("main")} onAdd={() => setScreen("pair")} />;
  if (screen === "pair") return <PairingScreen onBack={showDesktops} onPaired={() => setScreen("main")} />;
  // The local desktop registry owns startup routing. Credential validity and
  // connection health update inside the paired UI without swapping screens.
  if (remote.desktops.length === 0) return <PairingScreen />;
  const inChat = !!remote.credentials && remote.connectionPresentation.level !== "disconnected" && Boolean(remote.selectedSessionId || remote.draft);
  return (
    <View style={styles.fill}>
      {/* Keep the list and its native scroll position in place on pop. A new
          list + reverse entry animation caused a visible restoration jump. */}
      <View
        style={styles.fill}
        pointerEvents={inChat ? "none" : "auto"}
        accessibilityElementsHidden={inChat}
        importantForAccessibility={inChat ? "no-hide-descendants" : "auto"}
      >
        <SessionsScreen
          active={!inChat}
          onManageDesktops={showDesktops}
        />
      </View>
      {inChat && (
        <View style={StyleSheet.absoluteFill}>
          <EnterTransition key={shareRevision} fromRight>
            <ChatScreen />
          </EnterTransition>
        </View>
      )}
    </View>
  );
}

export default function App() {
  const languageReady = useSystemLanguage();
  if (!languageReady) {
    return (
      <View style={styles.loading}>
        <ActivityIndicator color={colors.accent} size="large" />
      </View>
    );
  }
  return (
    <SafeAreaProvider initialMetrics={initialWindowMetrics}>
      <RemoteProvider>
        <StatusBar style="dark" />
        <AppContent />
        <ShareIntakeMenu />
        <AppDialogHost />
      </RemoteProvider>
    </SafeAreaProvider>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1, backgroundColor: colors.surface },
  loading: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: colors.canvas,
  },
});
