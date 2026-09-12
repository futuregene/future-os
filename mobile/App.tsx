import { StatusBar } from "expo-status-bar";
import { useEffect, useState, useSyncExternalStore, type PropsWithChildren } from "react";
import { ActivityIndicator, Animated, Easing, StyleSheet, View } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { ChatScreen } from "./src/features/chat/ChatScreen";
import { PairingScreen } from "./src/screens/PairingScreen";
import { SessionsScreen } from "./src/screens/SessionsScreen";
import { RemoteProvider, useRemoteControls as useRemote } from "./src/remote/RemoteContext";
import { shareLandedRevision, subscribeShareLanded } from "./src/share/shareInbox";
import { useShareIntake } from "./src/share/useShareIntake";
import { useUpdateReminder } from "./src/update/useUpdateReminder";
import { colors } from "./src/theme/tokens";

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
  useUpdateReminder();
  useShareIntake();
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
  // A revoked device (M1) has no usable credentials even though they're still
  // stored — route to the pairing screen so the user can re-pair.
  if (remote.phase === "revoked") {
    return (
      <EnterTransition fromRight={false}>
        <PairingScreen revoked />
      </EnterTransition>
    );
  }
  if (!remote.credentials) return <PairingScreen />;
  const inChat = Boolean(remote.selectedSessionId || remote.draft);
  return (
    <EnterTransition
      key={inChat ? `chat:${shareRevision}` : "sessions"}
      fromRight={inChat}
    >
      {inChat ? <ChatScreen /> : <SessionsScreen />}
    </EnterTransition>
  );
}

export default function App() {
  return (
    <SafeAreaProvider>
      <RemoteProvider>
        <StatusBar style="dark" />
        <AppContent />
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
