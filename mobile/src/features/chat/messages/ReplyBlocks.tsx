import { Brain, ChevronDown, ChevronUp, ListTree, Wrench } from "lucide-react-native";
import { Fragment, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import { MarkdownText } from "../../../components/MarkdownText";
import type { TimelineSegment } from "../../../remote/types";
import { useTheme } from "../../../theme/ThemeContext";
import { useStyles } from "./messageStyles";
import type { ToolTargetResolver } from "./types";
import { ToolRow } from "./ToolRow";
import { ThinkingRow } from "./ThinkingRow";
import { CompactionDivider } from "./MessageStatus";
import { ROW_HIT_SLOP } from "./touchTargets";

/** The two slice kinds a folded step run is made of. */
type StepSegment = Extract<TimelineSegment, { kind: "thinking" | "tool"; }>;

/**
 * What a folded run counts, and the order it always counts them in: tool calls
 * first, then reasoning. Every kind of tool — shell, read, write, edit — is one
 * "tool call" here: at this altitude the question is "how much work happened?",
 * and splitting it per tool kind turned the line into a list of four numbers
 * that nobody reads. Which tool it was is what the expanded rows are for.
 */
type StepRunKind = "tool" | "thinking";
const STEP_RUN_ORDER: StepRunKind[] = ["tool", "thinking"];

function isStepSegment(segment: TimelineSegment): segment is StepSegment {
  return segment.kind === "thinking" || segment.kind === "tool";
}
/**
 * Whether a step row may be hidden inside a folded run. Everything that has
 * stopped counts — a failure included: it is counted as a tool call and named
 * in the summary's accessibility label. A
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
  | { kind: "segment"; segment: TimelineSegment; }
  | { kind: "steps"; segments: StepSegment[]; };

/**
 * Whether a rendered block still carries output for a jump to stand on: prose,
 * or a tool call — the same two slice kinds the question ladder counts as
 * output. A folded run counts when any of its slices is a tool call; folded
 * reasoning alone is a pause, not output.
 */
function isOutputBlock(block: ReplyBlock): boolean {
  return block.kind === "segment"
    ? block.segment.kind === "text" || block.segment.kind === "tool"
    : block.segments.some(segment => segment.kind === "tool");
}

/**
 * The block a reply's tail rung lands on: the reply's *last* block that still
 * carries output, or null when nothing on screen is output (a reply that is so
 * far only reasoning, or one whose prose the compaction replaced).
 *
 * A press that reaches an answer is the reader asking for the answer, and an
 * exchange opens with the work: activity rows first, then the prose they led
 * to. Landing on the row's start therefore puts the reasoning the reader did
 * not ask for at the reading edge and pushes the output itself a screenful
 * below it. The anchor is that output — the last of it in the reply — and
 * whatever follows it (folded reasoning, a divider, the footer) stays below
 * the reading edge. A reply that ends on a pause thus anchors on the output
 * before the pause, the same way the ladder skips a reply that folded
 * entirely into thinking.
 *
 * null keeps the landing on the row's own start: a row with no output at all
 * has nothing better to aim at.
 */
export function replyTailBlock(blocks: readonly ReplyBlock[]): number | null {
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    if (isOutputBlock(blocks[index]!)) return index;
  }
  return null;
}

/**
 * Split a settled reply into the work that led to its answer and the answer
 * itself — the whole reply minus everything before its last piece of prose.
 *
 * "The final answer" is the reply's last non-empty prose block. An agent that
 * works through a task writes notes between its calls, and the prose after the
 * last call is what it is actually telling the reader; everything after that
 * prose stays with the answer, so a trailing pause changes nothing about which
 * block the reply opens on. A reply with no prose at all — still only work, or
 * one whose answer the compaction replaced — has no answer to isolate, so
 * nothing folds and the reader keeps the whole reply.
 *
 * Desktop's `splitFinalAnswer` decides the same thing for its own blocks; the
 * two must agree, because both clients promise the same shape for a settled
 * reply.
 */
