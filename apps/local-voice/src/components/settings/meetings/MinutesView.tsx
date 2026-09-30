import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  commands,
  events,
  type MeetingDocument,
  type MinutesMeta,
  type MinutesProgress,
} from "@/bindings";
import {
  minutesErrorCode,
  minutesErrorDetail,
  phaseKey,
  progressPercent,
} from "@/lib/meetingMinutes";
import { Button } from "../../ui/Button";
import { Alert } from "../../ui/Alert";
import Badge from "../../ui/Badge";
import { MarkdownContent } from "../../whats-new/MarkdownContent";
import { MeetingTemplatePicker } from "./notes/TemplatePicker";
import { Download } from "lucide-react";

interface MinutesViewProps {
  meetingId: string;
  meetingTitle: string;
}

export const MinutesView: React.FC<MinutesViewProps> = ({
  meetingId,
  meetingTitle,
}) => {
  const { t } = useTranslation();
  const [doc, setDoc] = useState<MeetingDocument | null>(null);
  const [loading, setLoading] = useState(true);
  // Der Lauf gehoert dem Backend (B14): `running` kommt aus dessen Zustand und
  // seinen Ereignissen, nicht aus dem Aufruf dieses Reiters. So bleibt der
  // Knopf gesperrt, wenn man den Reiter verlaesst und wieder oeffnet.
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<MinutesProgress | null>(null);
  const [meta, setMeta] = useState<MinutesMeta | null>(null);
  // Die Vorlage der Besprechung (geteilt mit den KI-Notizen); `null` = die
  // gemerkte bzw. die Standardvorlage.
  const [templateChoice, setTemplateChoice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [autoFile, setAutoFile] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // The rendered preview doubles as the source for the formatted clipboard
  // copy: reading its innerHTML guarantees that what lands in Word is exactly
  // what the user sees here, instead of a second Markdown renderer that could
  // drift from the first.
  const previewRef = useRef<HTMLDivElement>(null);

  const loadLatest = useCallback(async () => {
    setLoading(true);
    const result = await commands.meetingsGetDocuments(meetingId);
    setLoading(false);
    if (result.status !== "ok") {
      // Failing to load used to render as "no minutes yet" — which is exactly
      // what a deleted document looks like. Say which of the two it is.
      setError(t("meetings.minutes.loadError", { error: result.error }));
      return;
    }
    setError(null);
    const minutes = result.data
      .filter((d) => d.kind === "minutes")
      .sort((a, b) => b.version - a.version);
    setDoc(minutes[0] ?? null);
  }, [meetingId, t]);

  const refreshAutoFile = useCallback(async () => {
    const result = await commands.meetingsMinutesFile(meetingId);
    setAutoFile(result.status === "ok" ? result.data : null);
  }, [meetingId]);

  const loadMeta = useCallback(async () => {
    const result = await commands.meetingsMinutesMeta(meetingId);
    setMeta(result.status === "ok" ? (result.data ?? null) : null);
  }, [meetingId]);

  // Beim Einblenden fragen, ob schon ein Lauf besteht (Reiterwechsel).
  const loadRunState = useCallback(async () => {
    const result = await commands.meetingsMinutesState(meetingId);
    if (result.status !== "ok" || !result.data) return;
    setRunning(result.data.running);
    setProgress(result.data.running ? (result.data.progress ?? null) : null);
  }, [meetingId]);

  useEffect(() => {
    void loadLatest();
    void refreshAutoFile();
    void loadMeta();
    void loadRunState();
  }, [loadLatest, refreshAutoFile, loadMeta, loadRunState]);

  const errorText = useCallback(
    (raw: string) =>
      t(`meetings.minutes.errors.${minutesErrorCode(raw)}`, {
        error: minutesErrorDetail(raw),
        defaultValue: raw,
      }),
    [t],
  );

  // Fortschritt, Ende und Fehler des Laufs -- auch eines, den ein anderer
  // Aufruf oder ein frueher geoeffneter Reiter gestartet hat.
  useEffect(() => {
    const un = events.minutesEvent.listen((event) => {
      const payload = event.payload;
      if (payload.meeting_id !== meetingId) return;
      if (payload.kind === "progress") {
        setRunning(true);
        setProgress({
          phase: payload.phase,
          done: payload.done,
          total: payload.total,
        });
      } else if (payload.kind === "done") {
        setRunning(false);
        setProgress(null);
        setError(null);
        void loadLatest();
        void loadMeta();
        void refreshAutoFile();
      } else {
        setRunning(false);
        setProgress(null);
        // Ein Stopp ist kein Fehler.
        if (payload.code !== "minutes_cancelled") {
          setError(errorText(payload.code));
        }
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, [meetingId, loadLatest, loadMeta, refreshAutoFile, errorText]);

  const generate = async () => {
    setRunning(true);
    setProgress(null);
    setError(null);
    setSaved(null);
    const result = await commands.meetingsGenerateMinutes(
      meetingId,
      templateChoice,
    );
    if (result.status === "error") {
      // Ein zweiter Start wird abgewiesen, der erste Lauf laeuft weiter: der
      // Reiter zeigt ihn weiter als laufend, kein Fehler.
      if (minutesErrorCode(result.error) === "minutes_busy") {
        void loadRunState();
        return;
      }
      setRunning(false);
      setProgress(null);
      if (minutesErrorCode(result.error) !== "minutes_cancelled") {
        setError(errorText(result.error));
      }
      return;
    }
    setRunning(false);
    setProgress(null);
    setDoc(result.data);
    void refreshAutoFile();
    void loadMeta();
  };

  /**
   * Two flavours in one clipboard write: `text/html` so a paste into Word,
   * Outlook or a mail client keeps headings and lists, and `text/plain` with
   * the Markdown source for editors that want it raw. The receiving
   * application picks whichever it understands.
   */
  const copyMinutes = async () => {
    if (!doc) return;
    setError(null);
    const html = previewRef.current?.innerHTML;
    try {
      if (html && typeof ClipboardItem !== "undefined") {
        await navigator.clipboard.write([
          new ClipboardItem({
            "text/html": new Blob([html], { type: "text/html" }),
            "text/plain": new Blob([doc.body], { type: "text/plain" }),
          }),
        ]);
      } else {
        // A webview without ClipboardItem still gets the Markdown.
        await navigator.clipboard.writeText(doc.body);
      }
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (e) {
      setError(t("meetings.minutes.copyError") + `: ${String(e)}`);
    }
  };

  const exportMinutes = async () => {
    if (!doc) return;
    setError(null);
    setSaved(null);
    const safeName =
      meetingTitle.replace(/[\\/:*?"<>|]/g, "_").trim() || "protokoll";
    // Word zuerst: das Protokoll geht weiter an Menschen, nicht an einen
    // Editor. Das Format ergibt sich aus der gewählten Endung — deshalb
    // braucht es kein zweites Auswahlfeld neben dem Dialog.
    const target = await save({
      filters: [
        { name: "Word", extensions: ["docx"] },
        { name: "Text", extensions: ["txt"] },
        { name: "Markdown", extensions: ["md"] },
      ],
      defaultPath: `${safeName}.docx`,
    });
    if (typeof target !== "string") return;
    // Geschrieben wird im Backend: das fs-Plugin lässt nur $APPDATA zu und
    // scheiterte an jedem Ziel, das der Dialog anbietet ("not allowed by ACL").
    const result = await commands.meetingsExportDocument(target, doc.body);
    if (result.status !== "ok") {
      setError(t("meetings.minutes.exportError") + `: ${result.error}`);
      return;
    }
    setSaved(target);
  };

  if (loading) {
    return (
      <p className="text-sm text-text/60 text-center py-3">
        {t("meetings.list.loading")}
      </p>
    );
  }

  return (
    <div className="space-y-3">
      {error && <Alert variant="error">{error}</Alert>}

      <MeetingTemplatePicker
        meetingId={meetingId}
        disabled={running}
        onValueChange={setTemplateChoice}
      />

      <div className="flex gap-2 items-center flex-wrap">
        <Button onClick={generate} disabled={running}>
          {doc
            ? t("meetings.detail.regenerate")
            : t("meetings.detail.generate")}
        </Button>
        {doc && (
          <>
            <Button variant="secondary" onClick={copyMinutes}>
              {copied
                ? t("meetings.detail.copied")
                : t("meetings.minutes.copy")}
            </Button>
            <Button
              variant="secondary"
              onClick={exportMinutes}
              title={t("meetings.detail.export")}
              aria-label={t("meetings.detail.export")}
            >
              <Download width={16} height={16} />
            </Button>
          </>
        )}
        {running && (
          <Badge variant="secondary">
            <span data-testid="minutes-running">
              {t("meetings.minutes.generating")}
            </span>
          </Badge>
        )}
        {saved && (
          <span className="text-xs text-text/60 break-all">
            {t("meetings.minutes.exportSaved", { path: saved })}
          </span>
        )}
      </div>

      {running && <MinutesProgressBar progress={progress} />}

      {!running && meta?.incomplete && (
        <Alert variant="warning">
          <span data-testid="minutes-incomplete">
            {t("meetings.minutes.incomplete", {
              gaps: meta.gaps.length > 0 ? meta.gaps.join(", ") : "?",
            })}
          </span>
        </Alert>
      )}

      {/* Written automatically on generation, so the minutes exist as a file
          even for someone who never opens the export dialog. */}
      {doc && autoFile && (
        <p className="text-xs text-text/60 break-all">
          {t("meetings.minutes.autoSaved")}{" "}
          <button
            type="button"
            onClick={() => void revealItemInDir(autoFile)}
            className="underline hover:text-logo-primary cursor-pointer"
          >
            {autoFile}
          </button>
        </p>
      )}

      {doc && meta?.template_title && (
        <p className="text-xs text-text/60" data-testid="minutes-created-with">
          {t(
            meta.auto?.outcome === "model"
              ? "meetings.minutes.createdWithAuto"
              : "meetings.minutes.createdWith",
            { title: meta.template_title },
          )}
        </p>
      )}

      {doc ? (
        <div
          ref={previewRef}
          className="rounded-lg border border-mid-gray/20 p-4"
        >
          <MarkdownContent markdown={doc.body} />
        </div>
      ) : (
        !running && (
          <p className="text-sm text-text/60">{t("meetings.minutes.empty")}</p>
        )
      )}
    </div>
  );
};

/**
 * Fortschrittsbalken der Protokoll-Erzeugung: Phase, Schritt und Prozent. Ohne
 * bekannte Gesamtzahl (Vorlagenwahl, erster Augenblick) laeuft er unbestimmt.
 */
const MinutesProgressBar: React.FC<{ progress: MinutesProgress | null }> = ({
  progress,
}) => {
  const { t } = useTranslation();
  const percent = progressPercent(progress);
  return (
    <div className="space-y-1" data-testid="minutes-progress">
      <div className="flex items-center justify-between text-xs text-text/60">
        <span data-testid="minutes-phase">
          {progress
            ? t(phaseKey(progress.phase))
            : t("meetings.minutes.generating")}
        </span>
        <span data-testid="minutes-percent">
          {percent !== null && progress
            ? `${t("meetings.minutes.stepOf", { done: progress.done, total: progress.total })} \u00b7 ${percent} %`
            : ""}
        </span>
      </div>
      <div
        role="progressbar"
        aria-label={t("meetings.minutes.progressLabel")}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        className="h-1.5 w-full overflow-hidden rounded-full bg-mid-gray/20"
      >
        <div
          className={`h-full rounded-full bg-logo-primary transition-[width] duration-300 ${
            percent === null ? "w-1/3 animate-pulse" : ""
          }`}
          style={percent === null ? undefined : { width: `${percent}%` }}
        />
      </div>
    </div>
  );
};
