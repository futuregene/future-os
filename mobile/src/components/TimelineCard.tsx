import {
  AlertTriangle,
  Brain,
  Check,
  ChevronDown,
  ChevronUp,
  CircleAlert,
  Copy,
  FileText,
  Paperclip,
  Pencil,
  TerminalSquare,
  TriangleAlert,
  Wrench,
  X,
} from "lucide-react-native";
import { Fragment, memo, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as Clipboard from "expo-clipboard";
import { Linking, Pressable, StyleSheet, Text, View } from "react-native";
import { AppAlert as Alert } from "./appAlerts";
import {
  approvalCommand,
  approvalDeletes,
  approvalPaths,
  escalationTitleKey,
  parseAction,
} from "@future-os/thread-projection";
import { friendlyRunError, friendlyRunErrorTitle } from "./errorMessage";
import { MarkdownText } from "./MarkdownText";
import { splitUserTextSegments } from "./userTextSegments";
import type {
  ApprovalPayload,
  HistoryAttachment,
  TimelineItem,
  TimelineSegment,
  TimelineToolRow,
} from "../remote/types";
import { basename } from "../remote/localPath";
import { canRecoverMessage } from "../remote/recovery";
import { chatTypography, colors, radius, spacing } from "../theme/tokens";
import { Button } from "./Button";
import { approvalDecisionDisabled } from "./approvalState";

interface TimelineCardProps {
  item: TimelineItem;
  isLatestAssistant?: boolean;
  onOpenAttachment?(attachment: HistoryAttachment): void;
  onOpenFile?(path: string): void;
  onRetry?(item: TimelineItem): void;
  onContinue?(item: TimelineItem): void;
}

// Same shape as the desktop footer (desktop/src/lib/date.ts formatDuration): "5s"
// under a minute, "1m 25s" above — so phone and desktop read identically.
function formatDuration(durationMs: number): string {
  const totalSeconds = Math.max(0, Math.round(durationMs / 1_000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  return `${Math.floor(totalSeconds / 60)}m ${totalSeconds % 60}s`;
}

/**
 * A step row draws as one muted 20px line to stay out of the reader's way, but
 * it is also a control. Extending the touch area vertically reaches the 44px
 * `layout.touchTarget` without giving the row back the height it was folded to
 * save — the visual line stays 24px, the tappable one is ~40px.
 */
const ROW_HIT_SLOP = { top: spacing.sm, bottom: spacing.sm } as const;

function RunIndicator({ startedAt }: { startedAt?: number }) {
  const { t } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!startedAt) return;
    const timer = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(timer);
  }, [startedAt]);
  return (
    <View style={styles.runIndicator}>
      <View style={styles.runDot} />
      <Text style={styles.runDuration}>{startedAt ? formatDuration(now - startedAt) : t("chat.generating")}</Text>
    </View>
  );
}

// Tool kinds mirror the desktop activity line (desktop AgentActivityList): the icon
// and label follow the *tool*, not a generic wrench, so phone and desktop read
// the same. The agent's tool_name is exactly one of these four.
type ToolKind = "shell" | "write" | "edit" | "read";

function toolKind(name: string): ToolKind {
  if (name === "shell" || name === "write" || name === "edit" || name === "read") return name;
  return "shell";
}

function ToolGlyph({ kind, failed = false }: { kind: ToolKind; failed?: boolean }) {
  const props = { color: colors.inkMuted, size: 14 };
  if (failed) return <TriangleAlert {...props} />;
  switch (kind) {
    case "shell":
      return <TerminalSquare {...props} />;
    case "read":
      return <FileText {...props} />;
    case "write":
    case "edit":
      return <Pencil {...props} />;
  }
}

function toolLabel(t: (key: string) => string, kind: ToolKind, complete: boolean): string {
  switch (kind) {
    case "read":
      return t(complete ? "chat.readCompleted" : "chat.reading");
    case "write":
      return t(complete ? "chat.writeCompleted" : "chat.writing");
    case "edit":
      return t(complete ? "chat.editCompleted" : "chat.editing");
    case "shell":
      return t(complete ? "chat.runCompleted" : "chat.runningCommand");
  }
}

function failedToolLabel(t: (key: string) => string, kind: ToolKind): string {
  switch (kind) {
    case "shell":
      return t("chat.runFailed");
    case "read":
      return t("chat.readFailed");
    case "write":
      return t("chat.writeFailed");
    case "edit":
      return t("chat.editFailed");
  }
}

function toolDetail(kind: ToolKind, detail: string): string {
  // Mobile intentionally shows only the filename for file tools. This differs
  // from desktop's full-path activity detail by product request: a phone row
  // cannot use the leading directories, and preserving the filename keeps the
  // useful part visible. Shell commands remain verbatim.
  return kind === "read" || kind === "write" || kind === "edit" ? basename(detail) : detail;
}

// Localized title/summary per approval kind — the agent ships English, so map
// by kind exactly like the desktop ApprovalPrompt, falling back to the wire
// strings for any unknown kind.
const APPROVAL_KIND_I18N: Record<string, { title: string; summary?: string }> = {
  file_read: { title: "approval.readTitle", summary: "approval.readSummary" },
  file_write: { title: "approval.writeTitle", summary: "approval.writeSummary" },
  outside_workspace_write: {
    title: "approval.outsideWriteTitle",
    summary: "approval.outsideWriteSummary",
  },
  shell_command: { title: "approval.shellTitle", summary: "approval.shellSummary" },
  sandbox_escalation: { title: "approval.escalationTitle" },
};

