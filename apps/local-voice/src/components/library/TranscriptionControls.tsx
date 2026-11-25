import { useState } from "react";
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

  if (asking)
    return (
      <div
        className="library-job__controls library-job__controls--ask"
        role="group"
        aria-label={t("library.transcription.cancelAsk")}
      >
        <span className="library-job__ask">
          {t("library.transcription.cancelAsk")}
        </span>
        <Button
          type="button"
          size="sm"
          disabled={locked}
          onClick={() => void act("keep")}
          data-testid="transcription-cancel-keep"
        >
          {t("library.transcription.cancelKeep")}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="danger-ghost"
          disabled={locked}
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
          onClick={() => setAsking(false)}
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
        type="button"
        size="sm"
        variant="ghost"
        icon={<Square size={13} aria-hidden="true" />}
        aria-label={t("library.transcription.cancel")}
        title={t("library.transcription.cancel")}
        disabled={locked || state === "cancelled"}
        onClick={() => setAsking(true)}
        data-testid="transcription-cancel"
      />
    </div>
  );
}
