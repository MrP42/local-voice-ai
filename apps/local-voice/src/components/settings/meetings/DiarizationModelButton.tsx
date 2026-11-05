import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { commands, type DiarizationModelStatus } from "@/bindings";
import { Button } from "../../ui/Button";

interface DownloadProgress {
  model_id: string;
  percentage: number;
}

/** Fehlertext im Fluent-Token, nicht in einer festen Tailwind-Farbe. */
const ERROR_STYLE: React.CSSProperties = {
  color: "var(--colorPaletteRedForeground1)",
};

/** Fortschritt wird nur in diesen Schritten angesagt (Bildschirmleser). */
const ANNOUNCE_STEP = 25;

/**
 * Laden des Sprechertrennungs-Modells (Sortformer) auf Knopfdruck. Steht in
 * den Besprechungs-Einstellungen und im Hinweis „Modell nicht installiert“
 * einer Besprechung.
 *
 * Barrierefreiheit (Codex-Abnahme #294): eine dauerhafte aria-live-Region
 * sagt Start, Fortschritt in 25-%-Schritten, Fertig, Abbruch und Fehler an;
 * der Fokus bleibt erhalten, wenn ein fokussierter Knopf durch „Installiert“
 * ersetzt wird; ein fehlgeschlagener Statusabruf zeigt „Wiederholen“.
 */
