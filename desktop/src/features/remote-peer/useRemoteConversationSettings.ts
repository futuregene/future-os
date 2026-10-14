import type { RemoteModelInfo, RemotePeerEvent } from "./remotePeerClient";
import type { RemoteSessionStateReading } from "./useRemoteTimeline";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { hostModels, useHostCatalogueInvalidation } from "./remoteHostCatalog";
import {
  getRemoteConversationSettings,
  parseRemoteConversationSettings,
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
 *
 * Only the conversation's own settings are here: the host's *catalogue* (its
 * models and its own app settings) moves on a different set of announcements,
 * which `remoteHostCatalog` owns.
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
 * it will run with; a model that declares its own level uses that; otherwise the
 * agent's default. The level is sent explicitly with the first prompt either way,
 * so this is not a guess about what the host will *choose* — it is the value the
 * host will be *asked* for.
 */
export function defaultRemoteThinkingLevel(
  models: RemoteModelInfo[],
  reference: string | null,
): string {
  const chosen = models.find(model =>
    (model.provider ? `${model.provider}/${model.id}` : model.id) === reference);
  if (chosen && chosen.reasoning === false)
    return "off";
  return chosen?.thinkingLevel?.trim() || DEFAULT_REMOTE_THINKING_LEVEL;
}

/**
 * A remote conversation's model and thinking level, and the writes for them.
 *
 * Optimistic on change: the picker shows the new value straight away and the
 * host's announcement is what confirms it. Waiting for a round trip to move a
 * select leaves the control looking stuck, and the host is the authority either
 * way — if it refuses, its next `get_state` or its error puts the real value
 * back.
 *
 * `sessionState` is where those values come from on open, and it is the one
 * argument worth reading twice: when a reader on this screen has already read the
 * conversation's state, this hook applies *that* reply instead of asking the host
 * the same question again. `undefined` means nobody has: a caller that owns the
 * state read itself (or a screen with no timeline) gets the old behaviour, one
 * `get_state` of its own.
 */
export function useRemoteConversationSettings(
  desktopId: string | null,
  sessionId: string | null,
  enabled: boolean,
  /**
   * The conversation's state as another reader read it, or `null` while that
   * reader is still asking.
   *
   * The timeline must read `get_state` *after* the page it belongs to — its reply
   * replaces the page's own copy of the run in flight — and that same reply
   * carries the model and the thinking level. Handing it over is what keeps an
   * open from spending two `get_state` commands where the phone spends one.
   */
  sessionState?: RemoteSessionStateReading | null,
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
  /** Whether another reader owns this conversation's state read (see the type). */
  const handedOver = sessionState !== undefined;
  const handed = sessionState !== undefined && sessionState !== null
    && sessionState.sessionId === sessionId
    ? parseRemoteConversationSettings(sessionState.state)
    : null;

  const read = useCallback(async (
    desktop: string,
    session: string,
  ): Promise<void> => {
    const epoch = ++epochRef.current;
    try {
      const [settings, catalogue] = await Promise.all([
        getRemoteConversationSettings(desktop, session),
        // The catalogue is per host, not per conversation: this is a cache hit
        // for a host this window has already read from (`remoteHostCatalog`).
        hostModels(desktop).catch(() => [] as RemoteModelInfo[]),
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

  /**
   * The host's catalogue, for the picker and for a draft's defaults.
   *
   * A failure is reported only for a draft: a conversation's own settings are
   * read in `read`, which is where a failure belongs — an empty picker beside a
   * conversation that otherwise answered is not the fault the reader needs told
   * about.
   */
  const loadModels = useCallback(async (
    desktop: string,
    epoch: number,
  ): Promise<RemoteModelInfo[]> => {
    try {
      const catalogue = await hostModels(desktop);
      if (epoch === epochRef.current)
        setModels(catalogue);
      return catalogue;
    }
    catch (err) {
      if (epoch === epochRef.current && !sessionId)
        setError(err instanceof Error ? err.message : String(err));
      return [];
    }
  }, [sessionId]);

  // Open: read. Cleared first so another conversation's model cannot show for a
  // frame under the new one's title.
  useEffect(() => {
    const epoch = ++epochRef.current;
    setModel(null);
    setThinkingLevel(null);
    setModels([]);
    setError(null);
    if (!enabled || !desktopId)
      return;
    if (!sessionId) {
      // A draft: there is no conversation on the host yet, so there is nothing to
      // *read* — but its catalogue is per machine and is exactly what the picker
      // needs, and the values chosen here travel with the first prompt. Without
      // this the model picker was empty (and so disabled) and the thinking level
      // fell back to "off", which is not a default anyone chose.
      void loadModels(desktopId, epoch).then((catalogue) => {
        if (epoch !== epochRef.current)
          return;
        const reference = defaultRemoteModel(catalogue);
        setModel(reference);
        setThinkingLevel(defaultRemoteThinkingLevel(catalogue, reference));
      });
      return;
    }
    // A conversation: its picker is drawn from the host's catalogue, which is
    // this window's cache for that host.
    void loadModels(desktopId, epoch);
    // Its model and thinking level come from the state read — this hook's own
    // when nobody hands one over, the timeline's when it does.
    if (!handedOver)
      void read(desktopId, sessionId);
  }, [enabled, desktopId, sessionId, handedOver, loadModels, read]);

  /**
   * The reply another reader already read for this conversation.
   *
   * Applied as it arrives rather than only at open: the timeline reads the state
   * again when a link comes back, and what it read then is newer than what the
   * picker is showing.
   */
  useEffect(() => {
    if (!handed)
      return;
    if (handed.model)
      setModel(handed.model);
    if (handed.thinkingLevel)
      setThinkingLevel(handed.thinkingLevel);
  }, [handed]);

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

  /**
   * The host's catalogue moved, for a conversation whose picker is drawn from it.
   *
   * The read behind it is dropped before this runs, so the picker asks the host
   * again rather than re-rendering the list it already has. Only the models
   * matter here: this conversation's own model and level are announced per
   * conversation, and the hook above is what follows those.
   */
  useHostCatalogueInvalidation(desktopId, (domain) => {
    if (domain === "models" && enabled && desktopId)
      void loadModels(desktopId, epochRef.current);
  });

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
    error,
    model,
    models,
    saving,
    setModel: setModelChoice,
    setThinkingLevel: setThinkingLevelChoice,
    thinkingLevel,
  };
}
