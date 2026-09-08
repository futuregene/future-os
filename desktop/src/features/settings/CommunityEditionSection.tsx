import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Switch } from "../../components/ui/Switch";
import { SettingsSection } from "./SettingsPrimitives";

/** A UI-only product mode, intentionally independent from platform environment. */
export function CommunityEditionSection({
  communityEdition,
  onChangeCommunityEdition,
}: {
  communityEdition: boolean;
  onChangeCommunityEdition: (value: boolean) => Promise<void>;
}) {
  const { t } = useTranslation("settings");
  const [selected, setSelected] = useState(communityEdition);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setSelected(communityEdition);
  }, [communityEdition]);

  async function handleSwitch() {
    if (selected === communityEdition)
      return;
    setSaving(true);
    try {
      await onChangeCommunityEdition(selected);
    }
    finally {
      setSaving(false);
    }
  }

  return (
    <SettingsSection>
      <div className="space-y-3 rounded-lg border border-line-soft p-4">
        <div className="flex items-center justify-between gap-4">
          <div>
            <div className="text-sm font-medium text-ink">{t("reset.communityEdition")}</div>
            <p className="mt-1 text-xs leading-5 text-ink-muted">{t("reset.communityEditionDescription")}</p>
          </div>
          <Switch checked={selected} disabled={saving} onChange={setSelected} />
        </div>
        <Button disabled={selected === communityEdition || saving} onClick={() => void handleSwitch()} size="sm" variant="primary">
          {saving ? t("reset.switching") : t("reset.communityEditionSwitch")}
        </Button>
      </div>
    </SettingsSection>
  );
}
