import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Dropdown } from "../../ui/Dropdown";
import { SettingContainer } from "../../ui/SettingContainer";
import { useSettings } from "../../../hooks/useSettings";
import { useModelStore } from "../../../stores/modelStore";
import { commands, type ParallelRecommendation } from "@/bindings";

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
 * (`meeting_import_parallel`: 0 = Automatisch, sonst 1 bis 4). Automatisch
 * folgt der Empfehlung des Backends (Kerne, Arbeits- und Grafikspeicher,
 * `queue::recommended_parallel`); die Zeile nennt sie samt Begründung. More
 * than one only while memory allows; otherwise the next file waits.
 */

const gb = (mb: number) =>
  (mb / 1024).toLocaleString(undefined, { maximumFractionDigits: 1 });

export const MeetingModelSetting: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const { models, loadModels } = useModelStore();

  useEffect(() => {
    if (models.length === 0) void loadModels();
  }, [models.length, loadModels]);

  const value = getSetting("meeting_model") ?? "";
  const finalValue = getSetting("meeting_final_model") || "auto";
  const accelerator = getSetting("transcribe_accelerator");
  const parallelSetting = getSetting("meeting_import_parallel") ?? 0;
  const parallelValue = String(parallelSetting);
  const [recommendation, setRecommendation] =
    useState<ParallelRecommendation | null>(null);

  // Neu messen, wenn sich Besprechungsmodell (Größe) oder Beschleuniger ändern.
  useEffect(() => {
    let alive = true;
    commands
      .getMeetingImportParallelRecommendation()
      .then((rec) => {
        if (alive) setRecommendation(rec);
      })
      .catch(() => {
        if (alive) setRecommendation(null);
      });
    return () => {
      alive = false;
    };
  }, [value, accelerator]);

  const maxParallel = recommendation?.max ?? 4;
  const parallelOptions = [
    {
      value: "0",
      label: recommendation
        ? t("meetings.model.parallelAuto", {
            count: recommendation.recommended,
          })
        : t("meetings.model.parallelAutoPending"),
    },
    ...Array.from({ length: maxParallel }, (_, i) => ({
      value: String(i + 1),
      label: t("meetings.model.parallelFixed", { count: i + 1 }),
    })),
  ];
  const parallelHardware = recommendation
    ? t("meetings.model.parallelHardware", {
        cpus: recommendation.cpus,
        ram: gb(recommendation.ram_total_mb),
        vram:
          recommendation.vram_total_mb != null
            ? t("meetings.model.parallelHardwareVram", {
                vram: gb(recommendation.vram_total_mb),
              })
            : "",
        model: gb(recommendation.model_size_mb),
        count: recommendation.recommended,
        limitedBy: t(
          `meetings.model.parallelLimitedBy.${recommendation.limited_by}`,
        ),
      })
    : "";

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
        description={[parallelHardware, t("meetings.model.parallelDescription")]
          .filter(Boolean)
          .join(" ")}
        grouped={true}
      >
        <Dropdown
          options={parallelOptions}
          selectedValue={parallelValue}
          onSelect={(v) => updateSetting("meeting_import_parallel", Number(v))}
          placeholder={t("meetings.model.parallelAutoPending")}
          disabled={isUpdating("meeting_import_parallel")}
        />
      </SettingContainer>
    </>
  );
};
