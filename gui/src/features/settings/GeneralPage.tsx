import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

export function GeneralPage({
  autoApprove,
  onToggleAutoApprove,
}: {
  autoApprove: boolean;
  onToggleAutoApprove: (value: boolean) => void;
}) {
  return (
    <SettingsSection>
      <SettingsList>
        <SettingsRow
          title="自动审批"
          description="权限请求将被自动批准"
        >
          <Switch checked={autoApprove} label="自动审批" onChange={onToggleAutoApprove} />
        </SettingsRow>
      </SettingsList>
    </SettingsSection>
  );
}
