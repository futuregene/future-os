import { getCurrentWindow, UserAttentionType } from "@tauri-apps/api/window";
import { useEffect } from "react";
import { playDoneBell } from "../../../lib/doneBell";
import { onFutureEvent } from "../../../lib/futureEvents";

/**
 * Global "agent done" alert: when any conversation's run finishes
 * (the `agent_end` bus event), play the WebAudio bell and ask the OS to
 * draw attention to the window (Dock bounce on macOS, taskbar flash on
 * Windows). Gated by the `bellOnComplete` app setting.
 *
 * `agent_end` fires once per finished run regardless of which thread it
 * belongs to — the hook intentionally lives at the shell level, not inside
 * a per-thread component.
 */
export function useAgentDoneBell(enabled: boolean) {
  useEffect(() => {
    if (!enabled)
      return;
    return onFutureEvent("agent_end", () => {
      playDoneBell();
      void getCurrentWindow().requestUserAttention(UserAttentionType.Critical);
    });
  }, [enabled]);
}
