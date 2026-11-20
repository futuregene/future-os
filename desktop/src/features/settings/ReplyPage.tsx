import { useTranslation } from "react-i18next";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

/**
 * What a reply itself does on every client: whether it may close with a choice
 * card, and whether a new conversation's first answer also produces its title.
 *
 * The completion bell used to sit here and does not: it is this machine's own
 * notification, played by this client and invisible to a paired phone, so it
 * lives with the other local preferences on the appearance page.
 */
export function ReplyPage({
  choiceCards,
  onToggleChoiceCards,
  autoTitleFirstTurn,
  onToggleAutoTitleFirstTurn,
}: {
  choiceCards: boolean;
  onToggleChoiceCards: (value: boolean) => void;
  autoTitleFirstTurn: boolean;
  onToggleAutoTitleFirstTurn: (value: boolean) => void;
}) {
  const { t } = useTranslation("settings");

  return (
    <div className="space-y-6">
      <SettingsSection>
        <SettingsList>
          {/* Paired with the prompt section: while this is off the agent is not
              taught the fence either, so turning it off leaves no card in a new
              reply and no raw code block in an old one. */}
          <SettingsRow
            title={t("choiceCards.title")}
            description={t("choiceCards.description")}
          >
            <Switch checked={choiceCards} label={t("choiceCards.title")} onChange={onToggleChoiceCards} />
          </SettingsRow>
          <SettingsRow
            title={t("autoTitleFirstTurn.title")}
            description={t("autoTitleFirstTurn.description")}
          >
            <Switch checked={autoTitleFirstTurn} label={t("autoTitleFirstTurn.title")} onChange={onToggleAutoTitleFirstTurn} />
          </SettingsRow>
        </SettingsList>
      </SettingsSection>
    </div>
  );
}
