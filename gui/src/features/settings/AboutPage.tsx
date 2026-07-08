import { Badge } from "../../components/ui/Badge";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

// The About page is intentionally English-only for now (no i18n), so its copy
// is inlined here rather than routed through the settings locale bundle.
const APP_NAME = "FutureOS";
const APP_TAGLINE = "AI agent desktop for FutureOS.";
const APP_LICENSE = "MIT License";
const APP_COPYRIGHT = "© 2026 FutureGene";

// Curated list of the primary open-source libraries FutureOS is built on, with
// the license each is distributed under (scanned from the frontend `node_modules`
// and the Tauri/agent Cargo dependencies). The full dependency tree is larger —
// these are the notable ones surfaced as acknowledgements.
const ACKNOWLEDGEMENTS: { license: string; name: string }[] = [
  { license: "MIT / Apache-2.0", name: "Tauri" },
  { license: "MIT", name: "React" },
  { license: "MIT", name: "Tokio" },
  { license: "MIT / Apache-2.0", name: "Serde" },
  { license: "MIT / Apache-2.0", name: "tonic · prost (gRPC)" },
  { license: "MIT / Apache-2.0", name: "reqwest" },
  { license: "Public Domain", name: "SQLite (rusqlite)" },
  { license: "MIT", name: "Shiki" },
  { license: "Apache-2.0", name: "PDF.js" },
  { license: "MIT", name: "unified · remark" },
  { license: "MIT", name: "i18next" },
  { license: "ISC", name: "Lucide" },
];

export function AboutPage() {
  const build = useBuildInfo();
  const isTestBuild = Boolean(build.data && !build.data.isRelease);

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <div className="flex size-14 shrink-0 items-center justify-center rounded-2xl bg-accent-soft text-2xl font-semibold text-accent">
          F
        </div>
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h3 className="text-lg font-semibold text-ink">{APP_NAME}</h3>
            {isTestBuild ? <Badge tone="warning">Test build</Badge> : null}
          </div>
          <p className="mt-0.5 text-sm text-ink-soft">
            {build.data ? `Version ${build.data.version}` : "Version —"}
          </p>
        </div>
      </div>

      <SettingsSection>
        <SettingsList>
          <SettingsRow title="Description">
            <span className="text-sm text-ink-soft">{APP_TAGLINE}</span>
          </SettingsRow>
          <SettingsRow title="License">
            <span className="text-sm text-ink-soft">{APP_LICENSE}</span>
          </SettingsRow>
          <SettingsRow title="Copyright">
            <span className="text-sm text-ink-soft">{APP_COPYRIGHT}</span>
          </SettingsRow>
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        description="FutureOS is built with open-source software. We gratefully acknowledge the projects below."
        title="Open source"
      >
        <SettingsList>
          {ACKNOWLEDGEMENTS.map(item => (
            <SettingsRow key={item.name} title={item.name}>
              <Badge tone="neutral">{item.license}</Badge>
            </SettingsRow>
          ))}
        </SettingsList>
      </SettingsSection>
    </div>
  );
}
