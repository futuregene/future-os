import { GraduationCap } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { openExternalUrl } from "../../integrations/storage/files";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import { fetchCoachPrompt, fetchSkillManualUrl } from "./skillGuidePrompt";

interface SkillGuideStripProps {
  /** Create the coach conversation with the given first-message content. */
  onStartCoachConversation: (content: string) => Promise<void>;
}

/**
 * Skills-page usage guide (top-right, beside the tab switcher): a one-line
 * hint plus 「教我使用技能」 (starts the coach conversation, same as the
 * banner) and 「使用手册」 (opens the platform manual link in the browser).
 * Always visible — contextual help, not a dismissible popup.
 */
export function SkillGuideStrip({ onStartCoachConversation }: SkillGuideStripProps) {
  const { t } = useTranslation("skills");
  const [busy, setBusy] = useState<"coach" | "manual" | null>(null);

  async function handleCoach() {
    if (busy)
      return;
    setBusy("coach");
    let prompt: string;
    try {
      prompt = await fetchCoachPrompt();
    }
    catch (error) {
      emitFutureEvent("toast", {
        message: t("guide.coachFailed", { message: errorMessage(error) }),
        tone: "error",
      });
      setBusy(null);
      return;
    }
    try {
      await onStartCoachConversation(prompt);
    }
    catch {
      // onStartCoachConversation surfaces its own toast.
    }
    finally {
      setBusy(null);
    }
  }

  async function handleManual() {
    if (busy)
      return;
    setBusy("manual");
    try {
      const url = await fetchSkillManualUrl();
      if (!url) {
        emitFutureEvent("toast", { message: t("guide.manualUnavailable"), tone: "info" });
        return;
      }
      await openExternalUrl(url);
    }
    catch {
      emitFutureEvent("toast", { message: t("guide.manualUnavailable"), tone: "error" });
    }
    finally {
      setBusy(null);
    }
  }

  return (
    <div className="flex max-w-sm shrink-0 flex-col gap-2 rounded-lg border border-info-line/60 bg-info-soft px-3 py-2">
      <div className="flex items-start gap-1.5 text-xs leading-5 text-ink-soft">
        <GraduationCap className="mt-0.5 size-3.5 shrink-0 text-accent" />
        <span>{t("guide.hint")}</span>
      </div>
      <div className="flex items-center gap-2">
        <Button disabled={busy === "coach"} onClick={() => void handleCoach()} size="sm" variant="primary">
          {t("guide.teachMe")}
        </Button>
        <Button disabled={busy === "manual"} onClick={() => void handleManual()} size="sm" variant="secondary">
          {t("guide.manual")}
        </Button>
      </div>
    </div>
  );
}
