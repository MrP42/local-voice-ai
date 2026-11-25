import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pause, Play, Square, X } from "lucide-react";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { Button } from "../ui/Button";

type Action = "pause" | "resume" | "keep" | "discard";

/**
 * Pause, Fortsetzen und Abbrechen einer Hörbuch-Transkription (#806).
 *
 * Kompakte Symbolknöpfe rechts in der Auftragszeile. Abbrechen fragt inline
 * nach, ob das Teilergebnis zum späteren Fortsetzen bleiben soll; Behalten ist
 * der Standard, Verwerfen ist die zerstörende Wahl.
 */
export function TranscriptionControls({
  bookId,
  state,
}: {
  bookId: string;
  state?: string;
}) {
  const { t } = useTranslation();
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
              ? "library.transcription.adopting"
              : "library.transcription.error",
          ),
        );
    } catch {
      toast.error(t("library.transcription.error"));
    } finally {
      setBusy(false);
      setAsking(false);
    }
  };

  const stopping = state === "stopping";
  const resumable =
    state === "paused" || state === "pausing" || state === "cancelled";
  const locked = busy || stopping;
  const cancellable = !locked && state !== "cancelled";

  // Ändert sich der Status (z. B. schon abgebrochen), ist die Rückfrage hinfällig.
  useEffect(() => {
    if (asking && (stopping || state === "cancelled")) setAsking(false);
  }, [asking, stopping, state]);

  // Fokus: beim Öffnen auf „Teil behalten“, beim Schließen zurück auf Abbrechen.
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

  if (!state || !["running", "pausing", "paused", "stopping", "cancelled"].includes(state)) return null;

  if (asking)
    return (
      <div
        className="library-job__controls library-job__controls--ask"
        role="group"
        aria-label={t("library.transcription.cancelAsk")}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            closeAsk();
          }
        }}
      >
        <span className="library-job__ask">
          {t("library.transcription.cancelAsk")}
        </span>
        <Button
          ref={keepRef}
          type="button"
          size="sm"
          disabled={!cancellable}
          onClick={() => void act("keep")}
          data-testid="transcription-cancel-keep"
        >
          {t("library.transcription.cancelKeep")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="danger-ghost"
          disabled={!cancellable}
          onClick={() => void act("discard")}
          data-testid="transcription-cancel-discard"
        >
          {t("library.transcription.cancelDiscard")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          icon={<X size={14} aria-hidden="true" />}
          aria-label={t("library.transcription.back")}
          title={t("library.transcription.back")}
          onClick={closeAsk}
        />
      </div>
    );

  const toggleLabel = t(
    `library.transcription.${resumable ? "resume" : "pause"}`,
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
        aria-label={toggleLabel}
        title={toggleLabel}
        disabled={locked}
        onClick={() => void act(resumable ? "resume" : "pause")}
        data-testid="transcription-toggle"
      />
      <Button
        ref={cancelRef}
        type="button"
        size="sm"
        variant="ghost"
        icon={<Square size={13} aria-hidden="true" />}
        aria-label={t("library.transcription.cancel")}
        title={t("library.transcription.cancel")}
        disabled={!cancellable}
        onClick={() => setAsking(true)}
        data-testid="transcription-cancel"
      />
    </div>
  );
}
