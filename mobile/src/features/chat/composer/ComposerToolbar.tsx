import { ChevronDown, Paperclip, Send, Slash, Square } from "lucide-react-native";
import { ActivityIndicator, Pressable, Text, View } from "react-native";
import type { TFunction } from "i18next";
import type { RemoteControls } from "../../../remote/RemoteContext";
import { colors } from "../../../theme/tokens";
import { useComposerCompletion } from "../useComposerCompletion";
import { useStopRequest } from "../useStopRequest";
import { styles } from "./composerStyles";
interface Props {
  remote: RemoteControls;
  t: TFunction;
  compactToolbar: boolean;
  compacting: boolean;
  activeModelLabel: string;
  selector: "model" | "thinking" | null;
  setSelector: (value: "model" | "thinking" | null) => void;
  completion: Pick<ReturnType<typeof useComposerCompletion>, "query" | "insertSlash">;
  editable: boolean;
  openAttachmentMenu: () => void;
  stopRequest: ReturnType<typeof useStopRequest>;
  skillEvaluating: boolean;
  canSend: boolean;
  send: () => Promise<void>;
}

export function ComposerToolbar({ remote, t, compactToolbar, compacting, activeModelLabel, selector, setSelector, completion, editable, openAttachmentMenu, stopRequest, skillEvaluating, canSend, send }: Props) {
  return (
    <View
      style={[
        styles.composerToolbar,
        compactToolbar && styles.composerToolbarCompact,
      ]}
    >
      <View
        style={[
          styles.composerSelectors,
          compactToolbar && styles.composerSelectorsCompact,
        ]}
      >
        <Pressable
          accessibilityLabel={`${t("chat.model")}: ${activeModelLabel}`}
          accessibilityRole="button"
          accessibilityState={{
            expanded: selector === "model",
            disabled:
              remote.streaming || compacting ||
              remote.connectionPresentation.level !== "connected",
          }}
          disabled={
            remote.streaming || compacting ||
            remote.connectionPresentation.level !== "connected"
          }
          onPress={() => setSelector("model")}
          style={({ pressed }) => [
            styles.selectorTrigger,
            styles.modelTrigger,
            compactToolbar && styles.selectorTriggerCompact,
            pressed && styles.selectorTriggerPressed,
            (remote.streaming || compacting) && styles.controlDisabled,
          ]}
        >
          <Text numberOfLines={1} style={styles.selectorText}>
            {activeModelLabel}
          </Text>
          <ChevronDown color={colors.inkMuted} size={14} />
        </Pressable>
        <Pressable
          accessibilityLabel={`${t("chat.thinkingLevel")}: ${t(`thinking.${remote.thinkingLevel}`)}`}
          accessibilityRole="button"
          accessibilityState={{
            expanded: selector === "thinking",
            disabled:
              remote.streaming || compacting ||
              remote.connectionPresentation.level !== "connected",
          }}
          disabled={
            remote.streaming || compacting ||
            remote.connectionPresentation.level !== "connected"
          }
          onPress={() => setSelector("thinking")}
          style={({ pressed }) => [
            styles.selectorTrigger,
            styles.thinkingTrigger,
            compactToolbar && styles.selectorTriggerCompact,
            pressed && styles.selectorTriggerPressed,
            (remote.streaming || compacting) && styles.controlDisabled,
          ]}
        >
          <Text numberOfLines={1} style={styles.selectorText}>
            {t(`thinking.${remote.thinkingLevel}`)}
          </Text>
          <ChevronDown color={colors.inkMuted} size={14} />
        </Pressable>
      </View>
      <Pressable
        accessibilityLabel={t("skills.choose")}
        accessibilityRole="button"
        accessibilityState={{
          expanded: !!completion.query,
          disabled: !editable,
        }}
        disabled={!editable}
        onPress={completion.insertSlash}
        hitSlop={{ left: 6, right: 6 }}
        style={({ pressed }) => [
          styles.attachmentButton,
          styles.skillButton,
          (pressed || !!completion.query) &&
          styles.selectorTriggerPressed,
          !editable && styles.controlDisabled,
        ]}
      >
        <Slash
          color={completion.query ? colors.accent : colors.inkSoft}
          size={16}
        />
      </Pressable>
      <Pressable
        accessibilityLabel={t("attachment.add")}
        accessibilityRole="button"
        disabled={
          !editable || remote.streaming || !remote.fileTransferSupported
        }
        onPress={openAttachmentMenu}
        style={({ pressed }) => [
          styles.attachmentButton,
          pressed && styles.selectorTriggerPressed,
          (!editable || remote.streaming ||
            !remote.fileTransferSupported) &&
          styles.controlDisabled,
        ]}
      >
        <Paperclip color={colors.inkSoft} size={17} />
      </Pressable>
      {remote.streaming ? (
        <Pressable
          accessibilityLabel={t(
            stopRequest.status === "requesting" ? "chat.stopping" : "chat.stop",
          )}
          accessibilityRole="button"
          accessibilityState={{
            busy: stopRequest.status === "requesting",
            disabled: stopRequest.status === "requesting",
          }}
          disabled={stopRequest.status === "requesting"}
          onPress={() => void stopRequest.stop()}
          android_ripple={{ color: "#ffffff40" }}
          style={({ pressed }) => [
            styles.sendButton,
            styles.stopButton,
            pressed && styles.sendPressed,
          ]}
        >
          {stopRequest.status === "requesting" ? (
            <ActivityIndicator color={colors.surface} size="small" />
          ) : (
            <Square color={colors.surface} fill={colors.surface} size={14} />
          )}
        </Pressable>
      ) : (
        <Pressable
          accessibilityLabel={t(
            skillEvaluating
              ? "chat.findingSkill"
              : compacting ? "chat.compacting" : "chat.send",
          )}
          accessibilityRole="button"
          accessibilityState={{ disabled: !canSend, busy: compacting || skillEvaluating }}
          disabled={!canSend}
          onPress={() => { if (canSend) void send(); }}
          style={({ pressed }) => [
            styles.sendButton,
            !canSend && styles.sendDisabled,
            pressed && styles.sendPressed,
          ]}
        >
          {compacting || skillEvaluating ? (
            <ActivityIndicator color={colors.surface} size="small" />
          ) : (
            <Send color={colors.surface} size={17} />
          )}
        </Pressable>
      )}
    </View>
  );
}
