import { FUSION_PHASES, fusionProgress, type FusionDiscussion } from "@future-os/thread-projection";
import { Check, ChevronDown, ChevronUp, Circle, Clock, LoaderCircle, TriangleAlert, UserRoundGroup } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AppState, Pressable, Text, View } from "react-native";
import type { TimelineToolRow } from "../../../remote/types";
import { useTheme } from "../../../theme/ThemeContext";
import type { ToolTargetResolver } from "./types";
import { useStyles } from "./messageStyles";

export function FusionToolRow({ tool, resolveTarget }: { tool: TimelineToolRow; resolveTarget?: ToolTargetResolver }) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [data, setData] = useState<FusionDiscussion | null>(null);
  const [error, setError] = useState(false);
  const progress = fusionProgress(data, tool.status);
  useEffect(() => {
    if (!open) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      if (!active) return;
      if (AppState.currentState !== "active") { timer = setTimeout(() => void load(), 1500); return; }
      try {
        if (!resolveTarget || !tool.toolCallId || !tool.runId) throw new Error("missing_fusion_source");
        const raw = await resolveTarget(tool.toolCallId, tool.runId, {});
        if (!active) return;
        if (!raw) throw new Error("missing_fusion_status");
        const value = JSON.parse(raw) as FusionDiscussion;
        setData(value); setError(false);
        if (!fusionProgress(value, tool.status).terminal) timer = setTimeout(() => void load(), 1500);
      } catch {
        if (active) { setError(true); timer = setTimeout(() => void load(), 3000); }
      }
    };
    void load();
    return () => { active = false; clearTimeout(timer); };
  }, [open, resolveTarget, tool.toolCallId, tool.runId, tool.status]);
  return <View style={styles.inlineTool}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: open }} onPress={() => setOpen(value => !value)} style={styles.toolHeader}>
      <UserRoundGroup size={14} color={colors.inkMuted} />
      <Text style={styles.toolText}>{t("chat.fusionTitle")}</Text>
      {open ? <ChevronUp size={14} color={colors.inkMuted} /> : <ChevronDown size={14} color={colors.inkMuted} />}
    </Pressable>
    {open ? <View style={styles.fusionProgress}>
      <View style={styles.fusionStageHeading}>
        <Text style={styles.fusionPhase}>{t(progress.terminal ? `chat.fusionStates.${progress.status}` : `chat.fusionPhases.${progress.phase}`)}</Text>
        <Text style={styles.fusionStatus}>{t("chat.fusionStageCount", { current: progress.status === "completed" ? 4 : Math.max(1, progress.stageIndex + 1), total: 4 })}</Text>
      </View>
      <View style={styles.fusionStageBar} accessibilityRole="progressbar" accessibilityLabel={t("chat.fusionProgress")} aria-valuemin={0} aria-valuemax={4} aria-valuenow={progress.completedStages} aria-valuetext={t(`chat.fusionPhases.${progress.phase}`)}>
        {FUSION_PHASES.map((phase, index) => <View key={phase} style={[styles.fusionStageSegment, { backgroundColor: index < progress.completedStages ? colors.accent : index === progress.stageIndex ? progress.terminal ? colors.danger : colors.accent : colors.line }]} />)}
      </View>
      {progress.terminal && progress.partial ? <Text style={styles.fusionStatus}>{t("chat.fusionPartialNotice")}</Text> : null}
      {progress.models.map(model => {
        const failed = ["failed", "timed_out", "output_limited"].includes(model.status);
        const Icon = model.status === "thinking" ? LoaderCircle : model.status === "completed" ? Check : model.status === "timed_out" ? Clock : failed || model.status === "partial" ? TriangleAlert : Circle;
        return <View key={model.id} style={styles.fusionModelRow}>
          <Icon size={14} color={failed ? colors.danger : model.status === "thinking" ? colors.accent : colors.inkMuted} />
          <Text style={styles.fusionModelName}>{model.role ? `${t(model.role === "moderator" ? "chat.fusionModeratorRole" : "chat.fusionParticipantRole", { number: model.number })} · ` : ""}{model.label}</Text>
          <Text style={[styles.fusionStatus, failed ? { color: colors.danger } : null]}>{t(`chat.fusionModelStates.${model.status}`)}</Text>
        </View>;
      })}
      {error ? <Text accessibilityRole="alert" style={[styles.fusionStatus, { color: colors.danger }]}>{t("chat.fusionStatusUnavailable")}</Text> : null}
    </View> : null}
  </View>;
}
