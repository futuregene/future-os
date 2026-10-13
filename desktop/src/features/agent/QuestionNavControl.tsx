import { ArrowDown, ArrowUp } from "lucide-react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";

export function QuestionNavControl({
  hasPrevious,
  hasNext,
  onPrevious,
  onNext,
}: {
  hasPrevious: boolean;
  hasNext: boolean;
  onPrevious: () => void;
  onNext: () => void;
}) {
  const { t } = useTranslation("agent");
  return (
    <div className="flex items-center gap-1">
      <IconButton
        className="size-8 disabled:cursor-default disabled:opacity-40"
        disabled={!hasPrevious}
        icon={<ArrowUp className="size-4" />}
        label={t("thread.questionPrevious")}
        onClick={onPrevious}
        type="button"
      />
      <IconButton
        className="size-8 disabled:cursor-default disabled:opacity-40"
        disabled={!hasNext}
        icon={<ArrowDown className="size-4" />}
        label={t("thread.questionNext")}
        onClick={onNext}
        type="button"
      />
    </div>
  );
}
