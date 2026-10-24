import { FileText, Pencil, TerminalSquare, TriangleAlert } from "lucide-react-native";
import { useContext, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AppState, Text, View } from "react-native";
import { TimelineVisibleContext } from "../../../components/PausedTimeline";
import { basename } from "../../../remote/localPath";
import { spacing } from "../../../theme/tokens";
import { useTheme } from "../../../theme/ThemeContext";
import { useStyles } from "./messageStyles";

// Same shape as the desktop footer (desktop/src/lib/date.ts formatDuration): "5s"
// under a minute, "1m 25s" above — so phone and desktop read identically.
export function formatDuration(durationMs: number): string {
  const totalSeconds = Math.max(0, Math.round(durationMs / 1_000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  return `${Math.floor(totalSeconds / 60)}m ${totalSeconds % 60}s`;
}

/**
 * A step row draws as one muted 20px line to stay out of the reader's way, but
 * it is also a control. Extending the touch area vertically reaches the 44px
 * `layout.touchTarget` without giving the row back the height it was folded to
 * save — the visual line stays 20px, the tappable one is 44px.
 *
 * 12 a side, not 8: 20 + 8 + 8 left the target at 36px, which measured short of
 * the target every other control in the app keeps (the copy button beside it
 * reaches 44 from a 28px box with the same arithmetic).
 */
export const ROW_HIT_SLOP = { top: spacing.md, bottom: spacing.md } as const;

export function RunIndicator({ startedAt }: { startedAt?: number; }) {
  const styles = useStyles();
  const { t } = useTranslation();
  const visible = useContext(TimelineVisibleContext);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!startedAt || !visible) return;
    let timer: ReturnType<typeof setInterval> | undefined;
    const updateActivity = (state: string | null | undefined) => {
      clearInterval(timer);
      if (state != null && state !== "active") return;
      // The label only displays whole seconds. Do not wake twice per second
      // or keep its timer alive while the app is backgrounded.
      timer = setInterval(() => setNow(Date.now()), 1000);
    };
    updateActivity(AppState.currentState);
    const subscription = AppState.addEventListener("change", updateActivity);
    return () => { clearInterval(timer); subscription.remove(); };
  }, [startedAt, visible]);
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
export type ToolKind = "shell" | "write" | "edit" | "read";

export function toolKind(name: string): ToolKind {
  if (name === "shell" || name === "write" || name === "edit" || name === "read") return name;
  return "shell";
}

export function ToolGlyph({ kind, failed = false }: { kind: ToolKind; failed?: boolean; }) {
  const { colors } = useTheme();
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

export function toolLabel(t: (key: string) => string, kind: ToolKind, complete: boolean): string {
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

export function failedToolLabel(t: (key: string) => string, kind: ToolKind): string {
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

export function toolDetail(kind: ToolKind, detail: string): string {
  // Mobile intentionally shows only the filename for file tools. This differs
  // from desktop's full-path activity detail by product request: a phone row
  // cannot use the leading directories, and preserving the filename keeps the
  // useful part visible. Shell commands remain verbatim.
  return kind === "read" || kind === "write" || kind === "edit" ? basename(detail) : detail;
}
