import type { RemoteModelInfo, RemotePeer } from "./remotePeerClient";
import { ChevronDown, ChevronRight } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Switch } from "../../components/ui/Switch";
import { TextInput } from "../../components/ui/TextInput";
import { errorMessage } from "../../lib/errors";
import { listRemoteSettingsModels, remoteModelReference } from "./remotePeerClient";

/**
 * The models a host offers, grouped by provider, with a visibility switch.
 *
 * It is a page of its own because it is long: a machine with several providers
 * configured offers dozens of models, and as a section in the settings column
 * every other setting sat below that list.
 *
 * "Visibility" is the host's own `hiddenModels` list, not a local filter — the
 * same setting the phone edits. Hiding a model there is what decides which models
 * that machine offers, so a model this page hides really is gone from the
 * machine's pickers (including, later, its own).
 */
export function RemoteModelsPanel({
  available,
  hidden,
  onChangeHidden,
  peer,
}: {
  available: boolean;
  /** The host's `hiddenModels`, as references (`provider/id`). */
  hidden: readonly string[];
  onChangeHidden: (next: string[]) => void;
  peer: RemotePeer;
}) {
  const { t } = useTranslation("remotePeer");
  const [models, setModels] = useState<RemoteModelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  /** Providers whose models are folded away. Searching overrides it. */
  const [folded, setFolded] = useState<ReadonlySet<string>>(() => new Set());

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      // The *settings* list, not the picker's: the picker's omits hidden models,
      // and a switch that cannot see a hidden model cannot switch it back on.
      setModels(await listRemoteSettingsModels(peer.desktopId));
    }
    catch (err) {
      setError(errorMessage(err));
    }
    finally {
      setLoading(false);
    }
  }, [peer.desktopId]);

  useEffect(() => {
    void load();
  }, [load]);

  const hiddenSet = useMemo(() => new Set(hidden), [hidden]);
  const needle = query.trim().toLowerCase();
  const searching = needle.length > 0;

  const groups = useMemo(() => {
    const byProvider = new Map<string, RemoteModelInfo[]>();
    for (const model of models) {
      // The provider id, not a display name: it is what the reference is built
      // from, so a group heading is the same string the host resolves. Reading
      // display names would mean a second request for a heading's sake.
      const provider = model.provider ?? "";
      if (needle && !`${provider} ${model.label ?? ""} ${model.id}`.toLowerCase().includes(needle))
        continue;
      byProvider.set(provider, [...(byProvider.get(provider) ?? []), model]);
    }
    return [...byProvider]
      .map(([id, rows]) => ({
        id,
        models: rows,
        // A search shows what matched: folding groups during a search would hide
        // the rows the user just asked for.
        visible: searching || !folded.has(id),
      }))
      .sort((left, right) => left.id.localeCompare(right.id));
  }, [folded, models, needle, searching]);

  function setVisibility(rows: RemoteModelInfo[], visible: boolean) {
    const next = new Set(hiddenSet);
    for (const model of rows) {
      const reference = remoteModelReference(model);
      if (visible)
        next.delete(reference);
      else
        next.add(reference);
    }
    onChangeHidden([...next]);
  }

  function toggleFolded(id: string) {
    setFolded((current) => {
      const next = new Set(current);
      if (next.has(id))
        next.delete(id);
      else
        next.add(id);
      return next;
    });
  }

  const rowsDisabled = !available || loading;

  return (
    <div className="space-y-4">
      <p className="text-xs leading-5 text-ink-muted">{t("modelsHiddenHint")}</p>
      <TextInput
        aria-label={t("modelsSearch")}
        onChange={event => setQuery(event.target.value)}
        placeholder={t("modelsSearch")}
        value={query}
      />

      {error
        ? (
            <div className="rounded-md border border-danger-line bg-danger-soft px-3 py-2 text-xs text-danger">
              <div>{error}</div>
              <Button className="mt-2" onClick={() => void load()} size="sm" variant="ghost">
                {t("retry")}
              </Button>
            </div>
          )
        : null}

      {loading ? <p className="text-xs text-ink-muted">{t("modelsLoading")}</p> : null}

      {!loading && !error && groups.length === 0
        ? <p className="text-xs text-ink-muted">{t("modelsEmpty")}</p>
        : null}

      <div className="space-y-3">
        {groups.map((group) => {
          const hiddenInGroup = group.models.filter(model => hiddenSet.has(remoteModelReference(model)));
          return (
            <div className="rounded-lg border border-line-soft bg-surface" key={group.id}>
              <div className="flex items-center gap-3 px-4 py-3">
                <button
                  className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                  disabled={searching}
                  onClick={() => toggleFolded(group.id)}
                  type="button"
                >
                  {group.visible
                    ? <ChevronDown aria-hidden className="size-3.5 shrink-0 text-ink-muted" />
                    : <ChevronRight aria-hidden className="size-3.5 shrink-0 text-ink-muted" />}
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium text-ink">
                      {group.id || t("models")}
                    </span>
                    <span className="block text-xs text-ink-muted">
                      {hiddenInGroup.length > 0
                        ? t("modelsCountHidden", { count: group.models.length, hidden: hiddenInGroup.length })
                        : t("modelsCount", { count: group.models.length })}
                    </span>
                  </span>
                </button>
                {/* The bulk switch sits outside the fold control, so flipping it
                    never folds the group (the local models page learned this). */}
                <Switch
                  checked={hiddenInGroup.length === 0}
                  disabled={rowsDisabled}
                  label={t("modelsToggleProvider", { name: group.id })}
                  onChange={visible => setVisibility(group.models, visible)}
                />
              </div>
              {group.visible
                ? (
                    <div className="border-t border-line-soft px-4 [&>*+*]:border-t [&>*+*]:border-line-soft">
                      {group.models.map((model) => {
                        const reference = remoteModelReference(model);
                        return (
                          <div className="flex items-center gap-3 py-2.5" key={reference}>
                            <div className="min-w-0 flex-1">
                              <div className="truncate text-sm text-ink">{model.label || model.id}</div>
                              {/* The reference, not the id: with two providers
                                  holding an id, only the reference names which. */}
                              <div className="truncate text-xs text-ink-muted">{reference}</div>
                            </div>
                            <Switch
                              checked={!hiddenSet.has(reference)}
                              disabled={rowsDisabled}
                              label={model.label || model.id}
                              onChange={visible => setVisibility([model], visible)}
                            />
                          </div>
                        );
                      })}
                    </div>
                  )
                : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
