import type { RemotePeerEvent } from "./remotePeerClient";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useState } from "react";
import { requestRemotePeer } from "./remotePeerClient";

/**
 * A pending approval on a remote host.
 *
 * The host's `approval_request` event is the only place a client learns a tool
 * is waiting: unlike a *local* session, there is no approval row in any local
 * store to read. So this hook watches the event stream, and answering is a
 * command back to the host (`approval_decision`).
 */
export interface RemoteApproval {
  id: string;
  sessionId: string;
  toolName: string;
  title: string;
  summary: string;
  riskLevel: string | null;
}

interface ApprovalState {
  approvals: RemoteApproval[];
  decide: (approval: RemoteApproval, decision: "allow" | "deny") => Promise<void>;
  /** A decision that failed to reach the host, keyed by approval id. */
  errors: Record<string, string>;
  /** The id currently being answered, if any. */
  pending: string | null;
}

/**
 * Pending approvals for one remote conversation.
 *
 * Scoped to the open session on purpose. Approvals on *other* sessions are real
 * and the host will keep them pending, but surfacing them here would need a
 * badge on every row and a decision about what to do when the user is looking
 * at something else — so this reports what the user can act on right now, and
 * the catalogue's own `streaming`/`status` flags are what hint that something
 * else is waiting.
 */
export function useRemoteApprovals(
  desktopId: string | null,
  sessionId: string | null,
): ApprovalState {
  const [approvals, setApprovals] = useState<RemoteApproval[]>([]);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [pending, setPending] = useState<string | null>(null);

  useEffect(() => {
    setApprovals([]);
    setErrors({});
    if (!desktopId || !sessionId)
      return;
    let dispose: (() => void) | undefined;
    let cancelled = false;
    void listen<RemotePeerEvent>("remote-peer-event", (event) => {
      const { desktopId: source, kind, payload } = event.payload;
      if (kind !== "event" || source !== desktopId)
        return;
      if (readString(payload, "sessionId") !== sessionId)
        return;
      const type = readString(payload, "type");
      const data = readJson(payload, "data");
      const id = readString(data, "approval_request_id");
      if (!id)
        return;
      if (type === "approval_request") {
        const approval: RemoteApproval = {
          id,
          sessionId,
          toolName: readString(data, "tool_name") ?? "",
          title: readString(data, "title") ?? "",
          summary: readString(data, "summary") ?? "",
          riskLevel: readString(data, "risk_level"),
        };
        // A re-delivered request replaces rather than duplicates: the host can
        // resend on reconnect, and two identical cards for one decision would
        // let the user answer the same question twice.
        setApprovals(current => [...current.filter(item => item.id !== id), approval]);
        return;
      }
      if (type === "approval_decision" || type === "agent_end" || type === "run_finished") {
        // Someone answered (possibly another client), or the run ended: either
        // way the card is no longer actionable here.
        setApprovals(current => current.filter(item => item.id !== id));
      }
    }).then((unlisten) => {
      if (cancelled)
        unlisten();
      else dispose = unlisten;
    }).catch(() => {
      // Not under Tauri (tests, harness): no live approvals, no error.
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [desktopId, sessionId]);

  const decide = useCallback(async (
    approval: RemoteApproval,
    decision: "allow" | "deny",
  ): Promise<void> => {
    if (!desktopId)
      return;
    setPending(approval.id);
    setErrors((current) => {
      const { [approval.id]: _dropped, ...rest } = current;
      return rest;
    });
    try {
      await requestRemotePeer(desktopId, {
        type: "approval_decision",
        sessionId: approval.sessionId,
        entryId: approval.id,
        decision,
      }, approval.sessionId);
      // The host broadcasts `approval_decision`; that is what removes the card.
      // Dropping it locally too keeps the UI honest if the push is lost —
      // a re-delivered request would bring it back, which is correct.
      setApprovals(current => current.filter(item => item.id !== approval.id));
    }
    catch (err) {
      setErrors(current => ({
        ...current,
        [approval.id]: err instanceof Error ? err.message : String(err),
      }));
    }
    finally {
      setPending(null);
    }
  }, [desktopId]);

  return useMemo(
    () => ({ approvals, decide, errors, pending }),
    [approvals, decide, errors, pending],
  );
}

function readString(value: unknown, key: string): string | null {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  return typeof field === "string" ? field : null;
}

function readJson(value: unknown, key: string): unknown {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  if (typeof field !== "string")
    return field ?? null;
  try {
    return JSON.parse(field);
  }
  catch {
    return null;
  }
}
