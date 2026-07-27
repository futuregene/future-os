import { StatusBar } from "expo-status-bar";
import { ActivityIndicator, StyleSheet, View } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { ChatScreen } from "./src/screens/ChatScreen";
import { PairingScreen } from "./src/screens/PairingScreen";
import { SessionsScreen } from "./src/screens/SessionsScreen";
import { RemoteProvider, useRemote } from "./src/remote/RemoteContext";
import { colors } from "./src/theme/tokens";

function AppContent() {
  const remote = useRemote();

  if (remote.phase === "booting") {
    return (
      <View style={styles.loading}>
        <ActivityIndicator color={colors.accent} size="large" />
      </View>
    );
  }
  if (!remote.credentials) return <PairingScreen />;
  if (remote.selectedSessionId || remote.draft) return <ChatScreen />;
  return <SessionsScreen />;
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
  loading: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: colors.canvas,
  },
});
