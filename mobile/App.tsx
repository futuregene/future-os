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
import { ThemeProvider, createThemedStyles, useAppearanceReady, useTheme } from "./src/theme/ThemeContext";
import { shareLandedRevision, subscribeShareLanded } from "./src/share/shareInbox";
import { ShareIntakeMenu } from "./src/share/ShareIntakeMenu";
import { useUpdateReminder } from "./src/update/useUpdateReminder";
import { useSystemLanguage } from "./src/i18n/useSystemLanguage";

// Enter transition for top-level screen swaps (there is no navigation library
// here — App switches screens by conditional render). The outgoing screen
// unmounts immediately; sliding/fading the incoming one over the shared
// background reads as a push/pop instead of a hard cut.
function EnterTransition({ fromRight, children }: PropsWithChildren<{ fromRight: boolean }>) {
  const styles = useStyles();
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
  const styles = useStyles();
  const { colors } = useTheme();
  const remote = useRemote();
  const [screen, setScreen] = useState<"main" | "desktops" | "pair">("main");
  const showDesktops = () => setScreen("desktops");
  // A device page's task runs can link to the conversation they ran in: the chat
  // belongs to this app, so leave the list and open the session there.
  const openFromDevices = (sessionId: string) => {
    setScreen("main");
    void remote.selectSession(sessionId);
  };
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
  if (screen === "desktops") return <DesktopsScreen onAdd={() => setScreen("pair")} onBack={() => setScreen("main")} onOpenConversation={openFromDevices} />;
  if (screen === "pair") return <PairingScreen onBack={showDesktops} onPaired={() => setScreen("main")} />;
  // No saved pair starts pairing. Saved pairs without a usable active choice
  // start in the picker; only a restored credential enters its session list.
  if (remote.desktops.length === 0) return <PairingScreen />;
  if (!remote.credentials) return <DesktopsScreen onAdd={() => setScreen("pair")} />;
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
  const styles = useStyles();
  // Both stored preferences are read before the first screen is drawn: the gate
  // is what keeps a saved language or scheme from flashing its default.
  const languageReady = useSystemLanguage();
  const appearanceReady = useAppearanceReady();
  return (
    <SafeAreaProvider initialMetrics={initialWindowMetrics}>
      <ThemeProvider>
        {/* The app's own canvas, full screen: on Android the system bars are
            drawn over it, so the strip below the content follows the scheme
            instead of the window's launch background. */}
        <View style={styles.root}>
          <ThemedStatusBar />
          {languageReady && appearanceReady ? (
            <RemoteProvider>
              <AppContent />
              <ShareIntakeMenu />
              <AppDialogHost />
            </RemoteProvider>
          ) : <BootScreen />}
        </View>
      </ThemeProvider>
    </SafeAreaProvider>
  );
}

/** Status-bar icons are content drawn on the scheme's canvas. */
function ThemedStatusBar() {
  const theme = useTheme();
  return <StatusBar style={theme.scheme === "dark" ? "light" : "dark"} />;
}

/** Shown while a stored preference is being read, before any screen is. */
function BootScreen() {
  const styles = useStyles();
  const { colors } = useTheme();
  return (
    <View style={styles.loading}>
      <ActivityIndicator color={colors.accent} size="large" />
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  root: { flex: 1, backgroundColor: theme.colors.canvas },
  fill: { flex: 1, backgroundColor: theme.colors.surface },
  loading: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: theme.colors.canvas,
  },
}));
