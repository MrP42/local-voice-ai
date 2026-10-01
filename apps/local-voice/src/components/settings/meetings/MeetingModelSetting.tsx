import React, { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../../ui/Dropdown";
import { SettingContainer } from "../../ui/SettingContainer";
import { useSettings } from "../../../hooks/useSettings";
import { useModelStore } from "../../../stores/modelStore";

/**
 * Dedicated transcription model for meetings/imports. Empty = use the
 * dictation model. Rationale: streaming models are tuned for live dictation;
 * meetings transcribe in batches and profit from batch models. The backend
 * swaps to this model for meeting work and restores the dictation model
 * afterwards (TranscriptionManager::meeting_model_target).
 *
 * M2-P2d: second field on the same card — the final pass after stopping
 * (`meeting_final_model`: `auto` | `off` | model id, see final_pass.rs).
 *
 * U7: third field — how many imports transcribe at the same time
 * (`meeting_import_parallel`: 1, 2 or 3). More than one only while memory
 * allows; otherwise the next file waits ("wartet auf Arbeitsspeicher").
 */
export const MeetingModelSetting: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const { models, loadModels } = useModelStore();

  useEffect(() => {
    if (models.length === 0) void loadModels();
  }, [models.length, loadModels]);

  const value = getSetting("meeting_model") ?? "";
  const finalValue = getSetting("meeting_final_model") || "auto";
  const parallelValue = String(getSetting("meeting_import_parallel") ?? 1);
  const parallelOptions = [1, 2, 3].map((n) => ({
    value: String(n),
    label: t(`meetings.model.parallel${n}`),
  }));

  const downloaded = models
    .filter((m) => m.is_downloaded)
    .map((m) => ({ value: m.id, label: m.name }));

  const options = [
    { value: "", label: t("meetings.model.likeDictation") },
    ...downloaded,
  ];

  const finalOptions = [
    { value: "auto", label: t("meetings.model.finalAuto") },
    { value: "off", label: t("meetings.model.finalOff") },
    ...downloaded,
  ];

  return (
    <>
      <SettingContainer
        title={t("meetings.model.title")}
        description={t("meetings.model.description")}
        grouped={true}
      >
        <Dropdown
          options={options}
          selectedValue={value}
          onSelect={(v) => updateSetting("meeting_model", v === "" ? null : v)}
          placeholder={t("meetings.model.likeDictation")}
          disabled={isUpdating("meeting_model")}
        />
      </SettingContainer>
      <SettingContainer
        title={t("meetings.model.finalTitle")}
        description={t("meetings.model.finalDescription")}
        grouped={true}
      >
        <Dropdown
          options={finalOptions}
          selectedValue={finalValue}
          onSelect={(v) => updateSetting("meeting_final_model", v || "auto")}
          placeholder={t("meetings.model.finalAuto")}
          disabled={isUpdating("meeting_final_model")}
        />
      </SettingContainer>
      <SettingContainer
        title={t("meetings.model.parallelTitle")}
        description={t("meetings.model.parallelDescription")}
        grouped={true}
      >
        <Dropdown
          options={parallelOptions}
          selectedValue={parallelValue}
          onSelect={(v) => updateSetting("meeting_import_parallel", Number(v))}
          placeholder={t("meetings.model.parallel1")}
          disabled={isUpdating("meeting_import_parallel")}
        />
      </SettingContainer>
    </>
  );
};
