import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pause, Play, Square, X } from "lucide-react";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { Button } from "../ui/Button";
import type { TranscriptJob } from "./useLibrary";

type Action = "pause" | "resume" | "keep" | "discard";

/** Fehlercodes ohne technische Details oder Inhalte in der Oberflaeche. */
export function TranscriptionFailure({ job, onRetry, onRemove }: {
  job: TranscriptJob;
  onRetry(): Promise<void>;
  onRemove?(): Promise<void>;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [asking, setAsking] = useState(false);
  const removeRef = useRef<HTMLButtonElement>(null);
  const backRef = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  useEffect(() => {
    if (asking) backRef.current?.focus();
    else if (returnFocus.current) { returnFocus.current = false; removeRef.current?.focus(); }
  }, [asking]);
  const close = () => { returnFocus.current = true; setAsking(false); };
  const act = async (action: () => Promise<void>) => {
    setBusy(true);
    try { await action(); setAsking(false); }
    catch { toast.error(t("library.transcriptionControls.error")); }
    finally { setBusy(false); }
  };
  const causes: Record<string, string> = {
    delta_store_failed: "storeFailed",
    import_store_busy: "storeBusy",
    store_failed: "storeFailed",
    audio_paths_failed: "storeFailed",
    audiobook_block_failed: "blockFailed",
    chunk_transcription_failed: "blockFailed",
    source_missing: "sourceMissing",
    source_not_found: "sourceMissing",
    import_source_missing: "sourceMissing",
    audiobook_model_not_installed: "modelMissing",
    audiobook_no_installed_model: "modelMissing",
    import_failed: "importFailed",
    job_missing: "jobMissing",
  };
  const keys = [...new Set((job.errors?.length ? job.errors : [job.status === "missing" ? "job_missing" : "unknown"]).map((code) =>
    Object.hasOwn(causes, code) ? causes[code] : "causeUnknown"))];
  return <div onKeyDown={(e) => {
    if (asking && e.key === "Escape") { e.stopPropagation(); close(); }
  }}>
    <div className="library-meta" data-testid="transcription-cause">
      {keys.map((key) => t(`library.transcriptionControls.${key}`)).join(" · ")}
    </div>
    {asking ? <div role="group" aria-label={t("library.transcriptionControls.removeAsk")}>
      <span>{t("library.transcriptionControls.removeAsk")}</span>
      <Button ref={backRef} size="sm" variant="ghost" onClick={close}>
        {t("library.transcriptionControls.back")}
      </Button>
      <Button size="sm" variant="danger-ghost" disabled={busy} onClick={() => onRemove && void act(onRemove)}>
        {t("library.transcriptionControls.removeConfirm")}
      </Button>
    </div> : <div className="library-job__controls">
      <Button size="sm" disabled={busy} onClick={() => void act(onRetry)}>
        {t("library.transcriptionControls.retry")}
      </Button>
      {onRemove && <Button ref={removeRef} size="sm" variant="danger-ghost" disabled={busy}
        onClick={() => job.hasPartial === false ? void act(onRemove) : setAsking(true)}>
        {t("library.transcriptionControls.remove")}
      </Button>}
    </div>}
  </div>;
}

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
        aria-label={t("library.transcriptionControls.cancelAsk")}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            closeAsk();
          }
        }}
      >
        <span className="library-job__ask">
          {t("library.transcriptionControls.cancelAsk")}
        </span>
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

  const toggleLabel = t(
    `library.transcriptionControls.${resumable ? "resume" : "pause"}`,
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
        aria-label={t("library.transcriptionControls.cancel")}
        title={t("library.transcriptionControls.cancel")}
        disabled={!cancellable}
        onClick={() => setAsking(true)}
        data-testid="transcription-cancel"
      />
    </div>
  );
}
