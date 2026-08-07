import type {
  ApprovalAction,
  StoredApprovalRequest,
} from "../../integrations/storage/types";
import { AlertTriangle, Check, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/TextInput";
import { saveApprovalRule } from "../../integrations/storage/runs";
import { formatRequestedAction, parseAction, parseSaveSuggestion } from "./approvalPayload";

// Localized title/summary per approval kind (the agent sends English). An
// unmapped kind falls back to the agent-provided strings.
const KIND_I18N: Record<string, { title: string; summary?: string }> = {
  file_read: { summary: "approval.readSummary", title: "approval.readTitle" },
  file_write: { summary: "approval.writeSummary", title: "approval.writeTitle" },
  outside_workspace_write: {
    summary: "approval.outsideWriteSummary",
    title: "approval.outsideWriteTitle",
  },
  sandbox_escalation: { title: "approval.escalationTitle" }, // note carries the rest
  shell_command: { summary: "approval.shellSummary", title: "approval.shellTitle" },
};

interface ApprovalPromptProps {
  approval: StoredApprovalRequest;
  onDecision: (approval: StoredApprovalRequest, status: "approved" | "rejected") => Promise<void>;
  /// Conversation type — a plain Chat's workspace is a throwaway temp dir, so
  /// "allow in this workspace" reads as "allow in this chat" to the user.
  threadMode?: "chat" | "workspace";
}

export function ApprovalPrompt({ approval, onDecision, threadMode }: ApprovalPromptProps) {
  const { t } = useTranslation("agent");
  const [error, setError] = useState<string | null>(null);
  const [deciding, setDeciding] = useState<"approved" | "rejected" | null>(null);

  const action = useMemo(() => parseAction(approval.actionPayload), [approval.actionPayload]);
  // Title/summary come from the agent in English; localize by kind, falling
  // back to the agent strings for any unmapped kind.
  const kindI18n = KIND_I18N[approval.kind];
  const titleText = kindI18n ? t(kindI18n.title) : approval.title;
  const summaryText = kindI18n
    ? (kindI18n.summary ? t(kindI18n.summary) : null)
    : approval.summary;
  const saveSuggestion = useMemo(
    () => parseSaveSuggestion(approval.saveSuggestion),
    [approval.saveSuggestion],
  );
  const fallbackAction = useMemo(
    () => (action ? null : formatRequestedAction(approval.requestedAction)),
    [action, approval.requestedAction],
  );

  // Inline "allow in this workspace" editor: the editable path glob to persist.
  const [editorOpen, setEditorOpen] = useState(false);
  const [pattern, setPattern] = useState("");

  const openRuleEditor = useCallback(() => {
    if (!saveSuggestion)
      return;
    setPattern(saveSuggestion.path);
    setEditorOpen(true);
  }, [saveSuggestion]);

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

  // Save the (possibly edited) rule to the workspace file, then approve once.
  const confirmRule = useCallback(async () => {
    if (deciding || !editorOpen || !saveSuggestion)
      return;
    const trimmed = pattern.trim();
    if (!trimmed) {
      setError(t("approval.rulePatternRequired"));
      return;
    }
    setError(null);
    setDeciding("approved");
    try {
      await saveApprovalRule({
        threadId: approval.threadId,
        path: trimmed,
        access: saveSuggestion.access,
      });
      await onDecision(approval, "approved");
    }
    catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
      setDeciding(null);
    }
  }, [approval, deciding, editorOpen, onDecision, pattern, saveSuggestion, t]);

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
        // Esc closes the rule editor first, then rejects on a second press.
        if (editorOpen) {
          setEditorOpen(false);
          return;
        }
        void decide("rejected");
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [decide, deciding, editorOpen]);

  return (
    <section className="rounded-lg border border-line bg-surface/95 p-4 shadow-panel backdrop-blur">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className="size-2 shrink-0 rounded-full bg-warning" />
          <h2 className="truncate text-base font-semibold text-ink">{titleText}</h2>
        </div>
      </div>
      {summaryText ? <p className="mt-2 text-sm leading-5 text-ink-soft">{summaryText}</p> : null}
      {action ? <ActionDetails action={action} /> : null}
      {fallbackAction
        ? (
            <pre className="mt-3 max-h-[33vh] overflow-auto whitespace-pre-wrap wrap-break-word rounded-md bg-surface-subtle p-3 text-xs leading-5 text-ink-soft">
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
      {editorOpen && saveSuggestion
        ? (
            <div className="mt-3 rounded-md border border-line-soft bg-surface-subtle p-3">
              <div className="mb-1.5 text-[11px] font-medium uppercase tracking-wide text-ink-soft">
                {t("approval.ruleSaveWorkspace")}
              </div>
              <TextInput
                autoFocus
                className="font-mono text-xs"
                onChange={event => setPattern(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") {
                    event.preventDefault();
                    void confirmRule();
                  }
                }}
                value={pattern}
              />
              <div className="mt-2 flex items-center justify-end gap-2">
                <Button disabled={deciding !== null} onClick={() => setEditorOpen(false)} variant="toolbar">
                  {t("approval.ruleCancel")}
                </Button>
                <Button
                  disabled={deciding !== null}
                  leftIcon={<Check className="size-3.5" />}
                  onClick={() => void confirmRule()}
                  variant="primary"
                >
                  {t("approval.ruleConfirm")}
                </Button>
              </div>
            </div>
          )
        : null}
      <div className="mt-4 flex items-center justify-between gap-3">
        <Button
          className="shadow-xs"
          disabled={deciding !== null}
          leftIcon={<X className="size-3.5" />}
          onClick={() => void decide("rejected")}
          variant="toolbar"
        >
          {deciding === "rejected" ? t("approval.denying") : t("approval.deny")}
        </Button>
        <div className="flex items-center gap-2">
          {saveSuggestion
            ? (
                <Button
                  disabled={deciding !== null}
                  onClick={() => openRuleEditor()}
                  variant="toolbar"
                >
                  {threadMode === "chat" ? t("approval.allowChat") : t("approval.allowWorkspace")}
                </Button>
              )
            : null}
          <Button
            className="shadow-xs"
            disabled={deciding !== null}
            leftIcon={<Check className="size-3.5" />}
            onClick={() => void decide("approved")}
            variant="primary"
          >
            {deciding === "approved" ? t("approval.allowing") : t("approval.allowOnce")}
          </Button>
        </div>
      </div>
    </section>
  );
}

