import type { RemoteModelInfo, RemotePeerEvent } from "./remotePeerClient";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { getRemoteConversationSettings, listRemoteModels, setRemoteConversationModel, setRemoteConversationThinkingLevel } from "./remotePeerClient";

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
  /**
   * True until this conversation's settings — and, on a draft, its host's
   * catalogue — have been read.
   *
   * The picker needs it to tell "not read yet" from "the host has no model":
   * both are a `null` model, and while the read is in flight that difference is
   * the whole of what the control can say. Without it, opening a conversation
   * (every read starts by clearing the previous one's value) put the control's
   * own *label* on screen for as long as the host took to answer.
   */
  loading: boolean;
  /** True while a change is in flight, so the picker can say so. */
  saving: boolean;
  /**
   * A failed read, and a failed write — kept apart because they are different
   * events with different reasons. A read that failed changed nothing, so the
   * write message ("could not change …") would describe something that never
   * happened.
   */
  readError: string | null;
  writeError: string | null;
  setModel: (reference: string) => Promise<void>;
  setThinkingLevel: (level: string) => Promise<void>;
}

/**
 * The thinking level a new conversation on that host will run with.
 *
 * The agent's own default (`agent/src/rpc/session.rs`, `DEFAULT_THINKING_LEVEL`)
 * and the value this app uses for its own new conversations. It used to be "off"
 * here, which is not a default anybody chose: a new remote conversation would run
 * with thinking disabled unless the reader noticed the picker and changed it.
 */
export const DEFAULT_REMOTE_THINKING_LEVEL = "medium";

/**
 * The model a new conversation on that host should start with.
 *
 * The host's own designation (`isDefault` on its catalogue), then the first model
 * it offers. The phone resolves a draft the same way — last used, then the
 * catalogue's default, then the first — and the *point* of resolving it here is
 * that the choice is then sent with the first prompt, so what the picker shows is
 * what runs.
 */
export function defaultRemoteModel(models: RemoteModelInfo[]): string | null {
  const preferred = models.find(model => model.isDefault) ?? models[0];
  if (!preferred)
    return null;
  return preferred.provider ? `${preferred.provider}/${preferred.id}` : preferred.id;
}

/**
 * The thinking level a draft should offer for the model it resolved.
 *
 * A model that does not accept thinking controls gets `off`, because that is what
 * it will run with; every other model gets this app's own default — the same level
 * this app's composer starts a conversation on.
 *
 * Deliberately *not* the level the host's catalogue declares. That field is not a
 * default anybody chose: the agent answers `list_models` with `high` for every
 * reasoning model and `off` for the rest (`agent/src/rpc/commands/providers.rs`),
 * so seeding from it ran every new remote conversation on a reasoning model at
 * high effort while the same model started on this machine ran at medium — and the
 * level travels with the first prompt, so the host really did run it at high. This
 * app's own composer ignores the same field for the same reason
 * (`modelThinkingLevel` in `integrations/agent/agentClient`).
 *
 * The level is sent explicitly with the first prompt either way: this is not a
 * guess about what the host will *choose*, it is the value the host is asked for.
 */
export function defaultRemoteThinkingLevel(
  models: RemoteModelInfo[],
  reference: string | null,
): string {
  const chosen = models.find(model =>
    (model.provider ? `${model.provider}/${model.id}` : model.id) === reference);
  if (chosen && chosen.reasoning === false)
    return "off";
  return DEFAULT_REMOTE_THINKING_LEVEL;
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
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [readError, setReadError] = useState<string | null>(null);
  const [writeError, setWriteError] = useState<string | null>(null);
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
    setLoading(true);
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
      setReadError(null);
    }
    catch (err) {
      if (epoch === epochRef.current)
        setReadError(err instanceof Error ? err.message : String(err));
    }
    finally {
      // Guarded like every other write here: a slow reply that lands after the
      // user moved on must not clear the *new* conversation's read state.
      if (epoch === epochRef.current)
        setLoading(false);
    }
  }, []);

  // Open: read. Cleared first so another conversation's model cannot show for a
  // frame under the new one's title.
  useEffect(() => {
    epochRef.current += 1;
    setModel(null);
    setThinkingLevel(null);
    setModels([]);
    setLoading(false);
    setReadError(null);
    setWriteError(null);
    if (!enabled || !desktopId)
      return;
    if (!sessionId) {
      // A draft: there is no conversation on the host yet, so there is nothing to
      // *read* — but its catalogue is per machine and is exactly what the picker
      // needs, and the values chosen here travel with the first prompt. Without
      // this the model picker was empty (and so disabled) and the thinking level
      // fell back to "off", which is not a default anyone chose.
      const epoch = ++epochRef.current;
      setLoading(true);
      void listRemoteModels(desktopId).then((catalogue) => {
        if (epoch !== epochRef.current)
          return;
        setModels(catalogue);
        setModel(defaultRemoteModel(catalogue));
        setThinkingLevel(defaultRemoteThinkingLevel(catalogue, defaultRemoteModel(catalogue)));
      }).catch((err: unknown) => {
        if (epoch === epochRef.current)
          setReadError(err instanceof Error ? err.message : String(err));
      }).finally(() => {
        if (epoch === epochRef.current)
          setLoading(false);
      });
      return;
    }
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
    setWriteError(null);
    try {
      await work();
    }
    catch (err) {
      // The host still has the old value, so the picker goes back to it — and
      // the re-read happens *before* the message is set, because a successful
      // read clears the read error as part of its job. Setting the message first
      // left the user with a value that snapped back and no reason why.
      if (desktopId && sessionId) {
        try {
          await read(desktopId, sessionId);
        }
        catch {
          // The restore is best effort: the message matters more than it.
        }
      }
      setWriteError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setSaving(false);
    }
  }, [desktopId, sessionId, read]);

  const setModelChoice = useCallback(async (reference: string): Promise<void> => {
    if (!desktopId)
      return;
    if (!sessionId) {
      // A draft: the value is local until the first prompt carries it, so it
      // moves here and the host is told nothing — there is no conversation on it
      // to set. Returning without this left the picker dead on a new
      // conversation, which is the state this whole path exists to fix.
      setModel(reference);
      setThinkingLevel(current =>
        defaultRemoteThinkingLevel(models, reference) === "off"
          ? "off"
          : current ?? DEFAULT_REMOTE_THINKING_LEVEL);
      return;
    }
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
  }, [desktopId, sessionId, model, models, change]);

  const setThinkingLevelChoice = useCallback(async (level: string): Promise<void> => {
    if (!desktopId)
      return;
    if (!sessionId) {
      // See `setModelChoice`: a draft's choice is carried by its first prompt.
      setThinkingLevel(level);
      return;
    }
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
    loading,
    model,
    models,
    readError,
    saving,
    setModel: setModelChoice,
    setThinkingLevel: setThinkingLevelChoice,
    thinkingLevel,
    writeError,
  };
}
