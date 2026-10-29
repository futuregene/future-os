import { fusionIssues, fusionTranscript, type FusionDiscussion, type FusionStep } from "@future-os/thread-projection";
import { ChevronDown, ChevronUp } from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { AppState, Pressable, Text, View } from "react-native";
import type { TimelineToolRow } from "../../../remote/types";
import type { ToolTargetResolver } from "./types";
import { useStyles } from "./messageStyles";

export function FusionToolRow({ tool, resolveTarget }: { tool: TimelineToolRow; resolveTarget?: ToolTargetResolver }) {
  const styles = useStyles();
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState("discussion");
  const [data, setData] = useState<FusionDiscussion | null>(null);
  const [error, setError] = useState("");
  const [step, setStep] = useState<FusionStep | null>(null);
  const [transcript, setTranscript] = useState("");
  useEffect(() => {
    if (!open || !resolveTarget || !tool.toolCallId || !tool.runId) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      if (!active) return;
      if (AppState.currentState !== "active") { timer = setTimeout(() => void load(), 1500); return; }
      try {
        const raw = await resolveTarget(tool.toolCallId!, tool.runId!, {});
        if (!active) return;
        if (!raw) throw new Error(t("chat.fusionUnavailable"));
        const value = JSON.parse(raw) as FusionDiscussion;
        setData(value); setError("");
        if (!value.run || ["preparing", "queued", "running", "delivering"].includes(value.run.status)) timer = setTimeout(() => void load(), 1500);
      } catch (failure) {
        if (active) { setError(String(failure)); timer = setTimeout(() => void load(), 3000); }
      }
    };
    void load();
    return () => { active = false; clearTimeout(timer); };
  }, [open, resolveTarget, tool.toolCallId, tool.runId, t]);
  const stepRequestRef=useRef(0);
  const showStep = async (id: string, offset = 0) => {
    const request=++stepRequestRef.current;
    try {
      if (!resolveTarget || !tool.toolCallId || !tool.runId) throw new Error(t("chat.fusionUnavailable"));
      const raw = await resolveTarget(tool.toolCallId, tool.runId, { stepId: id, offset });
      if (request!==stepRequestRef.current) return;
      const value = JSON.parse(raw ?? "null") as FusionStep | null;
      if (!value) throw new Error(t("chat.fusionUnavailable"));
      setStep(value); setTranscript(old => offset ? old + (value.transcript ?? "") : value.transcript ?? ""); setError("");
    } catch (failure) { setError(String(failure)); }
  };
  const issues = fusionIssues(data);
  return <View style={styles.inlineTool}>
    <Pressable accessibilityRole="button" accessibilityState={{ expanded: open }} onPress={() => setOpen(value => !value)} style={styles.toolHeader}>
      <Text style={styles.toolText}>Fusion · {t(tool.status === "running" ? "chat.fusionDiscussing" : "chat.fusionDiscussion")}</Text>
      {open ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
    </Pressable>
    {tool.detail ? <Text style={styles.toolDetailText}>{tool.detail}</Text> : null}
    {open ? <View style={styles.inlineToolChildren}>
      <Text style={styles.toolDetailText}>{data?.run ? `${t(`chat.fusionPhases.${data.run.phase}`)} · ${t(`chat.fusionStates.${data.run.status}`)} · ${t(`chat.fusionCompleteness.${data.run.completeness}`)}` : t("chat.fusionLoading")}</Text>
      {error ? <Text accessibilityRole="alert" style={styles.toolDetailText}>{error}</Text> : null}
      {data?.run?.error ? <Text selectable style={styles.toolDetailText}>{data.run.error}</Text> : null}
      <View style={styles.toolHeader}>{["discussion", "issues", "diagnostics"].map(name => <Pressable key={name} accessibilityRole="button" accessibilityState={{ selected: tab === name }} onPress={() => setTab(name)}><Text style={styles.toolText}>{t(`chat.fusionTabs.${name}`)} </Text></Pressable>)}</View>
      {tab === "discussion" ? data?.steps?.map(value => <View key={value.id}>
        <Pressable accessibilityRole="button" onPress={() => void showStep(value.id)}><Text style={styles.toolText}>{t(`chat.fusionPhases.${value.phase}`)} · {value.participant} · {value.model} · {t(`chat.fusionStates.${value.status}`)}</Text></Pressable>
        <Text selectable style={styles.toolDetailText}>{value.error ?? value.output}</Text>
      </View>) : null}
      {tab === "issues" ? <Text selectable style={styles.toolDetailText}>{issues.length ? issues.map(issue => `${issue.id} · ${issue.topic}\n${issue.stances.map(stance => `${stance.participant}: ${stance.position}`).join("\n")}`).join("\n\n") : t("chat.fusionNoIssues")}</Text> : null}
      {tab === "diagnostics" ? <Text selectable style={styles.toolDetailText}>{JSON.stringify(data, null, 2)}</Text> : null}
      {data?.run?.result ? <Text selectable style={styles.toolDetailText}>{data.run.result.conclusion}</Text> : null}
      {step ? <View><Text style={styles.toolText}>{step.model} · {t("chat.fusionOriginal")}</Text><Text selectable style={styles.toolDetailText}>{fusionTranscript(transcript)}</Text>
        {(step.next_offset ?? 0) < (step.total_bytes ?? 0) ? <Pressable accessibilityRole="button" onPress={() => void showStep(step.id, step.next_offset)}><Text style={styles.toolText}>{t("chat.fusionMore")}</Text></Pressable> : null}
        <Text selectable style={styles.toolDetailText}>{step.input}</Text></View> : null}
    </View> : null}
  </View>;
}