interface ActionDetailsProps {
  action: ApprovalAction;
}

function ActionDetails({ action }: ActionDetailsProps) {
  const { t } = useTranslation("agent");
  if (action.command) {
    const isEscalation = action.category === "sandbox_escalation";
    return (
      <div className="mt-3 space-y-2">
        <div>
          <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-ink-soft">
            {t("approval.shellCommand")}
          </div>
          <pre className="max-h-[33vh] overflow-auto whitespace-pre-wrap wrap-break-word rounded-md bg-surface-subtle p-3 font-mono text-xs leading-5 text-ink">
            <code className="block min-w-0">{action.command}</code>
          </pre>
        </div>
        {isEscalation && action.justification
          ? (
              <div>
                <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-ink-soft">
                  {t("approval.justification")}
                </div>
                <p className="rounded-md bg-surface-subtle p-3 text-xs leading-5 text-ink-soft">
                  {action.justification}
                </p>
              </div>
            )
          : null}
        {isEscalation && action.blockedPaths && action.blockedPaths.length > 0
          ? (
              <div>
                <div className="mb-1 text-[11px] font-medium uppercase tracking-wide text-ink-soft">
                  {t("approval.blockedPaths")}
                </div>
                <ul className="rounded-md bg-surface-subtle p-3 font-mono text-xs leading-5 text-ink">
                  {action.blockedPaths.map(path => (
                    <li key={path} className="break-all">{path}</li>
                  ))}
                </ul>
              </div>
            )
          : null}
        {isEscalation
          ? (
              <p className="text-xs leading-5 text-warning">
                {t("approval.unsandboxedNote")}
              </p>
            )
          : null}
      </div>
    );
  }

  if (action.writes && action.writes.length > 0) {
    return (
      <div className="mt-3 space-y-2">
        <div className="text-[11px] font-medium uppercase tracking-wide text-ink-soft">
          {action.writes.length === 1 ? t("approval.writeFile") : t("approval.writeFiles", { count: action.writes.length })}
        </div>
        {action.writes.map((entry, index) => (
          <div
            className="rounded-md bg-surface-subtle p-3"
            key={`${entry.path}-${String(index)}`}
          >
            <div className="font-mono text-xs text-ink">{entry.path}</div>
            {entry.preview
              ? (
                  <pre className="mt-2 max-h-32 overflow-auto whitespace-pre-wrap wrap-break-word text-xs leading-5 text-ink-soft">
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
          {action.deletes.length === 1 ? t("approval.deleteFile") : t("approval.deleteFiles", { count: action.deletes.length })}
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
          {t("approval.paths")}
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