export function splitFinalAnswer(blocks: readonly ReplyBlock[]): { process: ReplyBlock[]; answer: ReplyBlock[]; } {
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    const block = blocks[index]!;
    if (block.kind === "segment" && block.segment.kind === "text" && block.segment.text.trim().length > 0)
      return { process: blocks.slice(0, index), answer: blocks.slice(index) };
  }
  return { process: [], answer: [...blocks] };
}

/** Every step row a set of rendered blocks holds, in order. */
function blockSteps(blocks: readonly ReplyBlock[]): StepSegment[] {
  return blocks.flatMap((block) => {
    if (block.kind === "steps") return block.segments;
    return isStepSegment(block.segment) ? [block.segment] : [];
  });
}

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
export function buildReplyBlocks(segments: TimelineSegment[], streaming?: boolean): ReplyBlock[] {
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
 *
 * A run counts work, not rows: one of its slices may itself be a folded burst of
 * same-kind calls (the projection's own collapse), and that slice stands for
 * every call behind it — its `count`, the same number its row prints when the
 * run is opened. Counting rows instead made the badge contradict the list it
 * reveals: "运行 2 次" + 已写入 + 已运行 read ×3 over four calls.
 */
function stepRunCounts(segments: StepSegment[]): { kind: StepRunKind; count: number; }[] {
  const counts: Record<StepRunKind, number> = { tool: 0, thinking: 0 };
  for (const segment of segments) {
    counts[stepRunKind(segment)] += segment.kind === "tool" ? segment.tool.count ?? 1 : 1;
  }
  return STEP_RUN_ORDER
    .filter(kind => counts[kind] > 0)
    .map(kind => ({ kind, count: counts[kind] }));
}

/** The spoken form of those counts, for the row's accessibility label. */
function stepRunSummary(
  t: (key: string, options?: { count: number; action: string; }) => string,
  counts: { kind: StepRunKind; count: number; }[],
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
 * The summary stays on the left even once the run is open: the line the user
 * just tapped does not move, and its rows appear below in the reading column.
 * Failures remain named in the accessibility label and in the expanded rows;
 * the summary only paints the work counts and disclosure chevron.
 *
 * The row draws as one 20px line but is a real touch target (`ROW_HIT_SLOP`):
 * it is the only way into the run behind it, and a target the size of the text
 * is not enough on a phone.
 */
export function StepRunBlock({
  segments,
  onResolveToolTarget,
}: {
  segments: StepSegment[];
  onResolveToolTarget?: ToolTargetResolver;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const counts = stepRunCounts(segments);
  const summary = stepRunSummary(t, counts);
  // Keep failure context available to screen readers without a summary icon.
  const failedCount = segments.filter(
    segment => segment.kind === "tool" && segment.tool.status === "failed",
  ).length;
  const failureNote = failedCount > 0 ? t("chat.stepsFailed", { count: failedCount }) : null;
  return (
    <View style={[styles.inlineTool, styles.activityBlock]}>
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
              <ToolRow
                key={segment.id}
                opened
                tool={segment.tool}
                resolveTarget={onResolveToolTarget}
              />
            ),
          )}
        </View>
      ) : null}
    </View>
  );
}

/** One inline slice of an assistant reply, in stream order (desktop parity). */
export function ReplyBlockRow({
  block,
  streaming,
  replyStreaming,
  onOpenFile,
  onResolveToolTarget,
}: {
  block: ReplyBlock;
  streaming?: boolean;
  /** Whether the reply itself is still being written (see `ToolRow`). */
  replyStreaming?: boolean;
  onOpenFile?(path: string): void;
  onResolveToolTarget?: ToolTargetResolver;
}) {
  if (block.kind === "steps") {
    return <StepRunBlock segments={block.segments} onResolveToolTarget={onResolveToolTarget} />;
  }
  return (
    <SegmentBlock
      segment={block.segment}
      streaming={streaming}
      replyStreaming={replyStreaming}
      onOpenFile={onOpenFile}
      onResolveToolTarget={onResolveToolTarget}
    />
  );
}

