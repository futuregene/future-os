import type { ThinkingLevel } from "../../integrations/agent/agentClient";
import type { RemoteModelInfo, RemotePeerEvent } from "./remotePeerClient";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { defaultThinkingLevel, readLastUsedModel, readLastUsedThinkingLevel, rememberLastUsedModel, rememberLastUsedThinkingLevel, thinkingLevels } from "../../integrations/agent/agentClient";
import { getRemoteConversationSettings, listRemoteModels, remoteModelReference, setRemoteConversationModel, setRemoteConversationThinkingLevel } from "./remotePeerClient";

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
 * The model a new conversation on that host should start with.
 *
 * The phone's order (`newConversation` in
 * `mobile/src/remote/useConversationController.ts`), which is the order a new
 * conversation on this machine uses too (`resolveInitialModelId`): the model the
 * user last chose, if that host still offers it — then the host's own designation
 * (`isDefault` on its catalogue) — then the first it offers. The choice is sent
 * with the first prompt, so what the picker shows is what runs.
 */
export function defaultRemoteModel(models: RemoteModelInfo[], lastUsed: string | null): string | null {
  const remembered = lastUsed
    ? models.find(model => remoteModelReference(model) === lastUsed)
    : undefined;
  const preferred = remembered ?? models.find(model => model.isDefault) ?? models[0];
  if (!preferred)
    return null;
  return remoteModelReference(preferred);
}

/**
 * Whether that host's model accepts thinking controls at all.
 *
 * `undefined` is not `false`: a catalogue that does not say is treated as a model
 * that does think, which is what the agent's own list means by omitting nothing
 * (`reasoning` is always sent) and what this app does elsewhere
 * (`modelSupportsThinking` in `integrations/agent/agentClient`).
 */
export function remoteModelSupportsThinking(
  models: RemoteModelInfo[],
  reference: string | null,
): boolean {
  const chosen = models.find(model => remoteModelReference(model) === reference);
  return chosen?.reasoning !== false;
}

/**
 * The thinking level a draft should offer for the model it resolved.
 *
 * The phone's rule — start from the level the user last chose — with this app's own
 * floor, which is the rule this app already applies to its new conversations
 * (`resolveInitialThinkingLevel` in `integrations/agent/agentClient`): a level it
 * never sees, or one that is not a level at all, means the app default. The phone's
 * floor is `off`; this one is not, because `off` is what the user is not choosing:
 * thinking off is a choice the picker offers, not the state a conversation should
 * silently start in. A model that takes no thinking controls gets `off` either way,
 * because that is what it will run with.
 *
 * Deliberately *not* the level the host's catalogue declares for the model. That
 * field is not a default anybody chose: the agent answers `list_models` with `high`
 * for every reasoning model and `off` for the rest
 * (`agent/src/rpc/commands/providers.rs`), so seeding from it ran every new remote
 * conversation on a reasoning model at high effort while the same model started on
 * this machine ran at medium — and the level travels with the first prompt, so the
 * host really did run it at high. This app's own composer ignores the same field
 * for the same reason (`modelThinkingLevel` in `integrations/agent/agentClient`).
 *
 * The level is sent explicitly with the first prompt either way: this is not a
 * guess about what the host will *choose*, it is the value the host is asked for.
 */
export function defaultRemoteThinkingLevel(
  models: RemoteModelInfo[],
  reference: string | null,
  lastUsed: string | null,
): string {
  if (!remoteModelSupportsThinking(models, reference))
    return "off";
  // A remembered value is only a level if it is one of the six the agent accepts;
  // anything else (a hand-edited localStorage entry) is not a choice to repeat.
  return lastUsed && thinkingLevels.includes(lastUsed as ThinkingLevel)
    ? lastUsed
    : defaultThinkingLevel;
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
        // Resolved the way the phone resolves a draft, off its own memory of what
        // the user last chose (device-wide, so a pick made *here* — or in this
        // app's own composer — is where the next new conversation starts).
        const resolved = defaultRemoteModel(catalogue, readLastUsedModel());
        setModel(resolved);
        setThinkingLevel(defaultRemoteThinkingLevel(catalogue, resolved, readLastUsedThinkingLevel()));
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
    // Remembered the way this app's own composer remembers a pick, and the way the
    // phone does: it is what the *next* new conversation resolves from, on either
    // screen. Written before the change is applied, because a host that refuses it
    // is still a choice the user made — the same optimism the local composer has.
    rememberLastUsedModel(reference);
    if (!sessionId) {
      // A draft: the value is local until the first prompt carries it, so it
      // moves here and the host is told nothing — there is no conversation on it
      // to set. Returning without this left the picker dead on a new
      // conversation, which is the state this whole path exists to fix.
      setModel(reference);
      // Switching model keeps the level the user is on (the phone's rule) — but a
      // model that takes no thinking controls runs at `off`, so the draft must not
      // go on offering it one.
      if (!remoteModelSupportsThinking(models, reference))
        setThinkingLevel("off");
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
    // See `setModelChoice`: the choice is what the next new conversation starts
    // from, and it is remembered even for a conversation the host has to be told
    // about.
    rememberLastUsedThinkingLevel(level);
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
