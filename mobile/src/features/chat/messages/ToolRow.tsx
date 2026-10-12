import { ChevronDown, ChevronUp } from "lucide-react-native";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, Text, View } from "react-native";
import type { TimelineToolRow } from "../../../remote/types";
import { colors } from "../../../theme/tokens";
import { styles } from "./messageStyles";
import type { ToolTargetResolver } from "./types";
import { ROW_HIT_SLOP, toolKind, ToolGlyph, toolLabel, failedToolLabel, toolDetail } from "./messageUtils";
import type { ToolKind } from "./messageUtils";

/**
 * The inline tool-activity row inside a reply bubble (desktop parity:
 * AgentActivityLine). A failed tool (shell non-zero exit / error) swaps its
 * normal tool glyph for the desktop-matching alert glyph; a collapsed same-kind
 * burst carries its child calls on `tool.children` and its count — the row
 * reads "Ran N commands", and tapping it reveals the individual targets.
 *
 * Collapsed rows start at the reply's left edge. `opened` keeps rows inside an
 * expanded run full-width so their targets retain the existing reading layout.
 *
 * The target — a file name or a shell command — always renders on its own line
 * below the header, and the header is a wrapping-free one-liner because of it.
 * Inline was the alternative, and it cannot work: a target beside the label
 * claims the row's leftover width with `flex: 1`, which resolves to zero in the
 * shrink-wrapped rail badge and hid the file name entirely; and a name long
 * enough to need the room would push the label and its chevron off the line.
 * Below, the target gets the full width, wraps instead of being clipped, and
 * carries the header's own indent so it reads as that row's detail.
 */
export function ToolRow({
  tool,
  opened,
  resolveTarget,
}: {
  tool: TimelineToolRow;
  opened: boolean;
  resolveTarget?: ToolTargetResolver;
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  // A command the page omitted, fetched when the row is opened. Kept for this
  // row's lifetime, so collapsing and re-opening never asks again.
  const [fetchedTarget, setFetchedTarget] = useState<string | null>(null);
  const [fetching, setFetching] = useState(false);
  const open = opened || expanded;
  const kind = toolKind(tool.name);
  const failed = tool.status === "failed";
  const rawDetail = tool.detail ?? fetchedTarget ?? undefined;
  const detail = rawDetail?.trim() ? toolDetail(kind, rawDetail.trim()) : null;
  const children = tool.children && tool.children.length > 0 ? tool.children : null;
  // A row the page carried without a target can ask for it back — but only
  // with the call's identity, which is what a lean page still carries.
  const fetchable =
    !detail && !children && Boolean(tool.toolCallId && tool.runId && resolveTarget);
  const expandable = Boolean(detail || children || fetchable);
  const reveal = () => {
    if (!fetchable || fetching) return;
    setFetching(true);
    // The fetch is a convenience, never a dependency: a failure (offline, or a
    // desktop that predates this command) leaves the row exactly as it was and
    // must not surface as an unhandled rejection.
    Promise.resolve()
      .then(() => resolveTarget!(tool.toolCallId!, tool.runId!))
      .then(target => {
        if (target && target.trim()) {
          setFetchedTarget(target);
          setExpanded(true);
        }
      })
      .catch(() => undefined)
      .finally(() => setFetching(false));
  };
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
        accessibilityState={{ expanded }}
        disabled={!expandable}
        hitSlop={expandable ? ROW_HIT_SLOP : undefined}
        onPress={() => {
          // A row with something to show opens and closes as before; a row
          // that only has a target to fetch triggers the fetch (it opens when
          // the target arrives, so a failed fetch leaves it untouched).
          if (detail || children) setExpanded(value => !value);
          else reveal();
        }}
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
      </Pressable>
      {/* Every revealed target — file name or shell command — takes the line
          below the header, so it wraps at full width instead of being clipped
          into the label's line. */}
      {expanded && detail && !children ? (
        <Text selectable style={styles.toolDetailText}>
          {detail}
        </Text>
      ) : null}
      {expanded && children ? (
        <View style={styles.inlineToolChildren}>
          {children.map((child, index) => (
            <ToolChildRow
              key={`${child.name}:${child.detail ?? ""}:${index}`}
              child={child}
              resolveTarget={resolveTarget}
            />
          ))}
        </View>
      ) : null}
    </View>
  );
}

/**
 * One call inside an expanded burst, e.g. a command under "运行 2 次".
 *
 * A lean history page carries no shell `arguments`, so a burst's children are
 * labels — and the group row has no call identity of its own, so opening it
 * could never reveal what ran. Each child that still has its identity therefore
 * fetches its own command, the same way a single tool row does: one tap, one
 * command, and only for the calls the reader actually opens.
 */
function ToolChildRow({
  child,
  resolveTarget,
}: {
  child: TimelineToolRow;
  resolveTarget?: ToolTargetResolver;
}) {
  const { t } = useTranslation();
  const [fetchedTarget, setFetchedTarget] = useState<string | null>(null);
  const [fetching, setFetching] = useState(false);
  const kind = toolKind(child.name);
  const rawDetail = child.detail ?? fetchedTarget ?? undefined;
  const detail = rawDetail?.trim() ? toolDetail(kind, rawDetail.trim()) : null;
  const fetchable = !detail && Boolean(child.toolCallId && child.runId && resolveTarget);
  const label = toolLabel(t, kind, child.complete);
  if (!fetchable) {
    return (
      <Text selectable style={styles.inlineToolChild}>
        {detail ?? label}
      </Text>
    );
  }
  return (
    <Pressable
      accessibilityRole="button"
      disabled={fetching}
      hitSlop={ROW_HIT_SLOP}
      onPress={() => {
        if (fetching) return;
        setFetching(true);
        // A convenience, never a dependency: a failure leaves the line as it
        // was rather than surfacing an unhandled rejection.
        Promise.resolve()
          .then(() => resolveTarget!(child.toolCallId!, child.runId!))
          .then(target => {
            if (target && target.trim()) setFetchedTarget(target);
          })
          .catch(() => undefined)
          .finally(() => setFetching(false));
      }}
      style={styles.inlineToolChildRow}
    >
      <Text selectable style={styles.inlineToolChild}>
        {detail ?? label}
      </Text>
      {detail ? null : <ChevronDown color={colors.inkMuted} size={12} />}
    </Pressable>
  );
}

const TOOL_LABEL_KEYS: Record<ToolKind, string> = {
  shell: "chat.stepRun", read: "chat.stepRead", write: "chat.stepWrite", edit: "chat.stepEdit",
};
