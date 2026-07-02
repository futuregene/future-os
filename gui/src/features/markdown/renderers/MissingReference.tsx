import type { FutureReference } from "../futureMarkdownTypes";
import { useTranslation } from "react-i18next";

export function MissingReference({
  error,
  reference,
}: {
  error?: string | null;
  reference: FutureReference;
}) {
  const { t } = useTranslation("markdown");
  return (
    <span
      className="inline-flex max-w-full items-center rounded-md border border-danger-line bg-danger-soft px-1.5 py-0.5 text-[0.92em] text-danger"
      title={error ?? `${reference.targetType}:${reference.targetId}`}
    >
      {t("missingReference.label", { targetType: reference.targetType })}
    </span>
  );
}
