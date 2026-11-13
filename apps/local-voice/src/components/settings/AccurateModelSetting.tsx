import React, { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../ui/Dropdown";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { useModelStore } from "../../stores/modelStore";

/**
 * Genaues Modell für schwierige Aufnahmen (`accurate_model`). Leer = nicht
 * eingerichtet. Der Verlauf bietet damit „Mit genauem Modell erneut
 * transkribieren“ an: das Backend lädt dieses Modell als eigene Engine neben
 * dem Diktatmodell und gibt es nach dem Lauf wieder frei
 * (TranscriptionManager::transcribe_with_model). Wählbar ist jedes installierte
 * Transkriptionsmodell, auch eigene Whisper-Dateien aus dem Modellordner.
 */
export const AccurateModelSetting: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const { models, loadModels } = useModelStore();

  useEffect(() => {
    if (models.length === 0) void loadModels();
  }, [models.length, loadModels]);

  const value = getSetting("accurate_model") ?? "";
  const options = [
    { value: "", label: t("settings.accurateModel.none") },
    ...models
      .filter((m) => m.is_downloaded)
      .map((m) => ({ value: m.id, label: m.name })),
  ];

  return (
    <SettingContainer
      title={t("settings.accurateModel.title")}
      description={t("settings.accurateModel.description")}
      grouped={true}
    >
      <Dropdown
        options={options}
        selectedValue={value}
        onSelect={(v) => updateSetting("accurate_model", v === "" ? null : v)}
        placeholder={t("settings.accurateModel.none")}
        disabled={isUpdating("accurate_model")}
      />
    </SettingContainer>
  );
};