interface PendingApprovalCardProps {
  payload: ApprovalPayload;
  submitting: boolean;
  error: string | null;
  onDecision(decision: "approved" | "rejected"): void;
}

// Desktop-styled approval card (neutral surface + warning dot, not the old
// cream/warning fill). Rendered docked above the composer; it vanishes as soon
// as a decision lands because the caller only feeds undecided approvals.
export function PendingApprovalCard({
  payload,
  submitting,
  error,
  onDecision,
}: PendingApprovalCardProps) {
  const { t } = useTranslation();
  const [commandExpanded, setCommandExpanded] = useState(false);
  const action = parseAction(payload.action);
  const capabilityTargets =
    action?.category === "windows_write_capability" ? action.targets : undefined;
  const capabilityTarget = capabilityTargets?.length === 1 ? capabilityTargets[0] : undefined;
  const malformedCapability = payload.kind === "windows_write_capability" && !capabilityTargets;
  const decisionDisabled = approvalDecisionDisabled(submitting, malformedCapability);
  const kindI18n = APPROVAL_KIND_I18N[payload.kind ?? ""];
  const titleText = capabilityTargets
    ? capabilityTarget
      ? t(
          capabilityTarget.scope === "file"
            ? "approval.capabilityFileTitle"
            : "approval.capabilitySubtreeTitle",
          { path: capabilityTarget.path },
        )
      : t("approval.capabilityMultiTitle", { count: capabilityTargets.length })
    : kindI18n
      ? t(payload.kind === "sandbox_escalation" ? escalationTitleKey(action) : kindI18n.title)
      : payload.title || payload.tool_name || t("approval.title");
  const summaryText = capabilityTargets
    ? null
    : kindI18n?.summary
      ? t(kindI18n.summary)
      : payload.summary;
  const paths = approvalPaths(payload);
  const deletes = approvalDeletes(payload);
  const command = approvalCommand(payload);
  const isWrite =
    paths.length > 0 &&
    (payload.kind === "file_write" || payload.kind === "outside_workspace_write");
  const detailLabel = capabilityTargets
    ? capabilityTargets.length > 1
      ? t("approval.locations")
      : null
    : deletes.length > 0
      ? deletes.length === 1
        ? t("approval.deleteFile")
        : t("approval.deleteFiles", { count: deletes.length })
      : paths.length > 0
        ? isWrite
          ? paths.length === 1
            ? t("approval.writeFile")
            : t("approval.writeFiles", { count: paths.length })
          : t("approval.readPath")
        : command
          ? t("approval.command")
          : null;

  return (
    <View style={styles.approval}>
      <View style={styles.approvalHeader}>
        <View style={styles.approvalDot} />
        <Text numberOfLines={2} style={styles.approvalTitle}>
          {titleText}
        </Text>
      </View>
      {!!summaryText && <Text style={styles.approvalSummary}>{summaryText}</Text>}
      {detailLabel ? (
        <>
          <Text style={styles.approvalDetailLabel}>{detailLabel}</Text>
          <View style={styles.approvalDetail}>
            {capabilityTargets ? (
              capabilityTargets.map((target, index) => (
                <Text key={`${target.path}-${index}`} selectable style={styles.approvalPath}>
                  {target.path}
                </Text>
              ))
            ) : command && deletes.length === 0 ? (
              <Text selectable style={styles.approvalPath}>
                {command}
              </Text>
            ) : (
              (deletes.length > 0 ? deletes : paths).map((path, index) => (
                <Text
                  key={`${path}-${index}`}
                  numberOfLines={2}
                  selectable
                  style={styles.approvalPath}
                >
                  {path}
                </Text>
              ))
            )}
          </View>
        </>
      ) : null}
      {capabilityTargets && command ? (
        <View style={styles.approvalDetail}>
          <Pressable
            accessibilityRole="button"
            onPress={() => setCommandExpanded(value => !value)}
            style={styles.approvalCommandToggle}
          >
            <Text style={styles.approvalDetailLabel}>{t("approval.viewCommand")}</Text>
            {commandExpanded ? (
              <ChevronUp color={colors.inkMuted} size={14} />
            ) : (
              <ChevronDown color={colors.inkMuted} size={14} />
            )}
          </Pressable>
          {commandExpanded ? (
            <Text selectable style={styles.approvalPath}>
              {command}
            </Text>
          ) : null}
        </View>
      ) : null}
      {malformedCapability ? (
        <View style={styles.approvalError}>
          <AlertTriangle color={colors.danger} size={13} />
          <Text style={styles.approvalErrorText}>{t("approval.invalidRequest")}</Text>
        </View>
      ) : null}
      {!!error && (
        <View style={styles.approvalError}>
          <AlertTriangle color={colors.danger} size={13} />
          <Text style={styles.approvalErrorText}>{error}</Text>
        </View>
      )}
      <View style={styles.approvalActions}>
        <View style={styles.approvalActionLeft}>
          <Button
            compact
            disabled={decisionDisabled.rejected}
            icon={<X color={colors.ink} size={15} />}
            label={submitting ? t("approval.denying") : t("approval.deny")}
            loading={submitting}
            onPress={() => onDecision("rejected")}
            variant="secondary"
          />
        </View>
        <View style={styles.approvalActionRight}>
          <Button
            compact
            disabled={decisionDisabled.approved}
            icon={<Check color={colors.surface} size={15} />}
            label={submitting ? t("approval.allowing") : t("approval.allowOnce")}
            loading={submitting}
            onPress={() => onDecision("approved")}
            variant="primary"
          />
        </View>
      </View>
    </View>
  );
}

