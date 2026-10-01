import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
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
import { Alert } from "../../ui/Alert";
import Badge from "../../ui/Badge";
import { IconAction } from "../../ui/IconAction";
import {
  MarkdownContent,
  type SlideRefs,
} from "../../whats-new/MarkdownContent";
import { defaultDocBasis } from "./language/DocBasisFields";
import { Check, Copy, Download, RefreshCw, Sparkles } from "lucide-react";

interface MinutesViewProps {
  meetingId: string;
  meetingTitle: string;
  /** D5: Folienbelege (`[F7]`) im Protokoll anklickbar machen (Sprung zur Folie). */
  slideRefs?: SlideRefs;
}

export const MinutesView: React.FC<MinutesViewProps> = ({
  meetingId,
  meetingTitle,
  slideRefs,
}) => {
  const { t, i18n } = useTranslation();
  const [doc, setDoc] = useState<MeetingDocument | null>(null);
  const [loading, setLoading] = useState(true);
  // Der Lauf gehoert dem Backend (B14): `running` kommt aus dessen Zustand und
  // seinen Ereignissen, nicht aus dem Aufruf dieses Reiters. So bleibt der
  // Knopf gesperrt, wenn man den Reiter verlaesst und wieder oeffnet.
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<MinutesProgress | null>(null);
  const [meta, setMeta] = useState<MinutesMeta | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
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
    void loadMeta();
    void loadRunState();
  }, [loadLatest, loadMeta, loadRunState]);

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
  }, [meetingId, loadLatest, loadMeta, errorText]);

  const generate = async () => {
    setRunning(true);
    setProgress(null);
    setError(null);
    setSaved(null);
    // `null`: die Vorlage, die fuer diese Besprechung gewaehlt ist (Menue, Vorlage).
    // G5: aktive Fassung, Sprache = letzte Wahl bzw. die der App.
    const result = await commands.meetingsGenerateMinutes(
      meetingId,
      null,
      defaultDocBasis(i18n.language),
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
    // D5: die Folienmarken sind im Dokument Schaltflaechen; in der Zwischenablage stehen sie
    // wieder als Text (`[F7]`), wie im gespeicherten Protokoll.
    const preview = previewRef.current?.cloneNode(true) as
      HTMLElement | undefined;
    preview?.querySelectorAll("[data-slide-ref]").forEach((el) => {
      el.replaceWith(
        document.createTextNode(`[F${el.getAttribute("data-slide-ref")}]`),
      );
    });
    const html = preview?.innerHTML;
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
    <div className="space-y-2" data-testid="minutes-view">
      {/* Schmale Werkzeugzeile statt Vorlagenwahl und Textknoepfen: der Inhalt
          beginnt direkt unter den Reitern. Vorlage waehlen, Vorlagen verwalten und
          "Neu erzeugen mit Vorlage" stehen im Menue neben "Details". */}
      <div
        role="toolbar"
        aria-label={t("meetings.minutes.toolbar")}
        data-testid="minutes-toolbar"
        className="flex flex-wrap items-center gap-2"
      >
        <IconAction
          size="sm"
          icon={doc ? RefreshCw : Sparkles}
          label={
            doc
              ? t("meetings.detail.regenerate")
              : t("meetings.detail.generate")
          }
          description={t("meetings.minutes.regenerateHint")}
          testId="minutes-generate"
          disabled={running}
          onClick={generate}
        />
        {doc && (
          <>
            <IconAction
              size="sm"
              icon={copied ? Check : Copy}
              label={
                copied
                  ? t("meetings.detail.copied")
                  : t("meetings.minutes.copy")
              }
              description={t("meetings.minutes.copyHint")}
              testId="minutes-copy"
              onClick={copyMinutes}
            />
            <IconAction
              size="sm"
              icon={Download}
              label={t("meetings.minutes.download")}
              description={t("meetings.detail.export")}
              testId="minutes-export"
              onClick={exportMinutes}
            />
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
          <span className="min-w-0 break-all text-xs text-text/60">
            {t("meetings.minutes.exportSaved", { path: saved })}
          </span>
        )}
      </div>

      {error && <Alert variant="error">{error}</Alert>}

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

      {doc ? (
        <div
          ref={previewRef}
          data-testid="minutes-doc"
          className="rounded-lg border border-mid-gray/20 p-4"
        >
          <MarkdownContent markdown={doc.body} slideRefs={slideRefs} />
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
