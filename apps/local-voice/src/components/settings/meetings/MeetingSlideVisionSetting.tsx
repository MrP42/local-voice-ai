import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { commands, type SlideVisionStatus } from "@/bindings";
import { Button } from "../../ui/Button";
import { SettingContainer } from "../../ui/SettingContainer";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";

/** Katalog-Kennung des Bild-Projektors (`llm::vision::VISION_PROJECTOR_ID`). */
const PROJECTOR_ID = "llm-gemma4-e4b-mmproj";

interface DownloadProgress {
  model_id: string;
  percentage: number;
}

/** Gründe, aus denen die Bildanalyse auf diesem Rechner nicht angeboten wird. */
const NO_GPU = "vision_no_gpu";
const LOW_VRAM = "vision_low_vram";

/**
 * Bildanalyse für Folien (D3): Schalter (Standard aus), Download des Bild-Projektors nur per
 * Knopf und der Grund, warum die Analyse nicht möglich ist (keine Grafikkarte, zu wenig
 * Grafikspeicher, Modell fehlt). Zeilen der bestehenden Gruppe „Besprechungen“, kein
 * eigener Reiter. Ohne die Voraussetzungen bleibt es bei der Windows-Texterkennung.
 */
export const MeetingSlideVisionSetting: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [status, setStatus] = useState<SlideVisionStatus | null>(null);
  const [percent, setPercent] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const result = await commands.meetingSlideVisionStatus();
    if (result.status === "ok") setStatus(result.data);
  }, []);

  useEffect(() => {
    void refresh();
    const unProgress = listen<DownloadProgress>(
      "model-download-progress",
      (e) => {
        if (e.payload.model_id === PROJECTOR_ID) {
          setPercent(Math.round(e.payload.percentage));
        }
      },
    );
    const unDone = listen<string>("model-download-complete", (e) => {
      if (e.payload === PROJECTOR_ID) {
        setPercent(null);
        void refresh();
      }
    });
    return () => {
      void unProgress.then((f) => f());
      void unDone.then((f) => f());
    };
  }, [refresh]);

  const enabled = getSetting("meeting_slide_vision") ?? false;

  const download = async () => {
    setError(null);
    setPercent(0);
    const result = await commands.meetingSlideVisionDownload();
    setPercent(null);
    if (result.status === "error") setError(String(result.error));
    await refresh();
  };

  const downloading = percent !== null || (status?.downloading ?? false);
  // Ohne Grafikkarte (oder mit zu wenig freiem Grafikspeicher) wird die Analyse nicht
  // angeboten: der Schalter ist gesperrt, es sei denn, sie ist schon eingeschaltet
  // (dann lässt er sich ausschalten).
  const gpuReason =
    status?.availability === NO_GPU || status?.availability === LOW_VRAM
      ? status.availability
      : null;

  let state: React.ReactNode = null;
  if (status) {
    if (gpuReason === NO_GPU) {
      state = (
        <span
          className="text-sm text-mid-gray"
          data-testid="slide-vision-state"
        >
          {t("settings.meetings.slideVisionNoGpu")}
        </span>
      );
    } else if (gpuReason === LOW_VRAM) {
      state = (
        <span
          className="text-sm text-mid-gray"
          data-testid="slide-vision-state"
        >
          {t("settings.meetings.slideVisionLowVram")}
        </span>
      );
    } else if (!status.projector_ready) {
      state = (
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void download()}
          disabled={downloading}
          data-testid="slide-vision-download"
        >
          {downloading
            ? t("settings.meetings.slideVisionDownloading", {
                percent: percent ?? 0,
              })
            : t("settings.meetings.slideVisionDownload", {
                size: status.projector_size_mb,
              })}
        </Button>
      );
    } else if (!status.model_ready) {
      state = (
        <span
          className="text-sm text-mid-gray"
          data-testid="slide-vision-state"
        >
          {t("settings.meetings.slideVisionNoModel")}
        </span>
      );
    } else {
      state = (
        <span
          className="text-sm text-mid-gray"
          data-testid="slide-vision-state"
        >
          {t("settings.meetings.slideVisionReady")}
        </span>
      );
    }
  }

  return (
    <>
      <ToggleSwitch
        checked={enabled}
        onChange={(v) => {
          void updateSetting("meeting_slide_vision", v).then(() => refresh());
        }}
        isUpdating={isUpdating("meeting_slide_vision")}
        disabled={gpuReason !== null && !enabled}
        label={t("settings.meetings.slideVision")}
        description={t("settings.meetings.slideVisionDescription")}
        descriptionMode="tooltip"
        grouped={true}
        testId="slide-vision-toggle"
      />
      <SettingContainer
        title={t("settings.meetings.slideVisionState")}
        description={t("settings.meetings.slideVisionStateDescription")}
        grouped={true}
      >
        <div className="flex flex-col items-end gap-1">
          {state}
          {error && (
            <span className="text-xs text-red-400">
              {t("settings.meetings.slideVisionDownloadFailed")}
            </span>
          )}
        </div>
      </SettingContainer>
    </>
  );
};
