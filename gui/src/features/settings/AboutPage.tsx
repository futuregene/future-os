import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { openExternalUrl } from "../../integrations/storage/files";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

const APP_NAME = "FutureOS";
const HOMEPAGE_URL = "https://www.future-os.cn";
const GITHUB_URL = "https://github.com/futuregene/future-os";
const GITHUB_LABEL = "github.com/futuregene/future-os";
const OPEN_SOURCE = "Tauri, React, Tokio, Serde, SQLite, etc.";

function ExternalLink({ label, url }: { label: string; url: string }) {
  return (
    <button
      className="text-sm text-accent hover:underline"
      onClick={() => void openExternalUrl(url)}
      type="button"
    >
      {label}
    </button>
  );
}

export function AboutPage() {
  const { t } = useTranslation("settings");
  const build = useBuildInfo();
  const isTestBuild = Boolean(build.data && !build.data.isRelease);

  return (
    <div className="space-y-6">
      <div>
        <div className="flex items-center gap-2">
          <h3 className="text-lg font-semibold text-ink">{APP_NAME}</h3>
          {isTestBuild ? <Badge tone="warning">{t("dialog.testBuild")}</Badge> : null}
        </div>
        <p className="mt-0.5 text-sm text-ink-soft">
          {build.data ? build.data.version : "—"}
        </p>
      </div>

      <SettingsSection>
        <SettingsList>
          <SettingsRow title={t("about.license")}>
            <span className="text-sm text-ink-soft">{t("about.licenseValue")}</span>
          </SettingsRow>
          <SettingsRow title={t("about.website")}>
            <ExternalLink label={t("about.websiteLabel")} url={HOMEPAGE_URL} />
          </SettingsRow>
          <SettingsRow title="GitHub">
            <ExternalLink label={GITHUB_LABEL} url={GITHUB_URL} />
          </SettingsRow>
        </SettingsList>
      </SettingsSection>

      <SettingsSection title={t("about.openSource")}>
        <p className="text-sm leading-6 text-ink-soft">
          {t("about.builtWith", { projects: OPEN_SOURCE })}
        </p>
      </SettingsSection>
    </div>
  );
}
