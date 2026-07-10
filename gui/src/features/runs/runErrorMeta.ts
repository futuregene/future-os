import type { LucideIcon } from "lucide-react";
import type { RunErrorType } from "./runDisplayFormatters";
import { Bot, CircleHelp, CircleStop, Clock, TriangleAlert, Unplug } from "lucide-react";

/**
 * Per-error-type presentation: icon, category color, and label key. Colors are
 * deliberate category colors (COLOR.md exception), not semantic tokens. This is
 * a view-layer concern kept out of `runDisplayFormatters` (run-status logic),
 * keyed by the error type the store hands us.
 */
const ERROR_TYPE_META: Record<RunErrorType, { Icon: LucideIcon; color: string; labelKey: string }> = {
  stream_disconnected: { Icon: Unplug, color: "text-orange-600", labelKey: "errorType.streamDisconnected" },
  command_failed: { Icon: TriangleAlert, color: "text-red-600", labelKey: "errorType.commandFailed" },
  model_failed: { Icon: Bot, color: "text-purple-600", labelKey: "errorType.modelFailed" },
  abort_requested: { Icon: CircleStop, color: "text-gray-600", labelKey: "errorType.abortRequested" },
  timeout: { Icon: Clock, color: "text-yellow-600", labelKey: "errorType.timeout" },
  unknown: { Icon: CircleHelp, color: "text-gray-600", labelKey: "errorType.unknown" },
};

export function errorTypeMeta(errorType?: RunErrorType | null) {
  return errorType ? ERROR_TYPE_META[errorType] ?? null : null;
}
