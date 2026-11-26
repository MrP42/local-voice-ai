import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pause, Play, RotateCcw, Square, Trash2, X } from "lucide-react";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { Button } from "../ui/Button";
import type { TranscriptJob } from "./useLibrary";

type Action = "pause" | "resume" | "keep" | "discard";

/**
 * Was die Zeile im Block „In Arbeit“ anbietet (Patrick 26.11.2026: die
 * Knöpfe standen auch bei fehlgeschlagenen und fast fertigen Einträgen).
 *
 * - `none`: Transkript wird gerade übernommen, oder kein Steuerzustand.
 * - `failed`: nichts läuft mehr, mindestens ein Teil ist gescheitert.
 * - `control`: läuft, hält an, pausiert oder ist abgebrochen.
 */
export function controlMode(
  job: Pick<TranscriptJob, "status" | "controlState" | "failed" | "running">,
): "none" | "failed" | "control" {
  const state = job.controlState;
  if (job.status === "ready") return "none";
  if (state && state !== "running") return "control";
  if (
    job.status === "failed" ||
    job.status === "missing" ||
    (job.failed > 0 && job.running === 0)
  )
    return "failed";
  return state ? "control" : "none";
}

/**
 * Steuerung einer Hörbuch-Transkription im Block „In Arbeit“ (#806).
 *
 * Beschriftete Knöpfe unter dem Titel, je nach Zustand: laufend „Anhalten“
 * und „Abbrechen“, angehalten „Fortsetzen“, fehlgeschlagen „Erneut
 * versuchen“ und „Entfernen“, während der Übernahme nichts. Abbrechen und
 * Entfernen fragen inline nach; beim Abbrechen ist „Teil behalten“ der
 * Standard.
 */