function AttachmentChip({
  attachment,
  onOpen,
}: {
  attachment: HistoryAttachment;
  onOpen?(): void;
}) {
  return (
    <Pressable
      accessibilityLabel={attachment.name}
      accessibilityRole="button"
      disabled={!onOpen}
      onPress={onOpen}
      style={({ pressed }) => [styles.attachmentChip, pressed && styles.attachmentChipPressed]}
    >
      {attachment.kind === "file" ? (
        <FileText color={colors.inkSoft} size={13} />
      ) : (
        <Paperclip color={colors.inkSoft} size={13} />
      )}
      <Text numberOfLines={1} style={styles.attachmentName}>
        {attachment.name}
      </Text>
    </Pressable>
  );
}

// Copy feedback parity with the desktop CopyButton: flash a check while the
// clipboard write is in flight/just-done, then settle back to the copy glyph
// after a beat. Only the success path flips the icon — a failed write must
// not masquerade as a copied reply.
function useCopyState(resetMs = 1400) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const copy = () => {
    setCopied(true);
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setCopied(false), resetMs);
  };
  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    },
    [],
  );
  return { copied, copy };
}

// Inline divider marking where the agent auto-compacted the conversation
// (history summarized to fit the context window). A hairline rule with a small
// muted label — the only surfacing of compaction in the UI, since the agent
// otherwise continues silently. Shows the pre-compaction token count when known.
function CompactionDivider({
  tokensBefore,
  status = "completed",
  trigger,
}: {
  tokensBefore?: number;
  status?: "running" | "completed" | "failed";
  trigger?: string;
}) {
  const { t, i18n } = useTranslation();
  const manual = trigger === "manual";
  const label =
    status === "running"
      ? manual
        ? t("chat.manuallyCompacting")
        : t("chat.compacting")
      : status === "failed"
        ? manual
          ? t("chat.manualCompactionFailed")
          : t("chat.compactionFailed")
        : manual
          ? t("chat.manuallyCompacted")
          : tokensBefore && tokensBefore > 0
            ? t("chat.compactedTokens", {
                formattedCount: new Intl.NumberFormat(i18n.language).format(tokensBefore),
              })
            : t("chat.compacted");
  return <StatusDivider failed={status === "failed"} label={label} />;
}

function StatusDivider({ label, failed = false }: { label: string; failed?: boolean }) {
  const color = failed ? colors.danger : colors.inkMuted;
  const lineColor = failed ? colors.dangerLine : colors.line;
  return (
    <View
      accessibilityLabel={label}
      accessibilityRole={failed ? "alert" : "text"}
      style={styles.compactionDivider}
    >
      <View style={[styles.compactionLine, { backgroundColor: lineColor }]} />
      <Text style={[styles.compactionLabel, { color }]}>{label}</Text>
      <View style={[styles.compactionLine, { backgroundColor: lineColor }]} />
    </View>
  );
}

/**
 * The inline tool-activity row inside a reply bubble (desktop parity:
 * AgentActivityLine). A failed tool (shell non-zero exit / error) swaps its
 * normal tool glyph for the desktop-matching alert glyph; a collapsed same-kind
 * burst carries its child calls on `tool.children` and its count — the row
 * reads "Ran N commands", and tapping it reveals the individual targets.
 *
 * The row is a badge on the right rail only while nothing of it is revealed.
 * Once it — or the run around it — is open, it is content being read, and
 * content sits in the reading column with the rest of the reply: right-aligned
 * prose and right-aligned commands are hard to read. `opened` is required so a
 * call site cannot silently pick the wrong rail, which is how the unfolded rows
 * ended up on the left while folded ones sat on the right.
 */
function ToolRow({ tool, opened }: { tool: TimelineToolRow; opened: boolean }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const open = opened || expanded;
  const kind = toolKind(tool.name);
  const failed = tool.status === "failed";
  const detail = tool.detail?.trim() ? toolDetail(kind, tool.detail.trim()) : null;
  const children = tool.children && tool.children.length > 0 ? tool.children : null;
  const expandable = Boolean(detail || children);
  const label = failed
    ? failedToolLabel(t, kind)
    : tool.count != null && tool.count > 1
      ? t("chat.stepSummary", {
          count: tool.count,
          action: t(TOOL_LABEL_KEYS[kind]),
        })
      : toolLabel(t, kind, tool.complete);
  return (
    <View style={[styles.inlineTool, !open && styles.railBlock]}>
      <Pressable
        accessibilityRole="button"
        disabled={!expandable}
        hitSlop={expandable ? ROW_HIT_SLOP : undefined}
        onPress={() => setExpanded(value => !value)}
        style={[styles.toolHeader, !open && styles.railRow]}
      >
        <ToolGlyph failed={failed} kind={kind} />
        <Text style={styles.toolText}>{label}</Text>
        {expandable ? (
          expanded ? (
            <ChevronUp color={colors.inkMuted} size={14} />
          ) : (
            <ChevronDown color={colors.inkMuted} size={14} />
          )
        ) : null}
        {/* File targets remain compact beside the action label. Shell commands
            are intentionally rendered below: they can be arbitrarily long and
            must not be truncated on a phone. */}
        {expanded && detail && !children && kind !== "shell" ? (
          <Text
            ellipsizeMode="tail"
            numberOfLines={1}
            selectable
            style={styles.inlineToolTarget}
          >
            {detail}
          </Text>
        ) : null}
      </Pressable>
      {expanded && detail && !children && kind === "shell" ? (
        <Text selectable style={styles.toolDetailText}>
          {detail}
        </Text>
      ) : null}
      {expanded && children ? (
        <View style={styles.inlineToolChildren}>
          {children.map((child, index) => {
            const childKind = toolKind(child.name);
            const shellCommand = childKind === "shell" && Boolean(child.detail);
            return (
              <Text
                key={`${child.name}:${child.detail ?? ""}:${index}`}
                {...(!shellCommand ? { ellipsizeMode: "tail" as const, numberOfLines: 1 } : {})}
                selectable={shellCommand}
                style={styles.inlineToolChild}
              >
                {child.detail
                  ? toolDetail(childKind, child.detail)
                  : toolLabel(t, childKind, child.complete)}
              </Text>
            );
          })}
        </View>
      ) : null}
    </View>
  );
}

