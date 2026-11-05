import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { commands, type DiarizationModelStatus } from "@/bindings";
import { Button } from "../../ui/Button";

interface DownloadProgress {
  model_id: string;
  percentage: number;
}

/**
 * Laden des Sprechertrennungs-Modells (Sortformer) auf Knopfdruck. Steht in
 * den Besprechungs-Einstellungen und im Hinweis „Modell nicht installiert“
 * einer Besprechung. Ist das Modell da, zeigt der Knopf „Installiert“ bzw.
 * (im Hinweis) einen Satz, dass die nächste Neu-Transkription Sprecher trennt.
 */
export const DiarizationModelButton: React.FC<{
  /** Im Hinweis einer Besprechung: kompakter, mit Satz nach dem Laden. */
  inline?: boolean;
  onReady?: () => void;
}> = ({ inline = false, onReady }) => {
  const { t } = useTranslation();
  const [status, setStatus] = useState<DiarizationModelStatus | null>(null);
  const [percent, setPercent] = useState<number | null>(null);
  const [error, setError] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const result = await commands.meetingDiarizationModelStatus();
      if (result.status === "ok") setStatus(result.data);
    } catch {
      // Ohne Stand kein Knopf.
    }
  }, []);

  const modelId = status?.model_id;
  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (!modelId) return;
    const unProgress = listen<DownloadProgress>(
      "model-download-progress",
      (e) => {
        if (e.payload.model_id === modelId)
          setPercent(Math.round(e.payload.percentage));
      },
    );
    const unDone = listen<string>("model-download-complete", (e) => {
      if (e.payload === modelId) {
        setPercent(null);
        void refresh();
      }
    });
    return () => {
      void unProgress.then((f) => f());
      void unDone.then((f) => f());
    };
  }, [modelId, refresh]);

  const download = async () => {
    setError(false);
    setPercent(0);
    try {
      const result = await commands.meetingDiarizationModelDownload();
      if (result.status === "error") setError(true);
    } catch {
      setError(true);
    }
    setPercent(null);
    await refresh();
  };

  const ready = status?.model_ready ?? false;
  useEffect(() => {
    if (ready) onReady?.();
  }, [ready, onReady]);

  if (!status) return null;
  const downloading = percent !== null || status.downloading;

  if (ready) {
    return (
      <span
        className="text-sm text-mid-gray"
        data-testid="diarization-model-state"
      >
        {inline
          ? t("settings.meetings.diarizationModelReadyHint")
          : t("settings.meetings.diarizationModelReady")}
      </span>
    );
  }

  return (
    <span className="inline-flex flex-col items-start gap-1">
      <Button
        variant="secondary"
        size={inline ? "sm" : "md"}
        style={{ minHeight: 44 }}
        onClick={() => void download()}
        disabled={downloading}
        data-testid="diarization-model-download"
      >
        {downloading
          ? percent && percent > 0
            ? t("settings.meetings.diarizationModelDownloadingPercent", {
                percent,
              })
            : t("settings.meetings.diarizationModelDownloading")
          : t("settings.meetings.diarizationModelDownload", {
              size: status.size_mb,
            })}
      </Button>
      {error && (
        <span className="text-xs text-red-400">
          {t("settings.meetings.diarizationModelDownloadFailed")}
        </span>
      )}
    </span>
  );
};
