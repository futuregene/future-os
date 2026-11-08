import type {
  RemoteAvailableSkill,
  RemotePeer,
  RemoteSkillInfo,
} from "./remotePeerClient";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { getLanguage } from "../../i18n";
import { errorMessage } from "../../lib/errors";
import { SettingsList, SettingsRow, SettingsSection } from "../settings/SettingsPrimitives";
import {
  installRemoteSkill,
  listRemoteAvailableSkills,
  listRemoteInstalledSkills,
  localizedSkill,
  uninstallRemoteSkill,
} from "./remotePeerClient";

/**
 * The skills on another computer, and the two things one can do about them.
 *
 * Manageable rather than summarised because installing a skill is exactly the
 * kind of errand that makes "go to that machine" unreasonable: the user is
 * already looking at it from here.
 *
 * Changes are followed by a re-read of *both* lists rather than an optimistic
 * edit: installing moves a row from one list to the other, and which version the
 * host ended up with is its answer to give.
 */
export function RemoteSkillsPanel({
  available = true,
  peer,
}: {
  /** False while the host is unreachable, so nothing is offered that cannot land. */
  available?: boolean;
  peer: RemotePeer;
}) {
  const { t } = useTranslation("remotePeer");
  const [installed, setInstalled] = useState<RemoteSkillInfo[]>([]);
  const [catalogue, setCatalogue] = useState<RemoteAvailableSkill[]>([]);
  const [loading, setLoading] = useState(true);
  const [busySkill, setBusySkill] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [mine, theirs] = await Promise.all([
        listRemoteInstalledSkills(peer.desktopId),
        // A host with no catalogue still has its installed list, so one failure
        // must not blank the other.
        listRemoteAvailableSkills(peer.desktopId).catch(() => [] as RemoteAvailableSkill[]),
      ]);
      setInstalled(mine);
      setCatalogue(theirs);
      setError(null);
    }
    catch (err) {
      setError(errorMessage(err));
    }
    finally {
      setLoading(false);
    }
  }, [peer.desktopId]);

  useEffect(() => {
    setLoading(true);
    void refresh();
  }, [refresh]);

  async function change(skillId: string, work: () => Promise<unknown>) {
    setBusySkill(skillId);
    setError(null);
    try {
      await work();
      await refresh();
    }
    catch (err) {
      setError(errorMessage(err));
    }
    finally {
      setBusySkill(null);
    }
  }

  const language = getLanguage();
  // Only what the host does not already have: a catalogue entry that is already
  // installed would be a second Install button for something that is there.
  const installable = catalogue.filter(
    skill => !installed.some(mine => mine.id === skill.id),
  );
  const catalogueById = useMemo(
    () => new Map(catalogue.map(skill => [skill.id, skill] as const)),
    [catalogue],
  );

  /**
   * A row's text in the app's language, for either list.
   *
   * An installed row is described by its own `SKILL.md`, and a package built from
   * the repository carries only the English `description` — no `name_zh`, no
   * `description_zh`. The host's catalogue has both for the same id, so the
   * translation is taken from there first and the skill's own text is the
   * fallback. Without this the installed half of the page read in English while
   * the installable half beside it read in Chinese, which looked like the
   * translations were missing rather than being somewhere else. The phone merges
   * the two lists the same way (and the local Skills page does as well).
   */
  function localized(skill: RemoteSkillInfo | RemoteAvailableSkill) {
    const entry = catalogueById.get(skill.id);
    return localizedSkill({
      ...skill,
      nameZh: skill.nameZh || entry?.nameZh,
      descriptionZh: skill.descriptionZh || entry?.descriptionZh,
    }, language);
  }

  return (
    <SettingsSection description={t("skillsHint")} title={t("skills")}>
      {error ? <p className="px-3 py-2 text-xs text-danger">{error}</p> : null}
      {loading ? <p className="px-3 py-2 text-xs text-ink-muted">{t("skillsLoading")}</p> : null}
      <SettingsList>
        {!loading && installed.length === 0
          ? <SettingsRow title={t("skillsNoneInstalled")}><span /></SettingsRow>
          : null}
        {installed.map((skill) => {
          const text = localized(skill);
          return (
            <SettingsRow
              description={text.description}
              key={skill.id}
              title={text.name}
            >
              <Button
                disabled={!available || busySkill !== null}
                onClick={() => void change(skill.id, () => uninstallRemoteSkill(peer.desktopId, skill.id))}
                size="sm"
                variant="danger-soft"
              >
                {busySkill === skill.id ? t("skillsWorking") : t("skillsRemove")}
              </Button>
            </SettingsRow>
          );
        })}
        {installable.map((skill) => {
          const text = localized(skill);
          return (
            <SettingsRow
              description={text.description}
              key={skill.id}
              title={text.name}
            >
              <Button
                disabled={!available || busySkill !== null}
                onClick={() => void change(skill.id, () => installRemoteSkill(
                  peer.desktopId,
                  skill.id,
                  skill.latestVersion ?? "",
                ))}
                size="sm"
                variant="secondary"
              >
                {busySkill === skill.id ? t("skillsWorking") : t("skillsInstall")}
              </Button>
            </SettingsRow>
          );
        })}
      </SettingsList>
    </SettingsSection>
  );
}