export function TranscriptionControls({
  job,
  onRetry,
}: {
  job: TranscriptJob;
  onRetry?: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const bookId = job.bookId;
  const state = job.controlState;
  const mode = controlMode(job);
  const [busy, setBusy] = useState(false);
  const [asking, setAsking] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const keepRef = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);

  const act = async (action: Action) => {
    setBusy(true);
    try {
      const result =
        action === "pause"
          ? await commands.libraryTranscriptionPause(bookId)
          : action === "resume"
            ? await commands.libraryTranscriptionResume(bookId)
            : await commands.libraryTranscriptionCancel(
                bookId,
                action === "keep",
              );
      if (result.status === "error")
        toast.error(
          t(
            result.error === "audiobook_adopting"
              ? "library.transcriptionControls.adopting"
              : "library.transcriptionControls.error",
          ),
        );
    } catch {
      toast.error(t("library.transcriptionControls.error"));
    } finally {
      setBusy(false);
      setAsking(false);
    }
  };

  const retry = async () => {
    if (!onRetry) return;
    setBusy(true);
    try {
      await onRetry();
    } catch {
      toast.error(t("library.transcriptionControls.error"));
    } finally {
      setBusy(false);
    }
  };

  const stopping = state === "stopping";
  const resumable =
    state === "paused" || state === "pausing" || state === "cancelled";
  const locked = busy || stopping;
  const cancellable = !locked && state !== "cancelled";

  // Ändert sich der Zustand (z. B. schon abgebrochen oder in Übernahme),
  // ist die offene Rückfrage hinfällig.
  const askKind = mode === "failed" || state === "cancelled" ? "remove" : mode;
  const askedFor = useRef(askKind);
  useEffect(() => {
    if (!asking) askedFor.current = askKind;
    else if (askedFor.current !== askKind || stopping) setAsking(false);
  }, [asking, askKind, stopping]);

  // Fokus: beim Öffnen auf die Standardwahl, beim Schließen zurück auf den Knopf.
  useEffect(() => {
    if (asking) keepRef.current?.focus();
    else if (returnFocus.current) {
      returnFocus.current = false;
      cancelRef.current?.focus();
    }
  }, [asking]);

  const closeAsk = () => {
    returnFocus.current = true;
    setAsking(false);
  };

  if (mode === "none") return null;
  /** Nichts läuft mehr (gescheitert oder mit Teil abgebrochen): Entfernen. */
  const removing = mode === "failed" || state === "cancelled";

  if (asking) {
    const question = t(
      removing
        ? "library.transcriptionControls.removeAsk"
        : "library.transcriptionControls.cancelAsk",
    );
    return (
      <div
        className="library-job__controls library-job__controls--ask"
        role="group"
        aria-label={question}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            closeAsk();
          }
        }}
      >
        <span className="library-job__ask">{question}</span>
        {removing ? (
          <Button
            ref={keepRef}
            type="button"
            size="sm"
            variant="danger-ghost"
            disabled={busy || stopping}
            onClick={() => void act("discard")}
            data-testid="transcription-remove-confirm"
          >
            {t("library.transcriptionControls.remove")}
          </Button>
        ) : (
          <>
            <Button
              ref={keepRef}
              type="button"
              size="sm"
              disabled={!cancellable}
              onClick={() => void act("keep")}
              data-testid="transcription-cancel-keep"
            >
              {t("library.transcriptionControls.cancelKeep")}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="danger-ghost"
              disabled={!cancellable}
              onClick={() => void act("discard")}
              data-testid="transcription-cancel-discard"
            >
              {t("library.transcriptionControls.cancelDiscard")}
            </Button>
          </>
        )}
        <Button
          type="button"
          size="sm"
          variant="ghost"
          icon={<X size={14} aria-hidden="true" />}
          aria-label={t("library.transcriptionControls.back")}
          title={t("library.transcriptionControls.back")}
          onClick={closeAsk}
        />
      </div>
    );
  }

  if (mode === "failed")
    return (
      <div className="library-job__controls">
        {onRetry && (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            icon={<RotateCcw size={14} aria-hidden="true" />}
            title={t("library.transcriptionControls.retryHint")}
            disabled={busy}
            onClick={() => void retry()}
            data-testid="transcription-retry"
          >
            {t("library.transcriptionControls.retry")}
          </Button>
        )}
        <Button
          ref={cancelRef}
          type="button"
          size="sm"
          variant="ghost"
          icon={<Trash2 size={14} aria-hidden="true" />}
          title={t("library.transcriptionControls.removeHint")}
          disabled={busy}
          onClick={() => setAsking(true)}
          data-testid="transcription-remove"
        >
          {t("library.transcriptionControls.remove")}
        </Button>
      </div>
    );

  return (
    <div className="library-job__controls">
      <Button
        type="button"
        size="sm"
        variant="ghost"
        icon={
          resumable ? (
            <Play size={14} aria-hidden="true" />
          ) : (
            <Pause size={14} aria-hidden="true" />
          )
        }
        title={t(
          `library.transcriptionControls.${resumable ? "resumeHint" : "pauseHint"}`,
        )}
        disabled={locked}
        onClick={() => void act(resumable ? "resume" : "pause")}
        data-testid="transcription-toggle"
      >
        {t(`library.transcriptionControls.${resumable ? "resume" : "pause"}`)}
      </Button>
      {removing ? (
        <Button
          ref={cancelRef}
          type="button"
          size="sm"
          variant="ghost"
          icon={<Trash2 size={14} aria-hidden="true" />}
          title={t("library.transcriptionControls.removeHint")}
          disabled={busy}
          onClick={() => setAsking(true)}
          data-testid="transcription-remove"
        >
          {t("library.transcriptionControls.remove")}
        </Button>
      ) : (
        <Button
          ref={cancelRef}
          type="button"
          size="sm"
          variant="ghost"
          icon={<Square size={13} aria-hidden="true" />}
          title={t("library.transcriptionControls.cancelHint")}
          disabled={!cancellable}
          onClick={() => setAsking(true)}
          data-testid="transcription-cancel"
        >
          {t("library.transcriptionControls.cancel")}
        </Button>
      )}
    </div>
  );
}
