import { useTranslation } from "react-i18next";
import wechatQrCode from "../../assets/wechat-qrcode.png";
import { Button } from "../../components/ui/Button";
import { useCopyState } from "../../components/ui/useCopyState";
import { openExternalUrl } from "../../integrations/storage/files";
import { SettingsSection } from "./SettingsPrimitives";

// The account name to search for, and the account's follow QR code. The asset
// encodes `weixin.qq.com/r/mp/...`, which opens WeChat's follow page — the same
// payload WeChat's own artwork carries.
const ACCOUNT_NAME = "FutureOS";
const FIRST_ARTICLE_URL = "https://mp.weixin.qq.com/s/qefitj15rYRhK6lYmTj2dA";

/**
 * "Follow our official account" card on the About page.
 *
 * The QR code is the primary path — a phone can scan the desktop screen in one
 * step. Searching by name is the fallback for when the phone is not at hand, so
 * the copy button is not decoration: it is the only route that needs no camera.
 *
 * The asset is a plain code (no centre logo) re-encoded from the account's own
 * payload: same error-correction level (Q) in fewer modules, so every module is
 * larger at the size we render, and nothing covers the data area.
 *
 * WeChat's desktop build cannot open an account's follow page, so no "open in
 * WeChat" action is offered here — on the desktop it would be a dead button.
 */
export function WechatFollowCard() {
  const { t } = useTranslation("settings");
  const { copiedKey, copy } = useCopyState();
  const copied = copiedKey !== null;

  return (
    <SettingsSection title={t("about.follow")}>
      <div className="flex items-start gap-4 rounded-lg border border-line-soft p-4">
        {/* The asset already carries a 4-module quiet zone; the white plate keeps
            the code off whatever surface colour the section happens to use. A
            plain code depends on that margin, so never crop it away. */}
        <img
          alt={t("about.followQrAlt", { name: ACCOUNT_NAME })}
          className="size-32 shrink-0 rounded-md border border-line-soft bg-white"
          height={128}
          src={wechatQrCode}
          width={128}
        />
        <div className="min-w-0 flex-1 space-y-2">
          <p className="text-sm leading-5 text-ink-muted">
            {t("about.followDescription", { name: ACCOUNT_NAME })}
          </p>
          <p className="text-xs leading-5 text-ink-muted">
            {t("about.followHint", { name: ACCOUNT_NAME })}
          </p>
          <div className="flex flex-wrap gap-2 pt-0.5">
            <Button
              onClick={() => void copy(ACCOUNT_NAME)}
              size="sm"
              variant="secondary"
            >
              {copied ? t("about.followCopied") : t("about.followCopyName")}
            </Button>
            <Button
              onClick={() => void openExternalUrl(FIRST_ARTICLE_URL)}
              size="sm"
              variant="secondary"
            >
              {t("about.followReadArticle")}
            </Button>
          </div>
        </div>
      </div>
    </SettingsSection>
  );
}
