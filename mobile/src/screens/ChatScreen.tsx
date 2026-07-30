import { ArrowDown, ArrowLeft, Check, ChevronDown, Send, Square } from "lucide-react-native";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ActivityIndicator,
  BackHandler,
  FlatList,
  Keyboard,
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
import { SafeAreaView, useSafeAreaInsets } from "react-native-safe-area-context";
import Svg, { Defs, LinearGradient, Rect, Stop } from "react-native-svg";
import { PendingApprovalCard, TimelineCard } from "../components/TimelineCard";
import { useRemote } from "../remote/RemoteContext";
import { modelReference, type ThinkingLevel, type TimelineItem } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";

const thinkingLevels: ThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh"];

// How close to the bottom counts as "at latest" (px). Shared by the atLatest
// detection and the scroll target so the two never disagree.
const AT_LATEST_THRESHOLD = 32;

export function ChatScreen() {
  const { t } = useTranslation();
  const remote = useRemote();
  const { closeConversation } = remote;
  const insets = useSafeAreaInsets();
  const listRef = useRef<FlatList<TimelineItem>>(null);
  const [message, setMessage] = useState("");
  const [selector, setSelector] = useState<"model" | "thinking" | null>(null);
  const [showOffline, setShowOffline] = useState(false);
  const [atLatest, setAtLatest] = useState(true);
  // Measured height of the floating composer dock — the list's bottom padding
  // tracks it so the last message always lands just above the composer (no
  // gap, no hidden content) regardless of docked approvals or input height.
  const [composerHeight, setComposerHeight] = useState(0);
  // Android edge-to-edge: the built-in KeyboardAvoidingView is a no-op here
  // (behavior is undefined on Android) and RN's KAV mis-measures the keyboard
  // under edge-to-edge, so we lift the floating composer + list padding by the
  // measured keyboard height ourselves. iOS keeps relying on KAV padding.
  const [keyboardHeight, setKeyboardHeight] = useState(0);
  const contentHeightRef = useRef(0);
  const layoutHeightRef = useRef(0);
  const title = remote.draft ? t("chat.new") : remote.selectedTitle || t("sessions.unnamed");
  const activeModel = remote.models.find(model => modelReference(model) === remote.modelId);
  const activeModelLabel =
    activeModel?.label || activeModel?.id || remote.modelId || t("chat.model");

  // Approvals live docked above the composer (not inline in the transcript), and
  // only while undecided — once a decision lands the card disappears.
  const timelineItems = useMemo(() => remote.timeline.items, [remote.timeline]);
  const transcriptItems = useMemo(
    () => timelineItems.filter(item => item.kind !== "approval"),
    [timelineItems],
  );
  const pendingApprovals = useMemo(
    () =>
      timelineItems.filter(
        (item): item is Extract<TimelineItem, { kind: "approval" }> =>
          item.kind === "approval" && !item.decision,
      ),
    [timelineItems],
  );
  const [approvalSubmitting, setApprovalSubmitting] = useState<string | null>(null);
  const [approvalError, setApprovalError] = useState<string | null>(null);
  // History load is in flight (selectSession holds busy until it lands) — show
  // a spinner instead of flashing the "no history" empty state.
  const loadingHistory = !remote.draft && remote.busy && timelineItems.length === 0;
  // The first content render snaps to the end without animation; only later
  // appends (streaming, new messages) scroll animated.
  const landedRef = useRef(false);
  const decideApproval = useCallback(
    async (id: string, decision: "approved" | "rejected") => {
      setApprovalSubmitting(id);
      setApprovalError(null);
      try {
        await remote.decideApproval(id, decision);
      } catch {
        setApprovalError(t("approval.submitFailed"));
      } finally {
        setApprovalSubmitting(null);
      }
    },
    [remote, t],
  );

  useEffect(() => {
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      closeConversation();
      return true;
    });
    return () => subscription.remove();
  }, [closeConversation]);

  // Keyboard compensation is Android-only: iOS already adapts via KAV padding,
  // and stacking both would double-shift the composer. We subtract the bottom
  // safe-area inset because SafeAreaView's bottom edge already lifts the content
  // above the nav bar — without the subtraction the composer would float a
  // nav-bar-height gap above the keyboard.
  useEffect(() => {
    if (Platform.OS !== "android") return;
    const showSub = Keyboard.addListener("keyboardDidShow", event =>
      setKeyboardHeight(event.endCoordinates.height),
    );
    const hideSub = Keyboard.addListener("keyboardDidHide", () => setKeyboardHeight(0));
    return () => {
      showSub.remove();
      hideSub.remove();
    };
  }, []);

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

  // The bottom-most scroll offset: full content height minus the viewport,
  // never negative. Recomputed from measured sizes so it stays correct as the
  // composer (bottom padding) and content grow.
  const maxScrollOffset = () => Math.max(0, contentHeightRef.current - layoutHeightRef.current);

  // scrollToEnd is unreliable on Android for the first layout of a large
  // history (it can no-op or use a stale content size, leaving a gap). An
  // explicit offset computed from the measured content size is deterministic;
  // the rAF retry catches content that finishes laying out a frame late.
  const scrollToLatest = () => {
    setAtLatest(true);
    listRef.current?.scrollToOffset({ animated: true, offset: maxScrollOffset() });
    requestAnimationFrame(() =>
      listRef.current?.scrollToOffset({ animated: true, offset: maxScrollOffset() }),
    );
  };

  // Lift the composer so the floating *card* clears the keyboard, not the dock:
  // the dock's bottom edge includes composerArea's paddingBottom (spacing.md) of
  // empty space below the card, so lifting the dock flush to the keyboard top
  // still hides the card's bottom border + rounded corners + shadow by that
  // padding (exactly the clipping seen with Gboard). Re-add the padding, plus a
  // little shadow clearance, so the whole card floats above the IME.
  const KEYBOARD_CLEARANCE = spacing.md + spacing.sm;
  const keyboardLift =
    Platform.OS === "android" && keyboardHeight > 0
      ? Math.max(0, keyboardHeight - insets.bottom + KEYBOARD_CLEARANCE)
      : 0;

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
              { paddingBottom: composerHeight + spacing.lg + keyboardLift },
              timelineItems.length === 0 && styles.emptyTimeline,
            ]}
            data={transcriptItems}
            keyExtractor={item => item.id}
            ListEmptyComponent={
              loadingHistory ? (
                <ActivityIndicator color={colors.accent} />
              ) : (
                <Text style={styles.empty}>{t("chat.noHistory")}</Text>
              )
            }
            onContentSizeChange={(_w, h) => {
              contentHeightRef.current = h;
              if (!atLatest) return;
              if (!landedRef.current && transcriptItems.length === 0) return;
              listRef.current?.scrollToOffset({
                animated: landedRef.current,
                offset: maxScrollOffset(),
              });
              landedRef.current = true;
            }}
            onLayout={event => {
              layoutHeightRef.current = event.nativeEvent.layout.height;
            }}
            onScroll={event => {
              const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
              setAtLatest(
                contentOffset.y + layoutMeasurement.height >=
                  contentSize.height - AT_LATEST_THRESHOLD,
              );
            }}
            ref={listRef}
            renderItem={({ item }) => <TimelineCard item={item} />}
            scrollEventThrottle={16}
            scrollIndicatorInsets={{ bottom: 0 }}
            style={styles.timelineList}
            ItemSeparatorComponent={() => <View style={styles.itemGap} />}
          />

          <View
            onLayout={event => setComposerHeight(event.nativeEvent.layout.height)}
            style={[styles.composerDock, keyboardLift > 0 ? { bottom: keyboardLift } : null]}
          >
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
            {pendingApprovals.map(item => (
              <View key={item.id} style={styles.dockedApproval}>
                <PendingApprovalCard
                  error={
                    approvalSubmitting === item.payload.approval_request_id ? null : approvalError
                  }
                  onDecision={decision =>
                    void decideApproval(item.payload.approval_request_id, decision)
                  }
                  payload={item.payload}
                  submitting={approvalSubmitting === item.payload.approval_request_id}
                />
              </View>
            ))}
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
  timeline: { padding: spacing.lg },
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
  dockedApproval: {
    marginHorizontal: spacing.md,
    marginBottom: spacing.sm,
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
    paddingHorizontal: spacing.lg,
    paddingTop: spacing.md,
    paddingBottom: spacing.md,
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
