import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands } from "@/bindings";

/** Buchbefehle bleiben in der vorhandenen Auftragszeile. */
export function TranscriptionControls({ bookId, state }: { bookId: string; state?: string }) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [keepPartial, setKeepPartial] = useState(true);
  const act = async (action: "pause" | "resume" | "cancel") => {
    setBusy(true);
    try {
      const result = action === "pause"
        ? await commands.libraryTranscriptionPause(bookId)
        : action === "resume"
          ? await commands.libraryTranscriptionResume(bookId)
          : await commands.libraryTranscriptionCancel(bookId, keepPartial);
      if (result.status === "error") toast.error(t("library.transcription.error"));
    } catch {
      toast.error(t("library.transcription.error"));
    } finally {
      setBusy(false);
    }
  };
  const stopped = state === "stopping";
  const resumable = state === "paused" || state === "pausing" || state === "cancelled";
  return (
    <div>
      <button type="button" disabled={busy || stopped} onClick={() => void act(resumable ? "resume" : "pause")}>
        {t(`library.transcription.${resumable ? "resume" : "pause"}`)}
      </button>
      <button type="button" disabled={busy || stopped || state === "cancelled"} onClick={() => void act("cancel")}>
        {t("library.transcription.cancel")}
      </button>
      <label>
        <input type="checkbox" checked={keepPartial} disabled={busy || stopped} onChange={(e) => setKeepPartial(e.target.checked)} />
        {t("library.transcription.keepPartial")}
      </label>
    </div>
  );
}