export const DiarizationModelButton: React.FC<{
  /** Im Hinweis einer Besprechung: kompakter, mit Satz nach dem Laden. */
  inline?: boolean;
}> = ({ inline = false }) => {
  const { t } = useTranslation();
  const [status, setStatus] = useState<DiarizationModelStatus | null>(null);
  const [statusFailed, setStatusFailed] = useState(false);
  const [percent, setPercent] = useState<number | null>(null);
  const [error, setError] = useState(false);
  const [announcement, setAnnouncement] = useState("");
  const lastAnnounced = useRef(-1);
  const rootRef = useRef<HTMLSpanElement>(null);
  const readyRef = useRef<HTMLSpanElement>(null);
  const downloadRef = useRef<HTMLButtonElement>(null);
  const retryRef = useRef<HTMLButtonElement>(null);
  /** Lag der Fokus in diesem Bereich, unmittelbar bevor sich der Zustand änderte? */
  const focusHandoff = useRef(false);

  /**
   * Unmittelbar vor jeder Zustandsänderung, die ein Bedienelement entfernen
   * kann: liegt der Fokus jetzt hier, wird er nach dem Rendern übergeben.
   * Hat der Nutzer ihn inzwischen woanders hin bewegt, bleibt er dort.
   */
  const captureFocus = () => {
    focusHandoff.current =
      !!rootRef.current && rootRef.current.contains(document.activeElement);
  };

  const refresh = useCallback(async () => {
    try {
      const result = await commands.meetingDiarizationModelStatus();
      if (result.status === "ok") {
        captureFocus();
        setStatus(result.data);
        setStatusFailed(false);
        return;
      }
    } catch {
      // unten: sichtbarer Fehlerzustand
    }
    captureFocus();
    // Auch nach einem früheren Erfolg: kein veralteter Ladezustand.
    setPercent(null);
    setStatusFailed(true);
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
        if (e.payload.model_id !== modelId) return;
        const p = Math.round(e.payload.percentage);
        setPercent(p);
        const step = Math.floor(p / ANNOUNCE_STEP) * ANNOUNCE_STEP;
        if (step > lastAnnounced.current && step > 0 && step < 100) {
          lastAnnounced.current = step;
          setAnnouncement(
            t("settings.meetings.diarizationModelDownloadingPercent", {
              percent: step,
            }),
          );
        }
      },
    );
    const unDone = listen<string>("model-download-complete", (e) => {
      if (e.payload === modelId) {
        captureFocus();
        setPercent(null);
        void refresh();
      }
    });
    return () => {
      void unProgress.then((f) => f());
      void unDone.then((f) => f());
    };
  }, [modelId, refresh, t]);

  const download = async () => {
    setError(false);
    setPercent(0);
    lastAnnounced.current = 0;
    setAnnouncement(t("settings.meetings.diarizationModelDownloading"));
    try {
      const result = await commands.meetingDiarizationModelDownload();
      // Läuft schon (zweites Fenster, Doppelklick): Fortschritt kommt weiter per Ereignis.
      if (result.status === "error" && result.error === "download_running")
        return;
      if (result.status === "error") {
        if (result.error === "cancelled") {
          setAnnouncement(t("settings.meetings.diarizationModelCancelled"));
        } else {
          setError(true);
          setAnnouncement(
            t("settings.meetings.diarizationModelDownloadFailed"),
          );
        }
      }
    } catch {
      setError(true);
      setAnnouncement(t("settings.meetings.diarizationModelDownloadFailed"));
    }
    captureFocus();
    setPercent(null);
    await refresh();
  };

  const cancel = () => {
    void commands.meetingDiarizationModelCancel();
  };

  const ready = status?.model_ready ?? false;
  useEffect(() => {
    if (!ready) return;
    if (lastAnnounced.current >= 0) {
      setAnnouncement(t("settings.meetings.diarizationModelInstalledNow"));
    }
  }, [ready, t]);

  const downloading = percent !== null || (status?.downloading ?? false);

  // Fokusübergabe nach dem Rendern: nur wenn der Fokus vor der Änderung hier
  // lag und das fokussierte Element jetzt fehlt (Fokus auf <body>).
  useEffect(() => {
    if (!focusHandoff.current) return;
    focusHandoff.current = false;
    const root = rootRef.current;
    if (!root || root.contains(document.activeElement)) return;
    const target =
      readyRef.current ?? retryRef.current ?? downloadRef.current ?? null;
    target?.focus();
  }, [ready, downloading, statusFailed, status]);
  const size = inline ? "sm" : "md";

  let body: React.ReactNode = null;
  // Ein Statusfehler ist immer sichtbar und wiederholbar, auch nach einem
  // früheren Erfolg (dann steht er unter dem letzten bekannten Stand).
  const statusError = statusFailed ? (
    <>
      <span className="text-xs" style={ERROR_STYLE}>
        {t("settings.meetings.diarizationModelStatusFailed")}
      </span>
      <Button
        ref={retryRef}
        type="button"
        variant="secondary"
        size={size}
        style={{ minHeight: 44 }}
        onClick={() => {
          captureFocus();
          void refresh();
        }}
        data-testid="diarization-model-retry"
      >
        {t("settings.meetings.diarizationModelRetry")}
      </Button>
    </>
  ) : null;

  if (status && ready) {
    body = (
      <span
        ref={readyRef}
        tabIndex={-1}
        className="text-sm text-mid-gray outline-none"
        data-testid="diarization-model-state"
      >
        {inline
          ? t("settings.meetings.diarizationModelReadyHint")
          : t("settings.meetings.diarizationModelReady")}
      </span>
    );
  } else if (status) {
    body = (
      <>
        <span className="inline-flex flex-wrap items-center gap-2">
          <Button
            ref={downloadRef}
            type="button"
            variant="secondary"
            size={size}
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
          {downloading && (
            <Button
              type="button"
              variant="ghost"
              size={size}
              style={{ minHeight: 44 }}
              onClick={cancel}
              data-testid="diarization-model-cancel"
            >
              {t("settings.meetings.diarizationModelCancel")}
            </Button>
          )}
        </span>
        {error && (
          <span className="text-xs" style={ERROR_STYLE}>
            {t("settings.meetings.diarizationModelDownloadFailed")}
          </span>
        )}
      </>
    );
  }

  return (
    <span
      ref={rootRef}
      className="inline-flex flex-col items-start gap-1"
      data-testid="diarization-model"
    >
      {body}
      {statusError}
      <span className="sr-only" aria-live="polite" role="status">
        {announcement}
      </span>
    </span>
  );
};
