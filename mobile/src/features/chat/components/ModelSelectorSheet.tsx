import type { TFunction } from "i18next";
import { SelectionSheet } from "../../../components/SelectionSheet";
import { thinkingLevelLabel } from "../../../i18n/thinkingLevelLabel";
import type { useRemote } from "../../../remote/RemoteContext";
import { modelReference, modelThinkingLevels, resolveModelThinkingLevel } from "../../../remote/types";

type Remote = ReturnType<typeof useRemote>;

/** Chat adapter for the same sheet used by Fusion's independent role settings. */
export function ModelSelectorSheet({ selector, setSelector, remote, t }: {
  selector: "model" | "thinking" | null;
  setSelector: (value: "model" | "thinking" | null) => void;
  remote: Remote;
  t: TFunction;
}) {
  const selectedModel = remote.models.find(model => modelReference(model) === remote.modelId);
  const thinkingLevels = modelThinkingLevels(selectedModel);
  const thinkingLevel = resolveModelThinkingLevel(remote.thinkingLevel, selectedModel);
  return <SelectionSheet
    visible={selector !== null}
    title={t(selector === "model" ? "chat.model" : "chat.thinkingLevel")}
    closeLabel={t("common.close")}
    emptyLabel={t("connection.noModels")}
    options={selector === "model" ? remote.models.map(model => ({ value: modelReference(model), label: model.label || model.id, description: model.provider })) : thinkingLevels.map(level => ({ value: level, label: thinkingLevelLabel(t, level) }))}
    selectedValue={selector === "model" ? remote.modelId : thinkingLevel ?? null}
    onClose={() => setSelector(null)}
    onSelect={value => {
      if (selector === "model") void remote.setModel(value);
      else if (selector === "thinking") void remote.setThinkingLevel(value);
    }}
  />;
}
