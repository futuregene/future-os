import { Badge } from "../../components/ui/Badge";
import { openExternalUrl } from "../../integrations/storage/files";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

// The About page body is intentionally English-only for now (no i18n); only the
// nav tab label is translated.
const APP_NAME = "FutureOS";
const LICENSE = "MIT License";
const HOMEPAGE_URL = "https://www.future-os.cn";
const HOMEPAGE_LABEL = "www.future-os.cn";
const GITHUB_URL = "https://github.com/futuregene/future-os";
const GITHUB_LABEL = "github.com/futuregene/future-os";

// Just the major open-source projects FutureOS is built on — names only, big
// ones first, trailing off with "etc." since the full tree is long.
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
  const build = useBuildInfo();
  const isTestBuild = Boolean(build.data && !build.data.isRelease);

  return (
    <div className="space-y-6">
      <div>
        <div className="flex items-center gap-2">
          <h3 className="text-lg font-semibold text-ink">{APP_NAME}</h3>
          {isTestBuild ? <Badge tone="warning">Test build</Badge> : null}
        </div>
        <p className="mt-0.5 text-sm text-ink-soft">
          {build.data ? build.data.version : "—"}
        </p>
      </div>

      <SettingsSection>
        <SettingsList>
          <SettingsRow title="License">
            <span className="text-sm text-ink-soft">{LICENSE}</span>
          </SettingsRow>
          <SettingsRow title="Website">
            <ExternalLink label={HOMEPAGE_LABEL} url={HOMEPAGE_URL} />
          </SettingsRow>
          <SettingsRow title="GitHub">
            <ExternalLink label={GITHUB_LABEL} url={GITHUB_URL} />
          </SettingsRow>
        </SettingsList>
      </SettingsSection>

      <SettingsSection title="Open source">
        <p className="text-sm leading-6 text-ink-soft">
          {`Built with open-source software, including ${OPEN_SOURCE}`}
        </p>
      </SettingsSection>
    </div>
  );
}
