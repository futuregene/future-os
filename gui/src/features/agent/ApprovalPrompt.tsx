import type { StoredApprovalRequest } from "../../integrations/storage/threadStore";
import { AlertTriangle, Check, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

interface ApprovalPromptProps {
  approval: StoredApprovalRequest;
  onDecision: (approval: StoredApprovalRequest, status: "approved" | "rejected") => Promise<void>;
}

export function ApprovalPrompt({ approval, onDecision }: ApprovalPromptProps) {
  const [error, setError] = useState<string | null>(null);
  const [deciding, setDeciding] = useState<"approved" | "rejected" | null>(null);
  const requestedAction = formatRequestedAction(approval.requestedAction);

  const decide = useCallback(async (status: "approved" | "rejected") => {
    if (deciding)
      return;

    setError(null);
    setDeciding(status);
    try {
      await onDecision(approval, status);
    }
    catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    }
    finally {
      setDeciding(null);
    }
  }, [approval, deciding, onDecision]);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (deciding)
        return;

      if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
        event.preventDefault();
        void decide("approved");
        return;
      }

      if (event.key === "Escape") {
        event.preventDefault();
        void decide("rejected");
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [decide, deciding]);

  return (
    <section className="rounded-lg border border-line bg-white/95 p-4 shadow-panel backdrop-blur">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className="size-2 shrink-0 rounded-full bg-amber-500" />
          <h2 className="truncate text-base font-semibold text-ink">{approval.title}</h2>
        </div>
      </div>
      {approval.summary ? <p className="mt-3 text-sm leading-5 text-ink-soft">{approval.summary}</p> : null}
      {requestedAction
        ? (
            <pre className="mt-3 max-h-[33vh] overflow-auto whitespace-pre-wrap break-words rounded-md bg-surface-subtle p-3 text-xs leading-5 text-ink-soft">
              <code className="block min-w-0">{requestedAction}</code>
            </pre>
          )
        : null}
      {error
        ? (
            <div className="mt-3 flex items-center gap-2 text-xs leading-5 text-red-600">
              <AlertTriangle className="size-3.5 shrink-0" />
              <span>{error}</span>
            </div>
          )
        : null}
      <div className="mt-4 flex items-center justify-between gap-3">
        <button
          className="inline-flex h-9 items-center gap-2 rounded-md border border-line bg-white px-3 text-sm font-medium text-ink-soft shadow-sm transition-colors hover:bg-surface-subtle hover:text-ink disabled:cursor-not-allowed disabled:opacity-60"
          disabled={deciding !== null}
          onClick={() => void decide("rejected")}
          type="button"
        >
          <X className="size-3.5" />
          {deciding === "rejected" ? "Denying" : "Deny"}
        </button>
        <button
          className="inline-flex h-9 items-center gap-2 rounded-md border border-accent bg-accent px-3 text-sm font-medium text-white shadow-sm transition-colors hover:bg-blue-700 disabled:cursor-not-allowed disabled:opacity-60"
          disabled={deciding !== null}
          onClick={() => void decide("approved")}
          type="button"
        >
          <Check className="size-3.5" />
          {deciding === "approved" ? "Allowing" : "Allow once"}
        </button>
      </div>
    </section>
  );
}

function formatRequestedAction(action: string | null | undefined) {
  if (!action)
    return "";

  try {
    const parsed = parseNestedJson(action);
    if (isRecord(parsed) && typeof parsed.command === "string") {
      return parsed.command;
    }
    return JSON.stringify(parsed, null, 2);
  }
  catch {
    return action;
  }
}

function parseNestedJson(value: string) {
  let current: unknown = value;
  for (let index = 0; index < 3; index += 1) {
    if (typeof current !== "string")
      return current;
    current = JSON.parse(current) as unknown;
  }
  return current;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
