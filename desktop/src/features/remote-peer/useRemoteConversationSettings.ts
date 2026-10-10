import type { RemoteModelInfo, RemotePeerEvent } from "./remotePeerClient";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  getRemoteConversationSettings,
  listRemoteModels,
  setRemoteConversationModel,
  setRemoteConversationThinkingLevel,
} from "./remotePeerClient";

/**
 * The event kinds that mean a conversation's settings moved.
 *
 * The host applies these itself and announces them, so a change made on that
 * machine's own screen — or by the agent at the start of a run — has to be
 * followed here. Without it the picker would show what the user last chose
 * locally while the host was using something else.
 */
const SETTINGS_EVENTS = new Set(["model_changed", "thinking_level_changed"]);

export interface RemoteConversationSettingsState {
  /** This conversation's model reference, or `null` before the host has said. */
  model: string | null;
  thinkingLevel: string | null;
  /** The host's catalogue, for the picker. Empty until it answers. */
  models: RemoteModelInfo[];
  /** True while a change is in flight, so the picker can say so. */
  saving: boolean;
  /** A failed read or write, for the caller to show. */
  error: string | null;
  setModel: (reference: string) => Promise<void>;
  setThinkingLevel: (level: string) => Promise<void>;
}

/**
 * A remote conversation's model and thinking level, and the writes for them.
 *
 * Optimistic on change: the picker shows the new value straight away and the
 * host's* announcement is what confirms it. Waiting for a round trip to move a
 * select leaves the control looking stuck, and the host is the authority either
 * way — if it refuses, its next `get_state` or its error puts the real value
 * back.
 */
export function useRemoteConversationSettings(
  desktopId: string | null,
  sessionId: string | null,
  enabled: boolean,
): RemoteConversationSettingsState {
  const [model, setModel] = useState<string | null>(null);
  const [thinkingLevel, setThinkingLevel] = useState<string | null>(null);
  const [models, setModels] = useState<RemoteModelInfo[]>([]);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /**
   * Guards against a slow reply landing after the user has moved to another
   * conversation — the same hazard the timeline has, and the same fix.
   */
  const epochRef = useRef(0);

  const read = useCallback(async (
    desktop: string,
    session: string,
  ): Promise<void> => {
    const epoch = ++epochRef.current;
    try {
      const [settings, catalogue] = await Promise.all([
        getRemoteConversationSettings(desktop, session),
        // The catalogue is per host, not per conversation, but reading it here
        // keeps one epoch and one failure path for both.
        listRemoteModels(desktop).catch(() => [] as RemoteModelInfo[]),
      ]);
      if (epoch !== epochRef.current)
        return;
      setModel(settings.model);
      setThinkingLevel(settings.thinkingLevel);
      setModels(catalogue);
      setError(null);
    }
    catch (err) {
      if (epoch === epochRef.current)
        setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  // Open: read. Cleared first so another conversation's model cannot show for a
  // frame under the new one's title.
  useEffect(() => {
    epochRef.current += 1;
    setModel(null);
    setThinkingLevel(null);
    setModels([]);
    setError(null);
    if (!enabled || !desktopId || !sessionId)
      return;
    void read(desktopId, sessionId);
  }, [enabled, desktopId, sessionId, read]);

  // Live: follow the host's own changes for this conversation.
  useEffect(() => {
    if (!enabled || !desktopId || !sessionId)
      return;
    let dispose: (() => void) | undefined;
    let cancelled = false;
    void listen<RemotePeerEvent>("remote-peer-event", (event) => {
      const { desktopId: source, kind, payload } = event.payload;
      if (kind !== "event" || source !== desktopId)
        return;
      if (typeof payload !== "object" || payload === null)
        return;
      const record = payload as Record<string, unknown>;
      if (record.sessionId !== sessionId)
        return;
      const type = typeof record.type === "string" ? record.type : null;
      if (!type || !SETTINGS_EVENTS.has(type))
        return;
      // Re-read rather than apply the event's own value: the agent's
      // `model_changed` may carry a resolved reference the catalogue spells
      // differently, and a re-read cannot disagree with what the host will use.
      void read(desktopId, sessionId);
    }).then((unlisten) => {
      if (cancelled)
        unlisten();
      else dispose = unlisten;
    }).catch(() => {
      // Not under Tauri (tests, a browser harness): the reads still work.
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [enabled, desktopId, sessionId, read]);

  const change = useCallback(async (work: () => Promise<unknown>): Promise<void> => {
    setSaving(true);
    setError(null);
    try {
      await work();
    }
    catch (err) {
      // The host still has the old value, so the picker goes back to it — and
      // the re-read happens *before* the message is set, because a successful
      // read clears the error as part of its job. Setting the message first left
      // the user with a value that snapped back and no reason why.
      if (desktopId && sessionId) {
        try {
          await read(desktopId, sessionId);
        }
        catch {
          // The restore is best effort: the message matters more than it.
        }
      }
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setSaving(false);
    }
  }, [desktopId, sessionId, read]);

  const setModelChoice = useCallback(async (reference: string): Promise<void> => {
    if (!desktopId || !sessionId)
      return;
    const previous = model;
    setModel(reference);
    await change(async () => {
      try {
        await setRemoteConversationModel(desktopId, sessionId, reference);
      }
      catch (err) {
        setModel(previous);
        throw err;
      }
    });
  }, [desktopId, sessionId, model, change]);

  const setThinkingLevelChoice = useCallback(async (level: string): Promise<void> => {
    if (!desktopId || !sessionId)
      return;
    const previous = thinkingLevel;
    setThinkingLevel(level);
    await change(async () => {
      try {
        await setRemoteConversationThinkingLevel(desktopId, sessionId, level);
      }
      catch (err) {
        setThinkingLevel(previous);
        throw err;
      }
    });
  }, [desktopId, sessionId, thinkingLevel, change]);

  return {
    error,
    model,
    models,
    saving,
    setModel: setModelChoice,
    setThinkingLevel: setThinkingLevelChoice,
    thinkingLevel,
  };
}
