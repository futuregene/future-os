import { FUSION_PHASES, fusionProgress, type FusionDiscussion } from "@future-os/thread-projection";
import { Check, ChevronDown, ChevronUp, Circle, CircleMinus, CirclePause, CircleStop, CircleX, Clock, TriangleAlert, UserRoundGroup } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ActivityIndicator, AppState, Pressable, Text, View } from "react-native";
import Svg, { Circle as SvgCircle } from "react-native-svg";
import type { TimelineToolRow } from "../../../remote/types";
import { useTheme } from "../../../theme/ThemeContext";
import type { ToolTargetResolver } from "./types";
import { useStyles } from "./messageStyles";

const RING_CIRCUMFERENCE = 2 * Math.PI * 20;

export function FusionToolRow({ tool, resolveTarget }: { tool: TimelineToolRow; resolveTarget?: ToolTargetResolver }) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [retry, setRetry] = useState(0);
  const scope = `${tool.runId}:${tool.toolCallId}`;
  const [resource, setResource] = useState<{ scope: string; discussion: FusionDiscussion } | null>(null);
  const data = resource?.scope === scope ? resource.discussion : null;
  const [error, setError] = useState(false);
  const progress = fusionProgress(data, tool.status);
  const currentStage = progress.status === "completed" ? 4 : Math.max(0, progress.stageIndex + 1);
  const stageLabel = t(progress.terminal ? `chat.fusionStates.${progress.status}` : `chat.fusionPhases.${progress.phase}`);
  const stageCount = t("chat.fusionStageCount", { current: currentStage, total: 4 });
  const models = progress.phase === "issue_mapping"
    ? progress.models.filter(model => model.role === "moderator" || (!model.role && data?.steps?.some(step => step.participant === "moderator" && step.model === model.model)))
    : progress.models;
  useEffect(() => {
    if (!resolveTarget || !tool.toolCallId || !tool.runId) return;
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
        setResource({ scope, discussion: value }); setError(false);
        if (!fusionProgress(value, tool.status).terminal) timer = setTimeout(() => void load(), 1500);
      } catch {
        if (active) { setError(true); if (tool.status === "running") timer = setTimeout(() => void load(), 3000); }
      }
    };
    void load();
    return () => { active = false; clearTimeout(timer); };
  }, [resolveTarget, tool.toolCallId, tool.runId, tool.status, scope, retry]);
  return <View style={styles.inlineTool} testID="fusion-activity">
    <Pressable accessibilityRole="button" accessibilityLabel={`${t("chat.fusionTitle")}, ${stageLabel}, ${stageCount}`} accessibilityState={{ expanded: open }} onPress={() => { if (!open && error) setRetry(value => value + 1); setOpen(value => !value); }} style={styles.toolHeader}>
      <UserRoundGroup size={14} color={["failed", "interrupted"].includes(progress.status) ? colors.danger : colors.inkMuted} />
      <Text style={styles.toolText}>{t("chat.fusionTitle")}</Text>
      <View style={styles.fusionRing} accessibilityRole="progressbar" accessibilityLabel={t("chat.fusionProgress")} aria-valuemin={0} aria-valuemax={4} aria-valuenow={currentStage} aria-valuetext={stageLabel}>
        <Svg width={36} height={36} viewBox="0 0 48 48" style={styles.fusionRingSvg}>
          {FUSION_PHASES.map((phase, index) => <SvgCircle key={phase} cx={24} cy={24} r={20} fill="none" stroke={progress.stages[index] === "completed" || progress.stages[index] === "thinking" ? colors.accent : progress.stages[index] === "failed" ? colors.danger : colors.line} strokeWidth={3.5} strokeLinecap="round" strokeDasharray={[RING_CIRCUMFERENCE / 4 - 7, RING_CIRCUMFERENCE * 3 / 4 + 7]} strokeDashoffset={-(index * RING_CIRCUMFERENCE / 4 + 3.5)} transform="rotate(-90 24 24)" opacity={progress.stages[index] === "thinking" ? 0.65 : 1} />)}
        </Svg>
        <Text style={styles.fusionRingValue} accessible={false}>{stageCount}</Text>
      </View>
      {open ? <ChevronUp size={14} color={colors.inkMuted} /> : <ChevronDown size={14} color={colors.inkMuted} />}
    </Pressable>
    {open ? <View style={styles.fusionProgress}>
      {progress.terminal && progress.partial && progress.notices.length === 0 ? <Text style={styles.fusionStatus}>{t("chat.fusionPartialNotice")}</Text> : null}
      {progress.notices.length ? <Text accessibilityRole="text" style={styles.fusionStatus}>{progress.notices.map(notice => t(`chat.fusionNotices.${notice}`)).join(" ")}</Text> : null}
      {models.map(model => {
        const role = progress.phase === "issue_mapping" ? "moderator" : model.role;
        const name = `${role ? `${t(role === "moderator" ? "chat.fusionModeratorRole" : "chat.fusionParticipantRole", { number: model.number })} · ` : ""}${model.label}`;
        const failed = ["failed", "timed_out", "output_limited"].includes(model.status);
        const Icon = model.status === "completed" ? Check : model.status === "timed_out" ? Clock : model.status === "failed" ? CircleX : model.status === "interrupted" ? CirclePause : model.status === "cancelled" ? CircleStop : model.status === "skipped" ? CircleMinus : failed || model.status === "partial" ? TriangleAlert : Circle;
        return <View key={model.id} style={styles.fusionModelRow} accessible accessibilityLabel={`${name}, ${t(`chat.fusionModelStates.${model.status}`)}`}>
          {model.status === "thinking" ? <ActivityIndicator size="small" color={colors.accent} style={styles.fusionThinkingIcon} /> : <Icon size={14} color={model.status === "completed" ? colors.success : failed ? colors.danger : model.status === "partial" || model.status === "interrupted" ? colors.warning : colors.inkMuted} />}
          <Text style={styles.fusionModelName}>{name}</Text>
        </View>;
      })}
      {error || !resolveTarget || !tool.toolCallId || !tool.runId ? <Text accessibilityRole="alert" style={[styles.fusionStatus, { color: colors.danger }]}>{t("chat.fusionStatusUnavailable")}</Text> : null}
    </View> : null}
  </View>;
}
