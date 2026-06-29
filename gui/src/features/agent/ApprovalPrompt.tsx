import type {
  ApprovalAction,
  SandboxBoundary,
  StoredApprovalRequest,
} from "../../integrations/storage/types";
import { AlertTriangle, Check, ShieldAlert, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

interface ApprovalPromptProps {
  approval: StoredApprovalRequest;
  onDecision: (approval: StoredApprovalRequest, status: "approved" | "rejected") => Promise<void>;
}

export function ApprovalPrompt({ approval, onDecision }: ApprovalPromptProps) {
  const [error, setError] = useState<string | null>(null);
  const [deciding, setDeciding] = useState<"approved" | "rejected" | null>(null);

  const action = useMemo(() => parseAction(approval.actionPayload), [approval.actionPayload]);
  const sandboxBoundary = useMemo(
    () => parseSandbox(approval.sandboxBoundary),
    [approval.sandboxBoundary],
  );
  const fallbackAction = useMemo(
    () => (action ? null : formatRequestedAction(approval.requestedAction)),
    [action, approval.requestedAction],
  );

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
      if (isEditableTarget(event.target))
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
    <section className="rounded-lg border border-line bg-surface/95 p-4 shadow-panel backdrop-blur">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className="size-2 shrink-0 rounded-full bg-warning" />
          <h2 className="truncate text-base font-semibold text-ink">{approval.title}</h2>
        </div>
      </div>
      {approval.summary ? <p className="mt-2 text-sm leading-5 text-ink-soft">{approval.summary}</p> : null}
      {sandboxBoundary?.violation
        ? (
            <div className="mt-3 inline-flex items-center gap-1.5 rounded-md border border-warning-line bg-warning-soft px-2 py-1 text-xs font-medium text-warning">
              <ShieldAlert className="size-3" />
              <span>{formatViolation(sandboxBoundary.violation)}</span>
              <span className="text-warning/70">·</span>
              <span className="text-warning/70">
                sandbox:
                {" "}
                {sandboxBoundary.mode}
              </span>
            </div>
          )
        : null}
      {action ? <ActionDetails action={action} /> : null}
      {fallbackAction
        ? (
            <pre className="mt-3 max-h-[33vh] overflow-auto whitespace-pre-wrap break-words rounded-md bg-surface-subtle p-3 text-xs leading-5 text-ink-soft">
              <code className="block min-w-0">{fallbackAction}</code>
            </pre>
          )
        : null}
      {error
        ? (
            <div className="mt-3 flex items-center gap-2 text-xs leading-5 text-danger">
              <AlertTriangle className="size-3.5 shrink-0" />
              <span>{error}</span>
            </div>
          )
        : null}
      <div className="mt-4 flex items-center justify-between gap-3">
        <button
          className="inline-flex h-9 items-center gap-2 rounded-md border border-line bg-surface px-3 text-sm font-medium text-ink-soft shadow-sm transition-colors hover:bg-surface-subtle hover:text-ink disabled:cursor-not-allowed disabled:opacity-60"
          disabled={deciding !== null}
          onClick={() => void decide("rejected")}
          type="button"
        >
          <X className="size-3.5" />
          {deciding === "rejected" ? "Denying" : "Deny"}
        </button>
        <button
          className="inline-flex h-9 items-center gap-2 rounded-md border border-accent bg-accent px-3 text-sm font-medium text-white shadow-sm transition-colors hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-60"
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

interface ActionDetailsProps {
  action: ApprovalAction;
}

function ActionDetails({ action }: ActionDetailsProps) {
  if (action.command) {
    return (
      <div className="mt-3">
        <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-ink-soft">
          Shell command
        </div>
        <pre className="max-h-[33vh] overflow-auto whitespace-pre-wrap break-words rounded-md bg-surface-subtle p-3 font-mono text-xs leading-5 text-ink">
          <code className="block min-w-0">{action.command}</code>
        </pre>
      </div>
    );
  }

  if (action.writes && action.writes.length > 0) {
    return (
      <div className="mt-3 space-y-2">
        <div className="text-[11px] font-medium uppercase tracking-wide text-ink-soft">
          {action.writes.length === 1 ? "Write file" : `Write ${action.writes.length} files`}
        </div>
        {action.writes.map((entry, index) => (
          <div
            className="rounded-md bg-surface-subtle p-3"
            key={`${entry.path}-${String(index)}`}
          >
            <div className="font-mono text-xs text-ink">{entry.path}</div>
            {entry.preview
              ? (
                  <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap break-words text-xs leading-5 text-ink-soft">
                    <code className="block min-w-0">{entry.preview}</code>
                  </pre>
                )
              : null}
          </div>
        ))}
      </div>
    );
  }

  if (action.deletes && action.deletes.length > 0) {
    return (
      <div className="mt-3">
        <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-ink-soft">
          {action.deletes.length === 1 ? "Delete file" : `Delete ${action.deletes.length} files`}
        </div>
        <ul className="space-y-1 rounded-md bg-surface-subtle p-3 font-mono text-xs text-ink">
          {action.deletes.map((entry, index) => (
            <li key={`${entry.path}-${String(index)}`}>{entry.path}</li>
          ))}
        </ul>
      </div>
    );
  }

  if (action.paths && action.paths.length > 0) {
    return (
      <div className="mt-3">
        <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-ink-soft">
          Paths
        </div>
        <ul className="space-y-1 rounded-md bg-surface-subtle p-3 font-mono text-xs text-ink">
          {action.paths.map((entry, index) => (
            <li key={`${entry}-${String(index)}`}>{entry}</li>
          ))}
        </ul>
      </div>
    );
  }

  if (action.summary) {
    return (
      <p className="mt-3 rounded-md bg-surface-subtle p-3 text-xs leading-5 text-ink-soft">
        {action.summary}
      </p>
    );
  }

  return null;
}

function isEditableTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement))
    return false;

  const tagName = target.tagName.toLowerCase();
  return target.isContentEditable
    || tagName === "input"
    || tagName === "textarea"
    || tagName === "select";
}

function parseAction(payload: string | null | undefined): ApprovalAction | null {
  if (!payload)
    return null;
  try {
    const parsed = JSON.parse(payload) as unknown;
    if (isRecord(parsed) && typeof parsed.tool === "string" && typeof parsed.category === "string") {
      return parsed as unknown as ApprovalAction;
    }
    return null;
  }
  catch {
    return null;
  }
}

function parseSandbox(payload: string | null | undefined): SandboxBoundary | null {
  if (!payload)
    return null;
  try {
    const parsed = JSON.parse(payload) as unknown;
    if (isRecord(parsed) && typeof parsed.mode === "string") {
      return parsed as unknown as SandboxBoundary;
    }
    return null;
  }
  catch {
    return null;
  }
}

function formatViolation(violation: string) {
  return violation
    .replace(/_/g, " ")
    .replace(/^\w/, char => char.toUpperCase());
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
