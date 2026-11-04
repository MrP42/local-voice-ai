import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { commands, type VoiceprintStatus } from "@/bindings";
import { Button } from "../../ui/Button";
import { SettingContainer } from "../../ui/SettingContainer";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";

/** Katalog-Kennung des Stimmmodells (`voiceprint::MODEL_ID`). */
const MODEL_ID = "spk-campplus-voxceleb-16k";

interface DownloadProgress {
  model_id: string;
  percentage: number;
}

/**
 * Stimmprofile 2a: Vorschläge „klingen gleich“ in der Sprechertabelle. Schalter
 * (Standard an) und Download des Stimmmodells nur per Knopf. Ohne Modell gibt es
 * keine Vorschläge; Zusammenführen von Hand geht immer.
 */
export const MeetingVoiceSuggestionsSetting: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [status, setStatus] = useState<VoiceprintStatus | null>(null);
  const [percent, setPercent] = useState<number | null>(null);
  const [error, setError] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const result = await commands.meetingVoiceprintStatus();
      if (result.status === "ok") setStatus(result.data);
    } catch {
      // Ohne Stand bleibt nur der Schalter.
    }
  }, []);

  useEffect(() => {
    void refresh();
    const unProgress = listen<DownloadProgress>(
      "model-download-progress",
      (e) => {
        if (e.payload.model_id === MODEL_ID)
          setPercent(Math.round(e.payload.percentage));
      },
    );
    const unDone = listen<string>("model-download-complete", (e) => {
      if (e.payload === MODEL_ID) {
        setPercent(null);
        void refresh();
      }
    });
    return () => {
      void unProgress.then((f) => f());
      void unDone.then((f) => f());
    };
  }, [refresh]);

  const enabled = getSetting("meeting_voice_suggestions") ?? true;
  const downloading = percent !== null || (status?.downloading ?? false);

  const download = async () => {
    setError(false);
    setPercent(0);
    try {
      const result = await commands.meetingVoiceprintDownload();
      if (result.status === "error") setError(true);
    } catch {
      setError(true);
    }
    setPercent(null);
    await refresh();
  };

  let state: React.ReactNode = null;
  if (status?.model_ready) {
    state = (
      <span className="text-sm text-mid-gray" data-testid="voiceprint-state">
        {t("settings.meetings.voiceSuggestionsReady")}
      </span>
    );
  } else if (status) {
    state = (
      <Button
        variant="secondary"
        size="sm"
        onClick={() => void download()}
        disabled={downloading}
        data-testid="voiceprint-download"
      >
        {downloading
          ? t("settings.meetings.voiceSuggestionsDownloading")
          : t("settings.meetings.voiceSuggestionsDownload", {
              size: status.size_mb,
            })}
      </Button>
    );
  }

  return (
    <>
      <ToggleSwitch
        checked={enabled}
        onChange={(v) => {
          void updateSetting("meeting_voice_suggestions", v).then(() =>
            refresh(),
          );
        }}
        isUpdating={isUpdating("meeting_voice_suggestions")}
        label={t("settings.meetings.voiceSuggestions")}
        description={t("settings.meetings.voiceSuggestionsDescription")}
        descriptionMode="tooltip"
        grouped={true}
        testId="voiceprint-toggle"
      />
      {enabled && (
        <SettingContainer
          title={t("settings.meetings.voiceSuggestionsModel")}
          description={t("settings.meetings.voiceSuggestionsModelDescription")}
          grouped={true}
        >
          <div className="flex flex-col items-end gap-1">
            {state}
            {error && (
              <span className="text-xs text-red-400">
                {t("settings.meetings.voiceSuggestionsDownloadFailed")}
              </span>
            )}
          </div>
        </SettingContainer>
      )}
    </>
  );
};