/**
 * Inline thinking block inside a reply bubble. The mobile product decision
 * (audit D4) is that reasoning always renders collapsed — a muted one-line
 * label, expandable on tap — so long reasoning never floods the bubble or the
 * FlatList. The shared projection captures the full text; the collapse is a
 * render concern only. The label reads "thinking" while the run is streaming
 * (the block is still mid-reasoning) and "thought completed" once settled.
 *
 * The glyph matches the desktop's thinking activity glyph (Brain), so an
 * expanded reasoning row and a tool row are the same shape: icon, then words.
 * Like a tool row it is a badge on the right rail while collapsed and falls
 * back into the reading column once opened — see ToolRow.
 */
function ThinkingRow({ text, streaming, opened }: {
  text: string;
  streaming?: boolean;
  opened: boolean;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const open = opened || expanded;
  return (
    <View style={[styles.inlineThinking, !open && styles.railThinking]}>
      <Pressable
        accessibilityRole="button"
        hitSlop={ROW_HIT_SLOP}
        onPress={() => setExpanded(value => !value)}
        style={[styles.inlineThinkingHeader, !open && styles.railRow]}
      >
        <Brain color={colors.inkMuted} size={14} />
        <Text style={styles.inlineThinkingLabel}>
          {t(streaming ? "chat.thinking" : "chat.thoughtCompleted")}
        </Text>
        {expanded ? (
          <ChevronUp color={colors.inkMuted} size={14} />
        ) : (
          <ChevronDown color={colors.inkMuted} size={14} />
        )}
      </Pressable>
      {expanded && <Text style={styles.inlineThinkingText}>{text}</Text>}
    </View>
  );
}

/** The two slice kinds a folded step run is made of. */
type StepSegment = Extract<TimelineSegment, { kind: "thinking" | "tool" }>;

/**
 * What a folded run counts, and the order it always counts them in: tool calls
 * first, then reasoning. Every kind of tool — shell, read, write, edit — is one
 * "tool call" here: at this altitude the question is "how much work happened?",
 * and splitting it per tool kind turned the line into a list of four numbers
 * that nobody reads. Which tool it was is what the expanded rows are for.
 */
type StepRunKind = "tool" | "thinking";
const STEP_RUN_ORDER: StepRunKind[] = ["tool", "thinking"];

/**
 * The short label each tool kind is counted under *inside* the expanded run,
 * where the kind is the point. Deliberately not the single-row labels: a burst
 * stacks counts, and "已运行 3 次" reads as a sentence where "运行 3 次" reads as
 * a count.
 */
const TOOL_LABEL_KEYS: Record<ToolKind, string> = {
  shell: "chat.stepRun",
  read: "chat.stepRead",
  write: "chat.stepWrite",
  edit: "chat.stepEdit",
};

function isStepSegment(segment: TimelineSegment): segment is StepSegment {
  return segment.kind === "thinking" || segment.kind === "tool";
}
/**
 * Whether a step row may be hidden inside a folded run. Everything that has
 * stopped counts — a failure included: it is counted as a tool call and flagged
 * on the summary line, so the run still folds without the failure going quiet. A
 * call that is still running stays on screen: it is live progress, and folding it
 * would hide work in flight.
 */
function foldableStep(segment: StepSegment): boolean {
  return segment.kind === "thinking" || segment.tool.complete;
}

function stepRunKind(segment: StepSegment): StepRunKind {
  return segment.kind === "thinking" ? "thinking" : "tool";
}

/** One rendered slice of a reply: a pass-through segment, or a folded step run. */
type ReplyBlock =
  | { kind: "segment"; segment: TimelineSegment }
  | { kind: "steps"; segments: StepSegment[] };

/**
 * Fold a reply's inline slices into render blocks. A phone has no room for the
 * desktop's one-line-per-activity transcript: a single long exchange routinely
 * produces a dozen thinking/tool rows, each a full line, and they crowd out the
 * prose between them. Consecutive step rows that have stopped therefore collapse
 * into one summary line (expanded on tap), while prose, compaction dividers and
 * the slice the agent is working on right now pass through untouched — nothing
 * that is still changing is ever hidden.
 *
 * This is a render concern only, exactly like the per-block collapse in
 * ThinkingRow: the shared projection still carries every slice in stream order,
 * so the desktop transcript is unaffected.
 */
function buildReplyBlocks(segments: TimelineSegment[], streaming?: boolean): ReplyBlock[] {
  const blocks: ReplyBlock[] = [];
  // In a streaming reply the last slice is the one being written right now, so
  // it never joins a run (and a run never grows past it).
  const liveTail = streaming ? segments.length - 1 : -1;
  let index = 0;
  while (index < segments.length) {
    const segment = segments[index]!;
    if (index !== liveTail && isStepSegment(segment) && foldableStep(segment)) {
      const run: StepSegment[] = [segment];
      let cursor = index + 1;
      while (cursor !== liveTail && cursor < segments.length) {
        const next = segments[cursor]!;
        if (!isStepSegment(next) || !foldableStep(next)) break;
        run.push(next);
        cursor += 1;
      }
      if (run.length > 1) {
        blocks.push({ kind: "steps", segments: run });
        index = cursor;
        continue;
      }
    }
    blocks.push({ kind: "segment", segment });
    index += 1;
  }
  return blocks;
}

/**
 * The counts of a folded run: tool calls then reasoning, zeros dropped. The
 * summary paints these as a glyph per part, so the caller renders the icon and
 * the localized wording is spent on the accessibility label — an "×5" next to a
 * glyph reads instantly, and five words of it do not fit on a phone line.
 */
function stepRunCounts(segments: StepSegment[]): { kind: StepRunKind; count: number }[] {
  const counts: Record<StepRunKind, number> = { tool: 0, thinking: 0 };
  for (const segment of segments) counts[stepRunKind(segment)] += 1;
  return STEP_RUN_ORDER
    .filter(kind => counts[kind] > 0)
    .map(kind => ({ kind, count: counts[kind] }));
}

/** The spoken form of those counts, for the row's accessibility label. */
function stepRunSummary(
  t: (key: string, options?: { count: number; action: string }) => string,
  counts: { kind: StepRunKind; count: number }[],
): string {
  return counts
    .map(({ kind, count }) =>
      t("chat.stepSummary", {
        count,
        action: t(kind === "thinking" ? "chat.stepThink" : "chat.stepTool"),
      }),
    )
    .join(" · ");
}

/**
 * A folded run of step rows: one summary line while collapsed, the individual
 * rows once tapped — and each of those still opens its own detail, so the
 * desktop's two taps survive without the phone spending a line per activity.
 * The summary is a glyph and a count per kind (wrench ×5, brain ×3) rather than
 * the wording: the line has to survive in a phone's margin, and the wording
 * lives on in the accessibility label, so nothing is lost to a screen reader.
 * The tool glyph is a wrench, not the terminal it replaced: the count covers
 * every tool kind, and a shell prompt claimed it was all commands.
 *
 * The line sits against the right edge instead of the reading column: it is
 * apparatus, not prose, and keeping it out of the reader's eye line is the whole
 * point of folding it. A failure is marked the same way — the alert glyph, in
 * the same muted ink as the rest of the line, told apart by its shape and named
 * in the accessibility label rather than shouted in colour.
 *
 * The row draws as one 20px line but is a real touch target (`ROW_HIT_SLOP`):
 * it is the only way into the run behind it, and a target the size of the text
 * is not enough on a phone.
 */
function StepRunBlock({ segments }: { segments: StepSegment[] }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const counts = stepRunCounts(segments);
  const summary = stepRunSummary(t, counts);
  // A failed call folds like any other — and counts like any other, as a tool
  // call — so the counts alone cannot tell the user it happened.
  const failedCount = segments.filter(
    segment => segment.kind === "tool" && segment.tool.status === "failed",
  ).length;
  const failureNote = failedCount > 0 ? t("chat.stepsFailed", { count: failedCount }) : null;
  return (
    <View style={styles.inlineTool}>
      <Pressable
        accessibilityLabel={failureNote ? `${summary} · ${failureNote}` : summary}
        accessibilityRole="button"
        accessibilityState={{ expanded }}
        hitSlop={ROW_HIT_SLOP}
        onPress={() => setExpanded(value => !value)}
        style={[styles.toolHeader, styles.stepSummaryRow]}
      >
        {counts.map(({ kind, count }, index) => (
          <Fragment key={kind}>
            {index > 0 ? <Text style={styles.toolText}>·</Text> : null}
            {/* The glyph and its count are one unit: `stepSummaryItem` cancels
                the gap inside the pair, so the wrench and its `×5` read as a
                single token and only the kinds are spaced apart. */}
            <View style={styles.stepSummaryItem}>
              {kind === "thinking"
                ? <Brain color={colors.inkMuted} size={14} />
                : <Wrench color={colors.inkMuted} size={14} />}
              <Text style={styles.toolText}>{t("chat.stepCount", { count })}</Text>
            </View>
          </Fragment>
        ))}
        {failedCount > 0 ? <TriangleAlert color={colors.inkMuted} size={14} /> : null}
        {expanded ? (
          <ChevronUp color={colors.inkMuted} size={14} />
        ) : (
          <ChevronDown color={colors.inkMuted} size={14} />
        )}
      </Pressable>
      {expanded ? (
        <View style={styles.stepChildren}>
          {segments.map(segment =>
            segment.kind === "thinking" ? (
              // Every folded reasoning slice has already been closed by the
              // slice after it, so none of them is still streaming.
              <ThinkingRow key={segment.id} opened text={segment.text} />
            ) : (
              <ToolRow key={segment.id} opened tool={segment.tool} />
            ),
          )}
        </View>
      ) : null}
    </View>
  );
}

/** One inline slice of an assistant reply, in stream order (desktop parity). */
function SegmentBlock({
  segment,
  streaming,
  onOpenFile,
}: {
  segment: TimelineSegment;
  streaming?: boolean;
  onOpenFile?(path: string): void;
}) {
  if (segment.kind === "text") {
    return <MarkdownText text={segment.text} onOpenFile={onOpenFile} streaming={streaming} />;
  }
  if (segment.kind === "thinking") {
    return <ThinkingRow opened={false} streaming={streaming} text={segment.text} />;
  }
  if (segment.kind === "tool") {
    return <ToolRow opened={false} tool={segment.tool} />;
  }
  // compaction
  return (
    <CompactionDivider
      status={segment.status}
      tokensBefore={segment.tokensBefore}
      trigger={segment.trigger}
    />
  );
}

/**
 * User messages render as plain text (never markdown — the user's `*`/`#`/`1.`
 * stay literal), except `[name](./path)` file mentions and `[label](https://…)`
 * links, recognized exactly like the desktop UserMessageText. The bubble is
 * accent-blue, so both render white: mentions medium-weight, links underlined
 * and tappable (file mentions open like assistant file links, external links in
 * the browser).
 */
function UserMessageText({ text, onOpenFile }: { text: string; onOpenFile?(path: string): void }) {
  const { t } = useTranslation();
  return (
    <Text selectable style={[styles.messageText, styles.userText]}>
      {splitUserTextSegments(text).map(segment => {
        if (segment.kind === "mention") {
          return (
            <Text
              key={segment.key}
              onPress={onOpenFile && segment.href ? () => onOpenFile(segment.href!) : undefined}
              style={styles.userMention}
            >
              {segment.text}
            </Text>
          );
        }
        if (segment.kind === "link") {
          return (
            <Text
              key={segment.key}
              onPress={() => {
                void Linking.openURL(segment.href ?? "").catch(() => {
                  Alert.alert(t("attachment.title"), t("attachment.linkOpenFailed"));
                });
              }}
              style={styles.userLink}
            >
              {segment.text}
            </Text>
          );
        }
        return <Text key={segment.key}>{segment.text}</Text>;
      })}
    </Text>
  );
}

function TimelineCardView({
  item,
  isLatestAssistant,
  onOpenAttachment,
  onOpenFile,
  onRetry,
  onContinue,
}: TimelineCardProps) {
  const { t, i18n } = useTranslation();
  const { copied, copy } = useCopyState();

  if (item.kind === "message") {
    if (item.role === "assistant") {
      // Single "time · N tokens" line, joined like the desktop MessageMeta
      // footer (which renders `parts.join(" · ")`).
      const outputTokens = item.outputTokens ?? 0;
      const usage =
        outputTokens > 0
          ? t("chat.tokens", {
              formattedCount: new Intl.NumberFormat(i18n.language).format(outputTokens),
            })
          : null;
      const footerStats = [item.durationMs != null ? formatDuration(item.durationMs) : null, usage]
        .filter((part): part is string => !!part)
        .join(" · ");
      // A compaction-only message is a standalone divider (the agent replaced
      // summarized history with a marker) — render just the hairline rule, no
      // copy footer, mirroring the desktop's MessageBlock special case.
      const dividerOnly =
        !item.streaming && item.segments?.length === 1 && item.segments[0]?.kind === "compaction";
      const terminationNotice = item.stopped
        ? t("failure.userStopped")
        : item.failed
          ? friendlyRunError(item.error, t)
          : item.truncated
            ? t("failure.modelResponseError")
            : null;
      const copyableText = item.text;
      return (
        <View style={styles.assistantMessage}>
          {item.segments && item.segments.length > 0 ? (
            <View style={styles.segmentList}>
              {buildReplyBlocks(item.segments, item.streaming).map(block =>
                block.kind === "steps" ? (
                  <StepRunBlock key={block.segments[0]!.id} segments={block.segments} />
                ) : (
                  <SegmentBlock
                    key={block.segment.id}
                    segment={block.segment}
                    streaming={item.streaming}
                    onOpenFile={onOpenFile}
                  />
                ),
              )}
            </View>
          ) : item.text.trim().length > 0 ? (
            <MarkdownText text={item.text} onOpenFile={onOpenFile} streaming={item.streaming} />
          ) : null}
          {terminationNotice ? (
            <View style={styles.terminationNotice}>
              <StatusDivider
                label={
                  item.stopped ? t("chat.responseStopped") : friendlyRunErrorTitle(item.error, t)
                }
              />
              {isLatestAssistant && !item.stopped ? (
                <Text
                  selectable
                  style={[styles.terminationNoticeText, item.stopped && styles.stoppedNoticeText]}
                >
                  {terminationNotice}
                </Text>
              ) : null}
            </View>
          ) : null}
          {/* Desktop parity: the retry/continue row sits right below the bubble
              content, above the copy/stats footer. */}
          {canRecoverMessage(item, isLatestAssistant ? item.id : null) && (
            <View style={styles.recoveryRow}>
              <Button
                compact
                label={t("chat.retry")}
                onPress={() => onRetry?.(item)}
                variant="secondary"
              />
              <Button
                compact
                label={t("chat.continue")}
                onPress={() => onContinue?.(item)}
                variant="secondary"
              />
            </View>
          )}
          {dividerOnly ? null : item.streaming ? (
            // In-flight: the generating indicator occupies the same footer slot
            // the copy button uses once settled (desktop parity), so a streaming
            // reply never shows a copy button.
            <RunIndicator startedAt={item.startedAt} />
          ) : (
            <View style={styles.messageFooter}>
              {copyableText.trim() ? (
                <Pressable
                  accessibilityLabel={t("chat.copyResponse")}
                  accessibilityRole="button"
                  hitSlop={8}
                  onPress={() => {
                    Clipboard.setStringAsync(copyableText)
                      .then(() => copy())
                      .catch(() => {});
                  }}
                  style={styles.copyButton}
                >
                  {copied ? (
                    <Check color={colors.accent} size={15} />
                  ) : (
                    <Copy color={colors.inkMuted} size={15} />
                  )}
                </Pressable>
              ) : null}
              <Text style={styles.messageDuration}>
                {footerStats || (item.stopped ? t("chat.responseStopped") : item.failed || item.truncated ? t("common.error") : t("chat.responseCompleted"))}
              </Text>
            </View>
          )}
        </View>
      );
    }
    return (
      <View style={styles.userBlock}>
        {item.text.trim().length > 0 && (
          <View style={[styles.message, styles.userMessage]}>
            <UserMessageText onOpenFile={onOpenFile} text={item.text} />
          </View>
        )}
        {item.attachments && item.attachments.length > 0 && (
          <View style={styles.attachmentRow}>
            {item.attachments.map(attachment => (
              <AttachmentChip
                attachment={attachment}
                key={`${item.id}:${attachment.path}`}
                onOpen={onOpenAttachment ? () => onOpenAttachment(attachment) : undefined}
              />
            ))}
          </View>
        )}
      </View>
    );
  }

  if (item.kind === "notice") {
    const warning = item.tone === "warning";
    // Danger notices carry raw agent/relay error blobs — surface the friendly
    // classification, never the developer-oriented dump.
    const noticeText =
      item.text === "truncated"
        ? t("chat.truncated")
        : warning
          ? item.text
          : friendlyRunError(item.text, t);
    return (
      <View style={[styles.notice, warning ? styles.warningNotice : styles.dangerNotice]}>
        <CircleAlert color={warning ? colors.warning : colors.danger} size={17} />
        <Text style={[styles.noticeText, { color: warning ? colors.warning : colors.danger }]}>
          {noticeText}
        </Text>
      </View>
    );
  }

  // Approval items are rendered docked above the composer (see ChatScreen),
  // never inline in the transcript — so there is nothing to draw here.
  return null;
}

// Long conversations update the trailing streaming item frequently. Keep
// already-settled rows out of those renders; FlatList virtualization limits
// how many are mounted, while memoization limits work inside the mounted
// window.
export const TimelineCard = memo(TimelineCardView);

const styles = StyleSheet.create({
  message: {
    maxWidth: "92%",
    borderRadius: radius.lg,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
  },
  userMessage: { alignSelf: "flex-end", backgroundColor: colors.accent },
  userBlock: { alignItems: "flex-end", gap: spacing.xs },
  attachmentRow: {
    maxWidth: "88%",
    flexDirection: "row",
    flexWrap: "wrap",
    justifyContent: "flex-end",
    gap: spacing.xs,
  },
  attachmentChip: {
    maxWidth: 260,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.xs,
    borderWidth: 1,
    borderColor: colors.lineSoft,
    borderRadius: radius.md,
    backgroundColor: colors.surface,
  },
  attachmentChipPressed: { backgroundColor: colors.surfaceSubtle },
  attachmentName: { flexShrink: 1, color: colors.inkSoft, fontSize: 12 },
  assistantMessage: { alignSelf: "stretch", paddingHorizontal: spacing.xs },
  messageText: { color: colors.ink, ...chatTypography },
  userText: { color: colors.surface },
  userMention: { color: colors.surface, fontWeight: "600" },
  userLink: { color: colors.surface, fontWeight: "600", textDecorationLine: "underline" },
  terminationNotice: {
    marginTop: spacing.md,
  },
  terminationNoticeText: {
    color: colors.inkSoft,
    fontSize: 14,
    lineHeight: 24,
    marginTop: spacing.sm,
  },
  stoppedNoticeText: { color: colors.inkMuted },
  // The footer is apparatus like the step rows, and it sits on the same right
  // rail — which is also where the live timer already was, so the reply no longer
  // jumps to the left edge the moment its run settles. `alignSelf` keeps the copy
  // button's tap target clipped to the footer instead of spanning the bubble.
  messageFooter: {
    alignSelf: "flex-end",
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    gap: spacing.md,
    marginTop: spacing.sm,
  },
  segmentList: { gap: spacing.sm, marginTop: spacing.xs },
  inlineThinking: {
    borderLeftWidth: 2,
    borderLeftColor: colors.line,
    paddingLeft: spacing.md,
    paddingVertical: spacing.xs,
    gap: 2,
  },
  // Collapsed reasoning is a badge on the right rail: the rail follows it there.
  railThinking: {
    alignSelf: "flex-end",
    borderLeftWidth: 0,
    borderRightWidth: 2,
    borderRightColor: colors.line,
    paddingLeft: 0,
    paddingRight: spacing.md,
  },
  inlineThinkingHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
  },
  inlineThinkingLabel: { color: colors.inkMuted, fontSize: 13, lineHeight: 20 },
  inlineThinkingText: {
    color: colors.inkSoft,
    fontSize: 13,
    lineHeight: 19,
    marginTop: 2,
  },
  inlineTool: {
    paddingVertical: 2,
    gap: 2,
  },
  // ── The right rail, for collapsed badges only ───────────────────────────
  // A step row draws as a badge out of the prose's eye line while it is closed;
  // `railBlock` is what shrink-wraps it and parks it on the right, and `railRow`
  // keeps the tap target clipped to the badge instead of spanning the bubble.
  railBlock: { alignSelf: "flex-end" },
  railRow: { justifyContent: "flex-end" },
  inlineToolTarget: {
    flex: 1,
    marginLeft: spacing.sm,
    minWidth: 0,
    color: colors.inkSoft,
    fontFamily: "monospace",
    fontSize: 12,
    lineHeight: 18,
    maxHeight: 18,
    overflow: "hidden",
  },
  // Opened rows are reading content: left-aligned, and indented like the tool
  // burst children they belong to.
  inlineToolChildren: {
    gap: 2,
    marginTop: 2,
    paddingLeft: spacing.md + spacing.sm,
  },
  // The collapsed summary hugs the right edge, out of the prose's eye line. Its
  // gap is tighter than a tool row's: the counts are one cluster, and 8px on each
  // side of the `·` (16px between kinds) read as holes once the glyphs had been
  // pulled flush against their counts.
  stepSummaryRow: {
    alignSelf: "flex-end",
    justifyContent: "flex-end",
    gap: spacing.xs,
  },
  // The icon and its count are one token: no gap between them, so the wrench and
  // its `×5` read as a unit. The row's gap spaces the kinds around the `·`.
  stepSummaryItem: {
    flexDirection: "row",
    alignItems: "center",
    gap: 0,
  },
  // The expanded rows fall back into the reading column, indented one step so
  // they read as belonging to the summary line above them.
  stepChildren: {
    gap: spacing.xs,
    marginTop: 2,
    paddingLeft: spacing.md + spacing.sm,
  },
  inlineToolChild: {
    flexShrink: 1,
    color: colors.inkSoft,
    fontFamily: "monospace",
    fontSize: 12,
    lineHeight: 18,
  },
  compactionDivider: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingVertical: 4,
  },
  compactionLine: { flex: 1, height: StyleSheet.hairlineWidth, backgroundColor: colors.line },
  compactionLabel: { color: colors.inkMuted, fontSize: 12 },
  messageDuration: { color: colors.inkMuted, fontSize: 12 },
  recoveryRow: {
    flexDirection: "row",
    gap: spacing.sm,
    marginTop: spacing.sm,
  },
  copyButton: {
    flexDirection: "row",
    alignItems: "center",
    paddingVertical: spacing.xs,
  },
  // The live timer is apparatus like the step rows, and it follows the running
  // row it belongs to onto the same right-hand rail instead of hanging off the
  // left edge of the reply. It sits outside `segmentList`, so it carries the
  // same `marginTop` as the settled footer; its own padding is gone.
  runIndicator: {
    flexDirection: "row",
    alignItems: "center",
    alignSelf: "flex-end",
    gap: spacing.sm,
    marginTop: spacing.sm,
  },
  runDot: { width: 14, height: 14, borderRadius: radius.pill, backgroundColor: colors.generating },
  runDuration: { color: colors.inkMuted, fontSize: 12 },
  secondaryCard: {
    borderLeftWidth: 2,
    borderLeftColor: colors.line,
    paddingLeft: spacing.md,
    paddingVertical: spacing.xs,
  },
  cardHeader: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  cardLabel: { color: colors.inkSoft, fontSize: 13, fontWeight: "600" },
  secondaryText: { color: colors.inkSoft, fontSize: 13, lineHeight: 19, marginTop: spacing.sm },
  tool: { paddingHorizontal: spacing.xs, paddingVertical: spacing.xs },
  toolHeader: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  toolText: { color: colors.inkMuted, fontSize: 13, lineHeight: 20 },
  // The shell command under a tool row: left-aligned with the row it belongs to.
  toolDetailText: {
    marginTop: 2,
    paddingLeft: spacing.md + spacing.sm,
    color: colors.inkSoft,
    fontFamily: "monospace",
    fontSize: 12,
    lineHeight: 18,
  },
  notice: { flexDirection: "row", gap: spacing.sm, padding: spacing.md, borderRadius: radius.md },
  warningNotice: { backgroundColor: colors.warningSoft },
  dangerNotice: { backgroundColor: colors.dangerSoft },
  noticeText: { flex: 1, fontSize: 13, lineHeight: 19 },
  approval: {
    padding: spacing.lg,
    gap: spacing.sm,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.line,
    shadowColor: colors.inkStrong,
    shadowOpacity: 0.08,
    shadowRadius: 12,
    shadowOffset: { width: 0, height: 4 },
    elevation: 3,
  },
  approvalHeader: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  approvalDot: { width: 8, height: 8, borderRadius: radius.pill, backgroundColor: colors.warning },
  approvalTitle: { flex: 1, color: colors.ink, fontSize: 16, fontWeight: "600", lineHeight: 22 },
  approvalSummary: { color: colors.inkSoft, fontSize: 14, lineHeight: 20 },
  approvalDetailLabel: {
    marginTop: spacing.xs,
    color: colors.inkSoft,
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
  },
  approvalDetail: {
    padding: spacing.md,
    borderRadius: radius.md,
    backgroundColor: colors.surfaceSubtle,
    gap: 2,
  },
  approvalCommandToggle: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  approvalPath: { color: colors.ink, fontFamily: "monospace", fontSize: 12, lineHeight: 18 },
  approvalError: { flexDirection: "row", alignItems: "center", gap: spacing.xs },
  approvalErrorText: { flex: 1, color: colors.danger, fontSize: 12, lineHeight: 18 },
  approvalActions: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: spacing.sm,
    marginTop: spacing.sm,
  },
  approvalActionLeft: { flex: 1 },
  approvalActionRight: { flex: 1 },
});
