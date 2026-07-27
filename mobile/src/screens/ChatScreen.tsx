import { Picker } from "@react-native-picker/picker";
import { ArrowLeft, Pencil, Send, Square } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  BackHandler,
  FlatList,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  SafeAreaView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { Button } from "../components/Button";
import { ConnectionBadge } from "../components/ConnectionBadge";
import { TimelineCard } from "../components/TimelineCard";
import { useRemote } from "../remote/RemoteContext";
import type { ThinkingLevel, TimelineItem } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";

const thinkingLevels: ThinkingLevel[] = ["off", "low", "medium", "high"];

export function ChatScreen() {
  const { t } = useTranslation();
  const remote = useRemote();
  const { closeConversation } = remote;
  const listRef = useRef<FlatList<TimelineItem>>(null);
  const [message, setMessage] = useState("");
  const [renameOpen, setRenameOpen] = useState(false);
  const [name, setName] = useState("");
  const title = remote.draft ? t("chat.new") : remote.selectedTitle || t("sessions.unnamed");

  useEffect(() => {
    const subscription = BackHandler.addEventListener("hardwareBackPress", () => {
      closeConversation();
      return true;
    });
    return () => subscription.remove();
  }, [closeConversation]);

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

  const saveName = async () => {
    await remote.rename(name);
    setRenameOpen(false);
  };

  return (
    <SafeAreaView style={styles.safe}>
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
            <ConnectionBadge phase={remote.phase} desktopOnline={remote.desktopOnline} />
          </View>
          {!remote.draft ? (
            <Pressable
              accessibilityLabel={t("chat.rename")}
              accessibilityRole="button"
              onPress={() => {
                setName(title);
                setRenameOpen(true);
              }}
              style={styles.iconButton}
            >
              <Pencil color={colors.inkSoft} size={19} />
            </Pressable>
          ) : (
            <View style={styles.iconButton} />
          )}
        </View>

        <View style={styles.controls}>
          <View style={styles.model}>
            <Text style={styles.controlLabel}>{t("chat.model")}</Text>
            <View style={styles.pickerWrap}>
              <Picker
                dropdownIconColor={colors.inkSoft}
                enabled={!remote.timeline.streaming}
                onValueChange={value => void remote.setModel(String(value))}
                selectedValue={remote.modelId}
                style={styles.picker}
              >
                {remote.models.map(model => (
                  <Picker.Item
                    key={model.id}
                    label={`${model.provider ? `${model.provider}/` : ""}${model.label || model.id}`}
                    value={model.id}
                  />
                ))}
              </Picker>
            </View>
          </View>
          <View>
            <Text style={styles.controlLabel}>{t("chat.thinkingLevel")}</Text>
            <View style={styles.levels}>
              {thinkingLevels.map(level => (
                <Pressable
                  accessibilityRole="button"
                  disabled={remote.timeline.streaming}
                  key={level}
                  onPress={() => void remote.setThinkingLevel(level)}
                  style={[
                    styles.level,
                    remote.thinkingLevel === level && styles.levelActive,
                    remote.timeline.streaming && styles.controlDisabled,
                  ]}
                >
                  <Text
                    style={[
                      styles.levelText,
                      remote.thinkingLevel === level && styles.levelTextActive,
                    ]}
                  >
                    {t(`thinking.${level}`)}
                  </Text>
                </Pressable>
              ))}
            </View>
          </View>
        </View>

        <FlatList
          contentContainerStyle={[
            styles.timeline,
            remote.timeline.items.length === 0 && styles.emptyTimeline,
          ]}
          data={remote.timeline.items}
          keyExtractor={item => item.id}
          ListEmptyComponent={<Text style={styles.empty}>{t("chat.noHistory")}</Text>}
          onContentSizeChange={() => listRef.current?.scrollToEnd({ animated: true })}
          ref={listRef}
          renderItem={({ item }) => <TimelineCard item={item} onDecision={remote.decideApproval} />}
          ItemSeparatorComponent={() => <View style={styles.itemGap} />}
        />

        {!remote.desktopOnline && (
          <Text style={styles.offlineComposer}>{t("connection.offlineHint")}</Text>
        )}
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
          {remote.timeline.streaming ? (
            <Pressable
              accessibilityLabel={t("chat.stop")}
              accessibilityRole="button"
              onPress={() => void remote.abort()}
              style={[styles.sendButton, styles.stopButton]}
            >
              <Square color={colors.surface} fill={colors.surface} size={17} />
            </Pressable>
          ) : (
            <Pressable
              accessibilityLabel={t("chat.send")}
              accessibilityRole="button"
              disabled={!message.trim() || remote.busy || !remote.desktopOnline}
              onPress={() => void send()}
              style={({ pressed }) => [
                styles.sendButton,
                (!message.trim() || remote.busy || !remote.desktopOnline) && styles.sendDisabled,
                pressed && styles.sendPressed,
              ]}
            >
              <Send color={colors.surface} size={19} />
            </Pressable>
          )}
        </View>

        <Modal
          animationType="fade"
          onRequestClose={() => setRenameOpen(false)}
          transparent
          visible={renameOpen}
        >
          <View style={styles.overlay}>
            <View style={styles.dialog}>
              <Text style={styles.dialogTitle}>{t("chat.renameTitle")}</Text>
              <TextInput
                autoFocus
                onChangeText={setName}
                selectTextOnFocus
                style={styles.nameInput}
                value={name}
              />
              <View style={styles.dialogActions}>
                <View style={styles.dialogAction}>
                  <Button
                    label={t("chat.cancel")}
                    onPress={() => setRenameOpen(false)}
                    variant="secondary"
                  />
                </View>
                <View style={styles.dialogAction}>
                  <Button
                    disabled={!name.trim()}
                    label={t("chat.save")}
                    onPress={() => void saveName()}
                  />
                </View>
              </View>
            </View>
          </View>
        </Modal>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.canvas },
  keyboard: { flex: 1 },
  topbar: {
    minHeight: 70,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: spacing.md,
    borderBottomWidth: 1,
    borderBottomColor: colors.lineSoft,
    backgroundColor: colors.surface,
  },
  iconButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  titleWrap: { flex: 1, alignItems: "center", gap: 5 },
  title: { color: colors.inkStrong, fontSize: 16, fontWeight: "700", maxWidth: "90%" },
  controls: {
    gap: spacing.md,
    padding: spacing.md,
    borderBottomWidth: 1,
    borderBottomColor: colors.lineSoft,
    backgroundColor: colors.surface,
  },
  model: { gap: spacing.xs },
  controlLabel: { color: colors.inkMuted, fontSize: 11, fontWeight: "600" },
  pickerWrap: {
    height: 42,
    justifyContent: "center",
    overflow: "hidden",
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.sm,
    backgroundColor: colors.surface,
  },
  picker: { color: colors.ink, height: 50 },
  levels: { flexDirection: "row", gap: spacing.sm, marginTop: spacing.xs },
  level: {
    flex: 1,
    alignItems: "center",
    paddingVertical: 8,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.sm,
    backgroundColor: colors.surface,
  },
  levelActive: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
  levelText: { color: colors.inkSoft, fontSize: 12, fontWeight: "600" },
  levelTextActive: { color: colors.accent },
  controlDisabled: { opacity: 0.5 },
  timeline: { padding: spacing.lg, paddingBottom: spacing.xl },
  emptyTimeline: { flexGrow: 1, alignItems: "center", justifyContent: "center" },
  empty: { color: colors.inkMuted, fontSize: 14 },
  itemGap: { height: spacing.md },
  offlineComposer: {
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.sm,
    color: colors.warning,
    backgroundColor: colors.warningSoft,
    fontSize: 12,
    textAlign: "center",
  },
  composer: {
    flexDirection: "row",
    alignItems: "flex-end",
    gap: spacing.sm,
    padding: spacing.md,
    paddingBottom: spacing.lg,
    borderTopWidth: 1,
    borderTopColor: colors.lineSoft,
    backgroundColor: colors.surface,
  },
  input: {
    flex: 1,
    minHeight: 46,
    maxHeight: 130,
    color: colors.ink,
    fontSize: 15,
    lineHeight: 21,
    paddingHorizontal: spacing.md,
    paddingVertical: 11,
    borderWidth: 1,
    borderColor: colors.line,
    borderRadius: radius.md,
    backgroundColor: colors.canvas,
  },
  sendButton: {
    width: 46,
    height: 46,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
    backgroundColor: colors.accent,
  },
  stopButton: { backgroundColor: colors.danger },
  sendDisabled: { backgroundColor: colors.accentDisabled },
  sendPressed: { opacity: 0.78 },
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
