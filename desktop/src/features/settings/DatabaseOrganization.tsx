import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { formatBytes } from "../../lib/format";
import { SettingsSection } from "./SettingsPrimitives";

interface StorageSize {
  databaseBytes: number;
  walBytes: number;
  totalBytes: number;
}
interface Plan {
  sessionIds: string[];
  storage: StorageSize;
}
interface Step {
  status: "done" | "skipped" | "failed";
  reason?: string;
  hasMore: boolean;
}
interface Finish {
  hasMore: boolean;
  compacted?: boolean;
  deferredReason?: string | null;
  storage: StorageSize;
}
interface Progress {
  completed: number;
  total: number;
  skipped: number;
  failed: number;
  finishing: boolean;
}
interface Result {
  before: StorageSize;
  after: StorageSize;
  compacted: boolean;
}

function step<T,>(phase: "plan" | "session" | "finish", sessionId?: string) {
  return invokeCommand<T>("organize_agent_database", { phase, sessionId: sessionId ?? null });
}

export function DatabaseOrganization({ disabled, onBusyChange }: { disabled: boolean; onBusyChange: (busy: boolean) => void }) {
  const { t } = useTranslation("settings");
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);
  const [result, setResult] = useState<Result | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [failures, setFailures] = useState<string[]>([]);

  async function organize() {
    setRunning(true);
    onBusyChange(true);
    setResult(null);
    setProgress(null);
    setError(null);
    setFailures([]);
    const current: Progress = { completed: 0, total: 0, skipped: 0, failed: 0, finishing: false };
    const errors: string[] = [];
    try {
      const plan = await step<Plan>("plan");
      current.total = plan.sessionIds.length;
      setProgress({ ...current });
      for (const sessionId of plan.sessionIds) {
        try {
          let response: Step;
          do {
            response = await step<Step>("session", sessionId);
          } while (response.hasMore);
          if (response.status === "skipped") {
            current.skipped++;
          }
          else if (response.status === "failed") {
            current.failed++;
            errors.push(`#${current.completed + 1}: ${response.reason ?? t("reset.organizeSessionFailed")}`);
          }
        }
        catch (cause) {
          current.failed++;
          errors.push(`#${current.completed + 1}: ${cause instanceof Error ? cause.message : String(cause)}`);
        }
        current.completed++;
        setProgress({ ...current });
      }
      setFailures(errors);
      current.finishing = true;
      setProgress({ ...current });
      let finish: Finish;
      do {
        finish = await step<Finish>("finish");
      } while (finish.hasMore);
      setResult({ before: plan.storage, after: finish.storage, compacted: finish.compacted === true });
    }
    catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
    finally {
      setRunning(false);
      onBusyChange(false);
    }
  }

  const saved = result ? result.before.totalBytes - result.after.totalBytes : 0;
  return (
    <SettingsSection>
      <div className="space-y-3 rounded-lg border border-line-soft p-4">
        <div>
          <div className="text-sm font-medium text-ink">{t("reset.organizeDatabase")}</div>
          <p className="mt-1 text-xs leading-5 text-ink-muted">{t("reset.organizeDescription")}</p>
        </div>
        <div aria-live="polite" className="space-y-2 text-xs leading-5 text-ink-muted">
          {progress && (
            <p>
              {running && progress.finishing
                ? t("reset.organizeFinishing")
                : t("reset.organizeProgress", { completed: progress.completed, total: progress.total })}
              {" · "}
              {t("reset.organizeCounts", { skipped: progress.skipped, failed: progress.failed })}
            </p>
          )}
          {result && (
            <>
              <dl className="grid grid-cols-2 gap-x-4 gap-y-1 tabular-nums">
                <dt>{t("reset.organizeBefore")}</dt>
                <dd className="text-right text-ink">{formatBytes(result.before.totalBytes)}</dd>
                <dt>{t("reset.organizeAfter")}</dt>
                <dd className="text-right text-ink">{formatBytes(result.after.totalBytes)}</dd>
                <dt>{saved >= 0 ? t("reset.organizeSaved") : t("reset.organizeIncreased")}</dt>
                <dd className="text-right font-medium text-ink">{formatBytes(Math.abs(saved))}</dd>
              </dl>
              <p>{result.compacted ? t("reset.organizeComplete") : t("reset.organizeDeferred")}</p>
            </>
          )}
        </div>
        {error && <p className="text-xs text-danger">{error}</p>}
        {failures.length > 0 && (
          <details className="text-xs text-ink-muted">
            <summary className="cursor-pointer">{t("reset.organizeFailureDetails")}</summary>
            <ul className="mt-2 max-h-40 space-y-1 overflow-auto wrap-break-word">
              {failures.map(failure => <li key={failure}>{failure}</li>)}
            </ul>
          </details>
        )}
        <Button disabled={disabled || running} onClick={() => void organize()} size="sm" variant="secondary">
          {running ? t("reset.organizeRunning") : t("reset.organizeStart")}
        </Button>
      </div>
    </SettingsSection>
  );
}
