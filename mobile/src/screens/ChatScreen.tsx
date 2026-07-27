import { ArrowDown, ArrowLeft, Check, ChevronDown, Send, Square } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  BackHandler,
  FlatList,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  TouchableWithoutFeedback,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import Svg, { Defs, LinearGradient, Rect, Stop } from "react-native-svg";
import { TimelineCard } from "../components/TimelineCard";
import { useRemote } from "../remote/RemoteContext";
import { modelReference, type ThinkingLevel, type TimelineItem } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";

const thinkingLevels: ThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh"];

export function ChatScreen() {
  const { t } = useTranslation();
  const remote = useRemote();
  const { closeConversation } = remote;
  const listRef = useRef<FlatList<TimelineItem>>(null);
  const [message, setMessage] = useState("");
  const [selector, setSelector] = useState<"model" | "thinking" | null>(null);
  const [showOffline, setShowOffline] = useState(false);
  const [atLatest, setAtLatest] = useState(true);
  const title = remote.draft ? t("chat.new") : remote.selectedTitle || t("sessions.unnamed");
  const activeModel = remote.models.find(model => modelReference(model) === remote.modelId);
  const activeModelLabel =
    activeModel?.label || activeModel?.id || remote.modelId || t("chat.model");

  useEffect(() => {
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      closeConversation();
      return true;
    });
    return () => subscription.remove();
  }, [closeConversation]);

  useEffect(() => {
    const timer = setTimeout(
      () => setShowOffline(!remote.desktopOnline),
      remote.desktopOnline ? 0 : 3_000,
    );
    return () => clearTimeout(timer);
  }, [remote.desktopOnline]);

  const send = async () => {
    const value = message.trim();
    if (!value) return;
    setMessage("");
    try {
      await remote.sendMessage(value);
    } catch {
      setMessage(value);
    }
  };

  const scrollToLatest = () => {
    setAtLatest(true);
    listRef.current?.scrollToEnd({ animated: true });
    requestAnimationFrame(() => listRef.current?.scrollToEnd({ animated: true }));
  };

  return (
    <SafeAreaView edges={["top", "bottom"]} style={styles.safe}>
      <KeyboardAvoidingView
        behavior={Platform.OS === "ios" ? "padding" : undefined}
        style={styles.keyboard}
      >
        <View style={styles.topbar}>
          <Pressable
            accessibilityLabel={t("common.back")}
            accessibilityRole="button"
            onPress={closeConversation}
            style={styles.iconButton}
          >
            <ArrowLeft color={colors.ink} size={22} />
          </Pressable>
          <View style={styles.titleWrap}>
            <Text numberOfLines={1} style={styles.title}>
              {title}
            </Text>
          </View>
          <View style={styles.iconButton} />
        </View>

        <View style={styles.chatContent}>
          <FlatList
            contentContainerStyle={[
              styles.timeline,
              remote.timeline.items.length === 0 && styles.emptyTimeline,
            ]}
            data={remote.timeline.items}
            keyExtractor={item => item.id}
            ListEmptyComponent={<Text style={styles.empty}>{t("chat.noHistory")}</Text>}
            onContentSizeChange={() => {
              if (atLatest) listRef.current?.scrollToEnd({ animated: true });
            }}
            onScroll={event => {
              const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
              setAtLatest(contentOffset.y + layoutMeasurement.height >= contentSize.height - 24);
            }}
            ref={listRef}
            renderItem={({ item }) => (
              <TimelineCard item={item} onDecision={remote.decideApproval} />
            )}
            scrollEventThrottle={16}
            scrollIndicatorInsets={{ bottom: 0 }}
            style={styles.timelineList}
            ItemSeparatorComponent={() => <View style={styles.itemGap} />}
          />

          <View style={styles.composerDock}>
            <View pointerEvents="none" style={styles.composerFade}>
              <Svg height="100%" width="100%">
                <Defs>
                  <LinearGradient id="composerFade" x1="0" x2="0" y1="0" y2="1">
                    <Stop offset="0" stopColor={colors.surface} stopOpacity="0" />
                    <Stop offset="1" stopColor={colors.surface} stopOpacity="0.96" />
                  </LinearGradient>
                </Defs>
                <Rect fill="url(#composerFade)" height="100%" width="100%" />
              </Svg>
            </View>
            {!atLatest && (
              <Pressable
                accessibilityLabel={t("chat.backToLatest")}
                accessibilityRole="button"
                onPress={scrollToLatest}
                style={styles.backToLatest}
              >
                <ArrowDown color={colors.inkSoft} size={16} />
                <Text style={styles.backToLatestText}>{t("chat.backToLatest")}</Text>
              </Pressable>
            )}
            {showOffline && (
              <Text style={styles.offlineComposer}>{t("connection.offlineHint")}</Text>
            )}
            <View style={styles.composerArea}>
              <View style={styles.composer}>
                <TextInput
                  accessibilityLabel={t("chat.placeholder")}
                  editable={remote.desktopOnline && !remote.timeline.streaming && !remote.busy}
                  multiline
                  onChangeText={setMessage}
                  onSubmitEditing={() => void send()}
                  placeholder={t("chat.placeholder")}
                  placeholderTextColor={colors.inkMuted}
                  style={styles.input}
                  value={message}
                />
                <View style={styles.composerToolbar}>
                  <View style={styles.composerSelectors}>
                    <Pressable
                      accessibilityLabel={t("chat.model")}
                      accessibilityRole="button"
                      disabled={remote.timeline.streaming}
                      onPress={() => setSelector("model")}
                      style={({ pressed }) => [
                        styles.selectorTrigger,
                        pressed && styles.selectorTriggerPressed,
                        remote.timeline.streaming && styles.controlDisabled,
                      ]}
                    >
                      <Text numberOfLines={1} style={styles.selectorText}>
                        {activeModelLabel}
                      </Text>
                      <ChevronDown color={colors.inkMuted} size={14} />
                    </Pressable>
                    <Pressable
                      accessibilityLabel={t("chat.thinkingLevel")}
                      accessibilityRole="button"
                      disabled={remote.timeline.streaming}
                      onPress={() => setSelector("thinking")}
                      style={({ pressed }) => [
                        styles.selectorTrigger,
                        pressed && styles.selectorTriggerPressed,
                        remote.timeline.streaming && styles.controlDisabled,
                      ]}
                    >
                      <Text numberOfLines={1} style={styles.selectorText}>
                        {t(`thinking.${remote.thinkingLevel}`)}
                      </Text>
                      <ChevronDown color={colors.inkMuted} size={14} />
                    </Pressable>
                  </View>
                  {remote.timeline.streaming ? (
                    <Pressable
                      accessibilityLabel={t("chat.stop")}
                      accessibilityRole="button"
                      onPress={() => void remote.abort()}
                      style={[styles.sendButton, styles.stopButton]}
                    >
                      <Square color={colors.surface} fill={colors.surface} size={14} />
                    </Pressable>
                  ) : (
                    <Pressable
                      accessibilityLabel={t("chat.send")}
                      accessibilityRole="button"
                      disabled={!message.trim() || remote.busy || !remote.desktopOnline}
                      onPress={() => void send()}
                      style={({ pressed }) => [
                        styles.sendButton,
                        (!message.trim() || remote.busy || !remote.desktopOnline) &&
                          styles.sendDisabled,
                        pressed && styles.sendPressed,
                      ]}
                    >
                      <Send color={colors.surface} size={17} />
                    </Pressable>
                  )}
                </View>
              </View>
            </View>
          </View>
        </View>

        <Modal
          animationType="fade"
          onRequestClose={() => setSelector(null)}
          transparent
          visible={selector !== null}
        >
          <TouchableWithoutFeedback onPress={() => setSelector(null)}>
            <View style={styles.selectorOverlay}>
              <TouchableWithoutFeedback>
                <View style={styles.selectorMenu}>
                  <Text style={styles.selectorTitle}>
                    {selector === "model" ? t("chat.model") : t("chat.thinkingLevel")}
                  </Text>
                  <ScrollView bounces={false}>
                    {selector === "model"
                      ? remote.models.map(model => {
                          const selected = modelReference(model) === remote.modelId;
                          return (
                            <Pressable
                              key={`${model.provider ?? ""}/${model.id}`}
                              onPress={() => {
                                setSelector(null);
                                void remote.setModel(modelReference(model));
                              }}
                              style={({ pressed }) => [
                                styles.selectorOption,
                                selected && styles.selectorOptionSelected,
                                pressed && styles.selectorOptionPressed,
                              ]}
                            >
                              <View style={styles.selectorOptionCopy}>
                                <Text numberOfLines={1} style={styles.selectorOptionLabel}>
                                  {model.label || model.id}
                                </Text>
                                {model.provider ? (
                                  <Text numberOfLines={1} style={styles.selectorOptionMeta}>
                                    {model.provider}
                                  </Text>
                                ) : null}
                              </View>
                              {selected ? <Check color={colors.accent} size={18} /> : null}
                            </Pressable>
                          );
                        })
                      : thinkingLevels.map(level => {
                          const selected = level === remote.thinkingLevel;
                          return (
                            <Pressable
                              key={level}
                              onPress={() => {
                                setSelector(null);
                                void remote.setThinkingLevel(level);
                              }}
                              style={({ pressed }) => [
                                styles.selectorOption,
                                selected && styles.selectorOptionSelected,
                                pressed && styles.selectorOptionPressed,
                              ]}
                            >
                              <Text style={styles.selectorOptionLabel}>
                                {t(`thinking.${level}`)}
                              </Text>
                              {selected ? <Check color={colors.accent} size={18} /> : null}
                            </Pressable>
                          );
                        })}
                  </ScrollView>
                </View>
              </TouchableWithoutFeedback>
            </View>
          </TouchableWithoutFeedback>
        </Modal>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.surface },
  keyboard: { flex: 1, backgroundColor: colors.surface },
  topbar: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: spacing.md,
    borderBottomWidth: 1,
    borderBottomColor: colors.lineSoft,
    backgroundColor: colors.surface,
  },
  iconButton: {
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  titleWrap: { flex: 1, alignItems: "center" },
  title: { color: colors.inkStrong, fontSize: 16, fontWeight: "700", maxWidth: "90%" },
  controlDisabled: { opacity: 0.5 },
  chatContent: { flex: 1 },
  timelineList: { flex: 1 },
  timeline: { padding: spacing.lg, paddingBottom: 148 },
  emptyTimeline: { flexGrow: 1, alignItems: "center", justifyContent: "center" },
  empty: { color: colors.inkMuted, fontSize: 14 },
  itemGap: { height: spacing.md },
  offlineComposer: {
    marginHorizontal: spacing.md,
    marginBottom: spacing.xs,
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
    color: colors.warning,
    backgroundColor: colors.warningSoft,
    fontSize: 12,
    textAlign: "center",
  },
  composerDock: {
    position: "absolute",
    right: 0,
    bottom: 0,
    left: 0,
    backgroundColor: colors.surface,
  },
  composerFade: { position: "absolute", top: -80, right: 0, left: 0, height: 84 },
  backToLatest: {
    position: "absolute",
    top: -48,
    alignSelf: "center",
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderWidth: 1,
    borderColor: colors.lineSoft,
    borderRadius: radius.pill,
    backgroundColor: colors.surface,
    shadowColor: colors.inkStrong,
    shadowOpacity: 0.08,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 3 },
    elevation: 3,
  },
  backToLatestText: { color: colors.inkSoft, fontSize: 13, fontWeight: "600" },
  composerArea: {
    paddingHorizontal: spacing.md,
    paddingTop: spacing.xs,
    paddingBottom: spacing.md,
    backgroundColor: "transparent",
  },
  composer: {
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
    shadowColor: colors.inkStrong,
    shadowOpacity: 0.1,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 6 },
    elevation: 4,
  },
  input: {
    minHeight: 56,
    maxHeight: 160,
    color: colors.ink,
    fontSize: 15,
    lineHeight: 21,
    paddingHorizontal: spacing.md,
    paddingTop: spacing.xs,
    paddingBottom: spacing.sm,
    textAlignVertical: "top",
  },
  composerToolbar: {
    minHeight: 46,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    gap: spacing.sm,
    paddingHorizontal: spacing.md,
    paddingBottom: spacing.md,
  },
  composerSelectors: {
    minWidth: 0,
    flexGrow: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
  },
  selectorTrigger: {
    minWidth: 0,
    maxWidth: 154,
    height: 34,
    flexDirection: "row",
    alignItems: "center",
    gap: 3,
    paddingHorizontal: spacing.sm,
    borderRadius: radius.sm,
  },
  selectorTriggerPressed: { backgroundColor: colors.surfaceSubtle },
  selectorText: { flexShrink: 1, color: colors.inkSoft, fontSize: 12, fontWeight: "600" },
  sendButton: {
    width: 38,
    height: 38,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.sm,
    backgroundColor: colors.accent,
  },
  stopButton: { backgroundColor: colors.danger },
  sendDisabled: { backgroundColor: colors.accentDisabled },
  sendPressed: { opacity: 0.78 },
  selectorOverlay: {
    flex: 1,
    justifyContent: "flex-end",
    padding: spacing.md,
    paddingBottom: spacing.xl,
    backgroundColor: colors.overlay,
  },
  selectorMenu: {
    maxHeight: "60%",
    overflow: "hidden",
    padding: spacing.sm,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
  },
  selectorTitle: {
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.sm,
    color: colors.inkMuted,
    fontSize: 12,
    fontWeight: "700",
  },
  selectorOption: {
    minHeight: 50,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
  },
  selectorOptionSelected: { backgroundColor: colors.accentSoft },
  selectorOptionPressed: { opacity: 0.72 },
  selectorOptionCopy: { minWidth: 0, flex: 1 },
  selectorOptionLabel: { color: colors.ink, fontSize: 14, fontWeight: "600" },
  selectorOptionMeta: { marginTop: 2, color: colors.inkMuted, fontSize: 11 },
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
  nameInput: {
    minHeight: 48,
    paddingHorizontal: spacing.md,
    color: colors.ink,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
  },
  dialogActions: { flexDirection: "row", gap: spacing.md },
  dialogAction: { flex: 1 },
});
