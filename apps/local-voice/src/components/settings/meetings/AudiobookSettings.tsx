import React, { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../../ui/Dropdown";
import { SettingContainer } from "../../ui/SettingContainer";
import { useSettings } from "../../../hooks/useSettings";
import { useModelStore } from "../../../stores/modelStore";
import { SELECTABLE_LANGUAGES } from "../../../lib/constants/languages";

export const AudiobookSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const { models, loadModels } = useModelStore();
  useEffect(() => {
    if (!models.length) void loadModels();
  }, [models.length, loadModels]);
  const model = getSetting("audiobook_model") ?? "";
  const options = [
    { value: "", label: t("audiobook.modelAuto") },
    ...models.filter((m) => m.is_downloaded || m.id === model)
      .map((m) => ({ value: m.id, label: m.name })),
  ];
  // Eine gespeicherte, inzwischen entfernte Wahl bleibt sichtbar.
  if (model && !options.some((option) => option.value === model))
    options.push({ value: model, label: model });
  return (
    <>
      <SettingContainer title={t("audiobook.modelTitle")} description={t("audiobook.modelDescription")} grouped>
        <Dropdown options={options} selectedValue={model}
          onSelect={(value) => updateSetting("audiobook_model", value || null)}
          disabled={isUpdating("audiobook_model")} />
      </SettingContainer>
      <SettingContainer title={t("audiobook.languageTitle")} description={t("audiobook.languageDescription")} grouped>
        <Dropdown options={SELECTABLE_LANGUAGES.map((language) => language.value === "auto"
          ? { value: "auto", label: t("audiobook.languageAuto") } : language)}
          selectedValue={getSetting("audiobook_language") ?? "auto"}
          onSelect={(value) => updateSetting("audiobook_language", value)}
          disabled={isUpdating("audiobook_language")} />
      </SettingContainer>
    </>
  );
};
