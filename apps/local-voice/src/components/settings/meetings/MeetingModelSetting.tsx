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
  const explanation = recommendation ? (
    <ParallelExplanation rec={recommendation} />
  ) : null;

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
        layout="stacked"
      >
        <Dropdown
          options={parallelOptions}
          selectedValue={parallelValue}
          onSelect={(v) => updateSetting("meeting_import_parallel", Number(v))}
          placeholder={t("meetings.model.parallelAutoPending")}
          disabled={isUpdating("meeting_import_parallel")}
        />
        {explanation}
      </SettingContainer>
    </>
  );
};

/**
 * Herleitung der Empfehlung und Speicherbedingungen, sichtbar unter der
 * Auswahl (Codex zu lva#245: vollständig erreichbar, linksbündig; Herleitung
 * zuerst, Bedingungen getrennt; "Obergrenze der App" ehrlich benennen).
 */
const ParallelExplanation: React.FC<{ rec: ParallelRecommendation }> = ({
  rec,
}) => {
  const { t } = useTranslation();
  const summary =
    rec.limited_by === "maximum"
      ? t("meetings.model.parallelSummaryMaximum", { count: rec.recommended })
      : t("meetings.model.parallelSummary", {
          count: rec.recommended,
          limitedBy: t(`meetings.model.parallelLimitedBy.${rec.limited_by}`),
        });
  const ramLine =
    rec.by_ram != null
      ? t("meetings.model.parallelByRam", {
          ram: gb(rec.ram_total_mb),
          count: rec.by_ram,
          perRun: gb(rec.ram_per_run_mb),
        })
      : t("meetings.model.parallelByRamUnknown");
  const vramLine = !rec.gpu_model
    ? t("meetings.model.parallelByVramCpuModel")
    : rec.by_vram != null && rec.vram_total_mb != null
      ? t("meetings.model.parallelByVram", {
          vram: gb(rec.vram_total_mb),
          count: rec.by_vram,
          perRun: gb(rec.vram_per_run_mb ?? 0),
        })
      : t("meetings.model.parallelByVramUnknown");
  const derivation = [
    t("meetings.model.parallelByCpu", { cpus: rec.cpus, count: rec.by_cpu }),
    ramLine,
    vramLine,
    t("meetings.model.parallelModel", { model: gb(rec.model_size_mb) }),
  ];
  const conditions = [
    t("meetings.model.parallelConditionReload", {
      ram: gb(rec.ram_per_run_mb),
      vram:
        rec.gpu_model && rec.vram_per_run_mb != null
          ? t("meetings.model.parallelConditionReloadVram", {
              vram: gb(rec.vram_per_run_mb),
            })
          : "",
    }),
    t("meetings.model.parallelConditionFree"),
    t("meetings.model.parallelConditionOneLoad"),
    t("meetings.model.parallelConditionRecording"),
    t("meetings.model.parallelConditionAudiobook"),
  ];
  return (
    <div
      className="mt-2 text-sm text-left text-text-muted space-y-2"
      data-testid="parallel-explanation"
    >
      <p className="font-medium text-text">{summary}</p>
      <div>
        <p className="font-medium">
          {t("meetings.model.parallelDerivationTitle")}
        </p>
        <ul className="list-disc ps-5">
          {derivation.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      </div>
      <div>
        <p className="font-medium">
          {t("meetings.model.parallelConditionsTitle")}
        </p>
        <ul className="list-disc ps-5">
          {conditions.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      </div>
    </div>
  );
};
