import { useState } from "react";
import { useTranslation } from "react-i18next";
import { getLanguage } from "../../i18n";
import { sessionTitleSettings } from "../../integrations/agent/sessionTitleSettings";
import { errorMessage } from "../../lib/errors";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { SettingsRow, Switch } from "./SettingsPrimitives";

export function SessionTitleSetting() {
  const { t } = useTranslation("settings");
  const { data, loading, error, reload } = useAsyncResource(
    () => sessionTitleSettings(),
    [],
    { autoSessionTitle: false, uiLanguage: "" },
  );
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  async function change(enabled: boolean) {
    setSaving(true);
    setSaveError(null);
    try {
      await sessionTitleSettings({ autoSessionTitle: enabled, uiLanguage: getLanguage() });
      reload();
    }
    catch (cause) {
      setSaveError(errorMessage(cause));
    }
    finally {
      setSaving(false);
    }
  }

  return (
    <SettingsRow
      title={t("autoSessionTitle.title")}
      description={error || saveError
        ? t("autoSessionTitle.error", { message: error || saveError })
        : t("autoSessionTitle.description")}
    >
      <Switch
        checked={data.autoSessionTitle}
        disabled={loading || saving || !!error}
        label={t("autoSessionTitle.title")}
        onChange={value => void change(value)}
      />
    </SettingsRow>
  );
}