/**
 * The work a settled reply ran, folded above its answer.
 *
 * The phone's `default` output level keeps a reply to the answer it
 * ends on: the reasoning, calls and notes that produced it fold into this one
 * row, so the answer sits at the reading edge of a screen that holds a few
 * lines at a time. Nothing is discarded — opening the row draws exactly the
 * blocks a reader sees with the setting off, in order, each still opening its
 * own detail.
 *
 * The collapsed line follows the folded step run's idiom: glyphs and counts
 * rather than a sentence, because a phone's margin is the whole budget. The
 * wording lives on in the accessibility label.
 */
export function ProcessFold({
  blocks,
  onOpenFile,
  onResolveToolTarget,
}: {
  blocks: ReplyBlock[];
  onOpenFile?(path: string): void;
  onResolveToolTarget?: ToolTargetResolver;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const counts = stepRunCounts(blockSteps(blocks));
  const spoken = stepRunSummary(t, counts);
  const label = expanded ? t("chat.processHide") : t("chat.processShow");
  return (
    <View style={[styles.inlineTool, styles.activityBlock]}>
      <Pressable
        accessibilityLabel={spoken ? `${label} · ${spoken}` : label}
        accessibilityRole="button"
        accessibilityState={{ expanded }}
        hitSlop={ROW_HIT_SLOP}
        onPress={() => setExpanded(value => !value)}
        style={[styles.toolHeader, styles.stepSummaryRow]}
      >
        <ListTree color={colors.inkMuted} size={14} />
        <Text style={styles.toolText}>{label}</Text>
        {counts.map(({ kind, count }) => (
          <Fragment key={kind}>
            <View style={styles.stepSummaryItem}>
              {kind === "thinking"
                ? <Brain color={colors.inkMuted} size={14} />
                : <Wrench color={colors.inkMuted} size={14} />}
              <Text style={styles.toolText}>{t("chat.stepCount", { count })}</Text>
            </View>
          </Fragment>
        ))}
        {expanded ? (
          <ChevronUp color={colors.inkMuted} size={14} />
        ) : (
          <ChevronDown color={colors.inkMuted} size={14} />
        )}
      </Pressable>
      {expanded ? (
        <View style={styles.processChildren}>
          {blocks.map(
            block => (
              <ReplyBlockRow
                block={block}
                key={block.kind === "steps" ? block.segments[0]!.id : block.segment.id}
                onOpenFile={onOpenFile}
                onResolveToolTarget={onResolveToolTarget}
              />
            ),
          )}
        </View>
      ) : null}
    </View>
  );
}

/** One inline slice of an assistant reply, in stream order (desktop parity). */
export function SegmentBlock({
  segment,
  streaming,
  replyStreaming,
  onOpenFile,
  onResolveToolTarget,
}: {
  segment: TimelineSegment;
  streaming?: boolean;
  replyStreaming?: boolean;
  onOpenFile?(path: string): void;
  onResolveToolTarget?: ToolTargetResolver;
}) {
  const styles = useStyles();
  if (segment.kind === "text") {
    return <MarkdownText text={segment.text} onOpenFile={onOpenFile} streaming={streaming} />;
  }
  if (segment.kind === "thinking") {
    return (
      <View style={styles.activityBlock}>
        <ThinkingRow opened={false} streaming={streaming} text={segment.text} />
      </View>
    );
  }
  if (segment.kind === "tool") {
    return (
      <View style={styles.activityBlock}>
        <ToolRow
          opened={false}
          replyStreaming={replyStreaming}
          tool={segment.tool}
          resolveTarget={onResolveToolTarget}
        />
      </View>
    );
  }
  // compaction
  return (
    <CompactionDivider
      status={segment.status}
      tokensAfter={segment.tokensAfter}
      tokensBefore={segment.tokensBefore}
      trigger={segment.trigger}
    />
  );
}
