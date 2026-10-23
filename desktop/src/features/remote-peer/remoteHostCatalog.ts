import type { RemoteDesktopSettings, RemoteModelInfo, RemotePeerEvent, RemoteSkillInfo } from "./remotePeerClient";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { getRemoteDesktopSettings, listRemoteAvailableSkills, listRemoteInstalledSkills, listRemoteModels } from "./remotePeerClient";

/**
 * The reads that describe a *host*, rather than a conversation on it.
 *
 * A paired machine's model catalogue, its own settings and its skills belong to
 * the *machine*, and the phone reads them when a connection is established — once
 * per desktop, then again when that host announces a change. This app read them
 * on every conversation open instead, which put four commands (a second
 * `get_state`, `list_models`, `get_desktop_settings`, and the skills catalogue
 * behind it) ahead of the page that draws the transcript, on a conversation the
 * reader may only look at for a second. Half of that was paid again for every
 * other conversation on the same machine.
 *
 * So they are cached per host, read ahead of the conversation that will need them
 * as a connection is established (`warmHostCatalogue`), and dropped when the host
 * says they moved — the same three cases the host itself publishes, with the
 * conversation `_global` (`agent_events::publish_invalidation`, and
 * `store::app_settings::update_app_settings` for model visibility): a peer that
 * never hears one keeps what it read when it connected, exactly as the phone
 * does.
 *
 * Cached as *promises*: two consumers asking at once make one request, and a
 * failed read is dropped rather than remembered, so the next ask retries instead
 * of serving a refusal for the life of the connection.
 */
export type HostCatalogueDomain = "models" | "settings" | "skills";

const catalogues = new Map<string, Map<HostCatalogueDomain, Promise<unknown>>>();

function cached<T>(
  desktopId: string,
  domain: HostCatalogueDomain,
  read: () => Promise<T>,
): Promise<T> {
  let host = catalogues.get(desktopId);
  if (!host) {
    host = new Map();
    catalogues.set(desktopId, host);
  }
  const existing = host.get(domain) as Promise<T> | undefined;
  if (existing)
    return existing;
  const pending = read().catch((error: unknown) => {
    // A failure is not a catalogue. Dropping it here is what keeps an
    // unreachable host from looking like a host with no models.
    if (host.get(domain) === pending)
      host.delete(domain);
    throw error;
  });
  host.set(domain, pending);
  return pending;
}

/** A host's model catalogue, as its conversations may choose from. */
export function hostModels(desktopId: string): Promise<RemoteModelInfo[]> {
  return cached(desktopId, "models", () => listRemoteModels(desktopId));
}

/** A host's own app settings (what it wants offered on its conversations). */
export function hostDesktopSettings(desktopId: string): Promise<RemoteDesktopSettings> {
  return cached(desktopId, "settings", () => getRemoteDesktopSettings(desktopId));
}

/** A host's skills: what it could install, and what it already has. */
export interface RemoteHostSkillCatalogue {
  available: RemoteSkillInfo[];
  installed: RemoteSkillInfo[];
}

export function hostSkillCatalogue(desktopId: string): Promise<RemoteHostSkillCatalogue> {
  return cached(desktopId, "skills", async () => {
    const [available, installed] = await Promise.all([
      listRemoteAvailableSkills(desktopId),
      listRemoteInstalledSkills(desktopId),
    ]);
    return { available, installed };
  });
}

/**
 * Drop a host's catalogue, all of it or one domain, so the next read asks again.
 *
 * Called where the answer can have changed underneath us: the host announced a
 * change of its own, or the connection that answered is gone.
 */
export function forgetHostCatalogue(desktopId: string, domain?: HostCatalogueDomain): void {
  if (!domain) {
    catalogues.delete(desktopId);
    return;
  }
  catalogues.get(desktopId)?.delete(domain);
}

/**
 * Read a host's catalogue ahead of the conversation that will need it.
 *
 * Best-effort, and never awaited: this is a background fill behind a connection,
 * so a failure is dropped (and not cached) rather than reported — a host that
 * cannot answer pays one refused request per connection and nothing else. The
 * skills are read only for a host that asks for recommendations, since that is
 * the only thing on this side that wants a candidate set.
 */
export function warmHostCatalogue(desktopId: string): void {
  void hostModels(desktopId).catch(() => undefined);
  void hostDesktopSettings(desktopId).then(
    (settings) => {
      if (settings.skillRecommend)
        void hostSkillCatalogue(desktopId).catch(() => undefined);
    },
    () => undefined,
  );
}

/**
 * The host's announcements that a catalogue read has gone stale, by event type.
 *
 * The names are the host's own (`remote::publisher::is_catalog_event` publishes
 * each of these to its paired clients under the conversation `_global`, which is
 * how it says "not one conversation's business"). Kept as a table rather than a
 * chain of `if`s so a new event is one line and the domain it drops is legible.
 */
const INVALIDATIONS: Record<string, HostCatalogueDomain> = {
  app_settings_changed: "settings",
  model_visibility_changed: "models",
  // A provider's models are the catalogue's other half: an edit there changes
  // what `list_models` answers.
  provider_config_changed: "models",
  skills_changed: "skills",
};

/**
 * Follow the host's own changes to its catalogue.
 *
 * `onChange` is called with the domain that moved, after it has been dropped, so
 * a consumer re-reads (through the cache) rather than trusting what it holds. The
 * callback is read through a ref, so an effect that re-creates it per render does
 * not tear the subscription down and re-open it — the events it must not miss are
 * exactly the ones that arrive between the two.
 */
export function useHostCatalogueInvalidation(
  desktopId: string | null,
  onChange: (domain: HostCatalogueDomain) => void,
): void {
  const sinkRef = useRef(onChange);
  sinkRef.current = onChange;

  useEffect(() => {
    if (!desktopId)
      return;
    let dispose: (() => void) | undefined;
    let cancelled = false;
    void listen<RemotePeerEvent>("remote-peer-event", (event) => {
      const { desktopId: source, kind, payload } = event.payload;
      if (source !== desktopId)
        return;
      if (kind === "resumed") {
        forgetHostCatalogue(desktopId);
        for (const domain of ["settings", "models", "skills"] as const)
          sinkRef.current(domain);
        return;
      }
      if (kind !== "event")
        return;
      const type = readString(payload, "type");
      if (!type)
        return;
      const domain = INVALIDATIONS[type];
      if (!domain)
        return;
      forgetHostCatalogue(desktopId, domain);
      sinkRef.current(domain);
    }).then((unlisten) => {
      if (cancelled)
        unlisten();
      else dispose = unlisten;
    }).catch(() => {
      // Not under Tauri (tests, a browser harness): a cached read still works,
      // it simply cannot hear that it went stale.
    });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [desktopId]);
}

function readString(value: unknown, key: string): string | null {
  if (typeof value !== "object" || value === null)
    return null;
  const field = (value as Record<string, unknown>)[key];
  return typeof field === "string" ? field : null;
}
